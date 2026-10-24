//! Secrets in the Windows Credential Manager.
//!
//! One generic credential per secret, named `<service>/<key>`, kept for this Windows user (`Local`
//! persistence: it survives restarts and reinstalls of the app, and is not roamed to other PCs). The
//! store is the user's own, so unlike the macOS keychain it never asks anything of a program it does not
//! recognise.

use std::collections::HashMap;
use std::sync::Arc;

use keyring_core::api::CredentialStoreApi;
use keyring_core::{Entry, Error as KeyringError};
use windows_native_keyring_store::Store;

use crate::{Error, Result, SecretStore};

pub struct CredentialSecrets {
    service: String,
    store: Arc<Store>,
}

impl CredentialSecrets {
    pub fn new(service: impl Into<String>) -> Result<Self> {
        let store = Store::new().map_err(secrets_error)?;
        Ok(Self {
            service: service.into(),
            store,
        })
    }

    fn entry(&self, key: &str) -> Result<Entry> {
        let modifiers = HashMap::from([("persistence", "Local")]);
        self.store
            .build(&self.service, key, Some(&modifiers))
            .map_err(secrets_error)
    }
}

impl SecretStore for CredentialSecrets {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match self.entry(key)?.get_secret() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(e) => Err(secrets_error(e)),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.entry(key)?.set_secret(value).map_err(secrets_error)
    }

    fn delete(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(secrets_error(e)),
        }
    }
}

fn secrets_error(e: KeyringError) -> Error {
    Error::Secrets(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches the real Credential Manager, so it only runs when asked:
    /// `cargo test -p flint-registry -- --ignored`.
    #[test]
    #[ignore = "uses the real Windows Credential Manager"]
    fn the_credential_manager_round_trips_and_forgets() {
        let secrets = CredentialSecrets::new("dev.flint.test").unwrap();
        secrets.delete("test-item").unwrap();
        assert_eq!(secrets.get("test-item").unwrap(), None);
        assert!(!secrets.exists("test-item").unwrap());
        secrets.set("test-item", &[0, 1, 2, 255]).unwrap();
        assert_eq!(secrets.get("test-item").unwrap(), Some(vec![0, 1, 2, 255]));
        // a second handle (another process, say) sees it
        let other = CredentialSecrets::new("dev.flint.test").unwrap();
        assert!(other.exists("test-item").unwrap());
        secrets.delete("test-item").unwrap();
        assert_eq!(secrets.get("test-item").unwrap(), None);
        secrets.delete("test-item").unwrap(); // forgetting twice is fine
    }
}
