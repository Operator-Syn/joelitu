//! The `/spam` feature's public face: prepare a run, then hand it to a task.
//!
//! Callers — the command handlers — see three things and nothing else:
//! [`RawRequest`] going in, [`prepare`] validating and reserving, and [`start`]
//! spawning the work. Everything else in this module tree is an implementation
//! detail, which is what keeps the transport layer free of domain rules and the
//! domain free of Discord types.
//!
//! This module owns the one place that both touches the registry and writes the
//! terminal log line, so the registry's release invariant has a single home.

pub mod pacer;
pub mod plan;
pub mod registry;
pub mod runner;

use std::sync::Arc;

use serenity::all::{ChannelId, CreateAllowedMentions, CreateMessage, Http, UserId};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::{
    data::Data,
    error::{CommandError, SendError},
};
use pacer::Pacer;
use plan::{Plan, RawRequest};
use registry::{RunGuard, RunRegistry};
use runner::{Outcome, drive};

/// A validated run that has a reserved registry slot, awaiting its task.
///
/// Held only between [`prepare`] and [`start`]. Dropping it without calling
/// [`start`] leaves the registry entry allocated, so the caller must always do
/// one or the other.
pub struct PreparedRun {
    /// Registry id, used by the guard to release the slot.
    id: u64,
    /// Signal the runner watches; cancelled by `/cancel`.
    token: CancellationToken,
    /// The validated plan to execute.
    plan: Plan,
}

impl PreparedRun {
    /// The plan this run will execute, for building the acknowledgement text.
    pub fn plan(&self) -> &Plan {
        &self.plan
    }
}

/// Validate a request and reserve a registry slot for it.
///
/// # Arguments
///
/// * `data` - Shared state; only the registry is touched here.
/// * `channel_id` - Channel the run will post into. Doubles as the ceiling scope
///   and the `/cancel` scope.
/// * `raw` - The caller's request, validated into a [`Plan`].
///
/// # Returns
///
/// A [`PreparedRun`] holding the reserved slot. Nothing has been sent and no
/// task has been spawned.
///
/// # Errors
///
/// * Any [`crate::error::PlanError`], surfaced as [`CommandError::Plan`], when
///   the request does not validate.
/// * [`CommandError::ChannelBusy`] when the channel already holds the maximum
///   number of runs. The slot is not reserved in that case.
///
/// # Processing
///
/// 1. Validate before reserving, so a rejected request never occupies a slot.
/// 2. Reserve; on refusal report the channel's current occupancy.
///
/// # Constraints
///
/// The only limit on mentioning someone is the caller's count and the pacing
/// budgets. There is deliberately no per-target cooldown: an earlier version had a
/// five-minute one, which did not prevent a single 99-message run from pinging the
/// target ninety-nine times and did block all ordinary use in between.
pub fn prepare(
    data: &Data,
    channel_id: ChannelId,
    raw: RawRequest,
) -> Result<PreparedRun, CommandError> {
    // 1. Pure validation first: a bad request must not consume capacity.
    let plan = Plan::validate(raw)?;

    // 2. Reservation is the only shared-state mutation here, and it is atomic
    //    inside the registry, so two simultaneous commands cannot both observe a
    //    free slot.
    let (id, token) = data
        .runs
        .reserve(channel_id.get())
        .ok_or(CommandError::ChannelBusy {
            active: data.runs.active_in_channel(channel_id.get()),
        })?;

    Ok(PreparedRun { id, token, plan })
}

/// Spawn the task that executes a prepared run, and log how it ended.
///
/// # Arguments
///
/// * `run` - The reserved run from [`prepare`]. Consumed.
/// * `runs` - The registry the guard must release into. Cloned `Arc`, so the
///   spawned task can own it.
/// * `pacer` - Shared channel and process send budgets. Every send waits for a
///   permit, so a busy channel queues rather than failing or bursting.
/// * `http` - REST client used for every send. Cloned `Arc` per message, which is
///   a refcount bump rather than a connection.
/// * `channel_id` - Destination channel.
///
/// # Side effects
///
/// Spawns a task and, on completion, emits one `tracing` event at info or warn.
/// The task sends messages through Discord until the run completes, is cancelled,
/// or a send fails.
///
/// # Constraints
///
/// The send closure is the only place a message is constructed, and it always sets
/// `allowed_mentions` explicitly: either the single planned user, or empty. That is
/// what prevents `@everyone` typed into the body from pinging — the default is not
/// relied on.
///
/// The permit wait happens inside the send future, so the runner's cancellation race
/// covers it. Moving the wait outside the closure would make `/cancel` unresponsive
/// for the length of a queue.
pub fn start(
    run: PreparedRun,
    runs: Arc<RunRegistry>,
    pacer: Arc<Pacer>,
    http: Arc<Http>,
    channel_id: ChannelId,
) {
    let PreparedRun { id, token, plan } = run;

    tokio::spawn(async move {
        // The guard is dropped on every exit path, including a panic in the loop,
        // so a slot cannot outlive its task.
        let _guard = RunGuard::new(runs, id);

        let send = |_index: u8| {
            let content = plan.content();
            let allowed = match plan.mention_user_id() {
                Some(user_id) => CreateAllowedMentions::new().users([UserId::new(user_id)]),
                None => CreateAllowedMentions::new(),
            };
            let message = CreateMessage::new()
                .content(content)
                .allowed_mentions(allowed);
            let http = http.clone();
            let pacer = pacer.clone();

            async move {
                // Both budgets are checked here, immediately before the send, so the
                // permit cannot go stale between acquisition and use.
                pacer.acquire(channel_id.get()).await;

                channel_id
                    .send_message(&http, message)
                    .await
                    .map(|_| ())
                    .map_err(SendError::from)
            }
        };

        match drive(&plan, &token, send).await {
            Outcome::Completed { sent } => {
                info!(run_id = id, sent, "run completed");
            }
            Outcome::Cancelled { sent } => {
                info!(run_id = id, sent, "run cancelled");
            }
            Outcome::Failed { sent, error } => {
                warn!(run_id = id, sent, %error, "run failed");
            }
        }
    });
}
