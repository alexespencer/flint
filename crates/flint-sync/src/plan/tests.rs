use std::collections::{BTreeMap, BTreeSet};

use flint_core::{ContentHash, StorePath};
use flint_remote::ETag;
use flint_sync_state::Baseline;
use proptest::prelude::*;

use super::*;

fn p(s: &str) -> StorePath {
    StorePath::new(s).unwrap()
}

fn h(s: &str) -> ContentHash {
    ContentHash::of(s.as_bytes())
}

fn etag(s: &str) -> ETag {
    ETag::new(format!("\"{s}\""))
}

fn base(path: &str, content: &str, tag: &str) -> Baseline {
    Baseline {
        path: p(path),
        hash: h(content),
        etag: Some(format!("\"{tag}\"")),
        stamp: None,
    }
}

fn local(content: &str) -> LocalFile {
    LocalFile { hash: h(content) }
}

fn remote(tag: &str) -> RemoteFile {
    RemoteFile { etag: etag(tag) }
}

/// One path's worth of state, run through the planner.
fn one(l: Option<LocalFile>, r: Option<RemoteFile>, b: Option<Baseline>) -> Vec<Action> {
    let mut ls = BTreeMap::new();
    let mut rs = BTreeMap::new();
    let mut bs = BTreeMap::new();
    if let Some(l) = l {
        ls.insert(p("a.md"), l);
    }
    if let Some(r) = r {
        rs.insert(p("a.md"), r);
    }
    if let Some(b) = b {
        bs.insert(p("a.md"), b);
    }
    plan(&ls, &rs, &bs, &BTreeSet::new())
}

fn only(actions: Vec<Action>) -> Action {
    assert_eq!(actions.len(), 1, "{actions:?}");
    actions.into_iter().next().unwrap()
}

// ---- the plan's rules table, one test per row -------------------------------------------------
// The baseline throughout: content "base" agreed at ETag "e1".

fn agreed() -> Option<Baseline> {
    Some(base("a.md", "base", "e1"))
}

#[test]
fn unchanged_and_unchanged_does_nothing() {
    assert_eq!(one(Some(local("base")), Some(remote("e1")), agreed()), []);
}

#[test]
fn changed_locally_only_uploads_if_the_remote_is_still_the_version_we_saw() {
    let action = only(one(Some(local("edited")), Some(remote("e1")), agreed()));
    assert_eq!(
        action,
        Action::Upload {
            path: p("a.md"),
            mode: UploadMode::Replace(etag("e1"))
        }
    );
}

#[test]
fn new_locally_and_absent_remotely_creates() {
    let action = only(one(Some(local("new")), None, None));
    assert_eq!(
        action,
        Action::Upload {
            path: p("a.md"),
            mode: UploadMode::Create
        }
    );
}

#[test]
fn unchanged_locally_and_changed_remotely_downloads() {
    let action = only(one(Some(local("base")), Some(remote("e2")), agreed()));
    assert_eq!(action, Action::Download { path: p("a.md") });
}

#[test]
fn changed_on_both_sides_is_reconciled_by_comparing_contents() {
    let action = only(one(Some(local("mine")), Some(remote("e2")), agreed()));
    assert_eq!(
        action,
        Action::Reconcile {
            path: p("a.md"),
            reason: ReconcileReason::BothChanged
        }
    );
}

#[test]
fn new_on_both_sides_is_reconciled_by_comparing_contents() {
    let action = only(one(Some(local("mine")), Some(remote("theirs")), None));
    assert_eq!(
        action,
        Action::Reconcile {
            path: p("a.md"),
            reason: ReconcileReason::BothNew
        }
    );
}

#[test]
fn deleted_locally_and_unchanged_remotely_deletes_remotely_if_still_that_version() {
    let action = only(one(None, Some(remote("e1")), agreed()));
    assert_eq!(
        action,
        Action::DeleteRemote {
            path: p("a.md"),
            expect: etag("e1")
        }
    );
}

#[test]
fn unchanged_locally_and_deleted_remotely_moves_the_local_copy_to_the_trash() {
    let action = only(one(Some(local("base")), None, agreed()));
    assert_eq!(action, Action::TrashLocal { path: p("a.md") });
}

#[test]
fn changed_locally_and_deleted_remotely_is_a_conflict_that_keeps_the_local_file() {
    let action = only(one(Some(local("edited")), None, agreed()));
    assert_eq!(
        action,
        Action::Conflict {
            path: p("a.md"),
            kind: ConflictKind::LocalChangedRemoteDeleted
        }
    );
}

#[test]
fn deleted_locally_and_changed_remotely_is_a_conflict_that_restores_the_remote_copy() {
    let action = only(one(None, Some(remote("e2")), agreed()));
    assert_eq!(
        action,
        Action::Conflict {
            path: p("a.md"),
            kind: ConflictKind::LocalDeletedRemoteChanged
        }
    );
}

// ---- the rows the table leaves implicit -------------------------------------------------------

#[test]
fn new_remotely_and_absent_locally_downloads() {
    assert_eq!(
        only(one(None, Some(remote("e1")), None)),
        Action::Download { path: p("a.md") }
    );
}

#[test]
fn gone_from_both_sides_forgets_the_baseline() {
    assert_eq!(
        only(one(None, None, agreed())),
        Action::Forget { path: p("a.md") }
    );
}

#[test]
fn nothing_anywhere_is_nothing() {
    assert_eq!(one(None, None, None), []);
}

// ---- how "changed" is decided -----------------------------------------------------------------

#[test]
fn a_baseline_without_an_etag_makes_the_remote_count_as_changed() {
    let no_etag = Some(Baseline {
        etag: None,
        ..base("a.md", "base", "e1")
    });
    // local unchanged, remote might have moved: fetch it
    assert_eq!(
        only(one(
            Some(local("base")),
            Some(remote("e1")),
            no_etag.clone()
        )),
        Action::Download { path: p("a.md") }
    );
    // local changed too: compare contents rather than guess
    assert_eq!(
        only(one(
            Some(local("edited")),
            Some(remote("e1")),
            no_etag.clone()
        )),
        Action::Reconcile {
            path: p("a.md"),
            reason: ReconcileReason::BothChanged
        }
    );
    // local deleted: never delete the remote on a guess
    assert_eq!(
        only(one(None, Some(remote("e1")), no_etag)),
        Action::Conflict {
            path: p("a.md"),
            kind: ConflictKind::LocalDeletedRemoteChanged
        }
    );
}

#[test]
fn a_crash_after_uploading_and_before_recording_looks_like_both_changed_and_is_reconciled_not_conflicted()
 {
    // We uploaded "mine" (remote ETag became e2) but died before recording the baseline: the baseline is
    // still the old one. The planner sends it to Reconcile, where the executor sees the remote's SHA-256
    // equals the local hash, and just records it.
    let action = only(one(Some(local("mine")), Some(remote("e2")), agreed()));
    assert!(matches!(
        action,
        Action::Reconcile {
            reason: ReconcileReason::BothChanged,
            ..
        }
    ));
}

#[test]
fn a_file_that_changed_and_was_changed_back_is_unchanged() {
    // hashes, not timestamps: editing and undoing leaves nothing to sync
    assert_eq!(one(Some(local("base")), Some(remote("e1")), agreed()), []);
}

// ---- conflicts held back ----------------------------------------------------------------------

#[test]
fn held_paths_get_no_action_whatever_their_state() {
    let held: BTreeSet<_> = [p("a.md")].into();
    for (l, r, b) in [
        (Some(local("x")), None, None),
        (None, Some(remote("e")), None),
        (Some(local("x")), Some(remote("e")), None),
        (Some(local("edit")), Some(remote("e2")), agreed()),
        (None, None, agreed()),
        (Some(local("base")), Some(remote("e2")), agreed()),
    ] {
        let mut ls = BTreeMap::new();
        let mut rs = BTreeMap::new();
        let mut bs = BTreeMap::new();
        if let Some(l) = l {
            ls.insert(p("a.md"), l);
        }
        if let Some(r) = r {
            rs.insert(p("a.md"), r);
        }
        if let Some(b) = b {
            bs.insert(p("a.md"), b);
        }
        assert_eq!(plan(&ls, &rs, &bs, &held), []);
    }
}

#[test]
fn many_paths_are_planned_independently_and_in_path_order() {
    let ls = BTreeMap::from([
        (p("z.md"), local("new")),
        (p("m.md"), local("base")),
        (p("b.md"), local("edited")),
        (p("held.md"), local("x")),
    ]);
    let rs = BTreeMap::from([
        (p("m.md"), remote("e2")),
        (p("b.md"), remote("e1")),
        (p("only-remote.md"), remote("r")),
    ]);
    let bs = BTreeMap::from([
        (p("m.md"), base("m.md", "base", "e1")),
        (p("b.md"), base("b.md", "base", "e1")),
        (p("gone.md"), base("gone.md", "x", "e9")),
    ]);
    let held = BTreeSet::from([p("held.md")]);
    let got: Vec<_> = plan(&ls, &rs, &bs, &held)
        .into_iter()
        .map(|a| {
            let kind = match &a {
                Action::Upload {
                    mode: UploadMode::Create,
                    ..
                } => "create",
                Action::Upload { .. } => "replace",
                Action::Download { .. } => "download",
                Action::Forget { .. } => "forget",
                other => panic!("unexpected {other:?}"),
            };
            (a.path().to_string(), kind)
        })
        .collect();
    assert_eq!(
        got,
        [
            ("b.md".into(), "replace"),
            ("gone.md".into(), "forget"),
            ("m.md".into(), "download"),
            ("only-remote.md".into(), "download"),
            ("z.md".into(), "create")
        ]
    );
}

// ---- safety, over every state of the world ----------------------------------------------------

/// What one path can look like, each side drawn from a handful of values so that agreement, change
/// and absence all come up often.
#[derive(Debug, Clone)]
struct World {
    local: Option<u8>,
    remote: Option<u8>,
    baseline: Option<(u8, Option<u8>)>,
}

fn world() -> impl Strategy<Value = World> {
    (
        proptest::option::of(0u8..4),
        proptest::option::of(0u8..4),
        proptest::option::of((0u8..4, proptest::option::weighted(0.9, 0u8..4))),
    )
        .prop_map(|(local, remote, baseline)| World {
            local,
            remote,
            baseline,
        })
}

fn actions_for(w: &World) -> Vec<Action> {
    let content = |n: u8| format!("content-{n}");
    one(
        w.local.map(|n| local(&content(n))),
        w.remote.map(|n| remote(&format!("etag-{n}"))),
        w.baseline.map(|(c, e)| Baseline {
            etag: e.map(|n| format!("\"etag-{n}\"")),
            ..base("a.md", &content(c), "x")
        }),
    )
}

proptest! {
    /// No action may overwrite or remove anything that is not either unchanged since the last agreement
    /// or already present on the other side. This is "never silently lose a write", stated per action.
    #[test]
    fn no_action_destroys_an_unagreed_version(w in world()) {
        let content = |n: u8| format!("content-{n}");
        let base_hash = w.baseline.map(|(c, _)| h(&content(c)));
        let base_etag = w.baseline.and_then(|(_, e)| e).map(|n| format!("\"etag-{n}\""));
        let local_unchanged = w.local.is_some() && w.local.map(|n| h(&content(n))) == base_hash;
        let remote_unchanged = w.remote.is_some() && base_etag.is_some() && w.remote.map(|n| format!("\"etag-{n}\"")) == base_etag;

        for action in actions_for(&w) {
            match action {
                // replaces the remote: only a remote that is exactly the version we last agreed on, or no remote at all
                Action::Upload { mode: UploadMode::Replace(ref expected), .. } => {
                    prop_assert!(w.local.is_some() && !local_unchanged, "uploading something unchanged: {w:?}");
                    prop_assert!(remote_unchanged && Some(expected.as_str().to_owned()) == base_etag, "replace must be conditional on the agreed ETag: {w:?}");
                }
                Action::Upload { mode: UploadMode::Create, .. } => {
                    prop_assert!(w.local.is_some() && w.remote.is_none() && w.baseline.is_none(), "create over something: {w:?}");
                }
                // replaces the local file: only if it is absent or exactly what we last agreed on
                Action::Download { .. } => {
                    prop_assert!(w.remote.is_some(), "{w:?}");
                    prop_assert!(w.local.is_none() && w.baseline.is_none() || local_unchanged, "download over a local edit: {w:?}");
                }
                // removes the remote: only if we deleted it here and it is still the version we agreed
                Action::DeleteRemote { ref expect, .. } => {
                    prop_assert!(w.local.is_none() && remote_unchanged && Some(expect.as_str().to_owned()) == base_etag, "{w:?}");
                }
                // moves the local file away: only if it is unchanged and the remote is gone
                Action::TrashLocal { .. } => {
                    prop_assert!(local_unchanged && w.remote.is_none(), "{w:?}");
                }
                Action::Reconcile { reason: ReconcileReason::BothNew, .. } => {
                    prop_assert!(w.local.is_some() && w.remote.is_some() && w.baseline.is_none(), "{w:?}");
                }
                Action::Reconcile { reason: ReconcileReason::BothChanged, .. } => {
                    prop_assert!(w.local.is_some() && !local_unchanged && w.remote.is_some(), "{w:?}");
                }
                Action::Conflict { kind: ConflictKind::LocalChangedRemoteDeleted, .. } => {
                    prop_assert!(w.local.is_some() && !local_unchanged && w.remote.is_none(), "{w:?}");
                }
                Action::Conflict { kind: ConflictKind::LocalDeletedRemoteChanged, .. } => {
                    prop_assert!(w.local.is_none() && w.remote.is_some() && !remote_unchanged, "{w:?}");
                }
                Action::Forget { .. } => {
                    prop_assert!(w.local.is_none() && w.remote.is_none() && w.baseline.is_some(), "{w:?}");
                }
            }
        }
    }

    /// Whenever the two sides could differ, something happens: the planner never ignores a path where
    /// there is work, and it emits at most one action per path.
    #[test]
    fn at_most_one_action_and_never_idle_when_the_sides_differ(w in world()) {
        let actions = actions_for(&w);
        prop_assert!(actions.len() <= 1);
        let content = |n: u8| format!("content-{n}");
        let in_sync = match (w.local, w.remote, w.baseline) {
            (None, None, None) => true,
            (Some(l), Some(r), Some((c, Some(e)))) => l == c && r == e,
            _ => false,
        };
        prop_assert_eq!(actions.is_empty(), in_sync, "{:?} {:?}", w, content(0));
    }

    /// Many paths at once: the output is sorted by path, each path at most once, held paths never.
    #[test]
    fn the_whole_plan_is_sorted_unique_and_skips_held_paths(
        worlds in proptest::collection::btree_map("[a-d]{1,2}\\.md", world(), 0..12),
        held in proptest::collection::btree_set("[a-d]{1,2}\\.md", 0..4),
    ) {
        let content = |n: u8| format!("content-{n}");
        let mut ls = BTreeMap::new();
        let mut rs = BTreeMap::new();
        let mut bs = BTreeMap::new();
        for (name, w) in &worlds {
            let path = p(name);
            if let Some(n) = w.local { ls.insert(path.clone(), local(&content(n))); }
            if let Some(n) = w.remote { rs.insert(path.clone(), remote(&format!("etag-{n}"))); }
            if let Some((c, e)) = w.baseline {
                bs.insert(path.clone(), Baseline { etag: e.map(|n| format!("\"etag-{n}\"")), ..base(name, &content(c), "x") });
            }
        }
        let held: BTreeSet<_> = held.iter().map(|n| p(n)).collect();
        let actions = plan(&ls, &rs, &bs, &held);
        let paths: Vec<_> = actions.iter().map(|a| a.path().clone()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        sorted.dedup();
        prop_assert_eq!(&paths, &sorted, "sorted and one action per path");
        prop_assert!(paths.iter().all(|path| !held.contains(path)));
        // the planner agrees with itself path by path
        for action in &actions {
            let alone = plan(
                &ls.iter().filter(|(k, _)| *k == action.path()).map(|(k, v)| (k.clone(), v.clone())).collect(),
                &rs.iter().filter(|(k, _)| *k == action.path()).map(|(k, v)| (k.clone(), v.clone())).collect(),
                &bs.iter().filter(|(k, _)| *k == action.path()).map(|(k, v)| (k.clone(), v.clone())).collect(),
                &BTreeSet::new(),
            );
            prop_assert_eq!(alone, vec![action.clone()]);
        }
    }
}
