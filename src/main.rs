//! Process entry point: configuration, logging, client construction, shutdown.
//!
//! Wiring only. Every decision lives in a module this file calls, so `main` can be
//! read top to bottom as the composition root: read config, start logging, build
//! the registry, hand everything to the framework, run the gateway, and translate
//! SIGTERM into cancellation rather than an abrupt kill.

// Poise's handler signature forces `Result<_, CommandError>` on every command, and
// `CommandError` wraps `serenity::Error`, which is large. Boxing it would push the
// allocation into every `?` at the transport boundary for no gain: these errors are
// produced rarely and never on a hot path.
#![allow(clippy::result_large_err)]

mod commands;
mod config;
mod data;
mod error;
mod framework;
mod spam;

use std::sync::Arc;

use poise::serenity_prelude::{ClientBuilder, GatewayIntents};
use tokio::signal::unix::{SignalKind, signal};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::{
    config::Config,
    spam::registry::{DEFAULT_MAX_RUNS_PER_CHANNEL, RunRegistry},
};

/// Start the bot and block until the gateway stops or the process is signalled.
///
/// # Returns
///
/// `Ok(())` on a clean SIGTERM shutdown. Any startup failure — missing token,
/// unparsable config, client construction error — is returned so the process exits
/// non-zero and the container's restart policy sees a failure rather than a clean
/// exit.
///
/// # Side effects
///
/// Opens a gateway connection, registers slash commands, writes logs to stdout, and
/// sends messages in response to commands.
///
/// # Processing
///
/// 1. Initialise logging from `RUST_LOG`, defaulting to `info`.
/// 2. Load configuration; a missing token aborts here with the variable name.
/// 3. Build the registry and the framework around shared state.
/// 4. Construct the client with no privileged intents — interactions need none.
/// 5. Run the gateway, racing it against SIGTERM; on SIGTERM cancel in-flight runs
///    so they stop mid-pause instead of being cut off by the container stop.
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Logs to stdout only: the container's log driver is the sink, and a file
    //    inside the image would be lost on restart anyway.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // 2. Configuration is read once, here, and passed down. Nothing else reads env.
    let config = Arc::new(Config::load()?);

    // 3. The registry is created in `main` rather than inside setup so shutdown
    //    handling keeps a handle to it.
    let runs = Arc::new(RunRegistry::new(DEFAULT_MAX_RUNS_PER_CHANNEL));

    // 4. No intents are requested: slash commands arrive as interactions, which
    //    are not gated behind privileged intents, and the bot never reads messages.
    //    `ClientBuilder::new` is synchronous; poise adds `.framework(...)` and the
    //    builder is awaited to construct the client.
    let mut client = ClientBuilder::new(config.token.clone(), GatewayIntents::empty())
        .framework(framework::build(config.clone(), runs.clone()))
        .await?;

    // 5. SIGTERM is what `docker stop` sends. Cancelling inside the grace period
    //    lets a run stop cleanly instead of the process vanishing mid-loop.
    let mut sigterm = signal(SignalKind::terminate())?;

    tokio::select! {
        result = client.start() => {
            // A gateway client that returns on its own is not a normal exit; log it
            // so the restart is visible in the container logs.
            result?;
            warn!("gateway client stopped on its own");
        }
        _ = sigterm.recv() => {
            info!("SIGTERM received; cancelling in-flight runs");
            let cancelled = runs.cancel_all();
            info!(cancelled, "in-flight runs cancelled");
        }
    }

    Ok(())
}

/// Entry point. Reports a startup failure on stderr and exits non-zero so the
/// container's restart policy and the operator both see it.
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("joelitu failed to start: {error}");
        std::process::exit(1);
    }
}
