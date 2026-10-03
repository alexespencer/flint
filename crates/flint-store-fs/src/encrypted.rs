use flint_core::StorePath;
use flint_crypto::StoreKeys;

use crate::{Error, FileEntry, LocalFs, Result, is_note_path};

/// The app's view of a store: note contents are encrypted on the way to the folder and decrypted on
/// the way back. Only note paths are accepted ([`is_note_path`]); `.store/`, `.trash/` and `.obsidian/`
/// are not the app's to read or write through here.
///
/// Sync does not use this type: it talks to the inner [`LocalFs`] (see [`raw`](Self::raw)) so it
/// moves exactly the ciphertext that is on disk.
pub struct EncryptedFs<L> {
    fs: L,
    keys: StoreKeys,
}

impl<L: LocalFs> EncryptedFs<L> {
    pub fn new(fs: L, keys: StoreKeys) -> Self {
        Self { fs, keys }
    }

    /// The same folder as raw bytes, for sync.
    pub fn raw(&self) -> &L {
        &self.fs
    }

    pub fn keys(&self) -> &StoreKeys {
        &self.keys
    }

    /// Every note, sorted. `size` and `modified` describe the stored ciphertext.
    pub fn list(&self) -> Result<Vec<FileEntry>> {
        Ok(self
            .fs
            .list()?
            .into_iter()
            .filter(|e| is_note_path(&e.path))
            .collect())
    }

    pub fn exists(&self, path: &StorePath) -> Result<bool> {
        note(path)?;
        self.fs.exists(path)
    }

    /// Decrypts a note. Fails with a crypto error if the file is corrupt, was tampered with, or was
    /// copied to this path from another one.
    pub fn read(&self, path: &StorePath) -> Result<Vec<u8>> {
        note(path)?;
        Ok(self.keys.open(path, &self.fs.read(path)?)?)
    }

    /// Encrypts and writes a note atomically. Every write uses a fresh nonce, so writing identical
    /// contents twice still changes the bytes on disk.
    pub fn write(&self, path: &StorePath, plaintext: &[u8]) -> Result<()> {
        note(path)?;
        self.fs
            .write_atomic(path, &self.keys.seal(path, plaintext)?)
    }

    /// Renames a note. The path is part of what is encrypted, so the contents are decrypted and
    /// sealed again under the new name (a plain file move would not open). The new file is written
    /// before the old one is removed, so an interrupted rename leaves a duplicate, never a loss.
    pub fn rename(&self, from: &StorePath, to: &StorePath) -> Result<()> {
        note(from)?;
        note(to)?;
        if !self.fs.exists(from)? {
            return Err(Error::NotFound(from.clone()));
        }
        if from == to {
            return Ok(());
        }
        if self.fs.exists(to)? {
            return Err(Error::AlreadyExists(to.clone()));
        }
        let plaintext = self.read(from)?;
        self.write(to, &plaintext)?;
        self.fs.remove(from)
    }

    pub fn remove(&self, path: &StorePath) -> Result<()> {
        note(path)?;
        self.fs.remove(path)
    }

    /// Moves a note into `.trash/`, still encrypted (and so unreadable from there until restored),
    /// and returns its new path.
    pub fn trash(&self, path: &StorePath) -> Result<StorePath> {
        note(path)?;
        self.fs.trash(path)
    }
}

fn note(path: &StorePath) -> Result<()> {
    if is_note_path(path) {
        Ok(())
    } else {
        Err(Error::NotANotePath(path.clone()))
    }
}

#[cfg(test)]
mod tests {
    use flint_crypto::{KdfParams, KeyFile};
    use tempfile::TempDir;

    use super::*;
    use crate::{DiskFs, MemFs};

    fn p(s: &str) -> StorePath {
        StorePath::new(s).unwrap()
    }

    fn keys() -> StoreKeys {
        KeyFile::create(
            "pw",
            KdfParams {
                m_kib: 8,
                t: 1,
                p: 1,
            },
        )
        .unwrap()
        .1
    }

    fn mem() -> EncryptedFs<MemFs> {
        EncryptedFs::new(MemFs::new(), keys())
    }

    #[test]
    fn write_then_read_round_trips() {
        let fs = mem();
        fs.write(&p("notes/a.md"), b"# Title\n\nbody").unwrap();
        assert_eq!(fs.read(&p("notes/a.md")).unwrap(), b"# Title\n\nbody");
    }

    #[test]
    fn what_is_on_disk_is_ciphertext() {
        let dir = TempDir::new().unwrap();
        let fs = EncryptedFs::new(DiskFs::new(dir.path()), keys());
        let marker = b"PLAINTEXT-MARKER-4f9a1c";
        fs.write(&p("secret.md"), marker).unwrap();

        let on_disk = std::fs::read(dir.path().join("secret.md")).unwrap();
        assert_eq!(&on_disk[..4], b"FLNT");
        assert!(!on_disk.windows(marker.len()).any(|w| w == marker));
        assert_eq!(
            fs.raw().read(&p("secret.md")).unwrap(),
            on_disk,
            "sync sees the same ciphertext"
        );
    }

    #[test]
    fn writing_identical_contents_twice_changes_the_bytes() {
        let fs = mem();
        fs.write(&p("a.md"), b"same").unwrap();
        let first = fs.raw().read(&p("a.md")).unwrap();
        fs.write(&p("a.md"), b"same").unwrap();
        assert_ne!(first, fs.raw().read(&p("a.md")).unwrap());
    }

    #[test]
    fn a_file_copied_to_another_name_will_not_open() {
        let fs = mem();
        fs.write(&p("a.md"), b"secret").unwrap();
        let ciphertext = fs.raw().read(&p("a.md")).unwrap();
        fs.raw().write_atomic(&p("b.md"), &ciphertext).unwrap();
        assert!(matches!(
            fs.read(&p("b.md")),
            Err(Error::Crypto(flint_crypto::Error::Decrypt))
        ));
        assert_eq!(fs.read(&p("a.md")).unwrap(), b"secret");
    }

    #[test]
    fn a_plaintext_file_dropped_into_the_store_is_reported_not_returned() {
        let fs = mem();
        fs.raw()
            .write_atomic(&p("stray.md"), b"# not encrypted")
            .unwrap();
        assert!(matches!(
            fs.read(&p("stray.md")),
            Err(Error::Crypto(flint_crypto::Error::BadHeader))
        ));
    }

    #[test]
    fn another_stores_files_will_not_open() {
        let fs = mem();
        fs.write(&p("a.md"), b"mine").unwrap();
        let theirs = EncryptedFs::new(MemFs::new(), keys());
        theirs
            .raw()
            .write_atomic(&p("a.md"), &fs.raw().read(&p("a.md")).unwrap())
            .unwrap();
        assert!(matches!(theirs.read(&p("a.md")), Err(Error::Crypto(_))));
    }

    #[test]
    fn rename_moves_the_contents_and_removes_the_old_name() {
        let fs = mem();
        fs.write(&p("old/a.md"), b"contents").unwrap();
        fs.rename(&p("old/a.md"), &p("new/deeper/b.md")).unwrap();
        assert_eq!(fs.read(&p("new/deeper/b.md")).unwrap(), b"contents");
        assert!(!fs.exists(&p("old/a.md")).unwrap());
        assert_eq!(fs.list().unwrap().len(), 1);
    }

    #[test]
    fn rename_refuses_to_overwrite_and_to_invent_files() {
        let fs = mem();
        fs.write(&p("a.md"), b"A").unwrap();
        fs.write(&p("b.md"), b"B").unwrap();
        assert!(matches!(
            fs.rename(&p("a.md"), &p("b.md")),
            Err(Error::AlreadyExists(_))
        ));
        assert!(matches!(
            fs.rename(&p("nope.md"), &p("c.md")),
            Err(Error::NotFound(_))
        ));
        assert_eq!(fs.read(&p("a.md")).unwrap(), b"A");
        assert_eq!(fs.read(&p("b.md")).unwrap(), b"B");
        fs.rename(&p("a.md"), &p("a.md")).unwrap();
        assert_eq!(fs.read(&p("a.md")).unwrap(), b"A");
    }

    #[test]
    fn list_shows_notes_only() {
        let fs = mem();
        fs.write(&p("a.md"), b"A").unwrap();
        fs.raw().write_atomic(&p(crate::KEY_FILE), b"{}").unwrap();
        fs.raw().write_atomic(&p(".store/index.db"), b"x").unwrap();
        fs.raw()
            .write_atomic(&p(".obsidian/app.json"), b"{}")
            .unwrap();
        let listed: Vec<_> = fs
            .list()
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        assert_eq!(listed, ["a.md"]);
        assert_eq!(fs.raw().list().unwrap().len(), 2, "sync also sees key.json");
    }

    #[test]
    fn paths_that_are_not_notes_are_refused() {
        let fs = mem();
        for path in [
            crate::KEY_FILE,
            ".store/index.db",
            ".trash/a.md",
            ".obsidian/app.json",
            ".DS_Store",
        ] {
            assert!(
                matches!(fs.write(&p(path), b"x"), Err(Error::NotANotePath(_))),
                "{path}"
            );
            assert!(
                matches!(fs.read(&p(path)), Err(Error::NotANotePath(_))),
                "{path}"
            );
            assert!(
                matches!(fs.rename(&p("a.md"), &p(path)), Err(Error::NotANotePath(_))),
                "{path}"
            );
        }
    }

    #[test]
    fn trash_keeps_the_ciphertext_and_hides_the_note() {
        let fs = mem();
        fs.write(&p("a.md"), b"A").unwrap();
        let trashed = fs.trash(&p("a.md")).unwrap();
        assert_eq!(trashed, p(".trash/a.md"));
        assert!(fs.list().unwrap().is_empty());
        assert!(matches!(fs.read(&p("a.md")), Err(Error::NotFound(_))));
        assert_eq!(&fs.raw().read(&trashed).unwrap()[..4], b"FLNT");
    }

    #[test]
    fn remove_deletes() {
        let fs = mem();
        fs.write(&p("a.md"), b"A").unwrap();
        fs.remove(&p("a.md")).unwrap();
        assert!(matches!(fs.read(&p("a.md")), Err(Error::NotFound(_))));
        assert!(matches!(fs.remove(&p("a.md")), Err(Error::NotFound(_))));
    }
}
