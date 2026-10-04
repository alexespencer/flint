use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::Result;

/// Somewhere to keep small secrets safely. Keys are plain names like `<store id>/s3-secret`.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn set(&self, key: &str, value: &[u8]) -> Result<()>;
    /// Removing something that is not there is not an error.
    fn delete(&self, key: &str) -> Result<()>;

    /// Whether `key` is there, without reading it. A secret store that asks the user before revealing a
    /// value (the macOS keychain does, for a program it does not recognise) can answer without asking.
    fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.get(key)?.is_some())
    }
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
    fn exists(&self, key: &str) -> Result<bool> {
        (**self).exists(key)
    }
}
