use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error, PartialEq)]
pub enum Error {
    #[error("invalid store path {path:?}: {reason}")]
    InvalidStorePath { path: String, reason: &'static str },

    #[error("invalid content hash: {0}")]
    InvalidHash(#[from] hex::FromHexError),
}
