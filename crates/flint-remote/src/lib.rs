//! All S3 access for one store.
//!
//! [`RemoteStore`] is the narrow interface the sync engine uses: list, head, get, put with a
//! [`Precondition`], and delete. It speaks in [`StorePath`](flint_core::StorePath)s relative to the
//! store's bucket and prefix, so callers never see S3 keys. Everything it moves is opaque bytes:
//! this crate knows nothing about encryption (the sync engine decides what counts as valid).
//!
//! [`ObjectStoreRemote`] implements it over the `object_store` crate, which gives one code path for
//! AWS S3, other S3-compatible servers and the in-process emulator used in tests, plus an in-memory
//! backend. [`ObjectStoreRemote::s3`] refuses plain-`http` endpoints except on localhost.
//!
//! Writes are conditional: `put` with [`Precondition::MustNotExist`] or [`Precondition::IfMatch`] is
//! how a concurrent change is detected and never silently overwritten. A refused write is
//! [`Error::PreconditionFailed`]. Note that when a request succeeds but its reply is lost, a retry
//! sees a precondition failure for its own write; telling that apart (by comparing the stored SHA-256
//! with what was sent) is the caller's job, which is why [`RemoteEntry::sha256`] exists.

mod adapter;
mod error;
mod s3;
mod store;
mod types;

#[cfg(test)]
mod contract;
#[cfg(test)]
mod testing;

pub use adapter::ObjectStoreRemote;
pub use error::{Error, Result};
pub use s3::S3Config;
pub use store::{Capabilities, RemoteStore};
pub use types::{ETag, Precondition, RemoteEntry, RemoteObject};

#[cfg(test)]
mod backends {
    use crate::contract::remote_contract;
    use crate::testing::{External, Memory, S3sFs};

    mod in_memory {
        use super::*;
        async fn make() -> (Memory, ()) {
            (Memory::new(), ())
        }
        remote_contract!(make(); atomic = true;);
    }

    mod s3s_fs {
        use super::*;
        async fn make() -> (S3sFs, ()) {
            (S3sFs::spawn().await, ())
        }
        remote_contract!(make(); atomic = false;);
    }

    /// A real bucket: see docs/s3-testing.md. Run with `-- --ignored` and the FLINT_S3_* variables.
    mod external {
        use super::*;
        async fn make() -> (External, ()) {
            (External::from_env(), ())
        }
        remote_contract!(make(); atomic = true; #[ignore = "needs a real S3 bucket: set FLINT_S3_* (docs/s3-testing.md)"]);

        /// Not a pass/fail test: tries each conditional operation once against the real server and prints
        /// what it did, so a failing contract run can be explained. Run with `-- --ignored --nocapture`.
        #[tokio::test]
        #[ignore = "talks to a real bucket: set FLINT_S3_* (docs/s3-testing.md)"]
        async fn diagnose_the_backend() {
            use bytes::Bytes;
            use flint_core::{ContentHash, StorePath};

            use crate::{ETag, Precondition, RemoteStore, testing::Backend};

            let remote = External::from_env().remote("diagnose");
            let path = |s: &str| StorePath::new(s).unwrap();
            let hash = |b: &[u8]| ContentHash::of(b);
            let put = |name: &'static str, body: &'static [u8], precondition: Precondition| {
                let remote = &remote;
                async move {
                    remote
                        .put(
                            &path(name),
                            Bytes::from_static(body),
                            &hash(body),
                            precondition,
                        )
                        .await
                }
            };
            let show = |step: &str, expectation: &str, result: String| {
                println!("DIAG {step:<46} expect {expectation:<22} got {result}")
            };
            let brief = |r: &crate::Result<crate::RemoteEntry>| match r {
                Ok(e) => format!("Ok (etag {:?})", e.etag.as_ref().map(|t| t.as_str())),
                Err(e) => format!("Err: {e}"),
            };

            println!("DIAG --- endpoint-level checks");
            show(
                "list the (empty) prefix",
                "Ok []",
                format!("{:?}", remote.list().await.map(|l| l.len())),
            );
            show(
                "head a missing object",
                "Ok(None)",
                format!(
                    "{:?}",
                    remote.head(&path("nope.md")).await.map(|o| o.is_some())
                ),
            );
            show(
                "get a missing object",
                "Err NotFound",
                format!("{:?}", remote.get(&path("nope.md")).await.map(|_| ())),
            );

            println!("DIAG --- writes");
            let created = put("a.md", b"v1", Precondition::MustNotExist).await;
            show(
                "create (If-None-Match: *) on a new key",
                "Ok",
                brief(&created),
            );
            let etag = created.as_ref().ok().and_then(|e| e.etag.clone());
            show(
                "head after create",
                "Ok(Some)",
                format!(
                    "{:?}",
                    remote
                        .head(&path("a.md"))
                        .await
                        .map(|o| o.map(|e| (e.size, e.sha256.is_some())))
                ),
            );
            show(
                "create again over the existing key",
                "Err PreconditionFailed",
                brief(&put("a.md", b"v2", Precondition::MustNotExist).await),
            );
            show(
                "unconditional overwrite",
                "Ok",
                brief(&put("a.md", b"v2", Precondition::Unconditional).await),
            );
            show(
                "If-Match with the OLD (stale) etag",
                "Err PreconditionFailed",
                match &etag {
                    Some(old) => {
                        brief(&put("a.md", b"v3", Precondition::IfMatch(old.clone())).await)
                    }
                    None => "skipped (no etag)".into(),
                },
            );
            let current = remote
                .head(&path("a.md"))
                .await
                .ok()
                .flatten()
                .and_then(|e| e.etag);
            show(
                "If-Match with the CURRENT etag",
                "Ok",
                match &current {
                    Some(now) => {
                        brief(&put("a.md", b"v4", Precondition::IfMatch(now.clone())).await)
                    }
                    None => "skipped (no etag)".into(),
                },
            );
            show(
                "If-Match on a key that does not exist",
                "Err PreconditionFailed",
                brief(
                    &put(
                        "missing.md",
                        b"x",
                        Precondition::IfMatch(ETag::new("\"0123456789abcdef0123456789abcdef\"")),
                    )
                    .await,
                ),
            );
            show(
                "what a.md holds now (v4 means conditions worked)",
                "v4",
                format!(
                    "{:?}",
                    remote
                        .get(&path("a.md"))
                        .await
                        .map(|o| String::from_utf8_lossy(&o.bytes).into_owned())
                ),
            );

            println!("DIAG --- deletes");
            let stale_delete = match &etag {
                Some(old) => format!("{:?}", remote.delete(&path("a.md"), Some(old)).await),
                None => "skipped (no etag)".into(),
            };
            show(
                "delete with a stale etag",
                "Err PreconditionFailed",
                stale_delete,
            );
            show(
                "delete unconditionally",
                "Ok",
                format!("{:?}", remote.delete(&path("a.md"), None).await),
            );
            show(
                "list after cleanup",
                "Ok 0",
                format!("{:?}", remote.list().await.map(|l| l.len())),
            );
        }

        /// Removes everything the contract tests wrote (`flint-contract/` in the bucket).
        #[tokio::test]
        #[ignore = "deletes objects in a real bucket: set FLINT_S3_* (docs/s3-testing.md)"]
        async fn delete_everything_the_contract_tests_left_behind() {
            use crate::{RemoteStore, testing::Backend};
            let sweeper = External::sweeper().remote("");
            let mut deleted = 0;
            for entry in sweeper.list().await.unwrap() {
                sweeper.delete(&entry.path, None).await.unwrap();
                deleted += 1;
            }
            println!("deleted {deleted} objects under flint-contract/");
        }
    }
}

/// Behaviour that is not part of the contract: what the emulator is known to do differently, and how
/// failures look from the outside.
#[cfg(test)]
mod emulator_and_failures {
    use std::time::{Duration, Instant};

    use bytes::Bytes;
    use flint_core::{ContentHash, StorePath};

    use crate::testing::{Backend, Memory, S3sFs};
    use crate::{
        Capabilities, ETag, Error, ObjectStoreRemote, Precondition, RemoteEntry, RemoteObject,
        RemoteStore, Result, S3Config,
    };

    fn p(s: &str) -> StorePath {
        StorePath::new(s).unwrap()
    }

    async fn put(remote: &impl RemoteStore, path: &str, bytes: &'static [u8]) {
        remote
            .put(
                &p(path),
                Bytes::from_static(bytes),
                &ContentHash::of(bytes),
                Precondition::MustNotExist,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn s3s_fs_leaves_etags_out_of_listings_so_list_with_etags_must_head() {
        // If this starts failing, s3s-fs now returns ETags in LIST and the fallback is only needed for
        // other backends; the assertions below should then be inverted.
        let server = S3sFs::spawn().await;
        let remote = server.remote("store");
        put(&remote, "a.md", b"a").await;
        put(&remote, "dir/b.md", b"b").await;
        assert!(
            remote
                .list()
                .await
                .unwrap()
                .iter()
                .all(|e| e.etag.is_none())
        );
        let full = remote.list_with_etags().await.unwrap();
        assert_eq!(full.len(), 2);
        assert!(full.iter().all(|e| e.etag.is_some()));
    }

    /// A backend whose listings omit ETags, wrapped around one that has them, to test the fallback
    /// without depending on the emulator.
    struct NoEtagsInList<R>(R);

    impl<R: RemoteStore> RemoteStore for NoEtagsInList<R> {
        fn capabilities(&self) -> Capabilities {
            self.0.capabilities()
        }
        async fn list(&self) -> Result<Vec<RemoteEntry>> {
            Ok(self
                .0
                .list()
                .await?
                .into_iter()
                .map(|e| RemoteEntry { etag: None, ..e })
                .collect())
        }
        async fn head(&self, path: &StorePath) -> Result<Option<RemoteEntry>> {
            self.0.head(path).await
        }
        async fn get(&self, path: &StorePath) -> Result<RemoteObject> {
            self.0.get(path).await
        }
        async fn put(
            &self,
            path: &StorePath,
            bytes: Bytes,
            sha256: &ContentHash,
            precondition: Precondition,
        ) -> Result<RemoteEntry> {
            self.0.put(path, bytes, sha256, precondition).await
        }
        async fn delete(&self, path: &StorePath, expect: Option<&ETag>) -> Result<()> {
            self.0.delete(path, expect).await
        }
    }

    #[tokio::test]
    async fn list_with_etags_heads_only_the_entries_that_lack_one_and_drops_vanished_ones() {
        let remote = NoEtagsInList(Memory::new().remote("store"));
        put(&remote, "a.md", b"a").await;
        put(&remote, "b.md", b"b").await;
        assert!(
            remote
                .list()
                .await
                .unwrap()
                .iter()
                .all(|e| e.etag.is_none())
        );
        let full = remote.list_with_etags().await.unwrap();
        assert_eq!(full.len(), 2);
        for entry in &full {
            assert_eq!(
                entry.etag,
                remote.head(&entry.path).await.unwrap().unwrap().etag
            );
        }

        // an object that disappears between the list and the head is left out, not reported without an ETag
        struct Vanishing(
            NoEtagsInList<ObjectStoreRemote<std::sync::Arc<object_store::memory::InMemory>>>,
        );
        impl RemoteStore for Vanishing {
            fn capabilities(&self) -> Capabilities {
                self.0.capabilities()
            }
            async fn list(&self) -> Result<Vec<RemoteEntry>> {
                let listed = self.0.list().await?;
                self.0.delete(&p("b.md"), None).await?; // gone before list_with_etags gets to it
                Ok(listed)
            }
            async fn head(&self, path: &StorePath) -> Result<Option<RemoteEntry>> {
                self.0.head(path).await
            }
            async fn get(&self, path: &StorePath) -> Result<RemoteObject> {
                self.0.get(path).await
            }
            async fn put(
                &self,
                path: &StorePath,
                bytes: Bytes,
                sha256: &ContentHash,
                precondition: Precondition,
            ) -> Result<RemoteEntry> {
                self.0.put(path, bytes, sha256, precondition).await
            }
            async fn delete(&self, path: &StorePath, expect: Option<&ETag>) -> Result<()> {
                self.0.delete(path, expect).await
            }
        }
        let vanishing = Vanishing(NoEtagsInList(Memory::new().remote("store")));
        put(&vanishing, "a.md", b"a").await;
        put(&vanishing, "b.md", b"b").await;
        let paths: Vec<_> = vanishing
            .list_with_etags()
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        assert_eq!(paths, ["a.md"]);
    }

    #[tokio::test]
    async fn wrong_credentials_are_access_denied_not_a_retry_storm() {
        let server = S3sFs::spawn().await;
        let mut config = server.config("store");
        config.secret_access_key = "not the secret".into();
        let remote = ObjectStoreRemote::s3(&config).unwrap();
        let started = Instant::now();
        let result = remote.list().await;
        assert!(matches!(result, Err(Error::AccessDenied(_))), "{result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "an auth failure is not retried: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_missing_bucket_is_an_error() {
        // (s3s-fs creates a bucket folder on first write, so only the listing is checked here; real S3
        // refuses writes to a missing bucket as well)
        let server = S3sFs::spawn().await;
        let mut config = server.config("store");
        config.bucket = "no-such-bucket".into();
        let remote = ObjectStoreRemote::s3(&config).unwrap();
        let result = remote.list().await;
        assert!(result.is_err(), "{result:?}");
    }

    #[tokio::test]
    async fn an_unreachable_server_fails_in_bounded_time_with_a_backend_error() {
        // grab a free port, then close it so that nothing is listening
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let config = S3Config {
            bucket: "b".into(),
            prefix: "store".into(),
            region: "us-east-1".into(),
            endpoint: Some(format!("http://127.0.0.1:{port}")),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
        };
        let remote = ObjectStoreRemote::s3(&config).unwrap();
        let started = Instant::now();
        let result = remote.list().await;
        assert!(matches!(result, Err(Error::Backend(_))), "{result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "gave up after {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn the_capabilities_say_delete_is_not_atomic() {
        assert!(!Memory::new().remote("s").capabilities().conditional_delete);
        assert!(
            !S3sFs::spawn()
                .await
                .remote("s")
                .capabilities()
                .conditional_delete
        );
    }
}
