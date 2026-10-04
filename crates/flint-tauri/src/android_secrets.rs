//! Saved secrets on Android: encrypted by a hardware-backed Keystore key and kept in a file in the app's
//! private storage. A store's cached data key (`<id>/keys`) is the exception: it lives in memory only,
//! so it is gone when the process is, and the store is locked again.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flint_registry::{Error, Result, SecretStore};
use tauri::AppHandle;
use tauri_plugin_keystore::{DataRequest, KeystoreExt, Sealed};

pub struct AndroidSecrets {
    app: AppHandle,
    file: PathBuf,
    in_memory: Mutex<BTreeMap<String, Vec<u8>>>,
    /// Serialises read-modify-write of the file.
    file_lock: Mutex<()>,
}

impl AndroidSecrets {
    pub fn new(app: AppHandle, folder: PathBuf) -> Self {
        Self {
            app,
            file: folder.join("secrets.json"),
            in_memory: Mutex::default(),
            file_lock: Mutex::new(()),
        }
    }

    fn read_file(&self) -> Result<BTreeMap<String, Sealed>> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|e| secrets_error(format!("secrets file: {e}")))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    fn write_file(&self, map: &BTreeMap<String, Sealed>) -> Result<()> {
        use std::io::Write;
        let dir = self.file.parent().expect("a file has a folder");
        std::fs::create_dir_all(dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(serde_json::to_string(map).expect("plain data").as_bytes())?;
        tmp.as_file().sync_all()?;
        tmp.persist(&self.file).map_err(|e| e.error)?;
        Ok(())
    }
}

/// The cached data key is never written to disk.
fn is_volatile(key: &str) -> bool {
    key.ends_with("/keys")
}

fn secrets_error(message: impl ToString) -> Error {
    Error::Secrets(message.to_string())
}

impl SecretStore for AndroidSecrets {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        if is_volatile(key) {
            return Ok(self
                .in_memory
                .lock()
                .expect("not poisoned")
                .get(key)
                .cloned());
        }
        let _guard = self.file_lock.lock().expect("not poisoned");
        let Some(sealed) = self.read_file()?.remove(key) else {
            return Ok(None);
        };
        let opened = self.app.keystore().open(sealed).map_err(secrets_error)?;
        Ok(Some(STANDARD.decode(opened.data).map_err(secrets_error)?))
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        if is_volatile(key) {
            self.in_memory
                .lock()
                .expect("not poisoned")
                .insert(key.to_owned(), value.to_vec());
            return Ok(());
        }
        let sealed = self
            .app
            .keystore()
            .seal(DataRequest {
                data: STANDARD.encode(value),
            })
            .map_err(secrets_error)?;
        let _guard = self.file_lock.lock().expect("not poisoned");
        let mut map = self.read_file()?;
        map.insert(key.to_owned(), sealed);
        self.write_file(&map)
    }

    fn delete(&self, key: &str) -> Result<()> {
        if is_volatile(key) {
            self.in_memory.lock().expect("not poisoned").remove(key);
            return Ok(());
        }
        let _guard = self.file_lock.lock().expect("not poisoned");
        let mut map = self.read_file()?;
        if map.remove(key).is_some() {
            self.write_file(&map)?;
        }
        Ok(())
    }
}
