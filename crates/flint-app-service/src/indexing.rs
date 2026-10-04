use std::collections::{BTreeMap, BTreeSet};
use std::time::UNIX_EPOCH;

use flint_core::StorePath;
use flint_index::Index;
use flint_store_fs::{DiskFs, EncryptedFs, FileEntry, is_note_path};

use crate::Result;

/// What bringing the index up to date did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub indexed: Vec<StorePath>,
    pub removed: Vec<StorePath>,
    pub unchanged: usize,
    /// Files that could not be read or decrypted. They are left out of the index and tried again next time.
    pub failed: Vec<(StorePath, String)>,
}

/// Indexes the files that are new or changed (by size and modification time) and forgets the ones that
/// are gone. Notes are decrypted to be indexed; the index file itself is encrypted under the store's
/// index key.
pub(crate) fn refresh(fs: &EncryptedFs<DiskFs>, index: &mut Index) -> Result<IndexStats> {
    let mut stats = IndexStats::default();
    let known: BTreeMap<_, _> = index
        .entries()?
        .into_iter()
        .map(|e| (e.path, e.stamp))
        .collect();
    let files: Vec<FileEntry> = fs
        .list()?
        .into_iter()
        .filter(|e| is_note_path(&e.path))
        .collect();

    for entry in &files {
        let stamp = stamp(entry);
        if known.get(&entry.path) == Some(&stamp) {
            stats.unchanged += 1;
            continue;
        }
        let indexed = if is_markdown(&entry.path) {
            fs.read(&entry.path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    let text = String::from_utf8_lossy(&bytes);
                    index
                        .upsert_note(&entry.path, &text, &stamp)
                        .map_err(|e| e.to_string())
                })
        } else {
            index
                .upsert_attachment(&entry.path, &stamp)
                .map_err(|e| e.to_string())
        };
        match indexed {
            Ok(()) => stats.indexed.push(entry.path.clone()),
            Err(reason) => stats.failed.push((entry.path.clone(), reason)),
        }
    }

    let on_disk: BTreeSet<_> = files.iter().map(|e| &e.path).collect();
    for path in known.keys().filter(|p| !on_disk.contains(p)) {
        index.remove(path)?;
        stats.removed.push(path.clone());
    }
    Ok(stats)
}

/// Brings one file's index entry up to date, after the app itself wrote it.
pub(crate) fn refresh_one(
    fs: &EncryptedFs<DiskFs>,
    index: &mut Index,
    path: &StorePath,
) -> Result<()> {
    use flint_store_fs::LocalFs;
    let stamp = stamp(&fs.raw().entry(path)?);
    let text = String::from_utf8_lossy(&fs.read(path)?).into_owned();
    index.upsert_note(path, &text, &stamp)?;
    Ok(())
}

fn is_markdown(path: &StorePath) -> bool {
    path.as_path()
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

fn stamp(entry: &FileEntry) -> String {
    let nanos = entry
        .modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}", entry.size)
}
