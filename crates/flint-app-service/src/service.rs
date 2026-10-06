use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use flint_core::{ContentHash, StorePath};
use flint_crypto::{KdfParams, KeyFile, SNIFF_LEN, StoreKeys, is_sealed};
use flint_index::{Backlink, Index, SearchHit};
use flint_registry::{Registry, RemoteSettings, SecretStore, StoreId, StoreRecord};
use flint_remote::{ETag, Precondition, RemoteStore};
use flint_store_fs::{AdoptReport, DiskFs, EncryptedFs, KEY_FILE, LocalFs, is_note_path};
use flint_sync::{DeviceName, KeysCodec, Refusal, SyncReport, Syncer};
use flint_sync_state::{Baseline, SyncState};

use crate::indexing::{self, IndexStats};
use crate::{Connect, Error, Result};

/// How long a dropped-in file must have been quiet before it is encrypted (another program may still
/// be writing it).
const QUIET_PERIOD: Duration = Duration::from_secs(2);

/// What a new store, or a join, needs. The password is used once and not kept.
pub struct NewStore {
    pub name: String,
    pub folder: PathBuf,
    pub remote: RemoteSettings,
    pub s3_secret: String,
    pub password: String,
}

/// What one sync did, start to finish.
#[derive(Debug)]
pub struct SyncOutcome {
    /// Files dropped into the folder that were encrypted first.
    pub adopted: AdoptReport,
    pub report: SyncReport,
    pub indexed: IndexStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictInfo {
    pub path: StorePath,
    /// Where the other version was written; `None` if it was deleted on the other side.
    pub copy: Option<StorePath>,
}

#[derive(Debug)]
pub struct Status {
    pub id: StoreId,
    pub name: String,
    pub folder: PathBuf,
    pub unlocked: bool,
    /// Synced files in the folder, not counting `key.json`.
    pub files: usize,
    /// Files in the folder that are not encrypted yet (dropped in, waiting for the next sync or adopt).
    pub awaiting_encryption: Vec<StorePath>,
    pub conflicts: Vec<ConflictInfo>,
}

pub struct AppService<S, C> {
    registry: Registry<S>,
    connect: C,
    device: DeviceName,
    kdf: KdfParams,
    quiet_period: Duration,
}

impl<S: SecretStore, C: Connect> AppService<S, C> {
    pub fn new(registry: Registry<S>, connect: C, device: DeviceName) -> Self {
        Self {
            registry,
            connect,
            device,
            kdf: KdfParams::DEFAULT,
            quiet_period: QUIET_PERIOD,
        }
    }

    /// Replaces the password-hashing cost used for new key files. For tests only: the default is
    /// deliberately slow.
    pub fn with_kdf(mut self, kdf: KdfParams) -> Self {
        self.kdf = kdf;
        self
    }

    pub fn with_quiet_period(mut self, quiet_period: Duration) -> Self {
        self.quiet_period = quiet_period;
        self
    }

    pub fn registry(&self) -> &Registry<S> {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut Registry<S> {
        &mut self.registry
    }

    // ---- stores ---------------------------------------------------------------------------------

    /// The S3 secret key saved for another store, so a new store on the same account does not need it
    /// typed again. The key goes straight from the secret store into the new store's [`NewStore`]; it
    /// is never shown to a user interface.
    pub fn reuse_s3_secret(&self, from: &StoreId) -> Result<String> {
        self.registry.get(from)?;
        self.registry
            .s3_secret(from)?
            .ok_or_else(|| Error::MissingS3Secret(from.clone()))
    }

    /// Creates a brand-new store: a fresh key file, written to the folder and uploaded as the store's
    /// first object. Refuses if the bucket and prefix already hold a store, so one can never be
    /// overwritten. The store is left unlocked. Does not sync; files already in the folder are
    /// encrypted and uploaded by the first [`sync`](Self::sync).
    pub async fn create_store(&mut self, new: NewStore) -> Result<StoreId> {
        let record = self.record_for(&new);
        let remote = self.connect.connect(&record.remote, &new.s3_secret)?;
        if remote.list().await?.iter().any(|e| is_key_file(&e.path)) {
            return Err(Error::StoreExists);
        }
        let disk = DiskFs::new(&record.folder);
        if disk.exists(&key_path())? {
            return Err(Error::FolderIsAStore(KEY_FILE));
        }
        self.registry.add(record.clone())?;

        let created = self.finish_create(&record, &remote, &disk, &new).await;
        if created.is_err() {
            // leave nothing half-made behind
            let _ = disk.remove(&key_path());
            let _ = self.registry.remove(&record.id);
        }
        created.map(|()| record.id)
    }

    async fn finish_create(
        &self,
        record: &StoreRecord,
        remote: &C::Remote,
        disk: &DiskFs,
        new: &NewStore,
    ) -> Result<()> {
        let (key_file, keys) = KeyFile::create(&new.password, self.kdf)?;
        let json = key_file.to_json().into_bytes();
        disk.write_atomic(&key_path(), &json)?;
        let entry = match remote
            .put(
                &key_path(),
                Bytes::from(json.clone()),
                &ContentHash::of(&json),
                Precondition::MustNotExist,
            )
            .await
        {
            Ok(entry) => entry,
            Err(flint_remote::Error::PreconditionFailed(_)) => return Err(Error::StoreExists),
            Err(e) => return Err(e.into()),
        };
        self.agree_on_key_file(&record.id, &json, entry.etag)?;
        self.registry.set_s3_secret(&record.id, &new.s3_secret)?;
        self.registry.unlock(&record.id, &keys)?;
        Ok(())
    }

    /// Adds a store that already exists on S3 (made by another device) to this one, using only the
    /// bucket details and the password. Downloads the key file and checks the password against it; the
    /// notes themselves arrive with the first [`sync`](Self::sync). Leaves the store unlocked.
    pub async fn join_store(&mut self, new: NewStore) -> Result<StoreId> {
        let record = self.record_for(&new);
        let remote = self.connect.connect(&record.remote, &new.s3_secret)?;
        if !remote.list().await?.iter().any(|e| is_key_file(&e.path)) {
            return Err(Error::NoSuchStore);
        }
        let remote_key_object = remote.get(&key_path()).await?;
        let remote_key = remote_key_object.bytes.clone();
        let keys = parse_key_file(&remote_key)?.unlock(&new.password)?;

        let disk = DiskFs::new(&record.folder);
        if disk.exists(&key_path())? && disk.read(&key_path())? != remote_key.as_ref() {
            return Err(Error::FolderIsAStore(KEY_FILE)); // another store's key file is there
        }
        self.registry.add(record.clone())?;
        let joined = (|| -> Result<()> {
            disk.write_atomic(&key_path(), &remote_key)?;
            self.agree_on_key_file(
                &record.id,
                &remote_key,
                remote_key_object.entry.etag.clone(),
            )?;
            self.registry.set_s3_secret(&record.id, &new.s3_secret)?;
            self.registry.unlock(&record.id, &keys)?;
            Ok(())
        })();
        if joined.is_err() {
            let _ = self.registry.remove(&record.id);
        }
        joined.map(|()| record.id)
    }

    /// Unlocks a store with its password. If the password was changed on another device and this one has
    /// not synced since, the new password is checked against the key file on S3 and, if it fits, the
    /// local key file is brought up to date.
    pub async fn unlock(&self, id: &StoreId, password: &str) -> Result<()> {
        let record = self.registry.get(id)?;
        let disk = DiskFs::new(&record.folder);
        let local = read_key_file(&disk)?;
        let keys = match parse_key_file(&local)?.unlock(password) {
            Ok(keys) => keys,
            Err(flint_crypto::Error::Unlock) => {
                let remote = self.remote_for(record)?;
                let theirs = match remote.get(&key_path()).await {
                    Ok(object) => object,
                    Err(flint_remote::Error::NotFound(_)) => return Err(Error::WrongPassword),
                    Err(e) => return Err(e.into()),
                };
                let keys = parse_key_file(&theirs.bytes)?.unlock(password)?;
                disk.write_atomic(&key_path(), &theirs.bytes)?;
                self.agree_on_key_file(id, &theirs.bytes, theirs.entry.etag)?;
                keys
            }
            Err(e) => return Err(e.into()),
        };
        self.registry.unlock(id, &keys)?;
        Ok(())
    }

    /// [`unlock`](Self::unlock), and remembers whether this store's password is the empty one, so a shell
    /// knows whether to ask for a password at all. A password that was changed on another device since is
    /// noticed here: the empty password then no longer fits, and the store is marked as having one.
    pub async fn unlock_noting(&mut self, id: &StoreId, password: &str) -> Result<()> {
        let result = self.unlock(id, password).await;
        match &result {
            Ok(()) => self.registry.set_passwordless(id, password.is_empty())?,
            Err(Error::WrongPassword) if password.is_empty() => {
                self.registry.set_passwordless(id, false)?;
            }
            Err(_) => {}
        }
        result
    }

    pub fn lock(&self, id: &StoreId) -> Result<()> {
        self.registry.get(id)?;
        Ok(self.registry.lock(id)?)
    }

    /// Forgets the keys of every store on this device.
    pub fn lock_all(&self) -> Result<()> {
        Ok(self.registry.lock_all()?)
    }

    /// [`change_password`](Self::change_password), and records that the store now has a password.
    pub async fn change_password_noting(
        &mut self,
        id: &StoreId,
        old: &str,
        new: &str,
    ) -> Result<SyncReport> {
        let report = self.change_password(id, old, new).await?;
        self.registry.set_passwordless(id, false)?;
        Ok(report)
    }

    /// Changes the store's password (the new one cannot be empty; a store made without a password can be given
    /// one by passing `""` as the old one). Syncs first, so the change is made to the newest key file, then
    /// uploads the new one conditionally. If another device changed the password at the same moment,
    /// nothing is changed here ([`Error::PasswordChangedElsewhere`]). No file is re-encrypted, and other
    /// devices keep working: the data key is the same, and they pick up the new key file on their next
    /// sync.
    pub async fn change_password(&self, id: &StoreId, old: &str, new: &str) -> Result<SyncReport> {
        if new.is_empty() {
            return Err(Error::EmptyPassword);
        }
        let first = self.sync(id).await?.report;
        if key_file_diverged(&first) {
            return Err(Error::PasswordChangedElsewhere);
        }
        let record = self.registry.get(id)?;
        let disk = DiskFs::new(&record.folder);
        let current = read_key_file(&disk)?;
        let changed = parse_key_file(&current)?.change_password(old, new, self.kdf)?;
        disk.write_atomic(&key_path(), changed.to_json().as_bytes())?;

        let second = self.sync(id).await?.report;
        if key_file_diverged(&second) {
            disk.write_atomic(&key_path(), &current)?;
            return Err(Error::PasswordChangedElsewhere);
        }
        Ok(second)
    }

    // ---- sync -----------------------------------------------------------------------------------

    /// Encrypts anything dropped into the folder, runs one sync cycle, and brings the index up to date.
    pub async fn sync(&self, id: &StoreId) -> Result<SyncOutcome> {
        let (record, keys) = self.unlocked(id)?;
        let fs = EncryptedFs::new(DiskFs::new(&record.folder), keys.clone());
        let adopted = fs.adopt_plaintext(self.quiet_period)?;
        let mut syncer = self.syncer(&record, keys.clone())?;
        let report = syncer.sync().await?;
        // files that sync moved to the trash (deleted on another device) may leave empty folders behind
        let disk = DiskFs::new(&record.folder);
        for trashed in &report.trashed_local {
            disk.remove_empty_parents(trashed)?;
        }
        let indexed = self.refresh_index(&record, &keys, &fs)?;
        Ok(SyncOutcome {
            adopted,
            report,
            indexed,
        })
    }

    /// Encrypts files dropped into the folder, without syncing. A file must have been untouched for
    /// `min_age` (use `Duration::ZERO` to take everything now).
    pub fn adopt(&self, id: &StoreId, min_age: Duration) -> Result<(AdoptReport, IndexStats)> {
        let (record, keys) = self.unlocked(id)?;
        let fs = EncryptedFs::new(DiskFs::new(&record.folder), keys.clone());
        let adopted = fs.adopt_plaintext(min_age)?;
        let indexed = self.refresh_index(&record, &keys, &fs)?;
        Ok((adopted, indexed))
    }

    /// Lets sync act on a conflicted path again, once the user has dealt with it (see
    /// [`Syncer::resolve`]), and tidies up: the other version's conflict copy is moved to `.trash`. The
    /// copy is an ordinary file that synced to the other devices, so the next sync removes it from
    /// them too (they trash their own). Nothing is lost: the copy's text is in the trash.
    pub async fn resolve(&self, id: &StoreId, path: &StorePath) -> Result<()> {
        let (record, keys) = self.unlocked(id)?;
        let mut syncer = self.syncer(&record, keys.clone())?;
        let copy = syncer.state().conflict(path)?.and_then(|c| c.copy);
        syncer.resolve(path).await?;
        let disk = DiskFs::new(&record.folder);
        if let Some(copy) = copy {
            match disk.trash(&copy) {
                Ok(_) | Err(flint_store_fs::Error::NotFound(_)) => {} // already gone: fine
                Err(e) => return Err(e.into()),
            }
        }
        let fs = EncryptedFs::new(disk, keys.clone());
        self.refresh_index(&record, &keys, &fs)?;
        Ok(())
    }

    /// What is going on with a store. Works while the store is locked.
    pub fn status(&self, id: &StoreId) -> Result<Status> {
        let record = self.registry.get(id)?;
        let disk = DiskFs::new(&record.folder);
        let mut files = 0;
        let mut awaiting_encryption = Vec::new();
        for entry in disk.list()?.into_iter().filter(|e| is_note_path(&e.path)) {
            files += 1;
            let start = disk.read_prefix(&entry.path, SNIFF_LEN)?;
            if !is_sealed(&start, entry.size) {
                awaiting_encryption.push(entry.path);
            }
        }
        let state = SyncState::open(&self.registry.state_dir(id)?.join("sync.db"))?;
        let conflicts = state
            .conflicts()?
            .into_iter()
            .map(|c| ConflictInfo {
                path: c.path,
                copy: c.copy,
            })
            .collect();
        Ok(Status {
            id: id.clone(),
            name: record.name.clone(),
            folder: record.folder.clone(),
            unlocked: self.registry.is_unlocked(id)?,
            files,
            awaiting_encryption,
            conflicts,
        })
    }

    // ---- notes ----------------------------------------------------------------------------------

    /// Every note and attachment in the store.
    pub fn files(&self, id: &StoreId) -> Result<Vec<StorePath>> {
        let (record, keys) = self.unlocked(id)?;
        let fs = EncryptedFs::new(DiskFs::new(&record.folder), keys);
        Ok(fs.list()?.into_iter().map(|e| e.path).collect())
    }

    pub fn read_note(&self, id: &StoreId, path: &StorePath) -> Result<String> {
        let (record, keys) = self.unlocked(id)?;
        let fs = EncryptedFs::new(DiskFs::new(&record.folder), keys);
        if !fs.exists(path)? {
            return Err(Error::NotANote(path.clone()));
        }
        Ok(String::from_utf8_lossy(&fs.read(path)?).into_owned())
    }

    /// Writes a note (encrypted, atomically) and updates the index.
    pub fn save_note(&self, id: &StoreId, path: &StorePath, text: &str) -> Result<()> {
        let (record, keys) = self.unlocked(id)?;
        if !is_note_path(path) {
            return Err(Error::NotANote(path.clone()));
        }
        let fs = EncryptedFs::new(DiskFs::new(&record.folder), keys.clone());
        fs.write(path, text.as_bytes())?;
        let mut index = self.index(&record, &keys)?;
        indexing::refresh_one(&fs, &mut index, path)?;
        Ok(())
    }

    pub fn search(&self, id: &StoreId, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let (record, keys) = self.unlocked(id)?;
        Ok(self.index(&record, &keys)?.search(query, limit)?)
    }

    /// Where a `[[link]]` written in `from` points, or `None` if no file matches it.
    pub fn resolve_link(
        &self,
        id: &StoreId,
        from: &StorePath,
        target: &str,
    ) -> Result<Option<StorePath>> {
        let (record, keys) = self.unlocked(id)?;
        Ok(self.index(&record, &keys)?.resolve(from, target)?)
    }

    pub fn backlinks(&self, id: &StoreId, path: &StorePath) -> Result<Vec<Backlink>> {
        let (record, keys) = self.unlocked(id)?;
        Ok(self.index(&record, &keys)?.backlinks(path)?)
    }

    /// Rebuilds what the index knows from the files on disk (it is only a cache of them).
    pub fn reindex(&self, id: &StoreId) -> Result<IndexStats> {
        let (record, keys) = self.unlocked(id)?;
        let fs = EncryptedFs::new(DiskFs::new(&record.folder), keys.clone());
        self.refresh_index(&record, &keys, &fs)
    }

    // ---- internals ------------------------------------------------------------------------------

    /// Records that this device's `key.json` is exactly the remote's. Without it, a password change made
    /// elsewhere before this device's first sync would look like both sides changing the file.
    fn agree_on_key_file(&self, id: &StoreId, bytes: &[u8], etag: Option<ETag>) -> Result<()> {
        let mut state = SyncState::open(&self.registry.state_dir(id)?.join("sync.db"))?;
        state.record(&Baseline {
            path: key_path(),
            hash: ContentHash::of(bytes),
            etag: etag.map(|e| e.as_str().to_owned()),
            stamp: None,
        })?;
        Ok(())
    }

    fn record_for(&self, new: &NewStore) -> StoreRecord {
        StoreRecord {
            id: StoreId::generate(),
            name: new.name.trim().to_owned(),
            folder: new.folder.clone(),
            remote: new.remote.clone(),
            // an empty password given here either creates a store that has none, or, when joining, is checked
            // against the key file before the store is added at all
            passwordless: new.password.is_empty(),
        }
    }

    pub(crate) fn unlocked(&self, id: &StoreId) -> Result<(StoreRecord, StoreKeys)> {
        let record = self.registry.get(id)?.clone();
        let keys = self
            .registry
            .unlocked_keys(id)?
            .ok_or_else(|| Error::Locked(id.clone()))?;
        Ok((record, keys))
    }

    fn remote_for(&self, record: &StoreRecord) -> Result<C::Remote> {
        let secret = self
            .registry
            .s3_secret(&record.id)?
            .ok_or_else(|| Error::MissingS3Secret(record.id.clone()))?;
        Ok(self.connect.connect(&record.remote, &secret)?)
    }

    fn syncer(
        &self,
        record: &StoreRecord,
        keys: StoreKeys,
    ) -> Result<Syncer<DiskFs, C::Remote, KeysCodec>> {
        let state = SyncState::open(&self.registry.state_dir(&record.id)?.join("sync.db"))?;
        Ok(Syncer::new(
            DiskFs::new(&record.folder),
            self.remote_for(record)?,
            KeysCodec(keys),
            state,
            self.device.clone(),
        ))
    }

    fn index(&self, record: &StoreRecord, keys: &StoreKeys) -> Result<Index> {
        let path = self.registry.state_dir(&record.id)?.join("index.db");
        Ok(Index::open(&path, keys.index_key())?)
    }

    pub(crate) fn refresh_index(
        &self,
        record: &StoreRecord,
        keys: &StoreKeys,
        fs: &EncryptedFs<DiskFs>,
    ) -> Result<IndexStats> {
        indexing::refresh(fs, &mut self.index(record, keys)?)
    }
}

fn key_path() -> StorePath {
    StorePath::new(KEY_FILE).expect("a valid path")
}

fn is_key_file(path: &StorePath) -> bool {
    path.as_str() == KEY_FILE
}

fn read_key_file(disk: &DiskFs) -> Result<Vec<u8>> {
    match disk.read(&key_path()) {
        Ok(bytes) => Ok(bytes),
        Err(flint_store_fs::Error::NotFound(_)) => Err(Error::MissingKeyFile(KEY_FILE)),
        Err(e) => Err(e.into()),
    }
}

fn parse_key_file(bytes: &[u8]) -> Result<KeyFile> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| flint_crypto::Error::KeyFile("not text".into()))?;
    Ok(KeyFile::from_json(text)?)
}

fn key_file_diverged(report: &SyncReport) -> bool {
    report
        .refused
        .iter()
        .any(|(path, why)| is_key_file(path) && *why == Refusal::KeyFileDiverged)
}
