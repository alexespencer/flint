//! Which stores this device knows about, which one is active, and the secrets kept for them.
//!
//! The list lives in one JSON file in the app's config folder. Anything secret (the S3 secret key and a
//! store's unlocked data key) goes through a [`SecretStore`] instead: the login keychain on macOS, a
//! Keystore-backed implementation on Android, a fake in tests. The password is never kept anywhere.

mod error;
mod registry;
mod secrets;
mod types;

pub use error::{Error, Result};
pub use registry::Registry;
#[cfg(target_os = "macos")]
pub use secrets::KeychainSecrets;
pub use secrets::{MemorySecrets, SecretStore};
pub use types::{RemoteSettings, StoreId, StoreRecord};
