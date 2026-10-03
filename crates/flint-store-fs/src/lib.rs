//! Local file I/O for one store.
//!
//! Two layers over the same folder:
//!
//! - [`LocalFs`] moves raw bytes by [`StorePath`]. Sync uses this, so it sees
//!   exactly the ciphertext that is on disk. [`DiskFs`] is the real one, [`MemFs`] the in-memory fake.
//! - [`EncryptedFs`] wraps a `LocalFs` and seals/opens note contents with the store's keys. The app uses this.
//!
//! The folder tree and file names are the store's: a file's path is its S3 key. A few things are never
//! listed (and so never synced or indexed): `.trash/`, `.obsidian/`, everything in `.store/` except
//! `key.json`, macOS `.DS_Store` files, and our own `.flint-tmp-*` files from interrupted writes.

mod contract;
mod disk;
mod encrypted;
mod error;
mod local;
mod mem;

use flint_core::StorePath;

pub use disk::DiskFs;
pub use encrypted::EncryptedFs;
pub use error::{Error, Result};
pub use local::{FileEntry, LocalFs};
pub use mem::MemFs;

/// The one file in `.store/` that syncs: the wrapped data key and KDF parameters.
pub const KEY_FILE: &str = ".store/key.json";
/// Where deleted files go, keeping their relative path.
pub const TRASH_DIR: &str = ".trash";

const STORE_DIR: &str = ".store";
const OBSIDIAN_DIR: &str = ".obsidian";
const DS_STORE: &str = ".DS_Store";
const TMP_PREFIX: &str = ".flint-tmp-";

/// Whether `path` is part of what sync uploads: notes and attachments, plus `.store/key.json`.
pub fn is_sync_path(path: &StorePath) -> bool {
    if path.as_str() == KEY_FILE {
        return true;
    }
    let mut parts = path.as_str().split('/');
    if matches!(parts.next(), Some(STORE_DIR | TRASH_DIR | OBSIDIAN_DIR)) {
        return false;
    }
    !path
        .as_str()
        .split('/')
        .any(|part| part == DS_STORE || part.starts_with(TMP_PREFIX))
}

/// Whether `path` is something the app reads and writes through [`EncryptedFs`]: a synced path
/// other than the key file, which is plaintext JSON.
pub fn is_note_path(path: &StorePath) -> bool {
    is_sync_path(path) && path.as_str() != KEY_FILE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> StorePath {
        StorePath::new(s).unwrap()
    }

    #[test]
    fn classifies_paths() {
        for synced in [
            "a.md",
            "folder/a.md",
            "img/pic.png",
            ".hidden-note.md",
            "store/key.json",
            "a/.trashy",
        ] {
            assert!(is_sync_path(&p(synced)), "{synced}");
        }
        for ignored in [
            ".trash/a.md",
            ".trash",
            ".obsidian/app.json",
            ".store/index.db",
            ".store/sync.db",
            ".store/other/key.json",
            ".DS_Store",
            "folder/.DS_Store",
            ".flint-tmp-abc123",
            "folder/.flint-tmp-abc123",
        ] {
            assert!(!is_sync_path(&p(ignored)), "{ignored}");
        }
    }

    #[test]
    fn the_key_file_syncs_but_is_not_a_note() {
        assert!(is_sync_path(&p(KEY_FILE)));
        assert!(!is_note_path(&p(KEY_FILE)));
        assert!(is_note_path(&p("a.md")));
        assert!(!is_note_path(&p(".trash/a.md")));
    }
}
