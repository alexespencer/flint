//! Taking a store off this device, putting it back on S3, and deleting it everywhere.
//!
//! A store lives on S3; each device holds a working copy. **Removing** a store from a device is therefore safe:
//! it is all still on S3, and joining again brings it back. The folder is not deleted, though: it is renamed
//! (`<name>.removed-<date>-<time>`) so that anything that never reached S3 survives. **Deleting** a store is
//! the destructive one: it empties the store's folder on S3, for every device.

use std::path::{Path, PathBuf};

use flint_registry::{SecretStore, StoreId};
use flint_remote::RemoteStore;
use flint_store_fs::KEY_FILE;
use flint_sync_state::SyncState;

use crate::{AppService, Connect, Error, Result, SyncOutcome};

/// What taking a store off this device did.
#[derive(Debug)]
pub struct Removed {
    /// Where the store's folder went (renamed, not deleted); `None` if it was already gone.
    pub folder_kept_as: Option<PathBuf>,
}

impl<S: SecretStore, C: Connect> AppService<S, C> {
    /// Puts this device's copy of a store back onto S3, after the store was deleted there (or the bucket was
    /// emptied). Forgets what this device believed was synced, so every file is new to the next sync and is
    /// uploaded; other devices then see the store again. Conflict flags are cleared (both versions remain as
    /// files).
    pub async fn reupload_store(&self, id: &StoreId) -> Result<SyncOutcome> {
        self.unlocked(id)?;
        let mut state = SyncState::open(&self.registry().state_dir(id)?.join("sync.db"))?;
        state.clear_all()?;
        drop(state);
        self.sync(id).await
    }

    /// Takes a store off this device: forgets its settings, secrets, fingerprint keys and local database, and
    /// renames its folder to `<name>.removed-<date>-<time>`. Everything is still on S3.
    ///
    /// Unless `force`, the store is synced first and the removal is refused if anything would be left only in
    /// the folder: a sync that cannot run (offline, locked), a file that failed, was refused or is in
    /// conflict, or one that is not encrypted yet. `force` skips that check (the folder is kept either way).
    pub async fn remove_store(&mut self, id: &StoreId, force: bool) -> Result<Removed> {
        let record = self.registry().get(id)?.clone();
        if !force {
            let outcome = self.sync(id).await?;
            let mut problems = Vec::new();
            if !outcome.report.failed.is_empty() {
                problems.push(format!(
                    "{} file(s) could not be synced",
                    outcome.report.failed.len()
                ));
            }
            if !outcome.report.refused.is_empty() {
                problems.push(format!(
                    "{} file(s) were left alone by sync",
                    outcome.report.refused.len()
                ));
            }
            if !outcome.adopted.waiting.is_empty() {
                problems.push(format!(
                    "{} file(s) are not encrypted yet",
                    outcome.adopted.waiting.len()
                ));
            }
            let status = self.status(id)?;
            if !status.conflicts.is_empty() {
                problems.push(format!(
                    "{} note(s) are in conflict",
                    status.conflicts.len()
                ));
            }
            if !problems.is_empty() {
                return Err(Error::NotFullySynced(problems.join("; ")));
            }
        }
        let folder_kept_as = set_aside(&record.folder, &record.name, "removed")?;
        self.registry_mut().remove(id)?;
        Ok(Removed { folder_kept_as })
    }

    /// Deletes a store everywhere: every object under its prefix on S3, then the store is taken off this device
    /// as in [`remove_store`](Self::remove_store). Other devices find the store gone on their next sync and
    /// stop without touching their notes. Irreversible on S3, so the store's name must be typed. The key file
    /// goes first, so other devices notice at once. Returns how many objects were deleted. If deleting fails
    /// partway, nothing is changed on this device and the call can be repeated.
    pub async fn delete_store(&mut self, id: &StoreId, typed_name: &str) -> Result<usize> {
        let record = self.registry().get(id)?.clone();
        if typed_name != record.name {
            return Err(Error::NameMismatch);
        }
        let remote = self.remote_for(&record)?;
        let mut entries = remote.list().await?;
        entries.sort_by_key(|e| e.path.as_str() != KEY_FILE); // the key file first
        for entry in &entries {
            remote.delete(&entry.path, None).await?;
        }
        set_aside(&record.folder, &record.name, "deleted")?;
        self.registry_mut().remove(id)?;
        Ok(entries.len())
    }
}

/// Renames `folder` to `<its name>.<what>-<date>-<time>` beside it (with a number added if that exists), and
/// returns the new path; `None` if there is no such folder.
fn set_aside(folder: &Path, store_name: &str, what: &str) -> Result<Option<PathBuf>> {
    if !folder.exists() {
        return Ok(None);
    }
    let stamp = time::OffsetDateTime::now_utc()
        .format(&time::macros::format_description!(
            "[year][month][day]-[hour][minute]"
        ))
        .expect("a plain format");
    let name = folder.file_name().map_or_else(
        || store_name.to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let parent = folder.parent().unwrap_or_else(|| Path::new("."));
    let mut target = parent.join(format!("{name}.{what}-{stamp}"));
    for n in 2.. {
        if !target.exists() {
            break;
        }
        target = parent.join(format!("{name}.{what}-{stamp}-{n}"));
    }
    std::fs::rename(folder, &target)?;
    Ok(Some(target))
}
