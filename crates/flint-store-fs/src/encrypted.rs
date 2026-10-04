use std::time::{Duration, SystemTime};

use flint_core::{ContentHash, StorePath};
use flint_crypto::{SNIFF_LEN, StoreKeys, is_sealed};

use crate::{Error, FileEntry, LocalFs, Result, is_note_path};

/// What an adoption pass did. Files that are already ciphertext are not mentioned.
#[derive(Debug, Default)]
pub struct AdoptReport {
    /// Plaintext files that were encrypted in place.
    pub adopted: Vec<StorePath>,
    /// Plaintext files left alone for now: modified too recently (probably still being copied), or
    /// changed while being encrypted. Run the pass again later.
    pub waiting: Vec<StorePath>,
    /// Files that could not be adopted. The pass carries on past them.
    pub failed: Vec<(StorePath, Error)>,
}

enum Adoption {
    AlreadySealed,
    Adopted,
    Waiting,
}

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

    /// Encrypts, in place, every plaintext file in the store folder: the files a user drags into it.
    /// A file counts as plaintext if it does not start with Flint's header ([`is_sealed`]); one that
    /// does is never touched, even if damaged, so ciphertext is never encrypted twice. Hidden files
    /// (see [`is_note_path`]) are ignored.
    ///
    /// Plaintext modified less than `min_age` ago is left for a later pass, because another program
    /// may still be writing it, and a file that changes while it is being encrypted is also left.
    /// Safe to run any time and as often as wanted; a pass over an already-encrypted store reads
    /// only a few bytes of each file.
    ///
    /// Only call this for files that came from the user. Anything downloaded by sync must be
    /// validated as ciphertext instead, or a hostile server could plant "plaintext" notes.
    pub fn adopt_plaintext(&self, min_age: Duration) -> Result<AdoptReport> {
        let entries = self
            .fs
            .list()?
            .into_iter()
            .filter(|e| is_note_path(&e.path));
        Ok(self.adopt_entries(entries, min_age))
    }

    /// [`adopt_plaintext`](Self::adopt_plaintext) for just these paths (what a file watcher reports).
    /// Paths that are not notes, or no longer exist, are skipped.
    pub fn adopt_paths(&self, paths: &[StorePath], min_age: Duration) -> Result<AdoptReport> {
        let mut entries = Vec::new();
        for path in paths.iter().filter(|p| is_note_path(p)) {
            match self.fs.entry(path) {
                Ok(entry) => entries.push(entry),
                Err(Error::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(self.adopt_entries(entries, min_age))
    }

    fn adopt_entries(
        &self,
        entries: impl IntoIterator<Item = FileEntry>,
        min_age: Duration,
    ) -> AdoptReport {
        let now = SystemTime::now();
        let mut report = AdoptReport::default();
        for entry in entries {
            match self.adopt_one(&entry, now, min_age) {
                Ok(Adoption::AlreadySealed) => {}
                Ok(Adoption::Adopted) => report.adopted.push(entry.path),
                Ok(Adoption::Waiting) => report.waiting.push(entry.path),
                Err(error) => report.failed.push((entry.path, error)),
            }
        }
        report
    }

    fn adopt_one(&self, entry: &FileEntry, now: SystemTime, min_age: Duration) -> Result<Adoption> {
        let start = self.fs.read_prefix(&entry.path, SNIFF_LEN)?;
        if is_sealed(&start, entry.size) {
            return Ok(Adoption::AlreadySealed);
        }
        // A modification time in the future (clock skew) also counts as "too recent".
        if now
            .duration_since(entry.modified)
            .map_or(true, |age| age < min_age)
        {
            return Ok(Adoption::Waiting);
        }
        let plaintext = self.fs.read(&entry.path)?;
        let before = ContentHash::of(&plaintext);
        let sealed = self.keys.seal(&entry.path, &plaintext)?;
        // If another program wrote to it while we were encrypting, our version is stale: do not
        // replace their newer text with our encryption of the old text.
        if self.fs.hash(&entry.path)? != before {
            return Ok(Adoption::Waiting);
        }
        self.fs.write_atomic(&entry.path, &sealed)?;
        Ok(Adoption::Adopted)
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

    // ---- adopting plaintext files dropped into the folder --------------------------------------

    const NOW: Duration = Duration::ZERO;

    fn drop_in(fs: &EncryptedFs<MemFs>, path: &str, bytes: &[u8]) {
        fs.raw().write_atomic(&p(path), bytes).unwrap();
    }

    fn raw(fs: &EncryptedFs<MemFs>, path: &str) -> Vec<u8> {
        fs.raw().read(&p(path)).unwrap()
    }

    fn paths(list: &[StorePath]) -> Vec<&str> {
        list.iter().map(|p| p.as_str()).collect()
    }

    #[test]
    fn a_dropped_in_plaintext_note_is_encrypted_in_place_and_readable() {
        let fs = mem();
        drop_in(
            &fs,
            "Characters/Baron.md",
            b"# Baron\n\nA [[Vallaki]] noble.",
        );
        let report = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(paths(&report.adopted), ["Characters/Baron.md"]);
        assert!(report.waiting.is_empty() && report.failed.is_empty());

        assert_eq!(&raw(&fs, "Characters/Baron.md")[..4], b"FLNT");
        assert_eq!(
            fs.read(&p("Characters/Baron.md")).unwrap(),
            b"# Baron\n\nA [[Vallaki]] noble."
        );
    }

    #[test]
    fn attachments_and_empty_files_are_adopted_too() {
        let fs = mem();
        let png = [&b"\x89PNG\r\n\x1a\n"[..], &[0, 1, 2, 255, 254, 0, 0]].concat();
        drop_in(&fs, "Images/pic.png", &png);
        drop_in(&fs, "empty.md", b"");
        let report = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(paths(&report.adopted), ["Images/pic.png", "empty.md"]);
        assert_eq!(fs.read(&p("Images/pic.png")).unwrap(), png);
        assert_eq!(fs.read(&p("empty.md")).unwrap(), b"");
    }

    #[test]
    fn adoption_never_touches_ciphertext_and_a_second_pass_does_nothing() {
        let fs = mem();
        fs.write(&p("already.md"), b"written through the app")
            .unwrap();
        let sealed_before = raw(&fs, "already.md");
        drop_in(&fs, "dropped.md", b"plain");

        let first = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(paths(&first.adopted), ["dropped.md"]);
        assert_eq!(
            raw(&fs, "already.md"),
            sealed_before,
            "no re-encryption, so the nonce does not churn"
        );

        let adopted_bytes = raw(&fs, "dropped.md");
        let second = fs.adopt_plaintext(NOW).unwrap();
        assert!(second.adopted.is_empty() && second.waiting.is_empty() && second.failed.is_empty());
        assert_eq!(raw(&fs, "dropped.md"), adopted_bytes);
    }

    #[test]
    fn dropping_a_plaintext_file_over_an_existing_note_replaces_it_as_an_edit() {
        let fs = mem();
        fs.write(&p("a.md"), b"old contents").unwrap();
        drop_in(&fs, "a.md", b"new contents typed in another editor");
        let report = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(paths(&report.adopted), ["a.md"]);
        assert_eq!(
            fs.read(&p("a.md")).unwrap(),
            b"new contents typed in another editor"
        );
    }

    #[test]
    fn hidden_files_the_key_file_and_other_stores_internals_are_left_alone() {
        let fs = mem();
        for path in [
            ".git/config",
            ".git/objects/ab/cdef",
            ".claude/x.md",
            ".hidden.md",
            "folder/.hidden/y.md",
            ".DS_Store",
            ".trash/old.md",
            ".obsidian/app.json",
            crate::KEY_FILE,
            ".store/sync.db",
        ] {
            drop_in(&fs, path, b"plaintext that must stay as it is");
        }
        drop_in(&fs, "visible.md", b"adopt me");
        let report = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(paths(&report.adopted), ["visible.md"]);
        for path in [
            ".git/config",
            ".claude/x.md",
            ".hidden.md",
            crate::KEY_FILE,
            ".store/sync.db",
        ] {
            assert_eq!(
                raw(&fs, path),
                b"plaintext that must stay as it is",
                "{path}"
            );
        }
    }

    #[test]
    fn recently_modified_plaintext_waits_for_the_quiet_period() {
        let fs = mem();
        drop_in(&fs, "copying.md", b"half of a big copy");
        let report = fs.adopt_plaintext(Duration::from_secs(3600)).unwrap();
        assert!(report.adopted.is_empty());
        assert_eq!(paths(&report.waiting), ["copying.md"]);
        assert_eq!(
            raw(&fs, "copying.md"),
            b"half of a big copy",
            "untouched while it waits"
        );

        let later = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(paths(&later.adopted), ["copying.md"]);
    }

    #[test]
    fn a_file_that_changes_while_being_encrypted_is_not_overwritten() {
        // Another program appends to the file between our read and our write.
        struct Racy {
            inner: MemFs,
            interfere: std::cell::Cell<bool>,
        }
        impl LocalFs for Racy {
            fn list(&self) -> Result<Vec<FileEntry>> {
                self.inner.list()
            }
            fn exists(&self, path: &StorePath) -> Result<bool> {
                self.inner.exists(path)
            }
            fn read(&self, path: &StorePath) -> Result<Vec<u8>> {
                self.inner.read(path)
            }
            fn read_prefix(&self, path: &StorePath, len: usize) -> Result<Vec<u8>> {
                self.inner.read_prefix(path, len)
            }
            fn entry(&self, path: &StorePath) -> Result<FileEntry> {
                self.inner.entry(path)
            }
            fn write_atomic(&self, path: &StorePath, bytes: &[u8]) -> Result<()> {
                self.inner.write_atomic(path, bytes)
            }
            fn remove(&self, path: &StorePath) -> Result<()> {
                self.inner.remove(path)
            }
            fn move_file(&self, from: &StorePath, to: &StorePath) -> Result<()> {
                self.inner.move_file(from, to)
            }
            fn hash(&self, path: &StorePath) -> Result<ContentHash> {
                if self.interfere.replace(false) {
                    self.inner
                        .write_atomic(path, b"the first half, and then the rest arrived")
                        .unwrap();
                }
                self.inner.hash(path)
            }
        }

        let fs = EncryptedFs::new(
            Racy {
                inner: MemFs::new(),
                interfere: std::cell::Cell::new(true),
            },
            keys(),
        );
        fs.raw()
            .inner
            .write_atomic(&p("a.md"), b"the first half")
            .unwrap();
        let report = fs.adopt_plaintext(NOW).unwrap();
        assert!(report.adopted.is_empty());
        assert_eq!(paths(&report.waiting), ["a.md"]);
        assert_eq!(
            fs.raw().inner.read(&p("a.md")).unwrap(),
            b"the first half, and then the rest arrived",
            "their newer text survives"
        );

        // the next pass finds it stable and adopts the complete text
        let again = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(paths(&again.adopted), ["a.md"]);
        assert_eq!(
            fs.read(&p("a.md")).unwrap(),
            b"the first half, and then the rest arrived"
        );
    }

    #[test]
    fn damaged_ciphertext_is_not_mistaken_for_plaintext() {
        let fs = mem();
        fs.write(&p("a.md"), b"precious").unwrap();
        let mut damaged = raw(&fs, "a.md");
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        drop_in(&fs, "a.md", &damaged);

        let report = fs.adopt_plaintext(NOW).unwrap();
        assert!(report.adopted.is_empty() && report.failed.is_empty());
        assert_eq!(raw(&fs, "a.md"), damaged, "left exactly as it was");
        assert!(matches!(
            fs.read(&p("a.md")),
            Err(Error::Crypto(flint_crypto::Error::Decrypt))
        ));
    }

    #[test]
    fn a_short_plaintext_file_that_starts_with_the_magic_is_still_plaintext() {
        let fs = mem();
        drop_in(&fs, "a.md", b"FLNT is the name of a thing");
        let report = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(
            paths(&report.adopted),
            ["a.md"],
            "too short to be one of ours"
        );
        assert_eq!(fs.read(&p("a.md")).unwrap(), b"FLNT is the name of a thing");
    }

    #[test]
    fn adopt_paths_handles_only_the_named_files() {
        let fs = mem();
        drop_in(&fs, "one.md", b"1");
        drop_in(&fs, "two.md", b"2");
        drop_in(&fs, ".git/config", b"hidden");
        fs.write(&p("sealed.md"), b"s").unwrap();
        let report = fs
            .adopt_paths(
                &[
                    p("one.md"),
                    p(".git/config"),
                    p("missing.md"),
                    p("sealed.md"),
                    p(crate::KEY_FILE),
                ],
                NOW,
            )
            .unwrap();
        assert_eq!(paths(&report.adopted), ["one.md"]);
        assert_eq!(raw(&fs, "two.md"), b"2", "not named, so not touched");
        assert_eq!(raw(&fs, ".git/config"), b"hidden");
    }

    #[test]
    fn a_whole_folder_dragged_into_a_store_on_disk() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // what a user's Finder drag leaves behind: notes, an image, a transcript, and hidden stuff
        for (path, bytes) in [
            ("Back story.md", &b"# Back story\n[[Characters/Baron]]"[..]),
            ("Characters/Baron.md", b"#character\nlives in [[Vallaki]]"),
            (
                "Images/Pasted image.png",
                &[0x89, b'P', b'N', b'G', 0, 1, 2, 3][..],
            ),
            ("Transcripts/2026-07-03.txt", b"transcript text"),
            (".git/HEAD", b"ref: refs/heads/main"),
            (".claude/commands/x.md", b"# command"),
            (".DS_Store", b"\0\0\0\x01Bud1"),
        ] {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, bytes).unwrap();
        }

        let fs = EncryptedFs::new(DiskFs::new(root), keys());
        let report = fs.adopt_plaintext(NOW).unwrap();
        assert_eq!(
            paths(&report.adopted),
            [
                "Back story.md",
                "Characters/Baron.md",
                "Images/Pasted image.png",
                "Transcripts/2026-07-03.txt"
            ]
        );
        assert!(
            report.waiting.is_empty() && report.failed.is_empty(),
            "{report:?}"
        );

        // encrypted on disk, readable through the app, hidden files untouched
        assert_eq!(
            &std::fs::read(root.join("Characters/Baron.md")).unwrap()[..4],
            b"FLNT"
        );
        assert_eq!(
            fs.read(&p("Characters/Baron.md")).unwrap(),
            b"#character\nlives in [[Vallaki]]"
        );
        assert_eq!(
            std::fs::read(root.join(".git/HEAD")).unwrap(),
            b"ref: refs/heads/main"
        );
        assert_eq!(
            std::fs::read(root.join(".DS_Store")).unwrap(),
            b"\0\0\0\x01Bud1"
        );
        let leftovers: Vec<_> = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".flint-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temp files left behind");

        // a second pass reads only headers and changes nothing
        let before = std::fs::read(root.join("Back story.md")).unwrap();
        assert!(fs.adopt_plaintext(NOW).unwrap().adopted.is_empty());
        assert_eq!(std::fs::read(root.join("Back story.md")).unwrap(), before);
    }
}
