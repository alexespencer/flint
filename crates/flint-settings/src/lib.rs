//! The settings that belong to one store and are shared by every device that uses it (where pasted pictures go,
//! where new notes go, ...). Settings that belong to a device (auto-lock, Touch ID) are not here.
//!
//! Pure: no I/O, no clock, no encryption. The app service keeps a copy on each device and syncs another through S3
//! (encrypted, as `.store/settings.json`), using what is here to decide what a merged copy says.
//!
//! **The merge rule.** Every setting carries the time it was last changed. Merging two copies keeps, for each setting,
//! the later change; settings only one side knows are kept too, so a device running an older Flint passes a newer
//! Flint's settings through untouched. There is never a conflict to resolve and nothing to edit by hand. Merging is
//! commutative, associative and idempotent, so devices that sync in any order, any number of times, agree.
//!
//! **Versions.** A file is `{ "version": N, "settings": { ... } }`. A file from an older version is read and upgraded.
//! A file from a *newer* version than this one understands is left alone ([`Parsed::Newer`]): the device keeps using
//! its own copy, does not overwrite the shared one, and says so.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// The version of the file format this build writes and understands.
pub const VERSION: u32 = 1;

/// Where the shared copy lives in S3, relative to the store: encrypted like everything else.
pub const REMOTE_PATH: &str = ".store/settings.json";

/// The folder a pasted or dropped picture is saved in.
pub const ATTACHMENT_FOLDER: &str = "attachment_folder";
/// The folder a new note goes in when no folder is typed.
pub const NEW_NOTE_FOLDER: &str = "new_note_folder";

pub const DEFAULT_ATTACHMENT_FOLDER: &str = "Images";
pub const DEFAULT_NEW_NOTE_FOLDER: &str = "";

#[derive(Debug, Error, PartialEq)]
pub enum Error {
    #[error("the settings are damaged: {0}")]
    Damaged(String),
    #[error(
        "{0:?} is not a folder this store can use (no `..`, no backslash, no name starting with a dot, no name Windows refuses)"
    )]
    InvalidFolder(String),
}

/// One setting's value and when it was last changed (milliseconds since 1970, by the clock of the device that changed it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub value: Value,
    pub at: u64,
}

/// The settings of a store, as stored: every setting ever set, with when. Unknown settings are kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Settings(BTreeMap<String, Entry>);

/// What the app reads: every setting with its default filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSettings {
    pub attachment_folder: String,
    pub new_note_folder: String,
}

impl Default for StoreSettings {
    fn default() -> Self {
        Self {
            attachment_folder: DEFAULT_ATTACHMENT_FOLDER.to_owned(),
            new_note_folder: DEFAULT_NEW_NOTE_FOLDER.to_owned(),
        }
    }
}

/// A folder in the form the store uses: names joined by `/`, with no leading or trailing slash; `""` is the top level.
/// `None` if it cannot be a folder here: `..`, a backslash, a name starting with a dot (hidden folders are neither
/// encrypted nor synced), or a name Windows cannot hold (the setting is shared with devices that run Windows).
pub fn normalize_folder(input: &str) -> Option<String> {
    if input.contains('\\') {
        return None;
    }
    let names: Vec<&str> = input
        .split('/')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    let refused = |name: &&str| {
        *name == "."
            || *name == ".."
            || name.starts_with('.')
            || flint_core::portable::windows_name_problem(name).is_some()
    };
    if names.iter().any(refused) {
        return None;
    }
    Some(names.join("/"))
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The settings with their defaults filled in. A stored value that is not a usable folder (hand-edited, or from a
    /// version that allowed more) counts as not set.
    pub fn typed(&self) -> StoreSettings {
        let folder = |key: &str, default: &str| {
            self.0
                .get(key)
                .and_then(|entry| entry.value.as_str())
                .and_then(normalize_folder)
                .unwrap_or_else(|| default.to_owned())
        };
        StoreSettings {
            attachment_folder: folder(ATTACHMENT_FOLDER, DEFAULT_ATTACHMENT_FOLDER),
            new_note_folder: folder(NEW_NOTE_FOLDER, DEFAULT_NEW_NOTE_FOLDER),
        }
    }

    /// Sets a folder setting as of `at`. The folder is normalised first; one that cannot be used is refused.
    pub fn set_folder(&mut self, key: &str, folder: &str, at: u64) -> Result<(), Error> {
        let folder =
            normalize_folder(folder).ok_or_else(|| Error::InvalidFolder(folder.to_owned()))?;
        self.0.insert(
            key.to_owned(),
            Entry {
                value: Value::String(folder),
                at,
            },
        );
        Ok(())
    }

    /// The merge of two copies: for every setting, the later change. When both changed it at the same millisecond the
    /// larger value wins, so the result does not depend on which copy is `self`.
    pub fn merge(&self, other: &Settings) -> Settings {
        let mut merged = self.0.clone();
        for (key, theirs) in &other.0 {
            match merged.get(key) {
                Some(ours) if wins(ours, theirs) => {}
                _ => {
                    merged.insert(key.clone(), theirs.clone());
                }
            }
        }
        Settings(merged)
    }

    /// The file's text, in the current version.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&File {
            version: VERSION,
            settings: self.clone(),
        })
        .expect("plain data")
    }
}

/// Whether `ours` beats `theirs`: later, or on a tie the larger value (a total order, so merging is symmetric).
fn wins(ours: &Entry, theirs: &Entry) -> bool {
    (ours.at, ours.value.to_string()) >= (theirs.at, theirs.value.to_string())
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    settings: Settings,
}

/// What reading a settings file found.
#[derive(Debug, PartialEq)]
pub enum Parsed {
    /// A file this version understands (an older one has been upgraded).
    Current(Settings),
    /// A file written by a newer version of Flint. Leave it alone; it is not understood here.
    Newer(u32),
}

/// Reads the text of a settings file.
pub fn parse(text: &str) -> Result<Parsed, Error> {
    // the version is read on its own first, so a file with a shape this version does not know still reports its version
    let value: Value = serde_json::from_str(text).map_err(|e| Error::Damaged(e.to_string()))?;
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::Damaged("no version".into()))? as u32;
    if version > VERSION {
        return Ok(Parsed::Newer(version));
    }
    // (older versions would be upgraded here; there is only one so far)
    let file: File = serde_json::from_value(value).map_err(|e| Error::Damaged(e.to_string()))?;
    Ok(Parsed::Current(file.settings))
}

#[cfg(test)]
mod tests;
