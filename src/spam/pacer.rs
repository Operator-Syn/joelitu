//! Shared send budgets: one per channel, one for the whole process.
//!
//! Two independent limits decide whether a message may go out right now:
//!
//! * **Per channel** — five tokens with a refill of one per second, matching
//!   Discord's roughly-five-per-five-seconds allowance with a burst this bot can
//!   actually use.
//! * **Per process** — fifty tokens with a refill of fifty per second, matching the
//!   global bucket Discord applies to every bot. Without this, enough channels
//!   sending at once reproduce the same 429 storm one level up.
//!
//! A caller awaits a permit instead of being refused, so concurrency costs latency
//! rather than errors. Because the budget lives here and not in each run, several
//! runs may share a channel: they queue for the same tokens and the channel's total
//! output stays inside the allowance no matter how many are active.

use std::{collections::HashMap, sync::Mutex};

use tokio::time::{Duration, Instant};

/// Tokens a channel may accumulate while idle. This is Discord's burst allowance:
/// five messages may arrive together, then the channel is throttled to the refill.
const CHANNEL_CAPACITY: f64 = 5.0;

/// Channel refill rate in tokens per second, i.e. five messages per five seconds.
const CHANNEL_RATE: f64 = 1.0;

/// Tokens the process may accumulate while idle.
const GLOBAL_CAPACITY: f64 = 50.0;

/// Process-wide refill rate in tokens per second.
const GLOBAL_RATE: f64 = 50.0;

/// A channel bucket whose tokens were exhausted long ago holds nothing worth
/// keeping. Pruning keeps the map proportional to channels active in the last hour
/// rather than to every channel the bot has ever posted in.
const CHANNEL_IDLE_PRUNE: Duration = Duration::from_secs(3600);

/// A token bucket: fractional tokens, refilled by elapsed time.
///
/// Fractional rather than integer because a refill of one token per second against
/// a request that wants two per second has to average out to one, not to zero.
struct Bucket {
    /// Tokens available right now.
    tokens: f64,
    /// When the token count was last brought forward. Absolute, so a bucket that
    /// nobody touches for an hour refills correctly on the next use.
    updated: Instant,
}

impl Bucket {
    /// A full bucket, ready to burst up to `capacity`.
    fn full(capacity: f64) -> Self {
        Self {
            tokens: capacity,
            updated: Instant::now(),
        }
    }

    /// Bring the token count forward to now, capped at `capacity`.
    fn refill(&mut self, capacity: f64, rate: f64) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.updated).as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate).min(capacity);
        self.updated = now;
    }

    /// Try to spend one token.
    ///
    /// # Returns
    ///
    /// `None` when a token was spent. `Some(wait)` when the bucket was empty, where
    /// `wait` is how long until one token exists — the caller sleeps and retries.
    fn try_take(&mut self, capacity: f64, rate: f64) -> Option<Duration> {
        self.refill(capacity, rate);

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            None
        } else {
            Some(Duration::from_secs_f64((1.0 - self.tokens) / rate))
        }
    }

    /// Give back a token taken optimistically.
    fn refund(&mut self, capacity: f64) {
        self.tokens = (self.tokens + 1.0).min(capacity);
    }
}

/// Both buckets, behind one lock.
struct State {
    /// Process-wide allowance.
    global: Bucket,
    /// Per-channel allowances, keyed by channel id.
    channels: HashMap<u64, Bucket>,
}

/// Hands out permission to send, sleeping until it is available.
///
/// Shared by every run through an `Arc`. All state is behind a `std::sync::Mutex`
/// that is never held across an await: the lock is taken, a decision is made, and it
/// is released before any sleep, so a slow sender cannot block the others.
pub struct Pacer {
    /// Buckets and their state.
    state: Mutex<State>,
}

impl Pacer {
    /// Create a pacer with both buckets full, so a fresh process may burst.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                global: Bucket::full(GLOBAL_CAPACITY),
                channels: HashMap::new(),
            }),
        }
    }

    /// Wait until this channel may send one message, then return.
    ///
    /// # Arguments
    ///
    /// * `channel_id` - Channel the message will go to. Its bucket is created full on
    ///   first use, so the first run in a quiet channel can burst.
    ///
    /// # Side effects
    ///
    /// Spends one token from the channel bucket and one from the global bucket, and
    /// sleeps for as long as either is empty. Cancellation is the caller's job: this
    /// future should be raced against the run's cancellation token, not awaited
    /// blindly.
    ///
    /// # Processing
    ///
    /// 1. Take a channel token; if the channel is empty, wait out the channel's
    ///    refill and retry.
    /// 2. Take a global token; if the process bucket is empty, give the channel
    ///    token back and wait out the global refill, so a global wait does not
    ///    silently consume a channel's allowance.
    /// 3. Prune channels idle for over an hour.
    pub async fn acquire(&self, channel_id: u64) {
        loop {
            let wait = {
                let mut state = self.state.lock().expect("pacer mutex is never poisoned");

                // 1. Channel first: it is the tighter limit, so its wait is the one
                //    that usually matters.
                let channel_empty = {
                    let channel = state
                        .channels
                        .entry(channel_id)
                        .or_insert_with(|| Bucket::full(CHANNEL_CAPACITY));
                    channel.try_take(CHANNEL_CAPACITY, CHANNEL_RATE)
                };

                match channel_empty {
                    Some(wait) => Some(wait),
                    // 2. Then the process bucket. A refusal here refunds the channel
                    //    token, otherwise a global wait would burn channel budget
                    //    without sending anything.
                    None => match state.global.try_take(GLOBAL_CAPACITY, GLOBAL_RATE) {
                        None => None,
                        Some(wait) => {
                            if let Some(channel) = state.channels.get_mut(&channel_id) {
                                channel.refund(CHANNEL_CAPACITY);
                            }
                            Some(wait)
                        }
                    },
                }
            };

            match wait {
                None => {
                    // 3. Cheap sweep on the success path: the map is only touched
                    //    when it might have grown.
                    self.prune_idle();
                    return;
                }
                Some(wait) => tokio::time::sleep(wait).await,
            }
        }
    }

    /// Drop channel buckets that have been idle past [`CHANNEL_IDLE_PRUNE`].
    ///
    /// A pruned bucket comes back full, which is correct: an hour of silence is far
    /// longer than the five-second window the allowance is measured over.
    fn prune_idle(&self) {
        let mut state = self.state.lock().expect("pacer mutex is never poisoned");
        let now = Instant::now();

        state
            .channels
            .retain(|_, bucket| now.duration_since(bucket.updated) < CHANNEL_IDLE_PRUNE);
    }
}

impl Default for Pacer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Advance past a full refill so a burst allowance cannot mask a test's timing.
    async fn drain_wait() {
        tokio::time::advance(Duration::from_secs(6)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_quiet_channel_allows_its_burst_immediately() {
        let pacer = Pacer::new();
        let start = Instant::now();

        for _ in 0..5 {
            pacer.acquire(1).await;
        }

        assert_eq!(
            start.elapsed(),
            Duration::ZERO,
            "five sends inside the burst allowance must not wait"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_sixth_send_waits_for_the_refill() {
        let pacer = Pacer::new();
        for _ in 0..5 {
            pacer.acquire(1).await;
        }

        let start = Instant::now();
        pacer.acquire(1).await;

        // One token per second, so the sixth message waits about a second.
        assert!(
            start.elapsed() >= Duration::from_millis(900),
            "expected roughly a one-second wait, got {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn channels_do_not_share_their_burst() {
        let pacer = Pacer::new();

        for _ in 0..5 {
            pacer.acquire(1).await;
        }

        let start = Instant::now();
        pacer.acquire(2).await;

        assert_eq!(
            start.elapsed(),
            Duration::ZERO,
            "a different channel has its own five-token burst"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sustained_sending_settles_at_the_channel_rate() {
        let pacer = Pacer::new();
        let start = Instant::now();

        // Twenty messages: five burst tokens plus fifteen at one per second.
        for _ in 0..20 {
            pacer.acquire(1).await;
        }

        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_secs(14) && elapsed <= Duration::from_secs(17),
            "twenty messages should take about fifteen seconds, took {elapsed:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_process_wide_bucket_throttles_many_channels() {
        let pacer = Pacer::new();
        drain_wait().await;

        // One send each across sixty channels: the per-channel buckets are all
        // full, so only the fifty-token global bucket can stop this.
        let start = Instant::now();
        for channel in 0..60u64 {
            pacer.acquire(channel).await;
        }

        assert!(
            start.elapsed() > Duration::ZERO,
            "the global bucket must have throttled past fifty concurrent sends"
        );
    }

    #[test]
    fn a_pruned_bucket_comes_back_full() {
        let pacer = Pacer::new();
        {
            let mut state = pacer.state.lock().expect("lock");
            let bucket = state.channels.entry(7).or_insert_with(|| Bucket::full(5.0));
            bucket.tokens = 0.0;
            bucket.updated = Instant::now() - Duration::from_secs(7200);
        }

        pacer.prune_idle();

        let state = pacer.state.lock().expect("lock");
        assert!(
            !state.channels.contains_key(&7),
            "a bucket idle for two hours must be dropped"
        );
    }
}
