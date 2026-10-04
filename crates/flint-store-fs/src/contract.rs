//! One behavioural test suite for [`LocalFs`](crate::LocalFs), run against every implementation, so
//! the in-memory fake cannot drift from the real thing.
//!
//! `local_fs_contract!(make())` takes an expression giving `(fs, guard)`; the guard (a temp dir, or
//! `()`) stays alive for the duration of each test.

#[cfg(test)]
macro_rules! local_fs_contract {
    ($make:expr) => {
        mod contract {
            use flint_core::{ContentHash, StorePath};

            use super::*;
            use crate::{Error, KEY_FILE, LocalFs};

            fn p(s: &str) -> StorePath {
                StorePath::new(s).unwrap()
            }

            fn listed(fs: &impl LocalFs) -> Vec<String> {
                fs.list()
                    .unwrap()
                    .into_iter()
                    .map(|e| e.path.to_string())
                    .collect()
            }

            #[test]
            fn write_then_read_round_trips_including_empty_and_binary() {
                let (fs, _guard) = $make;
                for (path, bytes) in [
                    ("a.md", &b"hello"[..]),
                    ("empty.md", b""),
                    ("bin", &[0, 255, 10, 13, 0][..]),
                ] {
                    fs.write_atomic(&p(path), bytes).unwrap();
                    assert_eq!(fs.read(&p(path)).unwrap(), bytes, "{path}");
                }
            }

            #[test]
            fn writing_creates_parent_folders_and_overwrites() {
                let (fs, _guard) = $make;
                fs.write_atomic(&p("a/b/c/note.md"), b"one").unwrap();
                fs.write_atomic(&p("a/b/c/note.md"), b"two, longer")
                    .unwrap();
                assert_eq!(fs.read(&p("a/b/c/note.md")).unwrap(), b"two, longer");
                fs.write_atomic(&p("a/b/c/note.md"), b"3").unwrap();
                assert_eq!(
                    fs.read(&p("a/b/c/note.md")).unwrap(),
                    b"3",
                    "shorter content must not leave a tail"
                );
            }

            #[test]
            fn read_prefix_returns_at_most_the_asked_bytes() {
                let (fs, _guard) = $make;
                fs.write_atomic(&p("a.md"), b"0123456789").unwrap();
                fs.write_atomic(&p("empty.md"), b"").unwrap();
                assert_eq!(fs.read_prefix(&p("a.md"), 4).unwrap(), b"0123");
                assert_eq!(fs.read_prefix(&p("a.md"), 10).unwrap(), b"0123456789");
                assert_eq!(fs.read_prefix(&p("a.md"), 1000).unwrap(), b"0123456789");
                assert_eq!(fs.read_prefix(&p("a.md"), 0).unwrap(), b"");
                assert_eq!(fs.read_prefix(&p("empty.md"), 4).unwrap(), b"");
                assert!(matches!(
                    fs.read_prefix(&p("nope.md"), 4),
                    Err(Error::NotFound(_))
                ));
            }

            #[test]
            fn entry_reports_size_and_time_for_one_file() {
                let (fs, _guard) = $make;
                let before = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
                fs.write_atomic(&p("d/a.md"), b"12345").unwrap();
                let entry = fs.entry(&p("d/a.md")).unwrap();
                assert_eq!((entry.path.as_str(), entry.size), ("d/a.md", 5));
                assert!(
                    entry.modified >= before
                        && entry.modified
                            <= std::time::SystemTime::now() + std::time::Duration::from_secs(5)
                );
                assert!(matches!(fs.entry(&p("nope.md")), Err(Error::NotFound(_))));
                assert!(
                    matches!(fs.entry(&p("d")), Err(Error::NotFound(_))),
                    "a folder is not a file"
                );
                // entry and list agree
                assert_eq!(fs.list().unwrap()[0], entry);
            }

            #[test]
            fn missing_files_are_not_found() {
                let (fs, _guard) = $make;
                assert!(!fs.exists(&p("nope.md")).unwrap());
                assert!(matches!(fs.read(&p("nope.md")), Err(Error::NotFound(_))));
                assert!(matches!(fs.remove(&p("nope.md")), Err(Error::NotFound(_))));
                assert!(matches!(fs.hash(&p("nope.md")), Err(Error::NotFound(_))));
                assert!(matches!(fs.trash(&p("nope.md")), Err(Error::NotFound(_))));
                assert!(matches!(
                    fs.move_file(&p("nope.md"), &p("x.md")),
                    Err(Error::NotFound(_))
                ));
            }

            #[test]
            fn exists_and_remove() {
                let (fs, _guard) = $make;
                fs.write_atomic(&p("d/a.md"), b"x").unwrap();
                assert!(fs.exists(&p("d/a.md")).unwrap());
                fs.remove(&p("d/a.md")).unwrap();
                assert!(!fs.exists(&p("d/a.md")).unwrap());
            }

            #[test]
            fn an_empty_store_lists_nothing() {
                let (fs, _guard) = $make;
                assert!(fs.list().unwrap().is_empty());
            }

            #[test]
            fn list_is_sorted_and_reports_sizes() {
                let (fs, _guard) = $make;
                for path in [
                    "z.md",
                    "a.md",
                    "dir/b.md",
                    "dir.md",
                    "dir/sub/c.md",
                    "Ünï.md",
                ] {
                    fs.write_atomic(&p(path), path.as_bytes()).unwrap();
                }
                let entries = fs.list().unwrap();
                let mut sorted = entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
                sorted.sort();
                assert_eq!(
                    entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>(),
                    sorted
                );
                assert_eq!(entries.len(), 6);
                for e in &entries {
                    assert_eq!(e.size, e.path.as_str().len() as u64, "{}", e.path);
                }
            }

            #[test]
            fn list_hides_what_is_never_synced_but_keeps_the_key_file() {
                let (fs, _guard) = $make;
                for path in [
                    "note.md",
                    "folder/note.md",
                    KEY_FILE,
                    ".store/index.db",
                    ".store/sync.db",
                    ".trash/old.md",
                    ".obsidian/app.json",
                    ".obsidian/plugins/x/main.js",
                    ".DS_Store",
                    "folder/.DS_Store",
                    ".flint-tmp-abc",
                    ".git/config",
                    ".git/objects/ab/cdef",
                    ".claude/commands/x.md",
                    ".hidden-note.md",
                    "folder/.hidden/inside.md",
                    "folder/.hidden-too.md",
                ] {
                    fs.write_atomic(&p(path), b"x").unwrap();
                }
                assert_eq!(
                    listed(&fs),
                    [".store/key.json", "folder/note.md", "note.md"]
                );
            }

            #[test]
            fn hash_is_the_sha256_of_the_stored_bytes() {
                let (fs, _guard) = $make;
                fs.write_atomic(&p("a.md"), b"abc").unwrap();
                assert_eq!(
                    fs.hash(&p("a.md")).unwrap().to_string(),
                    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                );
                fs.write_atomic(&p("a.md"), b"abd").unwrap();
                assert_eq!(fs.hash(&p("a.md")).unwrap(), ContentHash::of(b"abd"));
            }

            #[test]
            fn move_file_renames_and_creates_folders_and_replaces() {
                let (fs, _guard) = $make;
                fs.write_atomic(&p("a.md"), b"A").unwrap();
                fs.write_atomic(&p("new/dir/b.md"), b"old B").unwrap();
                fs.move_file(&p("a.md"), &p("new/dir/b.md")).unwrap();
                assert!(!fs.exists(&p("a.md")).unwrap());
                assert_eq!(fs.read(&p("new/dir/b.md")).unwrap(), b"A");
                fs.move_file(&p("new/dir/b.md"), &p("deep/er/c.md"))
                    .unwrap();
                assert_eq!(fs.read(&p("deep/er/c.md")).unwrap(), b"A");
            }

            #[test]
            fn trash_moves_the_file_under_trash_keeping_its_path() {
                let (fs, _guard) = $make;
                fs.write_atomic(&p("folder/note.md"), b"precious").unwrap();
                let trashed = fs.trash(&p("folder/note.md")).unwrap();
                assert_eq!(trashed, p(".trash/folder/note.md"));
                assert!(!fs.exists(&p("folder/note.md")).unwrap());
                assert!(listed(&fs).is_empty(), "trash is not listed");
                assert_eq!(fs.read(&trashed).unwrap(), b"precious");
            }

            #[test]
            fn trashing_the_same_path_again_never_overwrites_the_trash() {
                let (fs, _guard) = $make;
                let mut trashed = Vec::new();
                for contents in ["first", "second", "third"] {
                    fs.write_atomic(&p("a/note.md"), contents.as_bytes())
                        .unwrap();
                    trashed.push((fs.trash(&p("a/note.md")).unwrap(), contents));
                }
                assert_eq!(trashed[0].0, p(".trash/a/note.md"));
                for (path, contents) in &trashed {
                    assert_eq!(fs.read(path).unwrap(), contents.as_bytes(), "{path}");
                }
                let distinct: std::collections::BTreeSet<_> =
                    trashed.iter().map(|(path, _)| path).collect();
                assert_eq!(distinct.len(), 3, "each trashed copy has its own name");
                assert!(
                    trashed[1].0.as_str().starts_with(".trash/a/note.")
                        && trashed[1].0.as_str().ends_with(".md")
                );
            }
        }
    };
}

#[cfg(test)]
pub(crate) use local_fs_contract;
