//! The pacing loop: send `plan.count()` messages, `plan.interval()` apart, until
//! done, cancelled, or a send fails.
//!
//! This module owns timing and cancellation only. It holds no registry, opens no
//! network connection, and knows nothing about Discord beyond the error type its
//! caller supplies. The send itself arrives as an injected closure, which is what
//! makes every branch here testable with `tokio::time::pause` and a recording
//! fake instead of a live guild.

use std::future::Future;

use tokio_util::sync::CancellationToken;

use crate::{error::SendError, spam::plan::Plan};

/// How a run ended. The caller logs this; nothing retries from here.
#[derive(Debug)]
pub enum Outcome {
    /// Every message was accepted by Discord.
    Completed {
        /// Messages actually accepted. Equal to the plan's count on this path.
        sent: u8,
    },
    /// The run was stopped before its final message.
    Cancelled {
        /// Messages accepted before the cancellation took effect.
        sent: u8,
    },
    /// A send failed, so the run stopped early.
    Failed {
        /// Messages accepted before the failure.
        sent: u8,
        /// The failure itself, handed to the caller for logging or reporting.
        error: SendError,
    },
}

/// Drive a plan to completion, pacing each message and honouring cancellation.
///
/// # Arguments
///
/// * `plan` - The validated run. Read only; the loop never mutates it.
/// * `token` - Cancellation signal. Checked before each send and raced against
///   each sleep, so a cancel during a pause stops the run without waiting out the
///   remaining interval.
/// * `send` - Closure that performs one send, given the zero-based index. Called
///   exactly once per attempt, in ascending index order, never concurrently.
///   Returning `Err` ends the run immediately; nothing is retried here because a
///   failure is already recorded by the caller.
///
/// # Returns
///
/// An [`Outcome`] describing how far the run got. Never returns an error: every
/// failure mode is folded into a variant, so the caller has one place to decide
/// what to log.
///
/// # Processing
///
/// 1. Before each message, check the token; a cancelled run stops at the count
///    already sent.
/// 2. Send the message for this index and count it on success.
/// 3. After all but the last message, race the interval sleep against
///    cancellation.
///
/// # Constraints
///
/// Pacing uses `sleep`, not `tokio::time::interval`: `interval` fires its first
/// tick immediately, which would send messages one and two back to back on every
/// run.
pub async fn drive<F, Fut>(plan: &Plan, token: &CancellationToken, mut send: F) -> Outcome
where
    F: FnMut(u8) -> Fut,
    Fut: Future<Output = Result<(), SendError>>,
{
    let mut sent: u8 = 0;

    for index in 0..plan.count() {
        // 1. A cancel that arrived during the previous sleep, or before the first
        //    send, stops here rather than after one more message.
        if token.is_cancelled() {
            return Outcome::Cancelled { sent };
        }

        // 2. One send, awaited to completion. A rejection ends the run: retrying
        //    would extend a burst the caller already lost control of.
        match send(index).await {
            Ok(()) => sent += 1,
            Err(error) => return Outcome::Failed { sent, error },
        }

        // 3. Pace between messages only. Sleeping after the last one would keep
        //    the run alive for an interval nobody is waiting on.
        if index + 1 < plan.count() {
            tokio::select! {
                () = token.cancelled() => return Outcome::Cancelled { sent },
                () = tokio::time::sleep(plan.interval()) => {}
            }
        }
    }

    Outcome::Completed { sent }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::spam::plan::{Plan, RawRequest};

    /// Build a plan with the given count and interval, so tests control both.
    fn plan(count: u8, interval_seconds: f64) -> Plan {
        Plan::validate(RawRequest {
            message: "hi".to_owned(),
            mention: None,
            count: Some(count),
            interval_seconds: Some(interval_seconds),
        })
        .expect("test plans must validate")
    }

    #[tokio::test(start_paused = true)]
    async fn sends_every_message_and_reports_the_count() {
        let plan = plan(3, 1.2);
        let token = CancellationToken::new();
        let mut indices = Vec::new();

        let outcome = drive(&plan, &token, |index| {
            indices.push(index);
            async { Ok(()) }
        })
        .await;

        assert!(matches!(outcome, Outcome::Completed { sent: 3 }));
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_before_the_first_send_sends_nothing() {
        let plan = plan(3, 1.2);
        let token = CancellationToken::new();
        token.cancel();

        let outcome = drive(&plan, &token, |_| async { Ok(()) }).await;

        assert!(matches!(outcome, Outcome::Cancelled { sent: 0 }));
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_during_the_pause_stops_without_waiting_it_out() {
        let plan = plan(3, 60.0);
        let token = CancellationToken::new();
        let cancel = token.clone();
        let mut sent = 0;

        let outcome = drive(&plan, &token, |_| {
            sent += 1;
            // Cancel once, while the first pause is in flight.
            cancel.cancel();
            async { Ok(()) }
        })
        .await;

        assert!(
            matches!(outcome, Outcome::Cancelled { sent: 1 }),
            "expected one message then cancellation, got {outcome:?}"
        );
        assert_eq!(sent, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_failure_stops_the_run_immediately() {
        let plan = plan(3, 1.2);
        let token = CancellationToken::new();
        let mut attempts = 0;

        let outcome = drive(&plan, &token, |index| {
            attempts += 1;
            async move {
                if index == 1 {
                    Err(SendError::Discord(serenity::Error::from(
                        std::io::Error::other("boom"),
                    )))
                } else {
                    Ok(())
                }
            }
        })
        .await;

        assert!(matches!(outcome, Outcome::Failed { sent: 1, .. }));
        assert_eq!(attempts, 2, "the third message must never be attempted");
    }

    #[tokio::test(start_paused = true)]
    async fn pacing_waits_the_configured_interval_between_messages() {
        let plan = plan(2, 1.2);
        let token = CancellationToken::new();
        let start = tokio::time::Instant::now();

        drive(&plan, &token, |_| async { Ok(()) }).await;

        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(1_200),
            "expected at least one interval of pacing, elapsed {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(2_400),
            "the final message must not be followed by a pause, elapsed {elapsed:?}"
        );
    }
}
