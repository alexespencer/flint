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
}
