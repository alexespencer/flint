use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("sync database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A value in the database no longer parses. Not repaired silently: baselines are the record of
    /// what was synced, and guessing could turn a conflict into an overwrite.
    #[error("corrupt sync state: {0}")]
    Corrupt(String),

    #[error(
        "sync.db has schema version {found}, but this build reads version {expected}; refusing to touch it"
    )]
    UnsupportedVersion { found: i64, expected: i64 },

    #[error("could not remove the sync database: {0}")]
    Reset(#[from] std::io::Error),
}
