use flint_core::{ContentHash, StorePath};

/// An object's version token, exactly as the backend reported it (quotes included). Compare for
/// equality only: it is not a content hash (that is [`RemoteEntry::sha256`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ETag(String);

impl ETag {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the remote says about one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub path: StorePath,
    pub size: u64,
    /// `None` only from [`RemoteStore::list`](crate::RemoteStore::list) on a backend whose listings
    /// omit ETags; [`list_with_etags`](crate::RemoteStore::list_with_etags) fills them in.
    pub etag: Option<ETag>,
    /// The SHA-256 recorded when the object was written (`x-amz-meta-sha256`). Never present in a
    /// listing. `None` from `head`/`get` means the object was not written by Flint.
    pub sha256: Option<ContentHash>,
}

/// An object and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteObject {
    pub entry: RemoteEntry,
    pub bytes: bytes::Bytes,
}

/// What must be true of the object for a write to go ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    /// Overwrite whatever is there (or create). Not for notes: nothing is detected.
    Unconditional,
    /// Create only: `If-None-Match: *`. Fails if the key already exists.
    MustNotExist,
    /// Replace only the version the caller last saw: `If-Match: <etag>`.
    IfMatch(ETag),
}
