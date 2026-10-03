use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use flint_core::StorePath;

use crate::{Error, FileEntry, LocalFs, Result, is_sync_path};

/// An in-memory store for tests. Behaves like [`DiskFs`](crate::DiskFs) (the contract tests prove it),
/// except that it has no folders, so it never reports an I/O error.
#[derive(Debug, Default)]
pub struct MemFs {
    files: Mutex<BTreeMap<StorePath, (Vec<u8>, SystemTime)>>,
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }

    fn files(&self) -> MutexGuard<'_, BTreeMap<StorePath, (Vec<u8>, SystemTime)>> {
        self.files.lock().expect("MemFs lock poisoned")
    }
}

impl LocalFs for MemFs {
    fn list(&self) -> Result<Vec<FileEntry>> {
        Ok(self
            .files()
            .iter()
            .filter(|(path, _)| is_sync_path(path))
            .map(|(path, (bytes, modified))| FileEntry {
                path: path.clone(),
                size: bytes.len() as u64,
                modified: *modified,
            })
            .collect())
    }

    fn exists(&self, path: &StorePath) -> Result<bool> {
        Ok(self.files().contains_key(path))
    }

    fn read(&self, path: &StorePath) -> Result<Vec<u8>> {
        self.files()
            .get(path)
            .map(|(bytes, _)| bytes.clone())
            .ok_or_else(|| Error::NotFound(path.clone()))
    }

    fn write_atomic(&self, path: &StorePath, bytes: &[u8]) -> Result<()> {
        self.files()
            .insert(path.clone(), (bytes.to_vec(), SystemTime::now()));
        Ok(())
    }

    fn remove(&self, path: &StorePath) -> Result<()> {
        self.files()
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| Error::NotFound(path.clone()))
    }

    fn move_file(&self, from: &StorePath, to: &StorePath) -> Result<()> {
        let mut files = self.files();
        let entry = files
            .remove(from)
            .ok_or_else(|| Error::NotFound(from.clone()))?;
        files.insert(to.clone(), entry);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::contract::local_fs_contract!((MemFs::new(), ()));
}
