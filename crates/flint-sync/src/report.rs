use flint_core::{ContentHash, StorePath};

use crate::Error;

/// Why a file was deliberately left alone (not an error: the data is safe, and the reason is reported).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The remote object does not start with Flint's file header. Nothing is downloaded or compared: it
    /// could be a planted file, and plaintext in the store folder would be adopted as a user's note.
    RemoteNotCiphertext,
    /// The local file starts like a Flint file but does not decrypt: it was damaged, or part of it was
    /// overwritten, outside Flint. Uploading it would replace the good copy on every other device with a
    /// file nobody can open, so it is left alone. The last good version is still on S3 and on your other devices.
    LocalNotDecryptable,
    /// The object on S3 starts like a Flint file but does not decrypt (damaged there, or put at the wrong name).
    /// It is not written here, so a good local copy is never replaced by an unreadable one.
    RemoteNotDecryptable,
    /// The local file is plaintext (a dropped-in file not yet adopted). Uploading it would leak it.
    LocalNotCiphertext,
    /// The downloaded bytes do not match the SHA-256 the remote recorded for them: damaged in transit
    /// or at rest.
    HashMismatch {
        expected: ContentHash,
        actual: ContentHash,
    },
    /// The local file changed while sync was working on it, so acting on the old plan could lose the
    /// newer edit. It will be planned again next time.
    LocalChangedDuringSync,
    /// `key.json` differs on both sides: two devices changed the password at once. Neither is overwritten.
    KeyFileDiverged,
    /// The remote kept changing under every retry for this file. Next time.
    Contended,
}

/// What happened to a file the two sides disagreed about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictOutcome {
    /// Both sides have different contents. The local file is untouched, the remote version was written
    /// to `copy`, and the path is held until resolved.
    Diverged { copy: StorePath },
    /// Edited here, deleted there. The local file is kept and the path is held until resolved.
    EditedHereDeletedThere,
    /// Deleted here, edited there. The remote version was restored locally so the edit is not lost.
    RestoredEditedRemotely,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictReport {
    pub path: StorePath,
    pub outcome: ConflictOutcome,
}

/// Everything one sync cycle did. A path appears in at most one list.
#[derive(Debug, Default)]
pub struct SyncReport {
    pub uploaded: Vec<StorePath>,
    pub downloaded: Vec<StorePath>,
    pub deleted_remote: Vec<StorePath>,
    /// Moved to `.trash/` because it was deleted on the other device.
    pub trashed_local: Vec<StorePath>,
    /// Both sides had changed to the same plaintext (or our own upload was acknowledged late): only the
    /// baseline was updated.
    pub agreed: Vec<StorePath>,
    pub forgotten: Vec<StorePath>,
    pub conflicts: Vec<ConflictReport>,
    pub refused: Vec<(StorePath, Refusal)>,
    /// Paths that could not be synced this time (network, disk, decryption). The cycle carried on.
    pub failed: Vec<(StorePath, Error)>,
}

impl SyncReport {
    /// Whether nothing needed doing and nothing went wrong.
    pub fn is_quiet(&self) -> bool {
        self.uploaded.is_empty()
            && self.downloaded.is_empty()
            && self.deleted_remote.is_empty()
            && self.trashed_local.is_empty()
            && self.agreed.is_empty()
            && self.forgotten.is_empty()
            && self.conflicts.is_empty()
            && self.refused.is_empty()
            && self.failed.is_empty()
    }
}
