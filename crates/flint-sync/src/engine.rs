use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use bytes::Bytes;
use flint_core::{ContentHash, StorePath};
use flint_crypto::{SNIFF_LEN, is_sealed};
use flint_remote::{ETag, Precondition, RemoteEntry, RemoteStore};
use flint_store_fs::{FileEntry, KEY_FILE, LocalFs, is_sync_path};
use flint_sync_state::{Baseline, LocalStamp, SyncState};

use crate::conflict::conflict_copy_path_numbered;
use crate::plan::{
    Action, ConflictKind, LocalFile, ReconcileReason, RemoteFile, UploadMode, plan, plan_path,
};
use crate::report::{ConflictOutcome, ConflictReport, Refusal, SyncReport};
use crate::{Codec, DeviceName, Error, Result};

/// How many times one file is re-planned after the remote refused a write, before giving up until next time.
const MAX_ATTEMPTS: usize = 4;

/// Runs sync cycles for one store: reads the local folder and the remote, plans, and carries the plan out.
///
/// Sync works on the stored bytes (ciphertext) and treats them as opaque, except through the [`Codec`]
/// where it must compare or re-encrypt contents. It never uploads a file that is not ciphertext, never
/// downloads one that is not, and records a baseline only after the action that established it succeeded.
pub struct Syncer<L, R, C> {
    fs: L,
    remote: R,
    codec: C,
    state: SyncState,
    device: DeviceName,
    clock: Box<dyn Fn() -> SystemTime + Send + Sync>,
}

enum Outcome {
    Done,
    /// The remote (or the file) changed under us: work out what to do from fresh state.
    Replan,
    Refused(Refusal),
}

struct Snapshot {
    baselines: BTreeMap<StorePath, Baseline>,
    local: BTreeMap<StorePath, LocalFile>,
    remote: BTreeMap<StorePath, RemoteFile>,
}

impl<L: LocalFs, R: RemoteStore, C: Codec> Syncer<L, R, C> {
    pub fn new(fs: L, remote: R, codec: C, state: SyncState, device: DeviceName) -> Self {
        Self {
            fs,
            remote,
            codec,
            state,
            device,
            clock: Box::new(SystemTime::now),
        }
    }

    /// Replaces the clock used to name conflict copies (for tests).
    pub fn with_clock(mut self, clock: impl Fn() -> SystemTime + Send + Sync + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    pub fn fs(&self) -> &L {
        &self.fs
    }

    pub fn remote(&self) -> &R {
        &self.remote
    }

    pub fn state(&self) -> &SyncState {
        &self.state
    }

    /// One sync cycle. Per-file problems end up in the report and the cycle carries on; only a failure to
    /// read either side as a whole (or the sync database) is an error.
    pub async fn sync(&mut self) -> Result<SyncReport> {
        let snapshot = self.snapshot().await?;
        let held: BTreeSet<StorePath> = self
            .state
            .conflicts()?
            .into_iter()
            .map(|c| c.path)
            .collect();
        let mut report = SyncReport::default();
        for action in plan(
            &snapshot.local,
            &snapshot.remote,
            &snapshot.baselines,
            &held,
        ) {
            self.run(action, &mut report).await;
        }
        Ok(report)
    }

    /// The user has dealt with a conflict (edited the local file, or decided to keep or drop it): lets
    /// sync act on the path again.
    ///
    /// For a diverged file the baseline becomes the remote's *current* version, so the local file counts as
    /// changed and the next cycle uploads it conditional on that version. If the remote changed again in the
    /// meantime that upload is refused and a fresh conflict results; nothing is overwritten. For a file edited
    /// here and deleted there, the baseline is dropped so the local file is uploaded as new.
    pub async fn resolve(&mut self, path: &StorePath) -> Result<()> {
        let conflict = self
            .state
            .conflict(path)?
            .ok_or_else(|| Error::NotInConflict(path.clone()))?;
        if conflict.copy.is_none() {
            return Ok(self.state.release(path)?);
        }
        match self.remote.head(path).await? {
            None => Ok(self.state.release(path)?),
            Some(head) => {
                let hash = match head.sha256 {
                    Some(hash) => hash,
                    None => ContentHash::of(&self.remote.get(path).await?.bytes),
                };
                let baseline = Baseline {
                    path: path.clone(),
                    hash,
                    etag: head.etag.map(|e| e.as_str().to_owned()),
                    stamp: None,
                };
                Ok(self.state.resolve(&baseline)?)
            }
        }
    }

    // ---- reading the two sides ------------------------------------------------------------------

    async fn snapshot(&self) -> Result<Snapshot> {
        let baselines: BTreeMap<_, _> = self
            .state
            .baselines()?
            .into_iter()
            .map(|b| (b.path.clone(), b))
            .collect();

        let mut local = BTreeMap::new();
        for entry in self.fs.list()? {
            // a file that is hashed fresh unless it visibly has not changed since its baseline
            let hash = match baselines.get(&entry.path) {
                Some(base) if base.local_looks_unchanged(&stamp_of(&entry)) => base.hash,
                _ => match self.fs.hash(&entry.path) {
                    Ok(hash) => hash,
                    Err(flint_store_fs::Error::NotFound(_)) => continue, // deleted since it was listed
                    Err(e) => return Err(e.into()),
                },
            };
            local.insert(entry.path, LocalFile { hash });
        }

        // objects that are not store paths (hidden, planted) are not ours: never planned, never downloaded
        let remote = self
            .remote
            .list_with_etags()
            .await?
            .into_iter()
            .filter(|e| is_sync_path(&e.path))
            .filter_map(|e| e.etag.map(|etag| (e.path, RemoteFile { etag })))
            .collect();
        Ok(Snapshot {
            baselines,
            local,
            remote,
        })
    }

    fn local_stamp(&self, path: &StorePath) -> Result<Option<LocalStamp>> {
        match self.fs.entry(path) {
            Ok(entry) => Ok(Some(stamp_of(&entry))),
            Err(flint_store_fs::Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The local file's current hash, or `None` if it is not there.
    fn local_hash(&self, path: &StorePath) -> Result<Option<ContentHash>> {
        match self.fs.hash(path) {
            Ok(hash) => Ok(Some(hash)),
            Err(flint_store_fs::Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Whether the local file is still what the plan was made from (`expected`: its hash, or `None` if absent).
    fn local_still_is(&self, path: &StorePath, expected: Option<&ContentHash>) -> Result<bool> {
        Ok(self.local_hash(path)?.as_ref() == expected)
    }

    // ---- running actions ------------------------------------------------------------------------

    async fn run(&mut self, first: Action, report: &mut SyncReport) {
        let path = first.path().clone();
        let mut action = first;
        for _ in 0..MAX_ATTEMPTS {
            match self.execute(&action, report).await {
                Ok(Outcome::Done) => return,
                Ok(Outcome::Refused(why)) => {
                    report.refused.push((path, why));
                    return;
                }
                Ok(Outcome::Replan) => match self.replan(&path).await {
                    Ok(Some(next)) => action = next,
                    Ok(None) => return,
                    Err(e) => {
                        report.failed.push((path, e));
                        return;
                    }
                },
                Err(e) => {
                    report.failed.push((path, e));
                    return;
                }
            }
        }
        report.refused.push((path, Refusal::Contended));
    }

    /// What to do about `path` given how things stand right now.
    async fn replan(&self, path: &StorePath) -> Result<Option<Action>> {
        if self.state.conflict(path)?.is_some() {
            return Ok(None);
        }
        let local = self.local_hash(path)?.map(|hash| LocalFile { hash });
        let remote = self
            .remote
            .head(path)
            .await?
            .and_then(|e| e.etag.map(|etag| RemoteFile { etag }));
        let baseline = self.state.baseline(path)?;
        Ok(plan_path(
            path,
            local.as_ref(),
            remote.as_ref(),
            baseline.as_ref(),
        ))
    }

    async fn execute(&mut self, action: &Action, report: &mut SyncReport) -> Result<Outcome> {
        match action {
            Action::Upload { path, mode } => self.upload(path, mode, report).await,
            Action::Download { path } => self.download(path, report).await,
            Action::DeleteRemote { path, expect } => self.delete_remote(path, expect, report).await,
            Action::TrashLocal { path } => self.trash_local(path, report),
            Action::Reconcile { path, reason } => self.reconcile(path, *reason, report).await,
            Action::Conflict { path, kind } => self.conflict(path, *kind, report).await,
            Action::Forget { path } => {
                self.state.forget(path)?;
                report.forgotten.push(path.clone());
                Ok(Outcome::Done)
            }
        }
    }

    async fn upload(
        &mut self,
        path: &StorePath,
        mode: &UploadMode,
        report: &mut SyncReport,
    ) -> Result<Outcome> {
        // the stamp is taken before the read: if the file changes during the read the stamp will not match
        // next time and the file is hashed again
        let Some(stamp) = self.local_stamp(path)? else {
            return Ok(Outcome::Replan);
        };
        let bytes = match self.fs.read(path) {
            Ok(bytes) => bytes,
            Err(flint_store_fs::Error::NotFound(_)) => return Ok(Outcome::Replan),
            Err(e) => return Err(e.into()),
        };
        if !looks_sealed(path, &bytes) {
            return Ok(Outcome::Refused(Refusal::LocalNotCiphertext));
        }
        let hash = ContentHash::of(&bytes);
        let precondition = match mode {
            UploadMode::Create => Precondition::MustNotExist,
            UploadMode::Replace(etag) => Precondition::IfMatch(etag.clone()),
        };
        match self
            .remote
            .put(path, Bytes::from(bytes), &hash, precondition)
            .await
        {
            Ok(entry) => {
                self.record(path, hash, entry.etag, Some(stamp))?;
                report.uploaded.push(path.clone());
                Ok(Outcome::Done)
            }
            Err(flint_remote::Error::PreconditionFailed(_)) => {
                // Either somebody else changed the remote, or an earlier attempt of ours landed and its reply
                // was lost, so the retry is refused by our own write. The recorded SHA-256 tells them apart.
                match self.remote.head(path).await? {
                    Some(head) if head.sha256 == Some(hash) => {
                        self.record(path, hash, head.etag, Some(stamp))?;
                        report.agreed.push(path.clone());
                        Ok(Outcome::Done)
                    }
                    _ => Ok(Outcome::Replan),
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn download(&mut self, path: &StorePath, report: &mut SyncReport) -> Result<Outcome> {
        let expected_local = self.state.baseline(path)?.map(|b| b.hash);
        let outcome = self.fetch_and_write(path, expected_local.as_ref()).await?;
        if matches!(outcome, Outcome::Done) {
            report.downloaded.push(path.clone());
        }
        Ok(outcome)
    }

    /// Downloads `path`, checks it, writes it locally if the local file is still `expected_local`
    /// (`None`: absent), and records the baseline.
    async fn fetch_and_write(
        &mut self,
        path: &StorePath,
        expected_local: Option<&ContentHash>,
    ) -> Result<Outcome> {
        let object = match self.remote.get(path).await {
            Ok(object) => object,
            Err(flint_remote::Error::NotFound(_)) => return Ok(Outcome::Replan),
            Err(e) => return Err(e.into()),
        };
        if let Some(refusal) = check_downloaded(path, &object.bytes, &object.entry) {
            return Ok(Outcome::Refused(refusal));
        }
        if !self.local_still_is(path, expected_local)? {
            return Ok(Outcome::Refused(Refusal::LocalChangedDuringSync));
        }
        self.fs.write_atomic(path, &object.bytes)?;
        let stamp = self.local_stamp(path)?;
        self.record(
            path,
            ContentHash::of(&object.bytes),
            object.entry.etag,
            stamp,
        )?;
        Ok(Outcome::Done)
    }

    async fn delete_remote(
        &mut self,
        path: &StorePath,
        expect: &ETag,
        report: &mut SyncReport,
    ) -> Result<Outcome> {
        match self.remote.delete(path, Some(expect)).await {
            Ok(()) | Err(flint_remote::Error::NotFound(_)) => {
                self.state.forget(path)?;
                report.deleted_remote.push(path.clone());
                Ok(Outcome::Done)
            }
            Err(flint_remote::Error::PreconditionFailed(_)) => Ok(Outcome::Replan),
            Err(e) => Err(e.into()),
        }
    }

    fn trash_local(&mut self, path: &StorePath, report: &mut SyncReport) -> Result<Outcome> {
        let Some(base) = self.state.baseline(path)? else {
            return Ok(Outcome::Replan);
        };
        if !self.local_still_is(path, Some(&base.hash))? {
            return Ok(Outcome::Refused(Refusal::LocalChangedDuringSync));
        }
        self.fs.trash(path)?;
        self.state.forget(path)?;
        report.trashed_local.push(path.clone());
        Ok(Outcome::Done)
    }

    async fn reconcile(
        &mut self,
        path: &StorePath,
        _reason: ReconcileReason,
        report: &mut SyncReport,
    ) -> Result<Outcome> {
        let Some(stamp) = self.local_stamp(path)? else {
            return Ok(Outcome::Replan);
        };
        let local_bytes = match self.fs.read(path) {
            Ok(bytes) => bytes,
            Err(flint_store_fs::Error::NotFound(_)) => return Ok(Outcome::Replan),
            Err(e) => return Err(e.into()),
        };
        let local_hash = ContentHash::of(&local_bytes);
        let object = match self.remote.get(path).await {
            Ok(object) => object,
            Err(flint_remote::Error::NotFound(_)) => return Ok(Outcome::Replan),
            Err(e) => return Err(e.into()),
        };
        if let Some(refusal) = check_downloaded(path, &object.bytes, &object.entry) {
            return Ok(Outcome::Refused(refusal));
        }

        let same = if object.entry.sha256 == Some(local_hash)
            || object.bytes.as_ref() == local_bytes.as_slice()
        {
            true // the very same bytes: for instance our own upload, acknowledged late
        } else if path.as_str() == KEY_FILE {
            return Ok(Outcome::Refused(Refusal::KeyFileDiverged));
        } else {
            self.codec
                .same_plaintext(path, &local_bytes, &object.bytes)?
        };
        if same {
            self.record(path, local_hash, object.entry.etag, Some(stamp))?;
            report.agreed.push(path.clone());
            return Ok(Outcome::Done);
        }

        // Different contents: keep the local file exactly as it is, put the remote version beside it, and
        // hold the path until the user has looked at both.
        let copy = self.free_copy_path(path)?;
        self.fs
            .write_atomic(&copy, &self.codec.reseal(path, &copy, &object.bytes)?)?;
        self.state.mark_conflict(path, Some(&copy))?;
        report.conflicts.push(ConflictReport {
            path: path.clone(),
            outcome: ConflictOutcome::Diverged { copy },
        });
        Ok(Outcome::Done)
    }

    async fn conflict(
        &mut self,
        path: &StorePath,
        kind: ConflictKind,
        report: &mut SyncReport,
    ) -> Result<Outcome> {
        match kind {
            ConflictKind::LocalChangedRemoteDeleted => {
                self.state.mark_conflict(path, None)?;
                report.conflicts.push(ConflictReport {
                    path: path.clone(),
                    outcome: ConflictOutcome::EditedHereDeletedThere,
                });
                Ok(Outcome::Done)
            }
            ConflictKind::LocalDeletedRemoteChanged => {
                // the file is gone here, so restoring the remote edit overwrites nothing
                let outcome = self.fetch_and_write(path, None).await?;
                if matches!(outcome, Outcome::Done) {
                    report.conflicts.push(ConflictReport {
                        path: path.clone(),
                        outcome: ConflictOutcome::RestoredEditedRemotely,
                    });
                }
                Ok(outcome)
            }
        }
    }

    // ---- small helpers --------------------------------------------------------------------------

    fn record(
        &mut self,
        path: &StorePath,
        hash: ContentHash,
        etag: Option<ETag>,
        stamp: Option<LocalStamp>,
    ) -> Result<()> {
        let baseline = Baseline {
            path: path.clone(),
            hash,
            etag: etag.map(|e| e.as_str().to_owned()),
            stamp,
        };
        Ok(self.state.record(&baseline)?)
    }

    /// A name for the conflict copy that no file has yet.
    fn free_copy_path(&self, path: &StorePath) -> Result<StorePath> {
        let now = (self.clock)();
        for attempt in 1.. {
            let candidate = conflict_copy_path_numbered(path, &self.device, now, attempt);
            if !self.fs.exists(&candidate)? {
                return Ok(candidate);
            }
        }
        unreachable!("the loop only ends by returning")
    }
}

fn stamp_of(entry: &FileEntry) -> LocalStamp {
    let mtime_ns = match entry.modified.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => after.as_nanos() as i64,
        Err(before) => -(before.duration().as_nanos() as i64),
    };
    LocalStamp {
        size: entry.size,
        mtime_ns,
    }
}

/// Whether bytes are fit to leave this device or to be written here: ciphertext, except `key.json`,
/// which is plaintext JSON by design.
fn looks_sealed(path: &StorePath, bytes: &[u8]) -> bool {
    path.as_str() == KEY_FILE || is_sealed(&bytes[..bytes.len().min(SNIFF_LEN)], bytes.len() as u64)
}

/// Why downloaded bytes must not be used, if they must not.
fn check_downloaded(path: &StorePath, bytes: &Bytes, entry: &RemoteEntry) -> Option<Refusal> {
    if !looks_sealed(path, bytes) {
        return Some(Refusal::RemoteNotCiphertext);
    }
    let expected = entry.sha256?;
    let actual = ContentHash::of(bytes);
    (actual != expected).then_some(Refusal::HashMismatch { expected, actual })
}

#[cfg(test)]
mod tests;
