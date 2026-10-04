//! Test fixtures: two devices sharing one remote, a remote that misbehaves on cue, and a local store
//! that notices (and can interfere with) what sync does to it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use bytes::Bytes;
use flint_core::{ContentHash, StorePath};
use flint_crypto::{KdfParams, KeyFile, StoreKeys};
use flint_remote::{ETag, ObjectStoreRemote, Precondition, RemoteEntry, RemoteObject, RemoteStore};
use flint_store_fs::{EncryptedFs, FileEntry, LocalFs, MemFs};
use flint_sync_state::SyncState;
use object_store::memory::InMemory;

use crate::{DeviceName, KeysCodec, SyncReport, Syncer};

pub(crate) fn p(s: &str) -> StorePath {
    StorePath::new(s).unwrap()
}

pub(crate) type Store = Arc<InMemory>;

/// A device on a plain remote, and one whose remote misbehaves on cue.
pub(crate) type MemDevice = Device<ObjectStoreRemote<Store>, Arc<MemFs>>;
pub(crate) type FaultyDevice = Device<Faulty<ObjectStoreRemote<Store>>, Arc<MemFs>>;

type Hook = Box<dyn FnOnce() + Send>;

/// 2026-10-04 12:34:56 UTC: the time every device believes it is, so conflict-copy names are exact.
pub(crate) const NOW_SECS: u64 = 1_791_117_296;

/// One store: a remote and the keys all its devices share.
pub(crate) struct World {
    pub store: Store,
    pub keys: StoreKeys,
    /// The store's `key.json`, as `create-store` would have written it (password "pw").
    pub key_file: KeyFile,
}

impl World {
    pub fn new() -> Self {
        let (key_file, keys) = KeyFile::create(
            "pw",
            KdfParams {
                m_kib: 8,
                t: 1,
                p: 1,
            },
        )
        .unwrap();
        Self {
            store: Arc::new(InMemory::new()),
            keys,
            key_file,
        }
    }

    pub fn remote(&self) -> ObjectStoreRemote<Store> {
        ObjectStoreRemote::new(self.store.clone(), "store")
    }

    pub fn device(&self, name: &str) -> MemDevice {
        self.device_on(name, Arc::new(MemFs::new()), self.remote())
    }

    pub fn device_on<R: RemoteStore, L: LocalFs + Clone>(
        &self,
        name: &str,
        fs: L,
        remote: R,
    ) -> Device<R, L> {
        let syncer = Syncer::new(
            fs.clone(),
            remote,
            KeysCodec(self.keys.clone()),
            SyncState::open_in_memory().unwrap(),
            DeviceName::new(name),
        )
        .with_clock(|| UNIX_EPOCH + Duration::from_secs(NOW_SECS));
        Device {
            enc: EncryptedFs::new(fs, self.keys.clone()),
            syncer,
        }
    }

    /// What the remote holds for `path`, decrypted.
    pub async fn remote_plain(&self, path: &str) -> Option<String> {
        let object = self.remote().get(&p(path)).await.ok()?;
        Some(String::from_utf8(self.keys.open(&p(path), &object.bytes).unwrap()).unwrap())
    }

    pub async fn remote_paths(&self) -> Vec<String> {
        let mut paths: Vec<_> = self
            .remote()
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        paths.sort();
        paths
    }
}

/// One device: its folder (as the app sees it, decrypted) and its syncer.
pub(crate) struct Device<R, L> {
    pub enc: EncryptedFs<L>,
    pub syncer: Syncer<L, R, KeysCodec>,
}

impl<R: RemoteStore, L: LocalFs> Device<R, L> {
    pub fn write(&self, path: &str, text: &str) {
        self.enc.write(&p(path), text.as_bytes()).unwrap();
    }

    pub fn read(&self, path: &str) -> Option<String> {
        match self.enc.read(&p(path)) {
            Ok(bytes) => Some(String::from_utf8(bytes).unwrap()),
            Err(flint_store_fs::Error::NotFound(_)) => None,
            Err(e) => panic!("reading {path}: {e}"),
        }
    }

    pub fn delete(&self, path: &str) {
        self.enc.remove(&p(path)).unwrap();
    }

    pub async fn sync(&mut self) -> SyncReport {
        self.syncer.sync().await.unwrap()
    }

    /// Files named like conflict copies, sorted.
    pub fn copies(&self) -> Vec<String> {
        let mut found: Vec<_> = self
            .syncer
            .fs()
            .list()
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .filter(|p| p.contains(".conflict-"))
            .collect();
        found.sort();
        found
    }
}

// ---- a remote that misbehaves on cue --------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    List,
    Head,
    Get,
    Put,
    Delete,
}

pub(crate) enum Fault {
    /// The request fails before reaching the server.
    Fail,
    /// The server applies the request, but the reply is lost: the caller sees an error.
    LoseReply,
    /// The server applies a PUT; the (retried) request is then refused as a failed precondition.
    AppliedThenRefused,
    /// Another device writes this content just before the request is handled.
    RivalWrites(Bytes),
    /// The server refuses the PUT as a failed precondition without applying it, however many times it is
    /// tried (a misbehaving server).
    Refuse,
}

struct Rule {
    op: Op,
    path: Option<StorePath>,
    fault: Fault,
    remaining: usize,
}

/// A [`RemoteStore`] that passes everything through, except where told otherwise.
pub(crate) struct Faulty<R> {
    pub inner: R,
    rules: Mutex<Vec<Rule>>,
}

impl<R: RemoteStore> Faulty<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            rules: Mutex::new(Vec::new()),
        }
    }

    /// Applies `fault` to the next `times` requests of kind `op` (for `path`, if given).
    pub fn inject(&self, op: Op, path: Option<&str>, fault: Fault, times: usize) {
        self.rules.lock().unwrap().push(Rule {
            op,
            path: path.map(p),
            fault,
            remaining: times,
        });
    }

    fn take(&self, op: Op, path: Option<&StorePath>) -> Option<Fault> {
        let mut rules = self.rules.lock().unwrap();
        let rule = rules.iter_mut().find(|r| {
            r.remaining > 0 && r.op == op && (r.path.is_none() || r.path.as_ref() == path)
        })?;
        rule.remaining -= 1;
        Some(match &rule.fault {
            Fault::Fail => Fault::Fail,
            Fault::LoseReply => Fault::LoseReply,
            Fault::AppliedThenRefused => Fault::AppliedThenRefused,
            Fault::RivalWrites(bytes) => Fault::RivalWrites(bytes.clone()),
            Fault::Refuse => Fault::Refuse,
        })
    }
}

fn network_down() -> flint_remote::Error {
    flint_remote::Error::Backend(object_store::Error::Generic {
        store: "test",
        source: "connection reset by peer".into(),
    })
}

impl<R: RemoteStore> RemoteStore for Faulty<R> {
    fn capabilities(&self) -> flint_remote::Capabilities {
        self.inner.capabilities()
    }

    async fn list(&self) -> flint_remote::Result<Vec<RemoteEntry>> {
        match self.take(Op::List, None) {
            Some(Fault::Fail) => Err(network_down()),
            _ => self.inner.list().await,
        }
    }

    async fn head(&self, path: &StorePath) -> flint_remote::Result<Option<RemoteEntry>> {
        match self.take(Op::Head, Some(path)) {
            Some(Fault::Fail) => Err(network_down()),
            _ => self.inner.head(path).await,
        }
    }

    async fn get(&self, path: &StorePath) -> flint_remote::Result<RemoteObject> {
        match self.take(Op::Get, Some(path)) {
            Some(Fault::Fail) => Err(network_down()),
            _ => self.inner.get(path).await,
        }
    }

    async fn put(
        &self,
        path: &StorePath,
        bytes: Bytes,
        sha256: &ContentHash,
        precondition: Precondition,
    ) -> flint_remote::Result<RemoteEntry> {
        match self.take(Op::Put, Some(path)) {
            Some(Fault::Fail) => Err(network_down()),
            Some(Fault::LoseReply) => {
                self.inner.put(path, bytes, sha256, precondition).await?;
                Err(network_down())
            }
            Some(Fault::Refuse) => Err(flint_remote::Error::PreconditionFailed(path.clone())),
            Some(Fault::AppliedThenRefused) => {
                self.inner.put(path, bytes, sha256, precondition).await?;
                Err(flint_remote::Error::PreconditionFailed(path.clone()))
            }
            Some(Fault::RivalWrites(rival)) => {
                self.inner
                    .put(
                        path,
                        rival.clone(),
                        &ContentHash::of(&rival),
                        Precondition::Unconditional,
                    )
                    .await?;
                self.inner.put(path, bytes, sha256, precondition).await
            }
            None => self.inner.put(path, bytes, sha256, precondition).await,
        }
    }

    async fn delete(&self, path: &StorePath, expect: Option<&ETag>) -> flint_remote::Result<()> {
        match self.take(Op::Delete, Some(path)) {
            Some(Fault::Fail) => Err(network_down()),
            Some(Fault::RivalWrites(rival)) => {
                self.inner
                    .put(
                        path,
                        rival.clone(),
                        &ContentHash::of(&rival),
                        Precondition::Unconditional,
                    )
                    .await?;
                self.inner.delete(path, expect).await
            }
            _ => self.inner.delete(path, expect).await,
        }
    }
}

// ---- a local store that notices and can interfere ---------------------------------------------------

/// Wraps a local store to count `hash` calls and to run a closure at a chosen moment, so a test can edit
/// a file exactly while sync is working on it.
#[derive(Clone)]
pub(crate) struct Spy<L> {
    pub inner: L,
    pub hash_calls: Arc<Mutex<Vec<StorePath>>>,
    on_hash: Arc<Mutex<Vec<(usize, Hook)>>>,
}

impl<L> Spy<L> {
    pub fn new(inner: L) -> Self {
        Self {
            inner,
            hash_calls: Arc::new(Mutex::new(Vec::new())),
            on_hash: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn hashed(&self) -> usize {
        self.hash_calls.lock().unwrap().len()
    }

    /// Runs `action` just before the `n`th (0-based, from now) call of `hash`.
    pub fn before_hash_call(&self, n: usize, action: impl FnOnce() + Send + 'static) {
        let base = self.hashed();
        self.on_hash
            .lock()
            .unwrap()
            .push((base + n, Box::new(action)));
    }
}

impl<L: LocalFs> LocalFs for Spy<L> {
    fn list(&self) -> flint_store_fs::Result<Vec<FileEntry>> {
        self.inner.list()
    }
    fn exists(&self, path: &StorePath) -> flint_store_fs::Result<bool> {
        self.inner.exists(path)
    }
    fn read(&self, path: &StorePath) -> flint_store_fs::Result<Vec<u8>> {
        self.inner.read(path)
    }
    fn read_prefix(&self, path: &StorePath, len: usize) -> flint_store_fs::Result<Vec<u8>> {
        self.inner.read_prefix(path, len)
    }
    fn entry(&self, path: &StorePath) -> flint_store_fs::Result<FileEntry> {
        self.inner.entry(path)
    }
    fn write_atomic(&self, path: &StorePath, bytes: &[u8]) -> flint_store_fs::Result<()> {
        self.inner.write_atomic(path, bytes)
    }
    fn remove(&self, path: &StorePath) -> flint_store_fs::Result<()> {
        self.inner.remove(path)
    }
    fn move_file(&self, from: &StorePath, to: &StorePath) -> flint_store_fs::Result<()> {
        self.inner.move_file(from, to)
    }
    fn hash(&self, path: &StorePath) -> flint_store_fs::Result<ContentHash> {
        let call = {
            let mut calls = self.hash_calls.lock().unwrap();
            calls.push(path.clone());
            calls.len() - 1
        };
        let due: Vec<_> = {
            let mut hooks = self.on_hash.lock().unwrap();
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut *hooks)
                .into_iter()
                .partition(|(n, _)| *n == call);
            *hooks = later;
            due
        };
        for (_, action) in due {
            action();
        }
        self.inner.hash(path)
    }
    fn trash(&self, path: &StorePath) -> flint_store_fs::Result<StorePath> {
        self.inner.trash(path)
    }
}

impl World {
    /// A device whose remote can be made to misbehave.
    pub fn faulty_device(&self, name: &str) -> FaultyDevice {
        self.device_on(name, Arc::new(MemFs::new()), Faulty::new(self.remote()))
    }
}
