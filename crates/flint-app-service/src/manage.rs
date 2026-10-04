//! Creating folders, deleting, and moving or renaming notes and folders.
//!
//! A folder is only a prefix of its files' paths, so everything here is about files. Because a file's path is
//! part of what is encrypted, moving one decrypts and encrypts it again under its new path; to sync, a move is
//! a delete at the old path and a new file at the new one. Deleting moves a file to the folder's hidden
//! `.trash`, which is never synced, so nothing is lost for good.

use std::collections::BTreeSet;

use flint_core::StorePath;
use flint_registry::{SecretStore, StoreId};
use flint_store_fs::{DiskFs, EncryptedFs, LocalFs, is_note_path};
use flint_sync_state::SyncState;

use crate::{AppService, Connect, Error, Result};

impl<S: SecretStore, C: Connect> AppService<S, C> {
    /// Every folder in the store, empty ones included (a folder with no file in it is not synced: S3 has
    /// no folders, only file paths).
    pub fn folders(&self, id: &StoreId) -> Result<Vec<StorePath>> {
        let (record, _) = self.unlocked(id)?;
        Ok(DiskFs::new(&record.folder).list_dirs()?)
    }

    /// [`folders`](Self::folders), each with when it was created on this device.
    pub fn folders_created(&self, id: &StoreId) -> Result<Vec<(StorePath, std::time::SystemTime)>> {
        let (record, _) = self.unlocked(id)?;
        Ok(DiskFs::new(&record.folder).list_dirs_with_created()?)
    }

    pub fn create_folder(&self, id: &StoreId, path: &StorePath) -> Result<()> {
        let (record, _) = self.unlocked(id)?;
        if !is_note_path(path) {
            return Err(Error::NotANote(path.clone()));
        }
        let disk = DiskFs::new(&record.folder);
        if disk.exists(path)? {
            return Err(flint_store_fs::Error::AlreadyExists(path.clone()).into());
        }
        Ok(disk.create_dir(path)?)
    }

    /// Moves a note, an attachment or a whole folder to the trash. Returns how many files went. Refuses a
    /// path that is in an unresolved conflict.
    pub fn delete_path(&self, id: &StoreId, path: &StorePath) -> Result<usize> {
        let (record, keys) = self.unlocked(id)?;
        let disk = DiskFs::new(&record.folder);
        let targets = self.targets(&record, &disk, path)?;
        for target in &targets {
            disk.trash(target)?;
        }
        if disk.is_dir(path) {
            disk.remove_empty_dirs(path)?;
        }
        self.refresh_index(&record, &keys, &EncryptedFs::new(disk, keys.clone()))?;
        Ok(targets.len())
    }

    /// Renames or moves a note, an attachment or a folder to `to` (a new path, not a folder to move into).
    /// Returns how many files moved. Links that point at what moved are not rewritten.
    pub fn move_path(&self, id: &StoreId, from: &StorePath, to: &StorePath) -> Result<usize> {
        let (record, keys) = self.unlocked(id)?;
        if !is_note_path(to) {
            return Err(Error::NotANote(to.clone()));
        }
        if from == to {
            return Ok(0);
        }
        if to.as_str().starts_with(&format!("{from}/")) {
            return Err(Error::InsideItself);
        }
        let disk = DiskFs::new(&record.folder);
        let targets = self.targets(&record, &disk, from)?;
        if disk.exists(to)? {
            return Err(flint_store_fs::Error::AlreadyExists(to.clone()).into());
        }
        let was_folder = disk.is_dir(from);
        let fs = EncryptedFs::new(disk.clone(), keys.clone());
        if was_folder {
            // the folder itself and every folder inside it, so empty ones move too
            disk.create_dir(to)?;
            let inside = format!("{from}/");
            for dir in disk.list_dirs()? {
                if let Some(rest) = dir.as_str().strip_prefix(&inside) {
                    disk.create_dir(&StorePath::new(format!("{to}/{rest}"))?)?;
                }
            }
        }
        for target in &targets {
            let rest = &target.as_str()[from.as_str().len()..];
            fs.rename(target, &StorePath::new(format!("{to}{rest}"))?)?;
        }
        if was_folder {
            disk.remove_empty_dirs(from)?;
        }
        self.refresh_index(&record, &keys, &fs)?;
        Ok(targets.len())
    }

    /// The files `path` stands for: itself if it is a file, everything under it if it is a folder (none for
    /// an empty folder). An error if there is nothing there, or if any of them is in a conflict.
    fn targets(
        &self,
        record: &flint_registry::StoreRecord,
        disk: &DiskFs,
        path: &StorePath,
    ) -> Result<Vec<StorePath>> {
        if !is_note_path(path) {
            return Err(Error::NotANote(path.clone()));
        }
        let under = format!("{path}/");
        let targets: Vec<StorePath> = disk
            .list()?
            .into_iter()
            .map(|e| e.path)
            .filter(|p| p == path || p.as_str().starts_with(&under))
            .collect();
        if targets.is_empty() && !disk.is_dir(path) {
            return Err(Error::NothingAt(path.clone()));
        }
        let held: BTreeSet<StorePath> =
            SyncState::open(&self.registry().state_dir(&record.id)?.join("sync.db"))?
                .conflicts()?
                .into_iter()
                .map(|c| c.path)
                .collect();
        if let Some(blocked) = targets.iter().find(|t| held.contains(*t)) {
            return Err(Error::InConflict(blocked.clone()));
        }
        Ok(targets)
    }
}
