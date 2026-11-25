//! Poise framework construction: command registration and shared-state wiring.
//!
//! The only decision in here is command scope. With `TEST_GUILD_ID` set, commands
//! register against that guild and appear immediately, which is what makes
//! development bearable; without it they register globally and can take up to an
//! hour to show up. Same command list either way, so there is one code path.

use std::sync::Arc;

use poise::serenity_prelude::{GuildId, Ready};

use crate::{
    commands,
    config::Config,
    data::Data,
    error::CommandError,
    spam::{pacer::Pacer, registry::RunRegistry},
};

/// Build the poise framework for this process.
///
/// # Arguments
///
/// * `config` - Deployment configuration; only the optional test guild id is read
///   during setup.
/// * `runs` - The registry every command will share. Cloned `Arc`; the same
///   instance must also be held by `main` for shutdown handling.
/// * `pacer` - The send budgets every run will share.
///
/// # Returns
///
/// A framework that poise will drive. Registration happens inside setup, once the
/// gateway reports ready, so the REST client is guaranteed usable.
///
/// # Panics
///
/// Does not panic. A registration failure propagates out of setup, which the
/// client treats as a fatal startup error.
pub fn build(
    config: Arc<Config>,
    runs: Arc<RunRegistry>,
    pacer: Arc<Pacer>,
) -> poise::Framework<Data, CommandError> {
    poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![commands::spam::spam(), commands::cancel::cancel()],
            ..Default::default()
        })
        .setup(move |ctx, _ready: &Ready, framework| {
            // Captured before the async block: setup receives a serenity context,
            // not the Data it is about to construct, so the parts are moved in.
            let config = config.clone();
            let runs = runs.clone();
            let pacer = pacer.clone();

            Box::pin(async move {
                let commands = &framework.options().commands;

                match config.test_guild_id {
                    Some(guild_id) => {
                        poise::builtins::register_in_guild(ctx, commands, GuildId::new(guild_id))
                            .await?;
                        tracing::info!(guild_id, "commands registered for the test guild");
                    }
                    None => {
                        poise::builtins::register_globally(ctx, commands).await?;
                        tracing::info!("commands registered globally");
                    }
                }

                Ok(Data::new(runs, pacer))
            })
        })
        .build()
}
