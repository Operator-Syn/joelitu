//! The shared state handed to every command handler.
//!
//! Poise passes `&Data` into handlers, and that borrow cannot be moved into
//! `tokio::spawn`, so every field is a cheap handle the handler can clone. Cloning
//! `Data` is two refcount bumps, never a deep copy.
//!
//! No poise or serenity type appears here. Keeping the shared state free of
//! transport types means the domain never has to name the framework.
//!
//! Configuration is deliberately not a field: the token is read once in `main` to
//! build the client and nothing afterwards needs it. A field no handler reads is a
//! field that rots.

use std::sync::Arc;

use crate::spam::{pacer::Pacer, registry::RunRegistry};

/// Process-wide state, constructed once in `main` and never mutated afterwards.
#[derive(Clone)]
pub struct Data {
    /// Active runs. Interior mutability lives inside the registry, behind its own
    /// lock, so this handle stays immutable.
    pub runs: Arc<RunRegistry>,
    /// Shared send budgets. One instance for the whole process, so every run
    /// competes for the same channel and global allowances instead of each assuming
    /// it owns them.
    pub pacer: Arc<Pacer>,
}

impl Data {
    /// Assemble the state from its parts.
    pub fn new(runs: Arc<RunRegistry>, pacer: Arc<Pacer>) -> Self {
        Self { runs, pacer }
    }
}
