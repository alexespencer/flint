//! One behavioural test suite for [`RemoteStore`](crate::RemoteStore), run against every backend, so
//! the in-memory backend and the emulator cannot drift from real S3.
//!
//! `remote_contract!(make(); atomic = bool; #[attrs])`: `make()` is an async expression giving
//! `(backend, guard)` (see `testing::Backend`). `atomic` says whether the backend checks conditional
//! writes atomically (the in-process emulator does not, so the real race tests skip it). The attributes
//! are put on every generated test, for example `#[ignore = "..."]`.

#[cfg(test)]
macro_rules! remote_contract {
    ($make:expr; atomic = $atomic:expr; $(#[$attr:meta])*) => {
        mod contract {
            use std::sync::Arc;

            use bytes::Bytes;
            use flint_core::{ContentHash, StorePath};
            use futures::StreamExt;

            use super::*;
            use crate::testing::Backend;
            use crate::{ETag, Error, Precondition, RemoteStore};

            fn p(s: &str) -> StorePath {
                StorePath::new(s).unwrap()
            }

            fn hash(bytes: &[u8]) -> ContentHash {
                ContentHash::of(bytes)
            }

            async fn create<R: RemoteStore>(remote: &R, path: &str, bytes: &[u8]) -> crate::RemoteEntry {
                remote.put(&p(path), Bytes::copy_from_slice(bytes), &hash(bytes), Precondition::MustNotExist).await.unwrap()
            }

            async fn sorted_paths<R: RemoteStore>(remote: &R) -> Vec<String> {
                let mut paths: Vec<_> = remote.list().await.unwrap().into_iter().map(|e| e.path.to_string()).collect();
                paths.sort();
                paths
            }

            $(#[$attr])*
            #[tokio::test]
            async fn put_get_head_round_trip_with_stable_etag_and_metadata() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let bytes = b"# A note\n\nwith [[links]] and \xc3\xbcnicode";
                let put = create(&remote, "notes/a.md", bytes).await;
                assert_eq!(put.size, bytes.len() as u64);
                assert!(put.etag.is_some(), "a PUT returns an ETag");
                assert_eq!(put.sha256, Some(hash(bytes)));

                let got = remote.get(&p("notes/a.md")).await.unwrap();
                assert_eq!(got.bytes.as_ref(), bytes);
                assert_eq!(got.entry.etag, put.etag, "GET reports the same ETag as the PUT");
                assert_eq!(got.entry.sha256, Some(hash(bytes)), "the SHA-256 round-trips as object metadata");
                assert_eq!(got.entry.size, bytes.len() as u64);

                let head = remote.head(&p("notes/a.md")).await.unwrap().expect("exists");
                assert_eq!(head, got.entry, "HEAD agrees with GET, without downloading");
            }

            $(#[$attr])*
            #[tokio::test]
            async fn empty_binary_and_large_objects_round_trip() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let big: Vec<u8> = (0..5 * 1024 * 1024u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
                for (path, bytes) in [("empty.md", Vec::new()), ("bin.dat", vec![0, 255, 10, 13, 0, 1]), ("big.bin", big)] {
                    let put = create(&remote, path, &bytes).await;
                    let got = remote.get(&p(path)).await.unwrap();
                    assert_eq!(got.bytes.as_ref(), bytes.as_slice(), "{path}");
                    assert_eq!(got.entry.etag, put.etag, "{path}: one single-part PUT has one stable ETag");
                    assert_eq!(remote.head(&p(path)).await.unwrap().unwrap().etag, put.etag, "{path}");
                }
            }

            $(#[$attr])*
            #[tokio::test]
            async fn missing_objects() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                assert!(remote.head(&p("nope.md")).await.unwrap().is_none());
                assert!(matches!(remote.get(&p("nope.md")).await, Err(Error::NotFound(_))));
                assert!(remote.list().await.unwrap().is_empty());
            }

            $(#[$attr])*
            #[tokio::test]
            async fn must_not_exist_refuses_to_overwrite() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let first = create(&remote, "a.md", b"first").await;
                let second = remote.put(&p("a.md"), Bytes::from_static(b"second"), &hash(b"second"), Precondition::MustNotExist).await;
                assert!(matches!(second, Err(Error::PreconditionFailed(_))), "{second:?}");
                let now = remote.get(&p("a.md")).await.unwrap();
                assert_eq!(now.bytes.as_ref(), b"first", "the existing object is untouched");
                assert_eq!(now.entry.etag, first.etag);
            }

            $(#[$attr])*
            #[tokio::test]
            async fn if_match_replaces_the_version_the_caller_saw_and_refuses_a_stale_one() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let v1 = create(&remote, "a.md", b"v1").await;

                // device A updates from v1
                let v2 = remote
                    .put(&p("a.md"), Bytes::from_static(b"v2 from A"), &hash(b"v2 from A"), Precondition::IfMatch(v1.etag.clone().unwrap()))
                    .await
                    .unwrap();
                assert_ne!(v2.etag, v1.etag, "new contents, new ETag");

                // device B still holds v1's ETag: refused, and A's write survives
                let stale = remote
                    .put(&p("a.md"), Bytes::from_static(b"v2 from B"), &hash(b"v2 from B"), Precondition::IfMatch(v1.etag.clone().unwrap()))
                    .await;
                assert!(matches!(stale, Err(Error::PreconditionFailed(_))), "{stale:?}");
                let now = remote.get(&p("a.md")).await.unwrap();
                assert_eq!(now.bytes.as_ref(), b"v2 from A");
                assert_eq!(now.entry.etag, v2.etag);
            }

            $(#[$attr])*
            #[tokio::test]
            async fn if_match_on_a_missing_object_is_a_refused_precondition() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let result = remote
                    .put(&p("gone.md"), Bytes::from_static(b"x"), &hash(b"x"), Precondition::IfMatch(ETag::new("\"0123456789abcdef0123456789abcdef\"")))
                    .await;
                assert!(matches!(result, Err(Error::PreconditionFailed(_))), "{result:?}");
                assert!(remote.head(&p("gone.md")).await.unwrap().is_none(), "nothing was created");
            }

            $(#[$attr])*
            #[tokio::test]
            async fn unconditional_put_overwrites() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                create(&remote, "a.md", b"old").await;
                remote.put(&p("a.md"), Bytes::from_static(b"new, longer"), &hash(b"new, longer"), Precondition::Unconditional).await.unwrap();
                assert_eq!(remote.get(&p("a.md")).await.unwrap().bytes.as_ref(), b"new, longer");
            }

            /// After a lost reply the retry is refused by its own earlier write. The stored SHA-256 is
            /// what lets the caller tell that apart from somebody else's write.
            $(#[$attr])*
            #[tokio::test]
            async fn a_retry_after_a_lost_reply_can_recognise_its_own_write() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let bytes = Bytes::from_static(b"sent once, acknowledged never");
                let sent = hash(&bytes);

                // create: the first attempt succeeded on the server; we never saw the reply
                remote.put(&p("n.md"), bytes.clone(), &sent, Precondition::MustNotExist).await.unwrap();
                let retry = remote.put(&p("n.md"), bytes.clone(), &sent, Precondition::MustNotExist).await;
                assert!(matches!(retry, Err(Error::PreconditionFailed(_))));
                assert_eq!(remote.head(&p("n.md")).await.unwrap().unwrap().sha256, Some(sent), "that is our write");

                // update: same story with If-Match
                let old = remote.head(&p("n.md")).await.unwrap().unwrap().etag.unwrap();
                let updated = Bytes::from_static(b"the update");
                remote.put(&p("n.md"), updated.clone(), &hash(&updated), Precondition::IfMatch(old.clone())).await.unwrap();
                let retry = remote.put(&p("n.md"), updated.clone(), &hash(&updated), Precondition::IfMatch(old)).await;
                assert!(matches!(retry, Err(Error::PreconditionFailed(_))));
                assert_eq!(remote.head(&p("n.md")).await.unwrap().unwrap().sha256, Some(hash(&updated)));
            }

            $(#[$attr])*
            #[tokio::test]
            async fn list_reports_every_object_with_its_size() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                for (path, bytes) in [("z.md", &b"zz"[..]), ("a.md", b"a"), ("dir/b.md", b"bbb"), ("dir/sub/c.md", b"cccc")] {
                    create(&remote, path, bytes).await;
                }
                let mut listed = remote.list().await.unwrap();
                listed.sort_by(|a, b| a.path.cmp(&b.path));
                let got: Vec<_> = listed.iter().map(|e| (e.path.as_str(), e.size)).collect();
                assert_eq!(got, [("a.md", 1), ("dir/b.md", 3), ("dir/sub/c.md", 4), ("z.md", 2)]);
                assert!(listed.iter().all(|e| e.sha256.is_none()), "a listing never carries the metadata");
            }

            $(#[$attr])*
            #[tokio::test]
            async fn list_with_etags_fills_in_what_a_listing_leaves_out() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                for path in ["a.md", "dir/b.md", "c.md"] {
                    create(&remote, path, path.as_bytes()).await;
                }
                let full = remote.list_with_etags().await.unwrap();
                assert_eq!(full.len(), 3);
                for entry in &full {
                    let head = remote.head(&entry.path).await.unwrap().unwrap();
                    assert!(entry.etag.is_some(), "{}", entry.path);
                    assert_eq!(entry.etag, head.etag, "{}", entry.path);
                }
            }

            $(#[$attr])*
            #[tokio::test]
            async fn list_pages_through_more_than_one_page_of_objects() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let n = 1_250; // S3 lists 1,000 keys per page
                let uploads: Vec<_> = futures::stream::iter(0..n)
                    .map(|i| {
                        let remote = &remote;
                        async move {
                            let path = format!("many/{:04}.md", i);
                            remote.put(&p(&path), Bytes::from(path.clone()), &hash(path.as_bytes()), Precondition::MustNotExist).await
                        }
                    })
                    .buffer_unordered(16)
                    .collect()
                    .await;
                let failed: Vec<_> = uploads.iter().filter_map(|r| r.as_ref().err()).collect();
                assert!(failed.is_empty(), "{} of {n} uploads failed; the first: {:?}", failed.len(), failed[0]);

                let paths = sorted_paths(&remote).await;
                let expected: std::collections::BTreeSet<String> = (0..n).map(|i| format!("many/{i:04}.md")).collect();
                let listed: std::collections::BTreeSet<String> = paths.iter().cloned().collect();
                let missing: Vec<_> = expected.difference(&listed).take(8).collect();
                let unexpected: Vec<_> = listed.difference(&expected).take(8).collect();
                assert!(
                    paths.len() == n && missing.is_empty() && unexpected.is_empty(),
                    "listed {} objects (and {} distinct) of {n}; first missing: {missing:?}; unexpected: {unexpected:?}",
                    paths.len(),
                    listed.len()
                );
            }

            $(#[$attr])*
            #[tokio::test]
            async fn awkward_file_names_round_trip_exactly() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let all_names = [
                    "a b.md",
                    "it's here.md",
                    "Ünïcode/日本語 note.md",
                    "cafe\u{301}.md",
                    "100%.md",
                    "50%25.md",
                    "hash#tag.md",
                    "what?.md",
                    "plus+sign.md",
                    "amp&er.md",
                    "semi;colon.md",
                    "dollar$.md",
                    "[brackets].md",
                    "{curly}.md",
                    "tilde~.md",
                    "eq=ual.md",
                    "comma,.md",
                    "colon:.md",
                    "at@.md",
                    "quote\".md",
                    "very/deep/nested/folder/structure/note.md",
                    "trailing.dot.",
                    "Back story.md",
                    "Wizard's tower.md",
                    "Esmerlada D'avenir.md",
                    "emoji 🧙.md",
                ];
                // The emulator keeps objects as files, and Windows cannot have a file called `colon:.md` (a real
                // bucket can have any key), so on Windows those names are left to the other backends.
                let names: Vec<&str> = all_names
                    .into_iter()
                    .filter(|name| {
                        !(cfg!(windows)
                            && backend.stores_names_as_files()
                            && p(name).windows_problem().is_some())
                    })
                    .collect();
                for name in names.iter().copied() {
                    let put = create(&remote, name, name.as_bytes()).await;
                    let got = remote.get(&p(name)).await.unwrap();
                    assert_eq!(got.bytes.as_ref(), name.as_bytes(), "{name}");
                    assert_eq!(got.entry.etag, put.etag, "{name}");
                }
                let mut expected: Vec<String> = names.iter().map(|n| n.to_string()).collect();
                expected.sort();
                assert_eq!(sorted_paths(&remote).await, expected, "listing gives back the same paths");
            }

            $(#[$attr])*
            #[tokio::test]
            async fn stores_under_different_prefixes_are_isolated() {
                let (backend, _guard) = $make.await;
                let (a, b, longer, inner) = (backend.remote("store"), backend.remote("store2"), backend.remote("store-b/x"), backend.remote("store/inner"));
                create(&a, "same.md", b"in a").await;
                create(&b, "same.md", b"in b").await;
                create(&longer, "other.md", b"elsewhere").await;
                create(&inner, "deep.md", b"inner").await;

                assert_eq!(sorted_paths(&a).await, ["inner/deep.md", "same.md"], "its own objects, including a nested store's folder, and no neighbours");
                assert_eq!(sorted_paths(&b).await, ["same.md"], "'store' is not a prefix of 'store2' as a folder");
                assert_eq!(sorted_paths(&longer).await, ["other.md"]);
                assert_eq!(a.get(&p("same.md")).await.unwrap().bytes.as_ref(), b"in a");
                assert_eq!(b.get(&p("same.md")).await.unwrap().bytes.as_ref(), b"in b");

                a.delete(&p("same.md"), None).await.unwrap();
                assert!(b.head(&p("same.md")).await.unwrap().is_some(), "deleting in one store does not touch another");
            }

            $(#[$attr])*
            #[tokio::test]
            async fn a_store_at_the_bucket_root_works() {
                let (backend, _guard) = $make.await;
                let root = backend.remote("");
                create(&root, "top.md", b"x").await;
                assert!(sorted_paths(&root).await.contains(&"top.md".to_string()));
                assert_eq!(root.get(&p("top.md")).await.unwrap().bytes.as_ref(), b"x");
            }

            $(#[$attr])*
            #[tokio::test]
            async fn delete_unconditionally_and_idempotently() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                create(&remote, "a.md", b"x").await;
                remote.delete(&p("a.md"), None).await.unwrap();
                assert!(remote.head(&p("a.md")).await.unwrap().is_none());
                remote.delete(&p("a.md"), None).await.unwrap();
                assert!(remote.list().await.unwrap().is_empty());
            }

            $(#[$attr])*
            #[tokio::test]
            async fn delete_with_an_expected_etag_only_removes_that_version() {
                let (backend, _guard) = $make.await;
                let remote = backend.remote("store");
                let v1 = create(&remote, "a.md", b"v1").await;
                let v2 = remote
                    .put(&p("a.md"), Bytes::from_static(b"v2"), &hash(b"v2"), Precondition::IfMatch(v1.etag.clone().unwrap()))
                    .await
                    .unwrap();

                // somebody changed it since this device last saw it
                let stale = remote.delete(&p("a.md"), v1.etag.as_ref()).await;
                assert!(matches!(stale, Err(Error::PreconditionFailed(_))), "{stale:?}");
                assert_eq!(remote.get(&p("a.md")).await.unwrap().bytes.as_ref(), b"v2", "still there");

                remote.delete(&p("a.md"), v2.etag.as_ref()).await.unwrap();
                assert!(remote.head(&p("a.md")).await.unwrap().is_none());

                let missing = remote.delete(&p("a.md"), v2.etag.as_ref()).await;
                assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");
            }

            $(#[$attr])*
            #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
            async fn concurrent_writers_never_both_win() {
                if !$atomic {
                    return; // this backend checks the precondition and writes in two steps, so a true race can slip through
                }
                let (backend, _guard) = $make.await;
                let remote = Arc::new(backend.remote("store"));

                // eight devices create the same file at once
                let outcomes = futures::future::join_all((0..8).map(|i| {
                    let remote = remote.clone();
                    tokio::spawn(async move {
                        let bytes = Bytes::from(format!("writer {i}"));
                        remote.put(&p("race.md"), bytes.clone(), &hash(&bytes), Precondition::MustNotExist).await
                    })
                }))
                .await;
                let winners: Vec<_> = outcomes.into_iter().map(|o| o.unwrap()).filter(|r| r.is_ok()).collect();
                assert_eq!(winners.len(), 1, "exactly one create wins");

                // eight devices update the same version at once
                let base = remote.head(&p("race.md")).await.unwrap().unwrap().etag.unwrap();
                let outcomes = futures::future::join_all((0..8).map(|i| {
                    let remote = remote.clone();
                    let base = base.clone();
                    tokio::spawn(async move {
                        let bytes = Bytes::from(format!("update {i}"));
                        remote.put(&p("race.md"), bytes.clone(), &hash(&bytes), Precondition::IfMatch(base)).await
                    })
                }))
                .await;
                let results: Vec<_> = outcomes.into_iter().map(|o| o.unwrap()).collect();
                let won = results.iter().filter(|r| r.is_ok()).count();
                assert_eq!(won, 1, "exactly one update from the same version wins");
                assert!(results.iter().filter(|r| r.is_err()).all(|r| matches!(r, Err(Error::PreconditionFailed(_)))));
            }
        }
    };
}

#[cfg(test)]
pub(crate) use remote_contract;
