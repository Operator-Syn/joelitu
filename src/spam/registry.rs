//! The in-memory record of runs that are currently in flight.
//!
//! This is the source of truth for two things the type system cannot enforce by
//! itself: how many runs a channel may hold at once, and that every reserved
//! entry is released on every exit path — including a panic inside a spawned task.
//!
//! There is no persistence. A run lives at most ~12 seconds, so losing the
//! registry on restart costs the unsent messages and nothing else.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio_util::sync::CancellationToken;

/// Default ceiling on concurrent runs within a single channel.
///
/// One. The earlier value of two made the pacing floor untrue: two runs each
/// pacing at 1.2 s put 2 messages per 1.2 s into one channel, which is about 8.3
/// per five seconds against Discord's allowance of roughly 5 — so a mixture of
/// successful sends and 429 retries, not the steady pace the plan promises. At the
/// current `MAX_COUNT` of 99 that overlap lasts two minutes instead of seconds.
///
/// Serialising per channel is what keeps "the interval is the interval" a fact.
/// A second `/spam` in the same channel is refused rather than made unreliable.
pub const DEFAULT_MAX_RUNS_PER_CHANNEL: usize = 1;

/// One active run, as the registry sees it.
struct RunHandle {
    /// Channel the run posts into. The scope `/cancel` operates on.
    channel_id: u64,
    /// Signal the owning task watches.
    token: CancellationToken,
}

/// Shared state behind the registry. Held only for the duration of a map
/// operation; nothing is awaited while the lock is held.
struct RegistryState {
    /// Active runs, keyed by run id.
    runs: HashMap<u64, RunHandle>,
    /// Next id to hand out. Monotonic for the process lifetime, so an id is never
    /// reused even after its run is gone.
    next_id: u64,
}

/// Tracks active runs and hands out run ids.
///
/// Cheap to share by `Arc`; the struct itself is not `Clone`. All methods are
/// synchronous and lock a `std::sync::Mutex` for a single map operation, so they
/// are safe to call from an async handler without holding the lock across an
/// await (which would be a deadlock hazard).
pub struct RunRegistry {
    /// Active runs and the id counter, behind one lock.
    state: Mutex<RegistryState>,
    /// Ceiling on concurrent runs within a single channel.
    max_per_channel: usize,
}

impl RunRegistry {
    /// Create an empty registry with the given per-channel ceiling.
    ///
    /// # Arguments
    ///
    /// * `max_per_channel` - Maximum concurrent runs in one channel. Values of
    ///   zero are treated as [`DEFAULT_MAX_RUNS_PER_CHANNEL`] rather than
    ///   rejecting every run, since a zero-ceiling registry can never do anything.
    pub fn new(max_per_channel: usize) -> Self {
        let max_per_channel = if max_per_channel == 0 {
            DEFAULT_MAX_RUNS_PER_CHANNEL
        } else {
            max_per_channel
        };

        Self {
            state: Mutex::new(RegistryState {
                runs: HashMap::new(),
                next_id: 0,
            }),
            max_per_channel,
        }
    }

    /// Reserve a slot for a new run and return its id and cancellation token.
    ///
    /// # Returns
    ///
    /// `(id, token)`. The id is unique for the process lifetime and monotonic. The
    /// token is cancelled automatically if [`RunRegistry::cancel_channel`] matches
    /// the run's channel.
    ///
    /// # Errors
    ///
    /// `None` when the channel already holds `max_per_channel` runs. The caller
    /// decides how to report it; nothing is reserved in that case.
    ///
    /// # Constraints
    ///
    /// The entry stays in the map until released. Callers must hold a
    /// [`RunGuard`] so release happens even on panic; dropping the token without
    /// releasing would leak a slot for the process lifetime.
    pub fn reserve(&self, channel_id: u64) -> Option<(u64, CancellationToken)> {
        let mut state = self.state.lock().expect("registry mutex is never poisoned");

        // Count first: the ceiling is per channel, not global, so the check is a
        // scan over a map that holds at most a handful of entries.
        let active_in_channel = state
            .runs
            .values()
            .filter(|run| run.channel_id == channel_id)
            .count();

        if active_in_channel >= self.max_per_channel {
            return None;
        }

        let id = state.next_id;
        state.next_id += 1;

        let token = CancellationToken::new();
        state.runs.insert(
            id,
            RunHandle {
                channel_id,
                token: token.clone(),
            },
        );

        Some((id, token))
    }

    /// Cancel every run in a channel and return how many were signalled.
    ///
    /// # Returns
    ///
    /// The number of runs whose tokens were cancelled. They are not removed here;
    /// each owning task removes its own entry when it observes the cancellation,
    /// so the count reflects entries present at call time.
    pub fn cancel_channel(&self, channel_id: u64) -> usize {
        let state = self.state.lock().expect("registry mutex is never poisoned");

        let mut cancelled = 0;
        for run in state
            .runs
            .values()
            .filter(|run| run.channel_id == channel_id)
        {
            run.token.cancel();
            cancelled += 1;
        }

        cancelled
    }

    /// Cancel every run in the process and return how many were signalled.
    ///
    /// # Returns
    ///
    /// The number of runs whose tokens were cancelled at call time. Used on
    /// shutdown, where the difference between per-channel and global cancellation
    /// stops mattering.
    pub fn cancel_all(&self) -> usize {
        let state = self.state.lock().expect("registry mutex is never poisoned");

        for run in state.runs.values() {
            run.token.cancel();
        }

        state.runs.len()
    }

    /// How many runs the channel currently holds.
    pub fn active_in_channel(&self, channel_id: u64) -> usize {
        let state = self.state.lock().expect("registry mutex is never poisoned");

        state
            .runs
            .values()
            .filter(|run| run.channel_id == channel_id)
            .count()
    }

    /// How many runs the process currently holds across every channel.
    ///
    /// Used by tests and by anyone reasoning about total load; the shutdown path
    /// reports the cancellation count instead.
    #[cfg(test)]
    pub fn active_total(&self) -> usize {
        let state = self.state.lock().expect("registry mutex is never poisoned");
        state.runs.len()
    }

    /// Remove a run's entry. Called by [`RunGuard`]'s destructor.
    ///
    /// Idempotent: releasing an id that is already gone is not an error, so a
    /// guard that outlives a manual release cannot panic.
    fn release(&self, id: u64) {
        let mut state = self.state.lock().expect("registry mutex is never poisoned");
        state.runs.remove(&id);
    }
}

/// Releases a registry slot when dropped, on every exit path.
///
/// This type exists because the invariant "every reservation is released" cannot
/// be expressed in the type system: a spawned task can return normally, return
/// early, or unwind. A destructor covers all three, which a chain of explicit
/// releases does not.
pub struct RunGuard {
    /// Held by `Arc` rather than a borrow so the guard can move into
    /// `tokio::spawn`, which requires a `'static` future.
    registry: Arc<RunRegistry>,
    /// Id of the run this guard is responsible for.
    id: u64,
}

impl RunGuard {
    /// Take ownership of releasing `id` when the guard drops.
    pub fn new(registry: Arc<RunRegistry>, id: u64) -> Self {
        Self { registry, id }
    }
}

impl Drop for RunGuard {
    /// Removes the registry entry. Runs during unwinding too, which is the case
    /// that matters — a panicking task would otherwise hold its slot forever.
    fn drop(&mut self) {
        self.registry.release(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry with a small ceiling, so capacity tests stay readable.
    fn registry() -> Arc<RunRegistry> {
        Arc::new(RunRegistry::new(2))
    }

    #[test]
    fn reserve_tracks_the_run_until_dropped() {
        let registry = registry();

        let (id, _token) = registry.reserve(10).expect("first run fits");
        assert_eq!(registry.active_in_channel(10), 1);
        assert_eq!(registry.active_total(), 1);

        drop(RunGuard::new(registry.clone(), id));

        assert_eq!(registry.active_in_channel(10), 0);
        assert_eq!(registry.active_total(), 0);
    }

    #[test]
    fn channel_ceiling_is_enforced_and_per_channel() {
        let registry = registry();

        let _first = registry.reserve(10).expect("first run fits");
        let _second = registry.reserve(10).expect("second run fits");
        assert!(registry.reserve(10).is_none(), "third must be refused");

        assert!(
            registry.reserve(11).is_some(),
            "a different channel has its own ceiling"
        );
    }

    #[test]
    fn cancel_channel_signals_only_that_channel() {
        let registry = registry();

        let (_id_a, token_a) = registry.reserve(10).expect("run a fits");
        let (_id_b, token_b) = registry.reserve(11).expect("run b fits");

        assert_eq!(registry.cancel_channel(10), 1);
        assert!(token_a.is_cancelled());
        assert!(!token_b.is_cancelled());
    }

    #[test]
    fn guard_releases_during_unwind() {
        let registry = registry();
        let (id, _token) = registry.reserve(10).expect("run fits");

        let unwind = std::panic::catch_unwind({
            let registry = registry.clone();
            move || {
                let _guard = RunGuard::new(registry, id);
                panic!("simulated task panic");
            }
        });

        assert!(unwind.is_err(), "the closure was expected to panic");
        assert_eq!(
            registry.active_total(),
            0,
            "a panicking run must not leak its slot"
        );
    }
}
