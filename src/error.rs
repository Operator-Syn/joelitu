//! Error types for every fallible boundary in the crate.
//!
//! Each subsystem owns one error enum rather than sharing a single catch-all.
//! The reason is diagnostic: a `SendError` in a log line says the failure was a
//! Discord call, not a validation or a registry problem, with no string parsing.
//!
//! Nothing here is `Clone`. `SendError` wraps `serenity::Error`, which is not
//! `Clone` either, so errors are moved to their single consumer — the task that
//! logs them.

use thiserror::Error;

/// Failures while reading and validating process configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A required environment variable is absent.
    #[error("environment variable {name} is required but not set")]
    Missing {
        /// The variable's exact name, so the operator can set it without guessing.
        name: &'static str,
    },

    /// A present environment variable could not be parsed.
    #[error("environment variable {name} is invalid: {value:?}")]
    Invalid {
        /// The variable's exact name.
        name: &'static str,
        /// The offending value, quoted in the message. Never a secret: this
        /// variant is only produced for non-secret variables such as the test
        /// guild id.
        value: String,
    },
}

/// Failures while validating a `/spam` request into a [`Plan`].
///
/// [`Plan`]: crate::spam::plan::Plan
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlanError {
    /// The message was empty or whitespace only.
    #[error("message must not be empty")]
    EmptyMessage,

    /// The message exceeded Discord's 2000-character content limit.
    #[error("message is {length} characters; the limit is {max}")]
    MessageTooLong {
        /// The length that was supplied.
        length: usize,
        /// The limit that was exceeded.
        max: usize,
    },

    /// A count of zero was requested.
    #[error("count must be between 1 and {max}")]
    CountOutOfRange {
        /// The maximum accepted count.
        max: u8,
    },

    /// The interval was NaN, infinite, or negative.
    #[error("interval must be a finite number of seconds")]
    InvalidInterval,
}

/// Failures while delivering one message to Discord.
#[derive(Debug, Error)]
pub enum SendError {
    /// The Discord API rejected the request, or the transport failed.
    #[error("discord rejected the message: {0}")]
    Discord(#[source] serenity::Error),
}

impl From<serenity::Error> for SendError {
    /// Lets `?` convert a serenity failure at the send site instead of at every
    /// caller.
    fn from(error: serenity::Error) -> Self {
        Self::Discord(error)
    }
}

/// The error type poise paths return.
///
/// Poise requires `From<serenity::Error>`, so an interaction reply that fails at
/// the transport level surfaces through this enum rather than being swallowed.
#[derive(Debug, Error)]
pub enum CommandError {
    /// The command's request did not validate into a plan.
    #[error(transparent)]
    Plan(#[from] PlanError),

    /// The channel already has the maximum number of runs in flight.
    #[error(
        "this channel already has {active} run(s) in flight; wait for one to finish or cancel it"
    )]
    ChannelBusy {
        /// How many runs the channel currently holds.
        active: usize,
    },

    /// A Discord call made by the command handler failed.
    #[error(transparent)]
    Discord(#[from] serenity::Error),
}
