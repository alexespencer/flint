use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Deliberately one error: a wrong password and a tampered key file are indistinguishable.
    #[error("wrong password, or the key file was tampered with")]
    Unlock,

    #[error("invalid key file: {0}")]
    KeyFile(String),

    #[error("unacceptable KDF parameters: {0}")]
    KdfParams(&'static str),

    #[error("key derivation failed: {0}")]
    Kdf(String),

    #[error("not a Flint file: missing or truncated header")]
    BadHeader,

    #[error("unsupported file format version {0}")]
    UnsupportedVersion(u8),

    #[error("file was encrypted under key id {found}, but this store's key id is {expected}")]
    KeyIdMismatch { expected: u32, found: u32 },

    /// Deliberately one error: corruption, tampering, a wrong key and a moved file look the same.
    #[error("cannot decrypt: the file is corrupted, tampered with, or was moved from another path")]
    Decrypt,

    #[error("encryption failed")]
    Encrypt,

    #[error("system random number generator failed: {0}")]
    Random(#[from] getrandom::Error),
}
