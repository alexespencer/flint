use std::future::Future;

use bytes::Bytes;
use flint_core::{ContentHash, StorePath};

use crate::{ETag, Precondition, RemoteEntry, RemoteObject, Result};

/// What a backend can do beyond the basics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether `delete` with an expected ETag is one atomic request. AWS has had conditional DELETE
    /// since September 2025, but `object_store` cannot send it and the emulators ignore it, so every
    /// backend here is `false` and [`RemoteStore::delete`] does HEAD then DELETE. The race window is a
    /// few milliseconds, which is acceptable for a single user.
    pub conditional_delete: bool,
}

/// One store's objects on S3 (or a stand-in). Implementations must be safe to share between tasks.
pub trait RemoteStore: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    /// Every object under the store's prefix, in no particular order. Pages through large stores.
    /// Entries carry size and, on most backends, the ETag; see [`list_with_etags`](Self::list_with_etags).
    /// Keys that are not valid store paths (such as the empty `folder/` objects the S3 console makes)
    /// are skipped.
    fn list(&self) -> impl Future<Output = Result<Vec<RemoteEntry>>> + Send;

    /// Metadata for one object without downloading it, or `None` if it does not exist.
    fn head(&self, path: &StorePath) -> impl Future<Output = Result<Option<RemoteEntry>>> + Send;

    /// Downloads one object. [`Error::NotFound`](crate::Error::NotFound) if it does not exist.
    fn get(&self, path: &StorePath) -> impl Future<Output = Result<RemoteObject>> + Send;

    /// Uploads `bytes` as a single-part PUT (so the ETag is stable), recording `sha256` as object
    /// metadata, if `precondition` holds. Returns the new entry.
    /// [`Error::PreconditionFailed`](crate::Error::PreconditionFailed) means nothing was written.
    fn put(
        &self,
        path: &StorePath,
        bytes: Bytes,
        sha256: &ContentHash,
        precondition: Precondition,
    ) -> impl Future<Output = Result<RemoteEntry>> + Send;

    /// Deletes an object. With `expect`, only if its current ETag matches: a missing object is
    /// [`NotFound`](crate::Error::NotFound) and a changed one is
    /// [`PreconditionFailed`](crate::Error::PreconditionFailed). Without it, deleting something that is
    /// already gone succeeds (S3 deletes are idempotent).
    fn delete(
        &self,
        path: &StorePath,
        expect: Option<&ETag>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// [`list`](Self::list), then a `head` for every entry the listing gave no ETag for. A listing is
    /// one request per thousand objects, so backends that include ETags (AWS, RustFS) cost nothing
    /// extra; the in-process emulator (`s3s-fs`) omits them and pays one `head` each. An object that
    /// disappears between the two calls is left out.
    fn list_with_etags(&self) -> impl Future<Output = Result<Vec<RemoteEntry>>> + Send {
        async move {
            let mut complete = Vec::new();
            for entry in self.list().await? {
                if entry.etag.is_some() {
                    complete.push(entry);
                } else if let Some(full) = self.head(&entry.path).await? {
                    complete.push(RemoteEntry {
                        etag: full.etag,
                        ..entry
                    });
                }
            }
            Ok(complete)
        }
    }
}
