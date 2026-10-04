//! Local file I/O for one store.
//!
//! Two layers over the same folder:
//!
//! - [`LocalFs`] moves raw bytes by [`StorePath`]. Sync uses this, so it sees
//!   exactly the ciphertext that is on disk. [`DiskFs`] is the real one, [`MemFs`] the in-memory fake.
//! - [`EncryptedFs`] wraps a `LocalFs` and seals/opens note contents with the store's keys. The app uses this.
//!
//! The folder tree and file names are the store's: a file's path is its S3 key. **Anything whose name
//! starts with `.` is hidden: never listed, synced, adopted or indexed** (`.git/`, `.obsidian/`,
//! `.DS_Store`, our own `.flint-tmp-*` and `.trash/`, and `.store/` except its `key.json`). That is
//! how Obsidian treats dot-files too, and it keeps a dragged-in vault's `.git` out of sync.

mod contract;
mod disk;
mod encrypted;
mod error;
mod local;
mod mem;

use flint_core::StorePath;

pub use disk::DiskFs;
pub use encrypted::{AdoptReport, EncryptedFs};
pub use error::{Error, Result};
pub use local::{FileEntry, LocalFs};
pub use mem::MemFs;

/// The one file in `.store/` that syncs: the wrapped data key and KDF parameters.
pub const KEY_FILE: &str = ".store/key.json";
/// Where deleted files go, keeping their relative path.
pub const TRASH_DIR: &str = ".trash";

const TMP_PREFIX: &str = ".flint-tmp-";

/// Whether `path` is part of what sync uploads: notes and attachments, plus `.store/key.json`.
/// Anything with a hidden (dot-prefixed) component is not.
pub fn is_sync_path(path: &StorePath) -> bool {
    path.as_str() == KEY_FILE || !path.as_str().split('/').any(|part| part.starts_with('.'))
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
            "store/key.json",
            "a..b.md",
            "dot.in.name.md",
        ] {
            assert!(is_sync_path(&p(synced)), "{synced}");
        }
        for hidden in [
            ".hidden-note.md",
            "a/.hidden-note.md",
            "a/.hidden/note.md",
            ".git/config",
            ".git/objects/ab/cdef",
            ".claude/commands/x.md",
            ".smart-env/multi/a.ajson",
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
            assert!(!is_sync_path(&p(hidden)), "{hidden}");
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
