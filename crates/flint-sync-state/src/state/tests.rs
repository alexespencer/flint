use flint_core::{ContentHash, StorePath};
use tempfile::TempDir;

use crate::{Baseline, Conflict, Error, LocalStamp, SeenVersion, SyncState};

fn p(s: &str) -> StorePath {
    StorePath::new(s).unwrap()
}

fn baseline(path: &str, content: &str) -> Baseline {
    Baseline {
        path: p(path),
        hash: ContentHash::of(content.as_bytes()),
        etag: Some(format!("\"etag-of-{content}\"")),
        stamp: Some(LocalStamp {
            size: content.len() as u64,
            mtime_ns: 1_700_000_000_123_456_789,
        }),
    }
}

fn state() -> SyncState {
    SyncState::open_in_memory().unwrap()
}

// ---- baselines ------------------------------------------------------------------------------

#[test]
fn a_recorded_baseline_reads_back_exactly() {
    let mut s = state();
    let full = baseline("notes/a.md", "contents");
    let bare = Baseline {
        path: p("bare.md"),
        hash: ContentHash::of(b"x"),
        etag: None,
        stamp: None,
    };
    s.record(&full).unwrap();
    s.record(&bare).unwrap();
    assert_eq!(s.baseline(&p("notes/a.md")).unwrap(), Some(full));
    assert_eq!(s.baseline(&p("bare.md")).unwrap(), Some(bare));
    assert_eq!(s.baseline(&p("missing.md")).unwrap(), None);
}

#[test]
fn recording_again_replaces_every_field() {
    let mut s = state();
    s.record(&baseline("a.md", "old")).unwrap();
    let newer = Baseline {
        etag: None,
        stamp: None,
        ..baseline("a.md", "new")
    };
    s.record(&newer).unwrap();
    assert_eq!(s.baseline(&p("a.md")).unwrap(), Some(newer));
    assert_eq!(s.baselines().unwrap().len(), 1);
}

#[test]
fn baselines_are_listed_sorted_by_path() {
    let mut s = state();
    for path in ["z.md", "a/b.md", "a.md", "Ünï.md", "a/a.md"] {
        s.record(&baseline(path, path)).unwrap();
    }
    let paths: Vec<_> = s
        .baselines()
        .unwrap()
        .into_iter()
        .map(|b| b.path.to_string())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
    assert_eq!(paths.len(), 5);
}

#[test]
fn touch_refreshes_only_the_hint() {
    let mut s = state();
    let original = baseline("a.md", "x");
    s.record(&original).unwrap();
    let newer = LocalStamp {
        size: 99,
        mtime_ns: -5,
    }; // pre-1970 modification times exist
    assert!(s.touch(&p("a.md"), &newer).unwrap());
    let after = s.baseline(&p("a.md")).unwrap().unwrap();
    assert_eq!(after.stamp, Some(newer));
    assert_eq!(
        (after.hash, after.etag),
        (original.hash, original.etag),
        "what was agreed is untouched"
    );
    assert!(
        !s.touch(&p("nope.md"), &newer).unwrap(),
        "no baseline, nothing to touch"
    );
    assert_eq!(
        s.baseline(&p("nope.md")).unwrap(),
        None,
        "touch never invents a baseline"
    );
}

#[test]
fn the_rehash_hint_needs_a_stamp_and_both_fields_to_match() {
    let b = baseline("a.md", "abc");
    let same = b.stamp.unwrap();
    assert!(b.local_looks_unchanged(&same));
    assert!(!b.local_looks_unchanged(&LocalStamp {
        size: same.size + 1,
        ..same
    }));
    assert!(!b.local_looks_unchanged(&LocalStamp {
        mtime_ns: same.mtime_ns + 1,
        ..same
    }));
    assert!(!Baseline { stamp: None, ..b }.local_looks_unchanged(&same));
}

#[test]
fn forget_removes_and_is_idempotent() {
    let mut s = state();
    s.record(&baseline("a.md", "x")).unwrap();
    s.forget(&p("a.md")).unwrap();
    s.forget(&p("a.md")).unwrap();
    assert_eq!(s.baseline(&p("a.md")).unwrap(), None);
}

#[test]
fn record_many_writes_everything_and_replaces_existing() {
    let mut s = state();
    s.record(&baseline("a.md", "old")).unwrap();
    let batch: Vec<_> = (0..500)
        .map(|i| baseline(&format!("dir/{i:03}.md"), &format!("v{i}")))
        .chain([baseline("a.md", "new")])
        .collect();
    s.record_many(&batch).unwrap();
    assert_eq!(s.baselines().unwrap().len(), 501);
    assert_eq!(
        s.baseline(&p("a.md")).unwrap().unwrap().hash,
        ContentHash::of(b"new")
    );
}

#[test]
fn awkward_paths_round_trip() {
    let mut s = state();
    for path in [
        "it's.md",
        "a \"quoted\" name.md",
        "100%.md",
        "semi;colon.md",
        "x'); DROP TABLE baselines; --.md",
        "日本語/メモ.md",
        "emoji 🧙.md",
    ] {
        s.record(&baseline(path, path)).unwrap();
        assert_eq!(s.baseline(&p(path)).unwrap().unwrap().path.as_str(), path);
    }
    assert_eq!(s.baselines().unwrap().len(), 7, "the table survived");
}

// ---- conflicts ------------------------------------------------------------------------------

#[test]
fn a_conflict_can_exist_without_a_baseline() {
    // the same new file name created on two devices: nothing was ever agreed
    let mut s = state();
    s.mark_conflict(
        &p("new.md"),
        Some(&p("new.conflict-phone-20261004-1200.md")),
        None,
    )
    .unwrap();
    assert_eq!(s.baseline(&p("new.md")).unwrap(), None);
    assert_eq!(
        s.conflict(&p("new.md")).unwrap(),
        Some(Conflict {
            path: p("new.md"),
            copy: Some(p("new.conflict-phone-20261004-1200.md")),
            seen: None,
        })
    );
    assert_eq!(s.conflict(&p("other.md")).unwrap(), None);
}

#[test]
fn conflicts_are_listed_and_marking_again_updates_the_copy() {
    let mut s = state();
    s.mark_conflict(&p("b.md"), Some(&p("b.conflict-1.md")), None)
        .unwrap();
    s.mark_conflict(&p("a.md"), Some(&p("a.conflict-1.md")), None)
        .unwrap();
    s.mark_conflict(&p("b.md"), Some(&p("b.conflict-2.md")), None)
        .unwrap();
    let listed: Vec<_> = s
        .conflicts()
        .unwrap()
        .into_iter()
        .map(|c| (c.path.to_string(), c.copy.unwrap().to_string()))
        .collect();
    assert_eq!(
        listed,
        [
            ("a.md".into(), "a.conflict-1.md".into()),
            ("b.md".into(), "b.conflict-2.md".into())
        ]
    );
}

#[test]
fn resolving_records_the_new_baseline_and_lifts_the_flag_together() {
    let mut s = state();
    s.record(&baseline("a.md", "old base")).unwrap();
    s.mark_conflict(&p("a.md"), Some(&p("a.conflict-x.md")), None)
        .unwrap();

    let after = baseline("a.md", "remote's version");
    s.resolve(&after).unwrap();
    assert_eq!(s.baseline(&p("a.md")).unwrap(), Some(after));
    assert_eq!(s.conflict(&p("a.md")).unwrap(), None);

    // also for a path that never had a baseline
    s.mark_conflict(&p("b.md"), Some(&p("b.conflict-x.md")), None)
        .unwrap();
    s.resolve(&baseline("b.md", "r")).unwrap();
    assert!(s.baseline(&p("b.md")).unwrap().is_some() && s.conflict(&p("b.md")).unwrap().is_none());
}

#[test]
fn clearing_a_conflict_leaves_the_baseline_alone() {
    let mut s = state();
    let b = baseline("a.md", "x");
    s.record(&b).unwrap();
    s.mark_conflict(&p("a.md"), Some(&p("a.conflict-x.md")), None)
        .unwrap();
    s.clear_conflict(&p("a.md")).unwrap();
    s.clear_conflict(&p("a.md")).unwrap();
    assert_eq!(s.baseline(&p("a.md")).unwrap(), Some(b));
    assert!(s.conflicts().unwrap().is_empty());
}

#[test]
fn resolving_one_conflict_does_not_touch_another() {
    let mut s = state();
    s.mark_conflict(&p("a.md"), Some(&p("a.c.md")), None)
        .unwrap();
    s.mark_conflict(&p("b.md"), Some(&p("b.c.md")), None)
        .unwrap();
    s.resolve(&baseline("a.md", "x")).unwrap();
    assert_eq!(s.conflicts().unwrap().len(), 1);
    assert!(s.conflict(&p("b.md")).unwrap().is_some());
}

// ---- durability and refusing what it cannot read ----------------------------------------------

#[test]
fn state_survives_closing_and_reopening() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sync.db");
    {
        let mut s = SyncState::open(&db).unwrap();
        s.record(&baseline("a.md", "x")).unwrap();
        s.mark_conflict(&p("b.md"), Some(&p("b.conflict.md")), None)
            .unwrap();
        // dropped here with no explicit close or flush, as when the process is killed
    }
    let s = SyncState::open(&db).unwrap();
    assert_eq!(s.baseline(&p("a.md")).unwrap(), Some(baseline("a.md", "x")));
    assert_eq!(s.conflicts().unwrap().len(), 1);
}

#[test]
fn two_handles_on_one_file_see_each_others_commits() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sync.db");
    let mut writer = SyncState::open(&db).unwrap();
    let reader = SyncState::open(&db).unwrap();
    writer.record(&baseline("a.md", "x")).unwrap();
    assert!(reader.baseline(&p("a.md")).unwrap().is_some());
}

#[test]
fn a_database_from_a_newer_version_is_refused_and_left_untouched() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sync.db");
    {
        let mut s = SyncState::open(&db).unwrap();
        s.record(&baseline("a.md", "x")).unwrap();
        s.conn.pragma_update(None, "user_version", 99).unwrap();
    }
    let before = std::fs::read(&db).unwrap();
    let err = SyncState::open(&db).err().expect("refused");
    assert!(
        matches!(
            err,
            Error::UnsupportedVersion {
                found: 99,
                expected: 1
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before,
        "the file is exactly as it was"
    );
}

#[test]
fn a_foreign_sqlite_file_is_refused_not_adopted() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sync.db");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE something_else (x); INSERT INTO something_else VALUES (1);",
        )
        .unwrap();
    }
    let before = std::fs::read(&db).unwrap();
    assert!(matches!(
        SyncState::open(&db).err(),
        Some(Error::UnsupportedVersion { found: 0, .. })
    ));
    assert!(
        std::fs::read(&db).unwrap() == before,
        "the foreign file was modified"
    );
}

#[test]
fn a_file_that_is_not_a_database_is_an_error_and_is_not_deleted() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sync.db");
    std::fs::write(&db, b"this is not sqlite ".repeat(200)).unwrap();
    let before = std::fs::read(&db).unwrap();
    assert!(matches!(SyncState::open(&db).err(), Some(Error::Sqlite(_))));
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before,
        "never silently replaced: that would turn every file into a conflict"
    );
}

#[test]
fn reset_is_explicit_and_leaves_a_usable_empty_database() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sync.db");
    {
        let mut s = SyncState::open(&db).unwrap();
        s.record(&baseline("a.md", "x")).unwrap();
    }
    SyncState::reset(&db).unwrap();
    SyncState::reset(&db).unwrap(); // nothing left to delete is fine
    assert!(!db.exists());
    assert!(
        SyncState::open(&db)
            .unwrap()
            .baselines()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn damaged_rows_are_reported_not_panicked_on_or_skipped() {
    let mut s = state();
    s.record(&baseline("good.md", "x")).unwrap();
    for (sql, what) in [
        (
            "INSERT INTO baselines (path, hash) VALUES ('bad-hash.md', 'not hex')",
            "hash",
        ),
        (
            "INSERT INTO baselines (path, hash) VALUES ('../escape.md', 'abcd')",
            "path",
        ),
        (
            "INSERT INTO baselines (path, hash, size, mtime_ns) VALUES ('neg.md', '0000000000000000000000000000000000000000000000000000000000000000', -1, 0)",
            "size",
        ),
    ] {
        s.conn.execute(sql, []).unwrap();
        let sub = match what {
            "hash" => "bad-hash.md",
            "path" => "../escape.md",
            _ => "neg.md",
        };
        let result = s.baselines();
        assert!(
            matches!(result, Err(Error::Corrupt(_))),
            "{what}: {result:?}"
        );
        s.conn
            .execute("DELETE FROM baselines WHERE path = ?1", [sub])
            .unwrap();
    }
    assert_eq!(s.baselines().unwrap().len(), 1, "back to the good row only");

    s.conn
        .execute(
            "INSERT INTO conflicts (path, copy_path) VALUES ('c.md', '/abs/copy.md')",
            [],
        )
        .unwrap();
    assert!(matches!(s.conflicts(), Err(Error::Corrupt(_))));
}

#[test]
fn a_conflict_can_have_no_copy_and_release_drops_the_baseline_with_the_flag() {
    // edited here, deleted there: no other version exists to keep
    let mut s = state();
    s.record(&baseline("a.md", "x")).unwrap();
    s.mark_conflict(&p("a.md"), None, None).unwrap();
    assert_eq!(
        s.conflict(&p("a.md")).unwrap(),
        Some(Conflict {
            path: p("a.md"),
            copy: None,
            seen: None,
        })
    );
    s.release(&p("a.md")).unwrap();
    assert_eq!(
        (
            s.baseline(&p("a.md")).unwrap(),
            s.conflict(&p("a.md")).unwrap()
        ),
        (None, None)
    );
    s.release(&p("never.md")).unwrap();
}

#[test]
fn a_conflict_remembers_the_remote_version_the_copy_was_made_from() {
    let mut s = state();
    let seen = SeenVersion {
        hash: ContentHash::of(b"remote then"),
        etag: Some("\"e-then\"".into()),
    };
    s.mark_conflict(&p("a.md"), Some(&p("a.conflict.md")), Some(&seen))
        .unwrap();
    let got = s.conflict(&p("a.md")).unwrap().unwrap();
    assert_eq!(got.seen, Some(seen.clone()));
    assert_eq!(s.conflicts().unwrap()[0].seen, Some(seen));

    // an ETag may be absent, and marking again replaces what was remembered
    let newer = SeenVersion {
        hash: ContentHash::of(b"later"),
        etag: None,
    };
    s.mark_conflict(&p("a.md"), Some(&p("a.conflict-2.md")), Some(&newer))
        .unwrap();
    assert_eq!(s.conflict(&p("a.md")).unwrap().unwrap().seen, Some(newer));
}

#[test]
fn a_damaged_seen_hash_is_reported() {
    let s = state();
    s.conn.execute("INSERT INTO conflicts (path, copy_path, seen_hash) VALUES ('a.md', 'a.c.md', 'not hex')", []).unwrap();
    assert!(matches!(s.conflicts(), Err(Error::Corrupt(_))));
}
