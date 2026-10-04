use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use camino::Utf8Path;
use flint_core::{ContentHash, StorePath};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::{Error, FileEntry, LocalFs, Result, TMP_PREFIX, is_sync_path};

/// A store folder on disk. Symlinks are not followed when listing; the store folder is assumed to be
/// the app's own, so a symlink planted inside it that points elsewhere is not defended against.
#[derive(Debug, Clone)]
pub struct DiskFs {
    root: PathBuf,
}

impl DiskFs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, path: &StorePath) -> PathBuf {
        self.root.join(path.as_str())
    }
}

impl LocalFs for DiskFs {
    fn list(&self) -> Result<Vec<FileEntry>> {
        let io_at = |path: &Path, source: io::Error| Error::Io {
            path: path.to_owned(),
            source,
        };
        // Do not walk into hidden folders (`.git` can hold thousands of files). `.store` is the one
        // exception, since `.store/key.json` syncs; `is_sync_path` filters the rest of it below.
        let walker = WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !(e.depth() > 0
                    && e.file_type().is_dir()
                    && name.starts_with('.')
                    && !(e.depth() == 1 && name == ".store"))
            });

        let mut files = Vec::new();
        for entry in walker {
            let entry = entry.map_err(|e| {
                let path = e.path().unwrap_or(&self.root).to_owned();
                io_at(&path, e.into())
            })?;
            if !entry.file_type().is_file() {
                continue; // folders, and symlinks (which are not followed)
            }
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .expect("walked entries are under the root");
            let relative = Utf8Path::from_path(relative)
                .ok_or_else(|| Error::NonUtf8Name(relative.to_owned()))?;
            let path = StorePath::new(relative.as_str())?;
            if !is_sync_path(&path) {
                continue;
            }
            let meta = entry
                .metadata()
                .map_err(|e| io_at(entry.path(), e.into()))?;
            let modified = meta.modified().map_err(|e| io_at(entry.path(), e))?;
            files.push(FileEntry {
                path,
                size: meta.len(),
                modified,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    fn exists(&self, path: &StorePath) -> Result<bool> {
        fs::exists(self.resolve(path)).map_err(|e| Error::io(path, e))
    }

    fn read(&self, path: &StorePath) -> Result<Vec<u8>> {
        fs::read(self.resolve(path)).map_err(|e| Error::io(path, e))
    }

    fn read_prefix(&self, path: &StorePath, len: usize) -> Result<Vec<u8>> {
        let read = || -> io::Result<Vec<u8>> {
            let mut bytes = Vec::with_capacity(len);
            File::open(self.resolve(path))?
                .take(len as u64)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        read().map_err(|e| Error::io(path, e))
    }

    fn entry(&self, path: &StorePath) -> Result<FileEntry> {
        let meta = fs::metadata(self.resolve(path)).map_err(|e| Error::io(path, e))?;
        if !meta.is_file() {
            return Err(Error::NotFound(path.clone()));
        }
        let modified = meta.modified().map_err(|e| Error::io(path, e))?;
        Ok(FileEntry {
            path: path.clone(),
            size: meta.len(),
            modified,
        })
    }

    fn write_atomic(&self, path: &StorePath, bytes: &[u8]) -> Result<()> {
        let dest = self.resolve(path);
        let dir = dest.parent().expect("a store path always has a parent");
        let write = || -> io::Result<()> {
            fs::create_dir_all(dir)?;
            // The temp file sits next to the destination so the rename stays on one filesystem.
            let mut tmp = tempfile::Builder::new()
                .prefix(TMP_PREFIX)
                .tempfile_in(dir)?;
            tmp.write_all(bytes)?;
            tmp.as_file().sync_all()?;
            tmp.persist(&dest).map_err(|e| e.error)?;
            File::open(dir)?.sync_all() // make the rename itself durable
        };
        write().map_err(|e| Error::io(path, e))
    }

    fn remove(&self, path: &StorePath) -> Result<()> {
        fs::remove_file(self.resolve(path)).map_err(|e| Error::io(path, e))
    }

    fn move_file(&self, from: &StorePath, to: &StorePath) -> Result<()> {
        let dest = self.resolve(to);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(to, e))?;
        }
        fs::rename(self.resolve(from), dest).map_err(|e| Error::io(from, e))
    }

    /// Streams the file through SHA-256 instead of loading it, so large attachments stay cheap.
    fn hash(&self, path: &StorePath) -> Result<ContentHash> {
        let stream = || -> io::Result<ContentHash> {
            let mut file = File::open(self.resolve(path))?;
            let mut hasher = Sha256::new();
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                match file.read(&mut chunk)? {
                    0 => break,
                    n => hasher.update(&chunk[..n]),
                }
            }
            Ok(ContentHash::from_bytes(hasher.finalize().into()))
        };
        stream().map_err(|e| Error::io(path, e))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn make() -> (DiskFs, TempDir) {
        let dir = TempDir::new().unwrap();
        (DiskFs::new(dir.path()), dir)
    }

    crate::contract::local_fs_contract!(make());

    fn p(s: &str) -> StorePath {
        StorePath::new(s).unwrap()
    }

    #[test]
    fn writes_leave_no_temp_files_behind() {
        let (fs, dir) = make();
        for i in 0..5 {
            fs.write_atomic(&p("notes/a.md"), format!("version {i}").as_bytes())
                .unwrap();
        }
        let names: Vec<_> = fs::read_dir(dir.path().join("notes"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.md"]);
    }

    #[test]
    fn a_failed_write_leaves_the_existing_file_and_no_debris() {
        let (fs, dir) = make();
        fs.write_atomic(&p("a"), b"i am a file").unwrap();
        // "a" is a file, so "a/b.md" cannot be created under it
        assert!(matches!(
            fs.write_atomic(&p("a/b.md"), b"x"),
            Err(Error::Io { .. })
        ));
        assert_eq!(fs.read(&p("a")).unwrap(), b"i am a file");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn leftover_temp_files_from_a_crash_are_ignored_by_list() {
        let (fs, dir) = make();
        fs.write_atomic(&p("a.md"), b"real").unwrap();
        fs::write(dir.path().join(".flint-tmp-AbC123"), b"half written").unwrap();
        let listed: Vec<_> = fs
            .list()
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        assert_eq!(listed, ["a.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_listed_or_followed() {
        let (fs, dir) = make();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.md"), b"outside the store").unwrap();
        fs.write_atomic(&p("a.md"), b"inside").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.md"), dir.path().join("link.md"))
            .unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked-dir")).unwrap();

        let listed: Vec<_> = fs
            .list()
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        assert_eq!(listed, ["a.md"]);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_file_name_that_is_not_utf8_is_an_error_not_a_silent_skip() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let (fs, dir) = make();
        fs::write(
            dir.path().join(OsStr::from_bytes(b"bad-\xff-name.md")),
            b"x",
        )
        .unwrap();
        assert!(matches!(fs.list(), Err(Error::NonUtf8Name(_))));
    }

    #[test]
    fn listing_a_missing_store_folder_is_an_error() {
        let fs = DiskFs::new("/definitely/not/a/real/folder");
        assert!(matches!(fs.list(), Err(Error::Io { .. })));
    }

    #[test]
    fn hash_streams_a_large_file() {
        let (fs, _dir) = make();
        let big = vec![0xAB; 5 * 1024 * 1024];
        fs.write_atomic(&p("big.bin"), &big).unwrap();
        assert_eq!(fs.hash(&p("big.bin")).unwrap(), ContentHash::of(&big));
    }
}
