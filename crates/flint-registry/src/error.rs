use std::path::PathBuf;

use thiserror::Error;

use crate::StoreId;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no store with id {0}")]
    UnknownStore(StoreId),

    #[error("there is already a store called {0:?}")]
    NameTaken(String),

    #[error("a store needs a name")]
    EmptyName,

    #[error("{0} is not an absolute path")]
    RelativeFolder(PathBuf),

    #[error("{folder} is inside the store {other:?}, or holds it; stores cannot be nested")]
    NestedStore { folder: PathBuf, other: String },

    #[error("the store list at {path} is from a newer version of Flint (format {found})")]
    UnsupportedVersion { path: PathBuf, found: u32 },

    #[error("the store list at {path} is damaged: {reason}")]
    Corrupt { path: PathBuf, reason: String },

    #[error("the secret store failed: {0}")]
    Secrets(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
