use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A store's identity on this device: random, never shown, and not the same on two devices. Names the
/// store's secrets and its private state folder, so renaming a store changes neither.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StoreId(String);

impl StoreId {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("the system random number generator");
        Self(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Accepts only what [`generate`](Self::generate) produces, since the id becomes a folder name.
    pub fn parse(text: &str) -> Option<Self> {
        let valid =
            text.len() == 16 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        valid.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a store's objects live. The secret key is not here: it is in the [`SecretStore`](crate::SecretStore).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSettings {
    pub bucket: String,
    /// The folder within the bucket; empty for the bucket root.
    pub prefix: String,
    pub region: String,
    /// `None` for AWS.
    pub endpoint: Option<String>,
    pub access_key_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreRecord {
    pub id: StoreId,
    pub name: String,
    /// The store's folder on this device.
    pub folder: PathBuf,
    pub remote: RemoteSettings,
    /// This store's password is the empty one: its data is still encrypted, but with a key anyone can derive,
    /// so shells need not ask for a password. Only a hint for the interface, kept per device; the key file is
    /// the truth. Absent in lists written before it existed, which counts as `false`.
    #[serde(default)]
    pub passwordless: bool,
}
