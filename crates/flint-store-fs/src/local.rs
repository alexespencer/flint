use std::time::{SystemTime, UNIX_EPOCH};

use flint_core::{ContentHash, StorePath};

use crate::{Error, Result, TRASH_DIR};

/// One synced file as the folder shows it. `size` and `modified` describe the stored bytes (for an
/// encrypted store, the ciphertext).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: StorePath,
    pub size: u64,
    pub modified: SystemTime,
}

/// Raw bytes in and out of one store's folder. Implementations: [`DiskFs`](crate::DiskFs) and
/// [`MemFs`](crate::MemFs), which must behave identically (the contract tests run against both).
pub trait LocalFs {
    /// Every synced file (see [`is_sync_path`](crate::is_sync_path)), sorted by path.
    fn list(&self) -> Result<Vec<FileEntry>>;

    fn exists(&self, path: &StorePath) -> Result<bool>;

    fn read(&self, path: &StorePath) -> Result<Vec<u8>>;

    /// Creates parent folders as needed. Readers see either the old contents or the new, never a mix,
    /// even if the process dies mid-write.
    fn write_atomic(&self, path: &StorePath, bytes: &[u8]) -> Result<()>;

    fn remove(&self, path: &StorePath) -> Result<()>;

    /// Renames `from` to `to`, creating `to`'s parent folders. Replaces `to` if it exists.
    fn move_file(&self, from: &StorePath, to: &StorePath) -> Result<()>;

    /// SHA-256 of the stored bytes.
    fn hash(&self, path: &StorePath) -> Result<ContentHash> {
        Ok(ContentHash::of(&self.read(path)?))
    }

    /// Moves a file to `.trash/<its path>` instead of deleting it, and returns where it went. If that
    /// name is taken, the new copy gets a millisecond timestamp before its extension, so nothing
    /// already in the trash is overwritten.
    fn trash(&self, path: &StorePath) -> Result<StorePath> {
        if !self.exists(path)? {
            return Err(Error::NotFound(path.clone()));
        }
        let mut target = StorePath::new(format!("{TRASH_DIR}/{path}"))?;
        let mut millis = now_millis();
        while self.exists(&target)? {
            target = with_timestamp(path, millis)?;
            millis += 1;
        }
        self.move_file(path, &target)?;
        Ok(target)
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// `.trash/dir/note.<millis>.md` for `dir/note.md`.
fn with_timestamp(path: &StorePath, millis: u128) -> Result<StorePath> {
    let p = path.as_path();
    let stem = p.file_stem().unwrap_or("");
    let name = match p.extension() {
        Some(ext) => format!("{stem}.{millis}.{ext}"),
        None => format!("{stem}.{millis}"),
    };
    let dir = p.parent().map(|d| d.as_str()).unwrap_or("");
    let relative = if dir.is_empty() {
        name
    } else {
        format!("{dir}/{name}")
    };
    Ok(StorePath::new(format!("{TRASH_DIR}/{relative}"))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamped_trash_names_keep_directory_and_extension() {
        let p = |s: &str| StorePath::new(s).unwrap();
        assert_eq!(
            with_timestamp(&p("dir/note.md"), 5).unwrap().as_str(),
            ".trash/dir/note.5.md"
        );
        assert_eq!(
            with_timestamp(&p("note.md"), 5).unwrap().as_str(),
            ".trash/note.5.md"
        );
        assert_eq!(
            with_timestamp(&p("a/README"), 5).unwrap().as_str(),
            ".trash/a/README.5"
        );
        assert_eq!(
            with_timestamp(&p("a/.hidden"), 5).unwrap().as_str(),
            ".trash/a/.hidden.5"
        );
        assert_eq!(
            with_timestamp(&p("a/b.tar.gz"), 5).unwrap().as_str(),
            ".trash/a/b.tar.5.gz"
        );
    }
}
