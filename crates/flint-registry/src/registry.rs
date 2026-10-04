use std::io::Write;
use std::path::{Path, PathBuf};

use flint_crypto::{Dek, StoreKeys};
use serde::{Deserialize, Serialize};

use crate::{Error, Result, SecretStore, StoreId, StoreRecord};

const FILE_NAME: &str = "stores.json";
const FORMAT: u32 = 1;
const DEK_LEN: usize = 32;

#[derive(Serialize, Deserialize)]
struct Data {
    format: u32,
    active: Option<StoreId>,
    stores: Vec<StoreRecord>,
}

/// The stores known on this device. Every change is written to disk before it returns.
pub struct Registry<S> {
    dir: PathBuf,
    secrets: S,
    data: Data,
}

impl<S: SecretStore> Registry<S> {
    /// Opens the registry kept in `dir` (the app's config folder), creating it empty if there is none.
    pub fn open(dir: impl Into<PathBuf>, secrets: S) -> Result<Self> {
        let dir = dir.into();
        let path = dir.join(FILE_NAME);
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let data: Data = serde_json::from_str(&text).map_err(|e| Error::Corrupt {
                    path: path.clone(),
                    reason: e.to_string(),
                })?;
                if data.format != FORMAT {
                    return Err(Error::UnsupportedVersion {
                        path,
                        found: data.format,
                    });
                }
                data
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data {
                format: FORMAT,
                active: None,
                stores: Vec::new(),
            },
            Err(e) => return Err(e.into()),
        };
        Ok(Self { dir, secrets, data })
    }

    pub fn list(&self) -> &[StoreRecord] {
        &self.data.stores
    }

    pub fn get(&self, id: &StoreId) -> Result<&StoreRecord> {
        self.data
            .stores
            .iter()
            .find(|s| &s.id == id)
            .ok_or_else(|| Error::UnknownStore(id.clone()))
    }

    /// Finds a store by name, ignoring case.
    pub fn find(&self, name: &str) -> Option<&StoreRecord> {
        self.data
            .stores
            .iter()
            .find(|s| s.name.to_lowercase() == name.to_lowercase())
    }

    pub fn active(&self) -> Option<&StoreRecord> {
        self.data.active.as_ref().and_then(|id| self.get(id).ok())
    }

    /// Adds a store. The first one added becomes the active one. Refuses a name already in use and a
    /// folder that is inside another store's folder, or holds one.
    pub fn add(&mut self, record: StoreRecord) -> Result<()> {
        self.check_name(&record.name, None)?;
        self.check_folder(&record.folder, None)?;
        if self.data.active.is_none() {
            self.data.active = Some(record.id.clone());
        }
        self.data.stores.push(record);
        self.save()
    }

    /// Forgets a store and everything kept for it here: its secrets and its private state. The store's
    /// folder and its objects on S3 are left alone.
    pub fn remove(&mut self, id: &StoreId) -> Result<()> {
        self.get(id)?;
        self.data.stores.retain(|s| &s.id != id);
        if self.data.active.as_ref() == Some(id) {
            self.data.active = self.data.stores.first().map(|s| s.id.clone());
        }
        self.save()?;
        for key in [s3_secret_key(id), keys_key(id)] {
            self.secrets.delete(&key)?;
        }
        match std::fs::remove_dir_all(self.state_path(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    pub fn set_active(&mut self, id: &StoreId) -> Result<()> {
        self.get(id)?;
        self.data.active = Some(id.clone());
        self.save()
    }

    pub fn rename(&mut self, id: &StoreId, name: &str) -> Result<()> {
        self.get(id)?;
        self.check_name(name, Some(id))?;
        self.record_mut(id).name = name.trim().to_owned();
        self.save()
    }

    /// Points a store at its folder's new location (the folder was moved by hand).
    pub fn set_folder(&mut self, id: &StoreId, folder: impl Into<PathBuf>) -> Result<()> {
        let folder = folder.into();
        self.get(id)?;
        self.check_folder(&folder, Some(id))?;
        self.record_mut(id).folder = folder;
        self.save()
    }

    /// A private folder for this store's databases (sync state, index), created if need be. It is on
    /// this device only and not inside the store's folder, so copying a store folder to another
    /// machine cannot carry this device's idea of what has been synced.
    pub fn state_dir(&self, id: &StoreId) -> Result<PathBuf> {
        self.get(id)?;
        let path = self.state_path(id);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    // ---- secrets --------------------------------------------------------------------------------

    pub fn s3_secret(&self, id: &StoreId) -> Result<Option<String>> {
        Ok(self
            .secrets
            .get(&s3_secret_key(id))?
            .and_then(|bytes| String::from_utf8(bytes).ok()))
    }

    pub fn set_s3_secret(&self, id: &StoreId, secret: &str) -> Result<()> {
        self.secrets.set(&s3_secret_key(id), secret.as_bytes())
    }

    /// Remembers the store's keys, so it can be opened without the password until it is
    /// [`lock`](Self::lock)ed.
    pub fn unlock(&self, id: &StoreId, keys: &StoreKeys) -> Result<()> {
        let mut bytes = keys.key_id().to_be_bytes().to_vec();
        bytes.extend_from_slice(keys.dek().as_bytes());
        self.secrets.set(&keys_key(id), &bytes)
    }

    /// The store's keys, if it was unlocked on this device and not locked since.
    pub fn unlocked_keys(&self, id: &StoreId) -> Result<Option<StoreKeys>> {
        let Some(bytes) = self.secrets.get(&keys_key(id))? else {
            return Ok(None);
        };
        let parsed = bytes.split_at_checked(4).and_then(|(key_id, dek)| {
            Some((
                <[u8; 4]>::try_from(key_id).ok()?,
                <[u8; DEK_LEN]>::try_from(dek).ok()?,
            ))
        });
        let Some((key_id, dek)) = parsed else {
            // not what we wrote: forget it, so the store is locked everywhere, including `is_unlocked`
            self.secrets.delete(&keys_key(id))?;
            return Ok(None);
        };
        Ok(Some(StoreKeys::new(
            u32::from_be_bytes(key_id),
            Dek::from_bytes(dek),
        )))
    }

    pub fn is_unlocked(&self, id: &StoreId) -> Result<bool> {
        self.secrets.exists(&keys_key(id))
    }

    pub fn lock(&self, id: &StoreId) -> Result<()> {
        self.secrets.delete(&keys_key(id))
    }

    pub fn lock_all(&self) -> Result<()> {
        for store in &self.data.stores {
            self.lock(&store.id)?;
        }
        Ok(())
    }

    // ---- internals ------------------------------------------------------------------------------

    fn state_path(&self, id: &StoreId) -> PathBuf {
        self.dir.join("stores").join(id.as_str())
    }

    fn record_mut(&mut self, id: &StoreId) -> &mut StoreRecord {
        self.data
            .stores
            .iter_mut()
            .find(|s| &s.id == id)
            .expect("checked by the caller")
    }

    fn check_name(&self, name: &str, except: Option<&StoreId>) -> Result<()> {
        if name.trim().is_empty() {
            return Err(Error::EmptyName);
        }
        let taken =
            self.data.stores.iter().any(|s| {
                Some(&s.id) != except && s.name.to_lowercase() == name.trim().to_lowercase()
            });
        if taken {
            Err(Error::NameTaken(name.trim().to_owned()))
        } else {
            Ok(())
        }
    }

    fn check_folder(&self, folder: &Path, except: Option<&StoreId>) -> Result<()> {
        if !folder.is_absolute() {
            return Err(Error::RelativeFolder(folder.to_owned()));
        }
        for other in self.data.stores.iter().filter(|s| Some(&s.id) != except) {
            if folder.starts_with(&other.folder) || other.folder.starts_with(folder) {
                return Err(Error::NestedStore {
                    folder: folder.to_owned(),
                    other: other.name.clone(),
                });
            }
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(&self.dir)?;
        tmp.write_all(
            serde_json::to_string_pretty(&self.data)
                .expect("plain data")
                .as_bytes(),
        )?;
        tmp.as_file().sync_all()?;
        tmp.persist(self.dir.join(FILE_NAME)).map_err(|e| e.error)?;
        Ok(())
    }
}

fn s3_secret_key(id: &StoreId) -> String {
    format!("{id}/s3-secret")
}

fn keys_key(id: &StoreId) -> String {
    format!("{id}/keys")
}

#[cfg(test)]
mod tests;
