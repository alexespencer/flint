//! Which stores this device knows about, which one is active, and the secrets kept for them.
//!
//! The list lives in one JSON file in the app's config folder. Anything secret (the S3 secret key and a
//! store's unlocked data key) goes through a [`SecretStore`] instead: the login keychain on macOS, the Credential
//! Manager on Windows, a Keystore-backed implementation on Android, a fake in tests. The password is never kept anywhere.

mod error;
#[cfg(target_os = "macos")]
mod keychain;
mod registry;
mod secrets;
mod types;
#[cfg(windows)]
mod windows;

pub use error::{Error, Result};
#[cfg(target_os = "macos")]
pub use keychain::KeychainSecrets;
pub use registry::Registry;
pub use secrets::{MemorySecrets, SecretStore};
pub use types::{RemoteSettings, StoreId, StoreRecord};
#[cfg(windows)]
pub use windows::CredentialSecrets;
