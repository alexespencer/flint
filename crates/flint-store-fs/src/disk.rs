use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

    /// Where `path` is on disk. Built name by name, never by pasting the string on, so what a name contains
    /// cannot change where it points; on Windows a name that system cannot hold is refused instead.
    fn resolve(&self, path: &StorePath) -> Result<PathBuf> {
        #[cfg(windows)]
        if let Some((name, why)) = path.windows_problem() {
            return Err(Error::UnsupportedName {
                path: path.clone(),
                name: name.to_owned(),
                why,
            });
        }
        Ok(self.join(path.as_str()))
    }

    /// `relative` (a `/`-separated path) under the root, name by name.
    fn join(&self, relative: &str) -> PathBuf {
        let mut full = self.root.clone();
        full.extend(relative.split('/').filter(|name| !name.is_empty()));
        full
    }

    /// Every folder in the store, including empty ones (which no file lists), as paths relative to the
    /// root, sorted. Hidden folders (`.git`, `.store`, `.trash`) and everything inside them are left out.
    pub fn list_dirs(&self) -> Result<Vec<StorePath>> {
        Ok(self
            .list_dirs_with_created()?
            .into_iter()
            .map(|(path, _)| path)
            .collect())
    }

    /// [`list_dirs`](Self::list_dirs), each with when the folder was created on this device (its modification
    /// time where the file system keeps no creation time, as on some Android file systems).
    pub fn list_dirs_with_created(&self) -> Result<Vec<(StorePath, SystemTime)>> {
        let mut dirs = Vec::new();
        let walker = WalkDir::new(&self.root)
            .follow_links(false)
            .min_depth(1)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'));
        for entry in walker {
            let entry = entry.map_err(|e| {
                let path = e.path().unwrap_or(&self.root).to_owned();
                Error::Io {
                    path,
                    source: e.into(),
                }
            })?;
            if !entry.file_type().is_dir() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .expect("walked entries are under the root");
            let relative = Utf8Path::from_path(relative)
                .ok_or_else(|| Error::NonUtf8Name(relative.to_owned()))?;
            let meta = entry.metadata().map_err(|e| Error::Io {
                path: entry.path().to_owned(),
                source: e.into(),
            })?;
            let created = meta
                .created()
                .or_else(|_| meta.modified())
                .map_err(|e| Error::Io {
                    path: entry.path().to_owned(),
                    source: e,
                })?;
            dirs.push((StorePath::new(slash_separated(relative))?, created));
        }
        dirs.sort();
        Ok(dirs)
    }

    /// Whether `path` is a folder.
    pub fn is_dir(&self, path: &StorePath) -> bool {
        self.resolve(path).is_ok_and(|dir| dir.is_dir())
    }

    /// Makes a folder (and any missing parents). An error if something that is not a folder is in the way.
    pub fn create_dir(&self, path: &StorePath) -> Result<()> {
        fs::create_dir_all(self.resolve(path)?).map_err(|e| Error::io(path, e))
    }

    /// Tidies up after a file was removed: removes the folder it was in if that is now empty, then that
    /// folder's parent if it is now empty too, and so on, stopping at the first one that holds anything. Does
    /// not touch empty folders elsewhere, so a folder someone made on purpose survives.
    pub fn remove_empty_parents(&self, path: &StorePath) -> Result<()> {
        for ancestor in path.as_path().ancestors().skip(1) {
            if ancestor.as_str().is_empty() {
                break;
            }
            let dir = self.join(ancestor.as_str());
            let Ok(entries) = fs::read_dir(&dir) else {
                continue; // already gone: carry on upwards
            };
            let mut only_clutter = true;
            for entry in entries {
                let entry = entry.map_err(|e| Error::io(path, e))?;
                if entry.file_name() != ".DS_Store" {
                    only_clutter = false;
                }
            }
            if !only_clutter {
                break;
            }
            let _ = fs::remove_file(dir.join(".DS_Store"));
            fs::remove_dir(&dir).map_err(|e| Error::io(path, e))?;
        }
        Ok(())
    }

    /// Tidies up after the files in a folder were moved or deleted: removes `path` and the folders
    /// inside it that are now empty, bottom up. A folder that still holds anything (a note, an attachment,
    /// a hidden folder such as `.git`) is left alone, and so are its parents. A stray `.DS_Store` does not
    /// count as something held.
    pub fn remove_empty_dirs(&self, path: &StorePath) -> Result<()> {
        fn prune(dir: &Path) -> io::Result<bool> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                // hidden folders (`.git`) are never touched, empty or not
                if entry.file_type()?.is_dir()
                    && !entry.file_name().to_string_lossy().starts_with('.')
                {
                    prune(&entry.path())?;
                }
            }
            let mut only_clutter = true;
            for entry in fs::read_dir(dir)? {
                if entry?.file_name() != ".DS_Store" {
                    only_clutter = false;
                }
            }
            if only_clutter {
                let _ = fs::remove_file(dir.join(".DS_Store"));
                fs::remove_dir(dir)?;
            }
            Ok(only_clutter)
        }
        let dir = self.resolve(path)?;
        if !dir.is_dir() {
            return Ok(());
        }
        prune(&dir).map(|_| ()).map_err(|e| Error::io(path, e))
    }
}

/// A relative path as names joined with `/` (on Windows `Utf8Path::as_str` would keep its backslashes).
fn slash_separated(relative: &Utf8Path) -> String {
    relative
        .components()
        .map(|component| component.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// Puts the finished temp file in place of `dest`. Windows refuses to replace a file that another program
/// has open (a virus scanner, a sync client) for a moment, so it is tried a few times there.
fn replace_file(tmp: tempfile::NamedTempFile, dest: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        tmp.persist(dest).map(|_| ()).map_err(|e| e.error)
    }
    #[cfg(windows)]
    {
        let mut tmp = tmp;
        for attempt in 0.. {
            match tmp.persist(dest) {
                Ok(_) => return Ok(()),
                Err(e) if attempt < 8 && e.error.kind() == io::ErrorKind::PermissionDenied => {
                    tmp = e.file;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => return Err(e.error),
            }
        }
        unreachable!()
    }
}

/// Makes a rename in `dir` durable. Not possible on Windows, which cannot open a folder to flush it.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
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
            // A name that is not a valid store path (it has a backslash in it) cannot be synced; it is left
            // alone, like a hidden file, rather than making the whole folder unreadable.
            let Ok(path) = StorePath::new(slash_separated(relative)) else {
                continue;
            };
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
        fs::exists(self.resolve(path)?).map_err(|e| Error::io(path, e))
    }

    fn read(&self, path: &StorePath) -> Result<Vec<u8>> {
        fs::read(self.resolve(path)?).map_err(|e| Error::io(path, e))
    }

    fn read_prefix(&self, path: &StorePath, len: usize) -> Result<Vec<u8>> {
        let full = self.resolve(path)?;
        let read = || -> io::Result<Vec<u8>> {
            let mut bytes = Vec::with_capacity(len);
            File::open(&full)?
                .take(len as u64)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        read().map_err(|e| Error::io(path, e))
    }

    fn entry(&self, path: &StorePath) -> Result<FileEntry> {
        let meta = fs::metadata(self.resolve(path)?).map_err(|e| Error::io(path, e))?;
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
        let dest = self.resolve(path)?;
        let dir = dest.parent().expect("a store path always has a parent");
        let write = || -> io::Result<()> {
            fs::create_dir_all(dir)?;
            // The temp file sits next to the destination so the rename stays on one filesystem.
            let mut tmp = tempfile::Builder::new()
                .prefix(TMP_PREFIX)
                .tempfile_in(dir)?;
            tmp.write_all(bytes)?;
            tmp.as_file().sync_all()?;
            replace_file(tmp, &dest)?;
            sync_dir(dir)
        };
        write().map_err(|e| Error::io(path, e))
    }

    fn remove(&self, path: &StorePath) -> Result<()> {
        fs::remove_file(self.resolve(path)?).map_err(|e| Error::io(path, e))
    }

    fn move_file(&self, from: &StorePath, to: &StorePath) -> Result<()> {
        let dest = self.resolve(to)?;
        let source = self.resolve(from)?;
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(to, e))?;
        }
        fs::rename(source, dest).map_err(|e| Error::io(from, e))
    }

    /// Streams the file through SHA-256 instead of loading it, so large attachments stay cheap.
    fn hash(&self, path: &StorePath) -> Result<ContentHash> {
        let full = self.resolve(path)?;
        let stream = || -> io::Result<ContentHash> {
            let mut file = File::open(&full)?;
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

#[cfg(test)]
mod folder_tests {
    use super::*;

    fn p(s: &str) -> StorePath {
        StorePath::new(s).unwrap()
    }

    #[test]
    fn folders_are_listed_empty_ones_included_and_hidden_ones_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let fs = DiskFs::new(dir.path());
        fs.write_atomic(&p("a/b/note.md"), b"x").unwrap();
        fs.create_dir(&p("empty/inner")).unwrap();
        fs.create_dir(&p(".git/objects")).unwrap();
        fs.create_dir(&p(".trash/old")).unwrap();
        fs.write_atomic(&p("top.md"), b"x").unwrap();
        let dirs: Vec<_> = fs
            .list_dirs()
            .unwrap()
            .iter()
            .map(|d| d.to_string())
            .collect();
        assert_eq!(dirs, ["a", "a/b", "empty", "empty/inner"]);
        assert!(fs.is_dir(&p("a/b")) && !fs.is_dir(&p("top.md")) && !fs.is_dir(&p("nope")));
    }

    #[test]
    fn removing_a_file_can_tidy_its_emptied_parents_and_only_those() {
        let dir = tempfile::tempdir().unwrap();
        let fs = DiskFs::new(dir.path());
        fs.write_atomic(&p("a/b/c/note.md"), b"x").unwrap();
        fs.write_atomic(&p("a/other.md"), b"x").unwrap();
        fs.create_dir(&p("a/b/made-on-purpose")).unwrap();
        fs.remove(&p("a/b/c/note.md")).unwrap();
        fs.remove_empty_parents(&p("a/b/c/note.md")).unwrap();
        assert!(!dir.path().join("a/b/c").exists(), "c was emptied");
        assert!(
            dir.path().join("a/b/made-on-purpose").exists(),
            "so b stays, and so does what was made on purpose"
        );

        fs.write_atomic(&p("x/y/z.md"), b"x").unwrap();
        fs.remove(&p("x/y/z.md")).unwrap();
        fs.remove_empty_parents(&p("x/y/z.md")).unwrap();
        assert!(
            !dir.path().join("x").exists(),
            "the whole emptied chain goes"
        );
        assert!(dir.path().join("a/other.md").exists());
        fs.remove_empty_parents(&p("top-level.md")).unwrap(); // nothing above a root file: fine
    }

    #[test]
    fn folders_come_with_when_they_were_created() {
        let dir = tempfile::tempdir().unwrap();
        let fs = DiskFs::new(dir.path());
        fs.create_dir(&p("first")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        fs.create_dir(&p("second")).unwrap();
        let listed = fs.list_dirs_with_created().unwrap();
        let time = |name: &str| listed.iter().find(|(d, _)| d.as_str() == name).unwrap().1;
        assert!(time("second") > time("first"));
        assert!(time("first") > SystemTime::now() - std::time::Duration::from_secs(60));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_with_a_backslash_in_its_name_is_left_out_of_the_listing_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let fs = DiskFs::new(dir.path());
        fs.write_atomic(&p("fine.md"), b"x").unwrap();
        std::fs::write(dir.path().join("odd\\name.md"), b"x").unwrap(); // legal on macOS and Linux
        let listed: Vec<_> = fs
            .list()
            .unwrap()
            .iter()
            .map(|e| e.path.to_string())
            .collect();
        assert_eq!(listed, ["fine.md"]);
    }

    #[test]
    fn a_folder_cannot_be_made_where_a_file_is() {
        let dir = tempfile::tempdir().unwrap();
        let fs = DiskFs::new(dir.path());
        fs.write_atomic(&p("a"), b"x").unwrap();
        assert!(matches!(fs.create_dir(&p("a/b")), Err(Error::Io { .. })));
    }

    #[test]
    fn emptied_folders_are_removed_but_never_ones_that_still_hold_something() {
        let dir = tempfile::tempdir().unwrap();
        let fs = DiskFs::new(dir.path());
        fs.create_dir(&p("gone/deeper/deepest")).unwrap();
        std::fs::write(dir.path().join("gone/.DS_Store"), b"junk").unwrap();
        fs.create_dir(&p("kept/sub")).unwrap();
        fs.write_atomic(&p("kept/sub/note.md"), b"x").unwrap();
        fs.create_dir(&p("repo/.git")).unwrap();

        fs.remove_empty_dirs(&p("gone")).unwrap();
        fs.remove_empty_dirs(&p("kept")).unwrap();
        fs.remove_empty_dirs(&p("repo")).unwrap();
        fs.remove_empty_dirs(&p("never-existed")).unwrap();

        assert!(
            !dir.path().join("gone").exists(),
            "empty, with a .DS_Store, goes"
        );
        assert!(dir.path().join("kept/sub/note.md").exists());
        assert!(
            dir.path().join("repo/.git").exists(),
            "a hidden folder inside counts as held"
        );
    }
}
