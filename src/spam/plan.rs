//! Request validation and message rendering: the domain's numeric policy.
//!
//! This module is deliberately free of I/O, async, and serenity. Everything it
//! decides — how many messages, how far apart, whether the mention is attached
//! to which message — is a pure function of the request, so it is testable with
//! no runtime, no network, and no Discord types.
//!
//! The limits live here rather than in [`crate::config`] because they are safety
//! properties, not deployment knobs. An environment variable that can set the
//! interval floor to zero is a way to get the bot rate-limited or flagged.

use std::time::Duration;

use crate::error::PlanError;

/// Messages sent when the caller does not pass a count.
pub const DEFAULT_COUNT: u8 = 5;

/// Hard ceiling on a single run, regardless of what the caller asks for.
///
/// Ten messages twenty seconds apart is already obnoxious; past that it stops
/// being a novelty and becomes abuse automation.
pub const MAX_COUNT: u8 = 10;

/// Fastest permitted pacing between two messages.
///
/// Discord allows roughly five messages per five seconds in a channel. Sending
/// faster than this queues and arrives in a lump, so the pacing would be a lie.
pub const MIN_INTERVAL: Duration = Duration::from_millis(1_200);

/// Discord's maximum message content length, in characters (not bytes).
pub const MAX_MESSAGE_CHARS: usize = 2000;

/// What the caller asked for, before any policy is applied.
///
/// Deliberately plain data: this is the shape the command layer produces from
/// slash-command options and nothing else, so tests can construct it directly.
#[derive(Debug, Clone, PartialEq)]
pub struct RawRequest {
    /// Message body as typed. May be empty or whitespace; validation rejects it.
    pub message: String,
    /// Discord user id to mention on the first message only, if any.
    pub mention: Option<u64>,
    /// Requested message count. `None` means [`DEFAULT_COUNT`].
    pub count: Option<u8>,
    /// Requested seconds between messages. `None` means [`MIN_INTERVAL`].
    pub interval_seconds: Option<f64>,
}

/// A validated, policy-applied run: everything needed to send, and nothing else.
///
/// Constructed only by [`Plan::validate`], so the fields are guaranteed
/// consistent: `count` is within `1..=MAX_COUNT`, `interval` is at least
/// [`MIN_INTERVAL`], and `message` is non-empty and within the character limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Message body, already validated.
    message: String,
    /// Number of messages to send; always at least 1.
    count: u8,
    /// Gap between messages; always at least [`MIN_INTERVAL`].
    interval: Duration,
    /// User to mention, on the first message only.
    mention: Option<u64>,
    /// Whether the caller's interval was raised to [`MIN_INTERVAL`]. Reported
    /// back to the caller rather than clamped silently.
    interval_clamped: bool,
}

impl Plan {
    /// Validate a raw request into a runnable plan, applying policy.
    ///
    /// # Arguments
    ///
    /// * `raw` - The caller's request. Consumed; the message is moved into the
    ///   plan rather than copied.
    ///
    /// # Returns
    ///
    /// A `Plan` whose invariants are described on the type. A missing count
    /// becomes [`DEFAULT_COUNT`]; a missing interval becomes [`MIN_INTERVAL`]; an
    /// interval below [`MIN_INTERVAL`] is raised to it and flagged in
    /// [`Plan::interval_was_clamped`].
    ///
    /// # Errors
    ///
    /// * [`PlanError::EmptyMessage`] when the message is empty or whitespace only.
    /// * [`PlanError::MessageTooLong`] when it exceeds [`MAX_MESSAGE_CHARS`]
    ///   characters. Measured in `char`s, not bytes, so multi-byte text is not
    ///   penalised.
    /// * [`PlanError::CountOutOfRange`] when a count of zero or above
    ///   [`MAX_COUNT`] is requested. Out-of-range counts are rejected rather
    ///   than clamped: a caller asking for 50 wants 50 and should be told no.
    /// * [`PlanError::InvalidInterval`] when the interval is NaN, infinite, or
    ///   negative. Zero is not an error — it is raised to the floor.
    ///
    /// # Processing
    ///
    /// 1. Reject an empty or whitespace-only message.
    /// 2. Reject a message over the character limit.
    /// 3. Resolve and range-check the count.
    /// 4. Resolve the interval, rejecting non-finite and negative values, then
    ///    raise it to the floor and record whether that happened.
    ///
    /// # Panics
    ///
    /// Does not panic. Constructing a `Duration` from a validated finite f64 is
    /// the only fallible step, and both inputs are checked first.
    pub fn validate(raw: RawRequest) -> Result<Self, PlanError> {
        // 1. Whitespace-only is empty as far as a channel is concerned, so it is
        //    rejected rather than rendered as a blank message. The message is
        //    stored trimmed only for the emptiness test; the original spacing is
        //    preserved for rendering.
        if raw.message.trim().is_empty() {
            return Err(PlanError::EmptyMessage);
        }

        // 2. Discord counts characters; `chars()` matches its limit and keeps
        //    multi-byte text from being rejected early.
        let length = raw.message.chars().count();
        if length > MAX_MESSAGE_CHARS {
            return Err(PlanError::MessageTooLong {
                length,
                max: MAX_MESSAGE_CHARS,
            });
        }

        // 3. A count outside the range is a mistake worth surfacing, not a value
        //    worth guessing at. `None` takes the default.
        let count = match raw.count {
            None => DEFAULT_COUNT,
            Some(0) => return Err(PlanError::CountOutOfRange { max: MAX_COUNT }),
            Some(count) if count > MAX_COUNT => {
                return Err(PlanError::CountOutOfRange { max: MAX_COUNT });
            }
            Some(count) => count,
        };

        // 4. Non-finite and negative intervals cannot be scheduled, so they are
        //    errors. Anything shorter than the floor is a policy violation that
        //    the caller may not know about, so it is raised and reported.
        let (interval, interval_clamped) = match raw.interval_seconds {
            None => (MIN_INTERVAL, false),
            Some(seconds) if !seconds.is_finite() || seconds < 0.0 => {
                return Err(PlanError::InvalidInterval);
            }
            Some(seconds) => {
                let requested = Duration::from_secs_f64(seconds);
                if requested < MIN_INTERVAL {
                    (MIN_INTERVAL, true)
                } else {
                    (requested, false)
                }
            }
        };

        Ok(Self {
            message: raw.message,
            count,
            interval,
            mention: raw.mention,
            interval_clamped,
        })
    }

    /// Number of messages this run will send, always `1..=MAX_COUNT`.
    pub fn count(&self) -> u8 {
        self.count
    }

    /// Gap between consecutive messages, always at least [`MIN_INTERVAL`].
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// The user id that will be mentioned on the first message, if any.
    ///
    /// The send site must build `allowed_mentions` from exactly this value, so a
    /// literal `@everyone` or role mention typed into the message body cannot
    /// ping anyone.
    pub fn mention_user_id(&self) -> Option<u64> {
        self.mention
    }

    /// Whether [`Plan::validate`] raised the caller's interval to the floor.
    ///
    /// Callers are expected to disclose this in their acknowledgement rather
    /// than clamping silently.
    pub fn interval_was_clamped(&self) -> bool {
        self.interval_clamped
    }

    /// Render the content for the message at `index` (zero-based).
    ///
    /// # Returns
    ///
    /// The message body for the first message, prefixed with a Discord user
    /// mention; the bare body for every later message. Indices at or beyond
    /// [`Plan::count`] are never called by the runner and still render the bare
    /// body rather than panicking.
    ///
    /// # Arguments
    ///
    /// * `index` - Zero-based position within the run. Only the first position
    ///   carries the mention, which is what makes "bounded mention" a rule
    ///   instead of a hope.
    pub fn content_for(&self, index: u8) -> String {
        match (index, self.mention) {
            (0, Some(user_id)) => format!("<@{user_id}> {}", self.message),
            _ => self.message.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a request that validates, so each test changes one field only.
    fn base() -> RawRequest {
        RawRequest {
            message: "hello".to_owned(),
            mention: None,
            count: None,
            interval_seconds: None,
        }
    }

    #[test]
    fn defaults_apply_when_options_are_absent() {
        let plan = Plan::validate(base()).expect("base request must validate");

        assert_eq!(plan.count(), DEFAULT_COUNT);
        assert_eq!(plan.interval(), MIN_INTERVAL);
        assert!(!plan.interval_was_clamped());
    }

    #[test]
    fn empty_and_whitespace_messages_are_rejected() {
        for message in ["", "   ", "\n\t "] {
            let request = RawRequest {
                message: message.to_owned(),
                ..base()
            };

            assert_eq!(
                Plan::validate(request),
                Err(PlanError::EmptyMessage),
                "message {message:?} must be rejected"
            );
        }
    }

    #[test]
    fn message_length_is_counted_in_characters() {
        // 2001 multi-byte characters: within bytes-per-char inflation, over the
        // character limit.
        let request = RawRequest {
            message: "é".repeat(MAX_MESSAGE_CHARS + 1),
            ..base()
        };

        assert_eq!(
            Plan::validate(request),
            Err(PlanError::MessageTooLong {
                length: MAX_MESSAGE_CHARS + 1,
                max: MAX_MESSAGE_CHARS
            })
        );
    }

    #[test]
    fn count_bounds_are_rejected_not_clamped() {
        for count in [0, MAX_COUNT + 1, 200] {
            let request = RawRequest {
                count: Some(count),
                ..base()
            };

            assert_eq!(
                Plan::validate(request),
                Err(PlanError::CountOutOfRange { max: MAX_COUNT }),
                "count {count} must be rejected"
            );
        }
    }

    #[test]
    fn shortest_interval_is_raised_to_the_floor_and_flagged() {
        let request = RawRequest {
            interval_seconds: Some(0.1),
            ..base()
        };

        let plan = Plan::validate(request).expect("short interval is legal but clamped");

        assert_eq!(plan.interval(), MIN_INTERVAL);
        assert!(plan.interval_was_clamped());
    }

    #[test]
    fn interval_at_the_floor_is_not_flagged_as_clamped() {
        let request = RawRequest {
            interval_seconds: Some(MIN_INTERVAL.as_secs_f64()),
            ..base()
        };

        let plan = Plan::validate(request).expect("floor interval is legal");

        assert_eq!(plan.interval(), MIN_INTERVAL);
        assert!(!plan.interval_was_clamped());
    }

    #[test]
    fn non_finite_and_negative_intervals_are_rejected() {
        for seconds in [f64::NAN, f64::INFINITY, -1.0] {
            let request = RawRequest {
                interval_seconds: Some(seconds),
                ..base()
            };

            assert_eq!(
                Plan::validate(request),
                Err(PlanError::InvalidInterval),
                "interval {seconds} must be rejected"
            );
        }
    }

    #[test]
    fn mention_appears_on_the_first_message_only() {
        let request = RawRequest {
            mention: Some(42),
            ..base()
        };

        let plan = Plan::validate(request).expect("mention without count is legal");

        assert_eq!(plan.content_for(0), "<@42> hello");
        assert_eq!(plan.content_for(1), "hello");
        assert_eq!(plan.mention_user_id(), Some(42));
    }

    #[test]
    fn absent_mention_renders_the_bare_message() {
        let plan = Plan::validate(base()).expect("base request must validate");

        assert_eq!(plan.content_for(0), "hello");
        assert_eq!(plan.mention_user_id(), None);
    }
}
