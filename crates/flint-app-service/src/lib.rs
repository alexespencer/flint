//! The one API every shell uses (the CLI today, the Tauri app later): create or join a store, unlock
//! it, sync it, resolve conflicts, read and write notes. All decisions live here and below; a shell
//! only collects input and shows results.
//!
//! A store is *unlocked* on this device when its keys are in the registry's secret store. Every call
//! that needs the keys fails with [`Error::Locked`] otherwise; the password is only ever used by
//! [`AppService::create_store`], [`join_store`](AppService::join_store),
//! [`unlock`](AppService::unlock) and [`change_password`](AppService::change_password).

mod connect;
mod error;
mod indexing;
mod service;

pub use connect::{Connect, InMemoryConnect, S3Connect};
pub use error::{Error, Result};
pub use indexing::IndexStats;
pub use service::{AppService, ConflictInfo, NewStore, Status, SyncOutcome};
