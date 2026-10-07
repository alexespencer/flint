use flint_core::StorePath;
use flint_registry::StoreId;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("the store is locked: unlock it with its password first")]
    Locked(StoreId),

    #[error("that password is not right for this store")]
    WrongPassword,

    #[error("the new password cannot be empty")]
    EmptyPassword,

    #[error("this bucket and prefix already hold a store; join it instead of creating another")]
    StoreExists,

    #[error("this bucket and prefix hold no store (there is no key.json); create one instead")]
    NoSuchStore,

    #[error(
        "this folder already holds a store's key ({0}), left from a store that was forgotten here or made elsewhere; \
         choose another folder, or rename or remove this one"
    )]
    FolderIsAStore(&'static str),

    #[error("the store folder has no {0}; it was moved or damaged")]
    MissingKeyFile(&'static str),

    #[error("the S3 secret key for this store is missing from the secret store")]
    MissingS3Secret(StoreId),

    #[error(
        "the password was changed on another device at the same time; nothing was changed here"
    )]
    PasswordChangedElsewhere,

    #[error("{0} is not a note path or does not exist")]
    NotANote(StorePath),

    #[error("there is nothing called {0}")]
    NothingAt(StorePath),

    #[error("{0} has an unresolved conflict; resolve it first")]
    InConflict(StorePath),

    #[error("a folder cannot be moved into itself")]
    InsideItself,

    #[error(transparent)]
    Registry(#[from] flint_registry::Error),

    #[error(transparent)]
    Remote(#[from] flint_remote::Error),

    #[error(transparent)]
    Local(#[from] flint_store_fs::Error),

    #[error(transparent)]
    Sync(flint_sync::Error),

    /// The store is no longer on S3 (deleted there, or the bucket was emptied). Nothing was changed here.
    #[error(
        "this store is no longer on S3 (deleted there, or the bucket was emptied); nothing was changed on this device"
    )]
    StoreMissingOnRemote,

    #[error("not everything is on S3 yet: {0}")]
    NotFullySynced(String),

    #[error("the name typed is not this store's name")]
    NameMismatch,

    #[error(transparent)]
    Settings(#[from] flint_settings::Error),

    /// The store's shared settings were written by a newer Flint than this one. They are left alone, and cannot be
    /// changed from this device until it is updated.
    #[error(
        "this store's shared settings are from a newer version of Flint (settings format {0}); update Flint on this device to change them"
    )]
    SettingsFromNewerVersion(u32),

    #[error(transparent)]
    State(#[from] flint_sync_state::Error),

    #[error(transparent)]
    Index(#[from] flint_index::Error),

    #[error(transparent)]
    Crypto(flint_crypto::Error),

    #[error(transparent)]
    Core(#[from] flint_core::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<flint_sync::Error> for Error {
    fn from(error: flint_sync::Error) -> Self {
        match error {
            flint_sync::Error::RemoteStoreMissing => Self::StoreMissingOnRemote,
            other => Self::Sync(other),
        }
    }
}

impl From<flint_crypto::Error> for Error {
    fn from(error: flint_crypto::Error) -> Self {
        match error {
            flint_crypto::Error::Unlock => Self::WrongPassword,
            other => Self::Crypto(other),
        }
    }
}
