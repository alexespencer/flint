//! What sync last agreed with the remote, kept in `.store/sync.db`.
//!
//! For every file the store has synced there is a [`Baseline`]: the hash of the stored (cipher)text and
//! the remote ETag at the moment local and remote last agreed. The planner compares "local now" and
//! "remote now" against it to tell who changed what, without ever looking at timestamps. A baseline
//! is recorded only *after* the action that produced it succeeded; if the process dies in between,
//! the next sync sees the file as changed on both sides with identical contents and records the baseline
//! then, so nothing is lost either way.
//!
//! The database holds only paths, hashes and ETags, never content. Unlike the index it cannot be rebuilt
//! from the notes: without it every file that differs between the two sides looks like a conflict.
//! So [`SyncState::open`] refuses a damaged or newer database and leaves the file alone, and
//! [`SyncState::reset`] is a separate, explicit act.

mod error;
mod state;
mod types;

pub use error::{Error, Result};
pub use state::SyncState;
pub use types::{Baseline, Conflict, LocalStamp};
