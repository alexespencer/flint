//! The sync engine.
//!
//! Sync is a three-way comparison per file: what is on disk now, what is on the remote now, and the
//! last state both sides agreed on (the baseline, from `flint-sync-state`). [`plan`] turns those three
//! into one [`Action`] per file. It is a pure function with no I/O, so every rule can be tested exactly.
//!
//! The one property everything here protects: **no action ever overwrites or removes a version that has
//! not been agreed or backed up.** Changed files are told apart by hash (local) and ETag (remote), never
//! by timestamp, and every upload is conditional, so a concurrent change is detected and surfaced.

mod conflict;
mod device;
mod plan;

pub use conflict::conflict_copy_path;
pub use device::DeviceName;
pub use plan::{Action, ConflictKind, LocalFile, ReconcileReason, RemoteFile, UploadMode, plan};
