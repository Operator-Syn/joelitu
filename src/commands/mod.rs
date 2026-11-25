//! Slash-command handlers. This module is transport only.
//!
//! Handlers map framework arguments into domain requests, call the feature module,
//! and turn results into replies. They contain no numeric limits and branch on no
//! business rule — every decision lives in `crate::spam`. Keeping the boundary that
//! sharp is what stops the two layers from drifting into duplicates.
//!
//! No re-exports here on purpose: `commands::spam::spam` reads oddly but points at
//! exactly one thing, and a flat `pub use` would collide with the `crate::spam`
//! module name.

pub mod cancel;
pub mod spam;
