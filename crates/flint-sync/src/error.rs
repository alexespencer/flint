use flint_core::StorePath;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Remote(#[from] flint_remote::Error),

    #[error(transparent)]
    Local(#[from] flint_store_fs::Error),

    #[error(transparent)]
    State(#[from] flint_sync_state::Error),

    #[error(transparent)]
    Crypto(#[from] flint_crypto::Error),

    #[error(transparent)]
    Core(#[from] flint_core::Error),

    #[error("{0} is not in conflict")]
    NotInConflict(StorePath),

    /// This device has synced the store before, but S3 no longer holds its `key.json`: the store was deleted
    /// there, or the bucket was emptied. Syncing now would look like every file being deleted, so nothing is
    /// done on this device.
    #[error(
        "the store is no longer on S3 (deleted there, or the bucket was emptied); nothing was changed here"
    )]
    RemoteStoreMissing,
}
