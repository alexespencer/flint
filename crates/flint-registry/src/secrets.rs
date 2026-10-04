use std::collections::BTreeMap;
use std::sync::Mutex;

#[cfg(target_os = "macos")]
use crate::Error;
use crate::Result;

/// Somewhere to keep small secrets safely. Keys are plain names like `<store id>/s3-secret`.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn set(&self, key: &str, value: &[u8]) -> Result<()>;
    /// Removing something that is not there is not an error.
    fn delete(&self, key: &str) -> Result<()>;
}

/// Keeps secrets in memory: for tests, and for a process that must not leave anything behind.
#[derive(Default)]
pub struct MemorySecrets(Mutex<BTreeMap<String, Vec<u8>>>);

impl MemorySecrets {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.0.lock().expect("not poisoned").get(key).cloned())
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.0
            .lock()
            .expect("not poisoned")
            .insert(key.to_owned(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().expect("not poisoned").remove(key);
        Ok(())
    }
}

impl<T: SecretStore + ?Sized> SecretStore for std::sync::Arc<T> {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        (**self).get(key)
    }
    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        (**self).set(key, value)
    }
    fn delete(&self, key: &str) -> Result<()> {
        (**self).delete(key)
    }
}

/// Secrets as generic-password items in the user's login keychain, under one service name. Only this
/// user, on this Mac, once the Mac is unlocked, can read them.
#[cfg(target_os = "macos")]
pub struct KeychainSecrets {
    service: String,
}

#[cfg(target_os = "macos")]
impl KeychainSecrets {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

/// `errSecItemNotFound`
#[cfg(target_os = "macos")]
const ITEM_NOT_FOUND: i32 = -25300;

#[cfg(target_os = "macos")]
impl SecretStore for KeychainSecrets {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match security_framework::passwords::get_generic_password(&self.service, key) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.code() == ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(Error::Secrets(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        security_framework::passwords::set_generic_password(&self.service, key, value)
            .map_err(|e| Error::Secrets(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<()> {
        match security_framework::passwords::delete_generic_password(&self.service, key) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ITEM_NOT_FOUND => Ok(()),
            Err(e) => Err(Error::Secrets(e.to_string())),
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// Touches the real login keychain, so it only runs when asked: `cargo test -p flint-registry -- --ignored`.
    #[test]
    #[ignore = "uses the real login keychain"]
    fn the_keychain_round_trips_and_forgets() {
        let secrets = KeychainSecrets::new("dev.flint.test");
        let key = "test-item";
        secrets.delete(key).unwrap();
        assert_eq!(secrets.get(key).unwrap(), None);
        secrets.set(key, &[0, 1, 2, 255]).unwrap();
        assert_eq!(secrets.get(key).unwrap(), Some(vec![0, 1, 2, 255]));
        secrets.set(key, b"replaced").unwrap();
        assert_eq!(secrets.get(key).unwrap(), Some(b"replaced".to_vec()));
        secrets.delete(key).unwrap();
        assert_eq!(secrets.get(key).unwrap(), None);
        secrets.delete(key).unwrap();
    }
}
