use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("index database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("could not reset the index file: {0}")]
    Reset(#[from] std::io::Error),

    /// A stored value no longer parses; the index is derived data, so rebuild it.
    #[error("corrupt index data: {0}")]
    Corrupt(String),

    /// A `.md` file is a note and anything else an attachment; the extension decides.
    #[error("{0} is the wrong kind of file for this call (notes end in .md)")]
    WrongKind(flint_core::StorePath),

    #[error(transparent)]
    Core(#[from] flint_core::Error),
}
