use bytes::Bytes;
use flint_core::{ContentHash, StorePath};
use futures::TryStreamExt;
use object_store::path::Path;
use object_store::{
    Attribute, AttributeValue, Attributes, GetOptions, GetResult, ObjectStore, ObjectStoreExt,
    PutMode, PutOptions, PutPayload, UpdateVersion,
};
use percent_encoding::percent_decode_str;

use crate::{
    Capabilities, ETag, Error, Precondition, RemoteEntry, RemoteObject, RemoteStore, Result,
};

/// The user-metadata key holding the SHA-256 of what was uploaded (sent as `x-amz-meta-sha256`).
const SHA256_METADATA: &str = "sha256";

/// A [`RemoteStore`] over any `object_store` backend, scoped to one prefix in it.
///
/// Object keys are the store path under the prefix. Characters `object_store` cannot put in a path
/// verbatim (such as `#`, `%`, `?` or `\`) are percent-encoded in the key, consistently on every
/// device; ordinary names, spaces, apostrophes and Unicode are stored as they are.
pub struct ObjectStoreRemote<S> {
    store: S,
    prefix: Path,
}

impl<S: ObjectStore> ObjectStoreRemote<S> {
    /// `prefix` is the folder within the bucket that holds this store (`""` for the bucket root).
    pub fn new(store: S, prefix: &str) -> Self {
        Self {
            store,
            prefix: Path::from(prefix.trim_matches('/')),
        }
    }

    fn key(&self, path: &StorePath) -> Path {
        Path::from_iter(self.prefix.parts().chain(Path::from(path.as_str()).parts()))
    }

    /// The store path for an object key, or `None` if it is not under the prefix or not a valid path.
    fn store_path(&self, key: &Path) -> Option<StorePath> {
        let parts: Vec<String> = key
            .prefix_match(&self.prefix)?
            .map(|part| {
                percent_decode_str(part.as_ref())
                    .decode_utf8()
                    .map(|s| s.into_owned())
            })
            .collect::<std::result::Result<_, _>>()
            .ok()?;
        StorePath::new(parts.join("/")).ok()
    }

    fn entry(
        &self,
        path: &StorePath,
        meta: &object_store::ObjectMeta,
        attributes: &Attributes,
    ) -> RemoteEntry {
        let sha256 = attributes
            .get(&Attribute::Metadata(SHA256_METADATA.into()))
            .and_then(|value| value.as_ref().parse::<ContentHash>().ok());
        RemoteEntry {
            path: path.clone(),
            size: meta.size,
            etag: meta.e_tag.clone().map(ETag::new),
            sha256,
        }
    }

    async fn fetch(&self, path: &StorePath, head_only: bool) -> Result<Option<GetResult>> {
        let options = GetOptions {
            head: head_only,
            ..Default::default()
        };
        match self.store.get_opts(&self.key(path), options).await {
            Ok(result) => Ok(Some(result)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(Error::from_backend(path, error)),
        }
    }
}

impl<S: ObjectStore> RemoteStore for ObjectStoreRemote<S> {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            conditional_delete: false,
        }
    }

    async fn list(&self) -> Result<Vec<RemoteEntry>> {
        let prefix = (!self.prefix.as_ref().is_empty()).then_some(&self.prefix);
        let mut stream = self.store.list(prefix);
        let mut entries = Vec::new();
        loop {
            match stream.try_next().await {
                Ok(Some(meta)) => {
                    // keys that are not store paths (a `folder/` placeholder, `..`) are not ours
                    if let Some(path) = self.store_path(&meta.location) {
                        entries.push(RemoteEntry {
                            path,
                            size: meta.size,
                            etag: meta.e_tag.map(ETag::new),
                            sha256: None,
                        });
                    }
                }
                Ok(None) => return Ok(entries),
                // an unparseable key is skipped like any other foreign object
                Err(object_store::Error::InvalidPath { .. }) => continue,
                Err(error) => return Err(Error::from_backend_listing(error)),
            }
        }
    }

    async fn head(&self, path: &StorePath) -> Result<Option<RemoteEntry>> {
        Ok(self
            .fetch(path, true)
            .await?
            .map(|result| self.entry(path, &result.meta, &result.attributes)))
    }

    async fn get(&self, path: &StorePath) -> Result<RemoteObject> {
        let result = self
            .fetch(path, false)
            .await?
            .ok_or_else(|| Error::NotFound(path.clone()))?;
        let (meta, attributes) = (result.meta.clone(), result.attributes.clone());
        let bytes = result
            .bytes()
            .await
            .map_err(|e| Error::from_backend(path, e))?;
        let mut entry = self.entry(path, &meta, &attributes);
        entry.size = bytes.len() as u64;
        Ok(RemoteObject { entry, bytes })
    }

    async fn put(
        &self,
        path: &StorePath,
        bytes: Bytes,
        sha256: &ContentHash,
        precondition: Precondition,
    ) -> Result<RemoteEntry> {
        let size = bytes.len() as u64;
        let missing_means_refused = matches!(precondition, Precondition::IfMatch(_));
        let mode = match precondition {
            Precondition::Unconditional => PutMode::Overwrite,
            Precondition::MustNotExist => PutMode::Create,
            Precondition::IfMatch(etag) => PutMode::Update(UpdateVersion {
                e_tag: Some(etag.as_str().to_owned()),
                version: None,
            }),
        };
        let mut attributes = Attributes::new();
        attributes.insert(
            Attribute::Metadata(SHA256_METADATA.into()),
            AttributeValue::from(sha256.to_string()),
        );
        let options = PutOptions {
            mode,
            attributes,
            ..Default::default()
        };

        match self
            .store
            .put_opts(&self.key(path), PutPayload::from_bytes(bytes), options)
            .await
        {
            Ok(put) => Ok(RemoteEntry {
                path: path.clone(),
                size,
                etag: put.e_tag.map(ETag::new),
                sha256: Some(*sha256),
            }),
            // "replace the version I saw" when there is no such object is also a refused precondition
            Err(object_store::Error::NotFound { .. }) if missing_means_refused => {
                Err(Error::PreconditionFailed(path.clone()))
            }
            Err(error) => Err(Error::from_backend(path, error)),
        }
    }

    async fn delete(&self, path: &StorePath, expect: Option<&ETag>) -> Result<()> {
        if let Some(expect) = expect {
            match self.head(path).await? {
                None => return Err(Error::NotFound(path.clone())),
                Some(current) if current.etag.as_ref() != Some(expect) => {
                    return Err(Error::PreconditionFailed(path.clone()));
                }
                Some(_) => {}
            }
        }
        match self.store.delete(&self.key(path)).await {
            Ok(()) => Ok(()),
            Err(object_store::Error::NotFound { .. }) if expect.is_none() => Ok(()),
            Err(error) => Err(Error::from_backend(path, error)),
        }
    }
}
