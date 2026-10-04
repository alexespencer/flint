use flint_core::{ContentHash, StorePath};

/// What the local file looked like when it was last hashed. Used only to skip re-hashing a file that
/// has visibly not changed; never to decide who is newer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalStamp {
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub mtime_ns: i64,
}

/// The state both sides agreed on at the last successful sync of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baseline {
    pub path: StorePath,
    /// SHA-256 of the stored bytes (ciphertext) at that moment.
    pub hash: ContentHash,
    /// The remote's ETag at that moment, exactly as the remote gave it.
    pub etag: Option<String>,
    /// How the local file looked when it was hashed, if known.
    pub stamp: Option<LocalStamp>,
}

impl Baseline {
    /// Whether the local file is visibly the same as when it was last hashed, so its hash need not be
    /// recomputed. `false` when there is no recorded stamp.
    pub fn local_looks_unchanged(&self, now: &LocalStamp) -> bool {
        self.stamp.as_ref() == Some(now)
    }
}

/// A path that sync is holding back until the user resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: StorePath,
    /// Where the other side's version was written, next to the original.
    pub copy: StorePath,
}
