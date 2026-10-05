use std::path::PathBuf;

use flint_core::StorePath;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0} does not exist")]
    NotFound(StorePath),

    #[error("{0} already exists")]
    AlreadyExists(StorePath),

    /// `.store/`, `.trash/`, `.obsidian/` and the like are not notes and are not encrypted by the app.
    #[error("{0} is not a note path")]
    NotANotePath(StorePath),

    /// Only on Windows, which refuses names that macOS, Linux and Android allow.
    #[error("{path} cannot be stored on Windows: \"{name}\" {why}")]
    UnsupportedName {
        path: StorePath,
        name: String,
        why: &'static str,
    },

    #[error("file name is not valid UTF-8: {0:?}")]
    NonUtf8Name(PathBuf),

    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error(transparent)]
    Core(#[from] flint_core::Error),

    #[error(transparent)]
    Crypto(#[from] flint_crypto::Error),
}

impl Error {
    /// Maps "no such file" to [`Error::NotFound`] and everything else to [`Error::Io`].
    pub(crate) fn io(path: &StorePath, source: std::io::Error) -> Self {
        if source.kind() == std::io::ErrorKind::NotFound {
            Self::NotFound(path.clone())
        } else {
            Self::Io {
                path: path.as_str().into(),
                source,
            }
        }
    }
}
