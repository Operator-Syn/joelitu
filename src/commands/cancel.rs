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
///
/// # Privacy
///
/// The reply is ephemeral: only the invoker sees which runs were stopped. Discord's
/// own "user used /cancel" line is rendered by the client and cannot be hidden by
/// the application.
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_MESSAGES")]
pub async fn cancel(ctx: poise::Context<'_, Data, CommandError>) -> Result<(), CommandError> {
    let data = ctx.data();

    let cancelled = data.runs.cancel_channel(ctx.channel_id().get());

    let text = match cancelled {
        0 => "no active run in this channel".to_owned(),
        1 => "cancelling 1 run".to_owned(),
        n => format!("cancelling {n} runs"),
    };

    // Ephemeral for the same reason as `/spam`'s acknowledgement: who is stopping
    // runs in a channel is the caller's business, not the channel's.
    let reply = poise::CreateReply::default().content(text).ephemeral(true);

    if let Err(error) = ctx.send(reply).await {
        return Err(CommandError::Discord(error));
    }

    Ok(())
}
