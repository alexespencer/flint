use std::sync::Arc;

use flint_registry::RemoteSettings;
use flint_remote::{ObjectStoreRemote, RemoteStore, S3Config};
use object_store::aws::AmazonS3;
use object_store::memory::InMemory;

/// How a store's remote is reached. The service asks for a connection whenever it needs one.
pub trait Connect: Send + Sync {
    type Remote: RemoteStore;

    fn connect(
        &self,
        settings: &RemoteSettings,
        secret_access_key: &str,
    ) -> flint_remote::Result<Self::Remote>;
}

/// Real S3 or an S3-compatible service (HTTPS only, except for localhost).
pub struct S3Connect;

impl Connect for S3Connect {
    type Remote = ObjectStoreRemote<AmazonS3>;

    fn connect(
        &self,
        settings: &RemoteSettings,
        secret_access_key: &str,
    ) -> flint_remote::Result<Self::Remote> {
        ObjectStoreRemote::s3(&S3Config {
            bucket: settings.bucket.clone(),
            prefix: settings.prefix.clone(),
            region: settings.region.clone(),
            endpoint: settings.endpoint.clone(),
            access_key_id: settings.access_key_id.clone(),
            secret_access_key: secret_access_key.to_owned(),
        })
    }
}

/// One in-process bucket shared by everything connected through it, with each store's prefix kept
/// apart. For tests of anything built on the service (devices are separate `AppService`s sharing one of
/// these).
#[derive(Clone, Default)]
pub struct InMemoryConnect(Arc<InMemory>);

impl InMemoryConnect {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Connect for InMemoryConnect {
    type Remote = ObjectStoreRemote<Arc<InMemory>>;

    fn connect(&self, settings: &RemoteSettings, _: &str) -> flint_remote::Result<Self::Remote> {
        Ok(ObjectStoreRemote::new(self.0.clone(), &settings.prefix))
    }
}
