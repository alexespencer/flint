//! Shared helpers for tests that span crates, and the end-to-end gates (see `tests/`).
//!
//! Nothing here is shipped. `index_store` is a stand-in for what `flint-app-service` will do.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use flint_core::StorePath;
use flint_crypto::{KdfParams, KeyFile, StoreKeys};
use flint_index::Index;
use flint_store_fs::{DiskFs, EncryptedFs, FileEntry, LocalFs};
use tempfile::TempDir;
use walkdir::WalkDir;

pub type BoxError = Box<dyn Error + Send + Sync>;

/// The cheapest Argon2 parameters, so tests that create stores stay fast. Never use outside tests.
pub const FAST_KDF: KdfParams = KdfParams {
    m_kib: 8,
    t: 1,
    p: 1,
};

/// A throwaway store: a temp folder, its keys, and the encrypted view of it. The folder is deleted
/// when this is dropped.
pub struct TestStore {
    pub dir: TempDir,
    pub keys: StoreKeys,
    pub fs: EncryptedFs<DiskFs>,
}

impl TestStore {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let (_, keys) = KeyFile::create("test password", FAST_KDF).expect("key file");
        let fs = EncryptedFs::new(DiskFs::new(dir.path()), keys.clone());
        Self { dir, keys, fs }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn index(&self) -> Index {
        Index::open_in_memory(self.keys.index_key()).expect("in-memory index")
    }
}

impl Default for TestStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Copies a folder tree exactly as a user's Finder drag would, hidden files included.
pub fn copy_dir_all(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in WalkDir::new(from).follow_links(false) {
        let entry = entry?;
        let target = to.join(entry.path().strip_prefix(from).expect("under source"));
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// A file under a folder that contains one of the needles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: PathBuf,
    pub needle: String,
}

/// Scans every file under `dir` (hidden folders included, symlinks not followed) for byte strings.
/// The check behind "no plaintext on disk": pass phrases that appear in the notes.
pub fn find_plaintext(dir: &Path, needles: &[&str]) -> Vec<Finding> {
    let mut found = Vec::new();
    for entry in WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        for needle in needles {
            if !needle.is_empty() && bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                // spelled with `/` on every platform, like a store path, so callers can compare it as text
                let relative = entry.path().strip_prefix(dir).unwrap_or(entry.path());
                let slashed: Vec<_> = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                found.push(Finding {
                    path: PathBuf::from(slashed.join("/")),
                    needle: needle.to_string(),
                });
            }
        }
    }
    found
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub indexed: Vec<StorePath>,
    pub unchanged: usize,
    pub removed: Vec<StorePath>,
}

/// Brings `index` up to date with the store: indexes files that are new or changed (by size and
/// modification time), forgets files that are gone. Decrypts notes to index them.
pub fn index_store<L: LocalFs>(
    fs: &EncryptedFs<L>,
    index: &mut Index,
) -> Result<IndexStats, BoxError> {
    let mut stats = IndexStats::default();
    let known: std::collections::BTreeMap<_, _> = index
        .entries()?
        .into_iter()
        .map(|e| (e.path, e.stamp))
        .collect();
    let files = fs.list()?;

    for entry in &files {
        let stamp = stamp(entry);
        if known.get(&entry.path) == Some(&stamp) {
            stats.unchanged += 1;
        } else if entry
            .path
            .as_path()
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            let text = String::from_utf8_lossy(&fs.read(&entry.path)?).into_owned();
            index.upsert_note(&entry.path, &text, &stamp)?;
            stats.indexed.push(entry.path.clone());
        } else {
            index.upsert_attachment(&entry.path, &stamp)?;
            stats.indexed.push(entry.path.clone());
        }
    }
    let on_disk: std::collections::BTreeSet<_> = files.iter().map(|e| &e.path).collect();
    for path in known.keys().filter(|p| !on_disk.contains(p)) {
        index.remove(path)?;
        stats.removed.push(path.clone());
    }
    Ok(stats)
}

fn stamp(entry: &FileEntry) -> String {
    let nanos = entry
        .modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}", entry.size)
}

/// Checks that backlinks and link resolution tell the same story: every link that resolves to a file
/// other than its own note is a backlink of exactly that file, and there are no other backlinks.
/// Returns how many resolved links there were.
pub fn assert_backlinks_agree_with_resolution(index: &Index) -> usize {
    let mut resolved_links = 0;
    let entries = index.entries().expect("entries");
    for entry in entries.iter().filter(|e| e.is_note) {
        for out in index.outgoing(&entry.path).expect("outgoing") {
            if let Some(target) = &out.resolved
                && target != &entry.path
                && !out.link.target.is_empty()
            {
                resolved_links += 1;
                let back = index.backlinks(target).expect("backlinks");
                assert!(
                    back.iter()
                        .any(|b| b.from == entry.path && b.link == out.link),
                    "{} -> {} is missing from the backlinks of {target}",
                    entry.path,
                    out.link.target
                );
            }
        }
    }
    let total: usize = entries
        .iter()
        .map(|e| index.backlinks(&e.path).expect("backlinks").len())
        .sum();
    assert_eq!(
        total, resolved_links,
        "backlinks and resolved links must be the same set"
    );
    resolved_links
}

/// Whether any component of a store-relative path is hidden (starts with a dot).
pub fn is_hidden(relative: &Path) -> bool {
    relative
        .components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
}
