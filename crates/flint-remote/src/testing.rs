//! Test backends: an in-process S3 server (`s3s-fs`) and builders for each way of reaching a store.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use object_store::aws::AmazonS3;
use object_store::memory::InMemory;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use s3s_fs::FileSystem;
use tempfile::TempDir;
use tokio::net::TcpListener;

use crate::{ObjectStoreRemote, RemoteStore, S3Config};

pub(crate) const BUCKET: &str = "flint-test";
const ACCESS_KEY: &str = "test-key";
const SECRET_KEY: &str = "test-secret";

/// A throwaway S3 server on a random localhost port, backed by a temp folder, dropped with the test.
pub(crate) struct S3sFs {
    pub addr: SocketAddr,
    _dir: TempDir,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for S3sFs {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl S3sFs {
    pub async fn spawn() -> Self {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(BUCKET)).unwrap();
        let service = {
            let mut builder = S3ServiceBuilder::new(FileSystem::new(dir.path()).unwrap());
            builder.set_auth(SimpleAuth::from_single(ACCESS_KEY, SECRET_KEY));
            builder.build()
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let service = service.clone();
                tokio::spawn(async move {
                    let _ = ConnBuilder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        Self {
            addr,
            _dir: dir,
            server,
        }
    }

    pub fn config(&self, prefix: &str) -> S3Config {
        S3Config {
            bucket: BUCKET.into(),
            prefix: prefix.into(),
            region: "us-east-1".into(),
            endpoint: Some(format!("http://{}", self.addr)),
            access_key_id: ACCESS_KEY.into(),
            secret_access_key: SECRET_KEY.into(),
        }
    }
}

/// Somewhere to run the contract suite: gives a remote scoped to any prefix of the same backend.
pub(crate) trait Backend {
    type Remote: RemoteStore + 'static;
    fn remote(&self, prefix: &str) -> Self::Remote;
}

/// `object_store`'s in-memory backend: fast, and atomic.
pub(crate) struct Memory(Arc<InMemory>);

impl Memory {
    pub fn new() -> Self {
        Self(Arc::new(InMemory::new()))
    }
}

impl Backend for Memory {
    type Remote = ObjectStoreRemote<Arc<InMemory>>;
    fn remote(&self, prefix: &str) -> Self::Remote {
        ObjectStoreRemote::new(self.0.clone(), prefix)
    }
}

impl Backend for S3sFs {
    type Remote = ObjectStoreRemote<AmazonS3>;
    fn remote(&self, prefix: &str) -> Self::Remote {
        ObjectStoreRemote::s3(&self.config(prefix)).unwrap()
    }
}

/// A server reachable through environment variables (RustFS, MinIO, or AWS), for the opt-in tests.
///
/// `FLINT_S3_ENDPOINT` (omit for AWS), `FLINT_S3_BUCKET`, `FLINT_S3_REGION` (default `us-east-1`),
/// `FLINT_S3_ACCESS_KEY`, `FLINT_S3_SECRET_KEY`. Every instance works under its own prefix, so
/// tests do not see each other's objects (they are left behind; use a scratch bucket).
pub(crate) struct External {
    base: String,
    template: S3Config,
}

impl External {
    pub fn from_env() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let need =
            |name: &str| var(name).unwrap_or_else(|| panic!("set {name} (see docs/rustfs.md)"));
        Self {
            base: format!(
                "flint-contract/{}-{}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ),
            template: S3Config {
                bucket: need("FLINT_S3_BUCKET"),
                prefix: String::new(),
                region: var("FLINT_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
                endpoint: var("FLINT_S3_ENDPOINT"),
                access_key_id: need("FLINT_S3_ACCESS_KEY"),
                secret_access_key: need("FLINT_S3_SECRET_KEY"),
            },
        }
    }
}

impl Backend for External {
    type Remote = ObjectStoreRemote<AmazonS3>;
    fn remote(&self, prefix: &str) -> Self::Remote {
        let prefix = if prefix.is_empty() {
            self.base.clone()
        } else {
            format!("{}/{prefix}", self.base)
        };
        ObjectStoreRemote::s3(&S3Config {
            prefix,
            ..self.template.clone()
        })
        .unwrap()
    }
}
