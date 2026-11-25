//! The `/spam` command: parse options, validate, acknowledge, spawn.
//!
//! This file deliberately contains no limits and no pacing arithmetic. It knows
//! how to turn poise options into a [`RawRequest`] and how to say something useful
//! about the result. Everything else is `crate::spam`'s job.

use poise::serenity_prelude as serenity;

use crate::{
    data::Data,
    error::CommandError,
    spam::{self, plan::RawRequest},
};

/// Post a message a bounded number of times, optionally mentioning someone on every
/// message.
///
/// Example: `/spam message:"good morning" count:3 interval:2`
///
/// # Arguments
///
/// * `message` - Text to post. Rejected when empty or longer than 2000 characters.
/// * `mention` - Optional user to mention. The mention is attached to the first
///   message only, so a run never becomes a ping burst.
/// * `count` - How many messages, 1 to 99. Defaults to 5.
/// * `interval` - Seconds between messages. Anything under 1.2 is raised to 1.2 and
///   the reply says so.
///
/// # Errors
///
/// Returns [`CommandError`] when validation fails, the channel already holds the
/// maximum number of runs, or a Discord call fails. Poise surfaces these to the
/// caller through the framework's error handler.
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_MESSAGES")]
pub async fn spam(
    ctx: poise::Context<'_, Data, CommandError>,
    #[description = "Text to post. Up to 2000 characters."] message: String,
    #[description = "User to mention on every message."] mention: Option<serenity::User>,
    #[description = "How many messages to post (1-99, default 5)."]
    #[min = 1]
    #[max = 99]
    count: Option<u8>,
    #[description = "Seconds between messages (minimum 1.2)."] interval: Option<f64>,
) -> Result<(), CommandError> {
    let data = ctx.data();

    // 1. Build the domain request. No policy decisions happen here: the presence
    //    or absence of each option is passed through verbatim.
    let raw = RawRequest {
        message,
        mention: mention.map(|user| user.id.get()),
        count,
        interval_seconds: interval,
    };

    // 2. Validate and reserve. A failure here means nothing was sent and no slot
    //    is held, so replying is the whole recovery.
    let run = spam::prepare(data, ctx.channel_id(), raw)?;

    // 3. Acknowledge before any message goes out. If this fails the run must not
    //    proceed: a burst the caller never saw confirmed is worse than none, and
    //    the prepared run is dropped here, releasing its slot.
    let plan = run.plan();
    let acknowledgement = if plan.interval_was_clamped() {
        format!(
            "posting {} message(s); interval raised to {:.1}s (minimum)",
            plan.count(),
            plan.interval().as_secs_f64()
        )
    } else {
        format!("posting {} message(s)", plan.count())
    };

    if let Err(error) = ctx.say(acknowledgement).await {
        return Err(CommandError::Discord(error));
    }

    // 4. Hand off. From here the spawned task owns the run and its registry guard.
    //    The `Arc<Http>` comes from the serenity context: poise's `ctx.http()` is a
    //    borrow, and the spawned task needs an owned handle.
    spam::start(
        run,
        data.runs.clone(),
        ctx.serenity_context().http.clone(),
        ctx.channel_id(),
    );

    Ok(())
}
