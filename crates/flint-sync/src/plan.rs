use std::collections::{BTreeMap, BTreeSet};

use flint_core::{ContentHash, StorePath};
use flint_remote::ETag;
use flint_sync_state::Baseline;

/// A file as it is on disk now: its path's stored bytes (ciphertext) hashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub hash: ContentHash,
}

/// A file as it is on the remote now. The ETag is all a listing gives; it is compared only for equality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub etag: ETag,
}

/// How an upload is made conditional, so a concurrent change is refused instead of overwritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadMode {
    /// A new file: `If-None-Match: *`.
    Create,
    /// A changed file: `If-Match: <the ETag last agreed>`.
    Replace(ETag),
}

/// Why the executor must look at the contents before deciding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileReason {
    /// Both sides changed since the last agreement.
    BothChanged,
    /// The file is new on both sides (or the baseline is gone).
    BothNew,
}

/// Disagreements that cannot be settled by looking at hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Edited here, deleted there. The local file stays; the user decides.
    LocalChangedRemoteDeleted,
    /// Deleted here, edited there. The remote version is restored locally so the edit is not lost.
    LocalDeletedRemoteChanged,
}

/// What to do about one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// PUT the local file.
    Upload {
        path: StorePath,
        mode: UploadMode,
    },
    /// GET the remote file and write it locally (temp file, then atomic rename), then record the baseline.
    Download {
        path: StorePath,
    },
    /// The file was deleted here and is unchanged there: delete it there, only if its ETag is still `expect`.
    DeleteRemote {
        path: StorePath,
        expect: ETag,
    },
    /// The file was deleted there and is unchanged here: move the local copy to `.trash/`.
    TrashLocal {
        path: StorePath,
    },
    /// Both sides have something and the hashes cannot say whether it is the same. The executor compares
    /// the decrypted contents: equal means record the baseline and nothing else; different is a conflict.
    Reconcile {
        path: StorePath,
        reason: ReconcileReason,
    },
    Conflict {
        path: StorePath,
        kind: ConflictKind,
    },
    /// Gone from both sides: drop the baseline.
    Forget {
        path: StorePath,
    },
}

impl Action {
    pub fn path(&self) -> &StorePath {
        match self {
            Self::Upload { path, .. }
            | Self::Download { path }
            | Self::DeleteRemote { path, .. }
            | Self::TrashLocal { path }
            | Self::Reconcile { path, .. }
            | Self::Conflict { path, .. }
            | Self::Forget { path } => path,
        }
    }
}

/// Decides what to do about every path that appears in `local`, `remote` or `baselines`.
///
/// - A local file has *changed* if its hash differs from the baseline's. A remote file has changed if
///   its ETag differs from the baseline's (or the baseline has none, so nothing can be assumed).
///   Timestamps are never consulted.
/// - Paths in `held` (unresolved conflicts) get no action.
///
/// The result has at most one action per path, sorted by path. A path where nothing needs doing has none.
pub fn plan(
    local: &BTreeMap<StorePath, LocalFile>,
    remote: &BTreeMap<StorePath, RemoteFile>,
    baselines: &BTreeMap<StorePath, Baseline>,
    held: &BTreeSet<StorePath>,
) -> Vec<Action> {
    let paths: BTreeSet<&StorePath> = local
        .keys()
        .chain(remote.keys())
        .chain(baselines.keys())
        .collect();
    paths
        .into_iter()
        .filter(|path| !held.contains(*path))
        .filter_map(|path| plan_one(path, local.get(path), remote.get(path), baselines.get(path)))
        .collect()
}

fn plan_one(
    path: &StorePath,
    local: Option<&LocalFile>,
    remote: Option<&RemoteFile>,
    baseline: Option<&Baseline>,
) -> Option<Action> {
    let path = path.clone();
    let Some(base) = baseline else {
        // never agreed on this path
        return match (local, remote) {
            (Some(_), None) => Some(Action::Upload {
                path,
                mode: UploadMode::Create,
            }),
            (None, Some(_)) => Some(Action::Download { path }),
            (Some(_), Some(_)) => Some(Action::Reconcile {
                path,
                reason: ReconcileReason::BothNew,
            }),
            (None, None) => None,
        };
    };

    let local_changed = |l: &LocalFile| l.hash != base.hash;
    // Without a recorded ETag there is no telling whether the remote moved, so assume it did: the
    // worst case is a comparison of contents that finds them equal.
    let remote_changed = |r: &RemoteFile| base.etag.as_deref() != Some(r.etag.as_str());

    match (local, remote) {
        (None, None) => Some(Action::Forget { path }),
        (None, Some(r)) if !remote_changed(r) => Some(Action::DeleteRemote {
            path,
            expect: r.etag.clone(),
        }),
        (None, Some(_)) => Some(Action::Conflict {
            path,
            kind: ConflictKind::LocalDeletedRemoteChanged,
        }),
        (Some(l), None) if !local_changed(l) => Some(Action::TrashLocal { path }),
        (Some(_), None) => Some(Action::Conflict {
            path,
            kind: ConflictKind::LocalChangedRemoteDeleted,
        }),
        (Some(l), Some(r)) => match (local_changed(l), remote_changed(r)) {
            (false, false) => None,
            // `remote_changed` was false, so the baseline has an ETag to be conditional on
            (true, false) => Some(Action::Upload {
                path,
                mode: UploadMode::Replace(r.etag.clone()),
            }),
            (false, true) => Some(Action::Download { path }),
            (true, true) => Some(Action::Reconcile {
                path,
                reason: ReconcileReason::BothChanged,
            }),
        },
    }
}

#[cfg(test)]
mod tests;
