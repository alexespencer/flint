//! A store's shared settings: kept on each device, and synced through S3 as an encrypted `.store/settings.json`.
//!
//! The object is invisible to the file sync (which ignores hidden paths), so it never produces a conflict to resolve;
//! the merge rule is in `flint-settings`. Syncing settings is part of [`sync`](AppService::sync) but a failure here
//! never fails the sync of notes.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use flint_core::{ContentHash, StorePath};
use flint_crypto::StoreKeys;
use flint_registry::{SecretStore, StoreId, StoreRecord};
use flint_remote::{Precondition, RemoteStore};
use flint_settings::{
    ATTACHMENT_FOLDER, NEW_NOTE_FOLDER, Parsed, REMOTE_PATH, Settings, StoreSettings,
};
use serde::{Deserialize, Serialize};

use crate::{AppService, Connect, Error, Result};

/// This device's copy. `remote_is_newer` is only ever kept here: the version of the shared file when it came from a
/// newer Flint (so it is left alone), or `None`.
#[derive(Default, Serialize, Deserialize)]
struct Local {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    remote_is_newer: Option<u32>,
}

impl Local {
    fn path(dir: &Path) -> PathBuf {
        dir.join("settings.json")
    }

    /// A missing or damaged file counts as no settings: they are only preferences, and the shared copy refills them.
    fn load(dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(dir))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, dir: &Path) -> Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(
            serde_json::to_string_pretty(self)
                .expect("plain data")
                .as_bytes(),
        )?;
        tmp.persist(Self::path(dir)).map_err(|e| e.error)?;
        Ok(())
    }
}

/// What the app shows and uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    pub settings: StoreSettings,
    /// Set when the store's shared settings came from a newer Flint: they are not used or changed here until Flint is updated.
    pub newer_version: Option<u32>,
}

/// A change to make: only the settings that are `Some`.
#[derive(Debug, Default, Clone)]
pub struct SettingsChange {
    pub attachment_folder: Option<String>,
    pub new_note_folder: Option<String>,
}

/// What syncing the settings did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SettingsSync {
    /// This device's settings changed because another device's were newer: the app should read them again.
    pub changed_here: bool,
    /// This device's changes were written to S3.
    pub shared: bool,
    /// The shared settings are from a newer Flint and were left alone.
    pub newer_version: Option<u32>,
    /// Why syncing the settings failed, if it did.
    pub error: Option<String>,
}

impl SettingsSync {
    pub(crate) fn failed(error: impl std::fmt::Display) -> Self {
        Self {
            error: Some(error.to_string()),
            ..Self::default()
        }
    }
}

impl<S: SecretStore, C: Connect> AppService<S, C> {
    /// The store's settings, with defaults for anything not set.
    pub fn settings(&self, id: &StoreId) -> Result<SettingsView> {
        self.registry().get(id)?;
        let local = Local::load(&self.registry().state_dir(id)?);
        Ok(SettingsView {
            settings: local.settings.typed(),
            newer_version: local.remote_is_newer,
        })
    }

    /// Changes settings on this device; the next sync shares them. A folder that cannot be used is refused, and so is
    /// any change while the shared settings are from a newer Flint.
    pub fn set_settings(&self, id: &StoreId, change: SettingsChange) -> Result<SettingsView> {
        self.registry().get(id)?;
        let dir = self.registry().state_dir(id)?;
        let mut local = Local::load(&dir);
        if let Some(version) = local.remote_is_newer {
            return Err(Error::SettingsFromNewerVersion(version));
        }
        let at = self.now_ms();
        let mut updated = local.settings.clone();
        if let Some(folder) = change.attachment_folder {
            updated.set_folder(ATTACHMENT_FOLDER, &folder, at)?;
        }
        if let Some(folder) = change.new_note_folder {
            updated.set_folder(NEW_NOTE_FOLDER, &folder, at)?;
        }
        local.settings = updated; // only once everything was accepted: a refused change changes nothing
        local.save(&dir)?;
        self.settings(id)
    }

    /// Brings this device's settings and the shared copy in S3 to the same state: pulls newer settings, pushes
    /// newer ones, and retries if another device changed S3 in between. Never fails the caller: problems are reported.
    pub(crate) async fn sync_settings(
        &self,
        record: &StoreRecord,
        keys: &StoreKeys,
        remote: &C::Remote,
    ) -> SettingsSync {
        match self.try_sync_settings(record, keys, remote).await {
            Ok(done) => done,
            Err(e) => SettingsSync::failed(e),
        }
    }

    async fn try_sync_settings(
        &self,
        record: &StoreRecord,
        keys: &StoreKeys,
        remote: &C::Remote,
    ) -> Result<SettingsSync> {
        let dir = self.registry().state_dir(&record.id)?;
        let path = StorePath::new(REMOTE_PATH)?;
        let mut local = Local::load(&dir);
        let mut report = SettingsSync::default();
        let seal = |settings: &Settings| -> Result<(Bytes, ContentHash)> {
            let sealed = keys.seal(&path, settings.to_json().as_bytes())?;
            let hash = ContentHash::of(&sealed);
            Ok((Bytes::from(sealed), hash))
        };

        // another device may change S3 between our read and our write: then the write is refused, and we start again
        for _ in 0..4 {
            let found = match remote.get(&path).await {
                Ok(object) => Some(object),
                Err(flint_remote::Error::NotFound(_)) => None,
                Err(e) => return Err(e.into()),
            };
            let Some(object) = found else {
                if local.settings.is_empty() {
                    break; // nothing to share yet
                }
                let (bytes, hash) = seal(&local.settings)?;
                match remote
                    .put(&path, bytes, &hash, Precondition::MustNotExist)
                    .await
                {
                    Ok(_) => {
                        report.shared = true;
                        break;
                    }
                    Err(flint_remote::Error::PreconditionFailed(_)) => continue,
                    Err(e) => return Err(e.into()),
                }
            };

            let parsed = keys
                .open(&path, &object.bytes)
                .ok()
                .and_then(|plain| String::from_utf8(plain).ok())
                .map(|text| flint_settings::parse(&text));
            let shared = match parsed {
                Some(Ok(Parsed::Current(settings))) => settings,
                Some(Ok(Parsed::Newer(version))) => {
                    // from a newer Flint: leave it exactly as it is
                    local.remote_is_newer = Some(version);
                    local.save(&dir)?;
                    report.newer_version = Some(version);
                    return Ok(report);
                }
                // damaged, or not openable with this store's key: counts as empty, so this device's copy replaces it
                _ => Settings::default(),
            };
            local.remote_is_newer = None;

            let merged = local.settings.merge(&shared);
            if merged != shared {
                let etag = object.entry.etag.clone().ok_or_else(|| {
                    Error::Remote(flint_remote::Error::InvalidConfig(
                        "the settings object has no ETag".into(),
                    ))
                })?;
                let (bytes, hash) = seal(&merged)?;
                match remote
                    .put(&path, bytes, &hash, Precondition::IfMatch(etag))
                    .await
                {
                    Ok(_) => report.shared = true,
                    Err(flint_remote::Error::PreconditionFailed(_)) => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            if merged != local.settings {
                local.settings = merged;
                report.changed_here = true;
            }
            break;
        }
        local.save(&dir)?;
        Ok(report)
    }
}
