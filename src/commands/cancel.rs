//! The `/cancel` command: stop every run in the calling channel.
//!
//! Cancellation is channel-scoped rather than id-scoped on purpose. Run ids are
//! process-local implementation detail, and a caller cannot know one — asking them
//! to name an id would push the registry's internals into the user contract.

use crate::{data::Data, error::CommandError};

/// Stop every `/spam` run currently active in this channel.
///
/// # Returns
///
/// Replies with how many runs were signalled. Zero is a normal answer, not an
/// error: a run that already finished leaves nothing to cancel.
///
/// # Errors
///
/// Returns [`CommandError::Discord`] when the reply itself cannot be sent.
///
/// # Side effects
///
/// Cancels the tokens of matching runs. The registry entries are removed by each
/// run's own guard as its task observes the cancellation, so the count reflects
/// runs present at the moment of the call, not runs that have already stopped.
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_MESSAGES")]
pub async fn cancel(ctx: poise::Context<'_, Data, CommandError>) -> Result<(), CommandError> {
    let data = ctx.data();

    let cancelled = data.runs.cancel_channel(ctx.channel_id().get());

    let reply = match cancelled {
        0 => "no active run in this channel".to_owned(),
        1 => "cancelling 1 run".to_owned(),
        n => format!("cancelling {n} runs"),
    };

    if let Err(error) = ctx.say(reply).await {
        return Err(CommandError::Discord(error));
    }

    Ok(())
}
