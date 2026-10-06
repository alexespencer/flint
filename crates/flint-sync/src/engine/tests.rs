use std::sync::Arc;

use bytes::Bytes;
use flint_store_fs::{KEY_FILE, LocalFs, MemFs};

use crate::report::{ConflictOutcome, Refusal};
use crate::testing::{Fault, MemDevice, Op, Spy, World, p};
use flint_remote::{Precondition, RemoteStore};

fn paths(list: &[flint_core::StorePath]) -> Vec<&str> {
    list.iter().map(|p| p.as_str()).collect()
}

// ---- the ordinary flows -------------------------------------------------------------------------

#[tokio::test]
async fn a_new_file_is_uploaded_and_then_everything_is_quiet() {
    let world = World::new();
    let mut mac = world.device("Mac");
    mac.write("notes/a.md", "hello");

    let first = mac.sync().await;
    assert_eq!(paths(&first.uploaded), ["notes/a.md"]);
    assert!(
        first.failed.is_empty() && first.refused.is_empty() && first.conflicts.is_empty(),
        "{first:?}"
    );
    assert_eq!(
        world.remote_plain("notes/a.md").await.as_deref(),
        Some("hello")
    );
    let baseline = mac
        .syncer
        .state()
        .baseline(&p("notes/a.md"))
        .unwrap()
        .expect("a baseline was recorded");
    assert!(baseline.etag.is_some() && baseline.stamp.is_some());

    for _ in 0..3 {
        assert!(
            mac.sync().await.is_quiet(),
            "nothing changed, nothing to do"
        );
    }
}

#[tokio::test]
async fn a_second_device_downloads_what_the_first_uploaded() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("Characters/Baron.md", "A [[Vallaki]] noble.");
    mac.write("Images/pic.png", "\u{89}PNG fake image bytes");
    mac.sync().await;

    let report = phone.sync().await;
    assert_eq!(
        paths(&report.downloaded),
        ["Characters/Baron.md", "Images/pic.png"]
    );
    assert_eq!(
        phone.read("Characters/Baron.md").as_deref(),
        Some("A [[Vallaki]] noble.")
    );
    assert_eq!(
        phone.read("Images/pic.png").as_deref(),
        Some("\u{89}PNG fake image bytes")
    );
    assert!(phone.sync().await.is_quiet());
    assert!(
        mac.sync().await.is_quiet(),
        "the first device sees no change either"
    );
}

#[tokio::test]
async fn an_edit_propagates() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;

    mac.write("a.md", "v2 with more");
    assert_eq!(paths(&mac.sync().await.uploaded), ["a.md"]);
    assert_eq!(paths(&phone.sync().await.downloaded), ["a.md"]);
    assert_eq!(phone.read("a.md").as_deref(), Some("v2 with more"));
    assert!(phone.sync().await.is_quiet() && mac.sync().await.is_quiet());
}

#[tokio::test]
async fn a_deletion_propagates_and_the_other_device_keeps_the_file_in_its_trash() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("a.md", "doomed");
    mac.sync().await;
    phone.sync().await;

    mac.delete("a.md");
    assert_eq!(paths(&mac.sync().await.deleted_remote), ["a.md"]);
    assert_eq!(world.remote_plain("a.md").await, None);
    assert!(mac.syncer.state().baseline(&p("a.md")).unwrap().is_none());

    assert_eq!(paths(&phone.sync().await.trashed_local), ["a.md"]);
    assert_eq!(phone.read("a.md"), None);
    assert!(
        phone.syncer.fs().exists(&p(".trash/a.md")).unwrap(),
        "recoverable from the trash"
    );
    assert!(phone.syncer.state().baseline(&p("a.md")).unwrap().is_none());
    assert!(phone.sync().await.is_quiet() && mac.sync().await.is_quiet());
}

#[tokio::test]
async fn a_rename_is_a_delete_plus_a_create() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("old.md", "contents");
    mac.sync().await;
    phone.sync().await;

    mac.enc.rename(&p("old.md"), &p("new/name.md")).unwrap();
    let report = mac.sync().await;
    assert_eq!(
        (paths(&report.uploaded), paths(&report.deleted_remote)),
        (vec!["new/name.md"], vec!["old.md"])
    );
    phone.sync().await;
    assert_eq!(
        (phone.read("old.md"), phone.read("new/name.md").as_deref()),
        (None, Some("contents"))
    );
}

#[tokio::test]
async fn hidden_files_are_neither_uploaded_nor_downloaded() {
    let world = World::new();
    let mut mac = world.device("Mac");
    mac.write("a.md", "x");
    // plaintext junk a user dragged in, and a hidden object somebody put on the remote
    mac.syncer
        .fs()
        .write_atomic(&p(".git/config"), b"[core]")
        .unwrap();
    mac.syncer
        .fs()
        .write_atomic(&p(".obsidian/app.json"), b"{}")
        .unwrap();
    let rival = world.remote();
    rival
        .put(
            &p(".hidden/planted.md"),
            Bytes::from_static(b"x"),
            &flint_core::ContentHash::of(b"x"),
            Precondition::Unconditional,
        )
        .await
        .unwrap();

    let report = mac.sync().await;
    assert_eq!(paths(&report.uploaded), ["a.md"]);
    assert!(
        report.refused.is_empty() && report.downloaded.is_empty(),
        "{report:?}"
    );
    assert_eq!(world.remote_paths().await, [".hidden/planted.md", "a.md"]);
    assert!(!mac.syncer.fs().exists(&p(".hidden/planted.md")).unwrap());
}

// ---- conflicts ----------------------------------------------------------------------------------

/// Both devices start from "v1"; A and B then each edit it differently, and A syncs first.
async fn diverged() -> (World, MemDevice, MemDevice) {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;
    mac.write("a.md", "edited on the Mac");
    phone.write("a.md", "edited on the Phone");
    mac.sync().await;
    (world, mac, phone)
}

#[tokio::test]
async fn editing_the_same_file_on_two_devices_keeps_both_versions_and_overwrites_neither() {
    let (world, _mac, mut phone) = diverged().await;
    let before = phone.syncer.fs().read(&p("a.md")).unwrap();

    let report = phone.sync().await;
    let copy = p("a.conflict-Phone-20261004-1234.md");
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
    assert_eq!(report.conflicts[0].path, p("a.md"));
    assert_eq!(
        report.conflicts[0].outcome,
        ConflictOutcome::Diverged { copy: copy.clone() }
    );
    assert!(report.uploaded.is_empty() && report.downloaded.is_empty());

    // the local file is byte-for-byte what it was, the other side's version sits beside it, readable
    assert_eq!(phone.syncer.fs().read(&p("a.md")).unwrap(), before);
    assert_eq!(phone.read("a.md").as_deref(), Some("edited on the Phone"));
    assert_eq!(
        phone.read("a.conflict-Phone-20261004-1234.md").as_deref(),
        Some("edited on the Mac")
    );
    // and the remote still has the version that got there first
    assert_eq!(
        world.remote_plain("a.md").await.as_deref(),
        Some("edited on the Mac")
    );
    assert_eq!(
        phone
            .syncer
            .state()
            .conflict(&p("a.md"))
            .unwrap()
            .unwrap()
            .copy,
        Some(copy)
    );
}

#[tokio::test]
async fn a_held_path_is_skipped_until_resolved_and_the_copy_syncs_like_any_other_file() {
    let (world, mut mac, mut phone) = diverged().await;
    phone.sync().await;

    let again = phone.sync().await;
    assert_eq!(
        paths(&again.uploaded),
        ["a.conflict-Phone-20261004-1234.md"],
        "the copy is an ordinary new file; a.md is held back"
    );
    assert!(again.conflicts.is_empty());
    assert_eq!(
        world.remote_plain("a.md").await.as_deref(),
        Some("edited on the Mac"),
        "still not overwritten"
    );

    let mac_report = mac.sync().await;
    assert_eq!(
        paths(&mac_report.downloaded),
        ["a.conflict-Phone-20261004-1234.md"]
    );
    assert_eq!(
        mac.read("a.conflict-Phone-20261004-1234.md").as_deref(),
        Some("edited on the Mac")
    );
}

#[tokio::test]
async fn resolving_pushes_the_users_final_version_conditionally() {
    let (world, mut mac, mut phone) = diverged().await;
    phone.sync().await;

    phone.write("a.md", "merged by hand");
    phone.syncer.resolve(&p("a.md")).await.unwrap();
    assert!(phone.syncer.state().conflict(&p("a.md")).unwrap().is_none());

    let report = phone.sync().await;
    assert!(paths(&report.uploaded).contains(&"a.md"), "{report:?}");
    assert_eq!(
        world.remote_plain("a.md").await.as_deref(),
        Some("merged by hand")
    );
    mac.sync().await;
    assert_eq!(mac.read("a.md").as_deref(), Some("merged by hand"));
    assert!(phone.sync().await.conflicts.is_empty());
}

#[tokio::test]
async fn resolving_when_the_remote_moved_again_conflicts_afresh_instead_of_overwriting() {
    let (world, mut mac, mut phone) = diverged().await;
    phone.sync().await;
    phone.write("a.md", "merged by hand");
    phone.syncer.resolve(&p("a.md")).await.unwrap();

    // before the phone syncs again, the Mac edits once more
    mac.write("a.md", "yet another Mac edit");
    mac.sync().await;

    let report = phone.sync().await;
    assert!(
        report.uploaded.iter().all(|u| u.as_str() != "a.md"),
        "{report:?}"
    );
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
    assert_eq!(
        world.remote_plain("a.md").await.as_deref(),
        Some("yet another Mac edit")
    );
    assert_eq!(phone.read("a.md").as_deref(), Some("merged by hand"));
}

/// Found by the simulation: the remote moves on *between* the conflict and the user resolving it.
/// Resolving used to adopt the remote's current version as the baseline, so the user's merge (made from
/// the version in the conflict copy) overwrote an edit nobody had looked at.
#[tokio::test]
async fn resolving_after_the_remote_moved_on_does_not_overwrite_the_edit_nobody_saw() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    // the same file is created on both devices, with different text
    mac.write("c.md", "mac's first");
    mac.sync().await;
    phone.write("c.md", "phone's first");
    let conflict = phone.sync().await;
    assert_eq!(conflict.conflicts.len(), 1, "{conflict:?}");

    // the Mac builds on the version the phone has a copy of, and uploads it
    mac.write("c.md", "mac's second, after the conflict");
    mac.sync().await;

    // the phone's user merges what they saw (their own file and the copy) and resolves
    phone.write("c.md", "phone's merge of its own file and mac's first");
    phone.syncer.resolve(&p("c.md")).await.unwrap();
    let report = phone.sync().await;

    assert!(
        report.uploaded.iter().all(|u| u.as_str() != "c.md"),
        "the merge must not be uploaded over an unseen edit: {report:?}"
    );
    assert_eq!(
        world.remote_plain("c.md").await.as_deref(),
        Some("mac's second, after the conflict"),
        "still there"
    );
    assert_eq!(
        report.conflicts.len(),
        1,
        "both are kept instead: {report:?}"
    );
    assert_eq!(
        phone.read("c.md").as_deref(),
        Some("phone's merge of its own file and mac's first")
    );
    assert!(
        phone
            .copies()
            .iter()
            .any(|c| phone.read(c).as_deref() == Some("mac's second, after the conflict")),
        "the unseen edit sits in a new conflict copy"
    );
}

#[tokio::test]
async fn identical_edits_on_both_devices_are_not_a_conflict() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;
    // the same fix, typed twice: different ciphertext, same plaintext
    mac.write("a.md", "typo fixed");
    phone.write("a.md", "typo fixed");
    mac.sync().await;

    let report = phone.sync().await;
    assert_eq!(paths(&report.agreed), ["a.md"]);
    assert!(
        report.conflicts.is_empty() && report.uploaded.is_empty() && report.downloaded.is_empty(),
        "{report:?}"
    );
    assert!(phone.copies().is_empty());
    assert!(phone.sync().await.is_quiet() && mac.sync().await.is_quiet());
}

#[tokio::test]
async fn the_same_new_file_name_on_two_devices() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("same.md", "from the Mac");
    phone.write("same.md", "from the Phone");
    phone.write("twin.md", "identical");
    mac.write("twin.md", "identical");
    mac.sync().await;

    let report = phone.sync().await;
    assert_eq!(paths(&report.agreed), ["twin.md"], "same text: just agree");
    assert_eq!(report.conflicts.len(), 1, "different text: keep both");
    assert_eq!(phone.read("same.md").as_deref(), Some("from the Phone"));
    assert_eq!(
        phone
            .read("same.conflict-Phone-20261004-1234.md")
            .as_deref(),
        Some("from the Mac")
    );
    assert_eq!(
        world.remote_plain("same.md").await.as_deref(),
        Some("from the Mac")
    );
}

#[tokio::test]
async fn edited_here_and_deleted_there_keeps_the_local_file_and_asks() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;
    mac.delete("a.md");
    mac.sync().await;
    phone.write("a.md", "I kept working on it");

    let report = phone.sync().await;
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(
        report.conflicts[0].outcome,
        ConflictOutcome::EditedHereDeletedThere
    );
    assert_eq!(
        phone.read("a.md").as_deref(),
        Some("I kept working on it"),
        "never trashed"
    );
    assert!(phone.sync().await.is_quiet(), "held until the user decides");

    // the user decides to keep it: it is uploaded as a new file
    phone.syncer.resolve(&p("a.md")).await.unwrap();
    assert_eq!(paths(&phone.sync().await.uploaded), ["a.md"]);
    assert_eq!(
        world.remote_plain("a.md").await.as_deref(),
        Some("I kept working on it")
    );
}

#[tokio::test]
async fn deleted_here_and_edited_there_restores_the_edit_so_it_is_not_lost() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;
    phone.delete("a.md");
    mac.write("a.md", "valuable new text");
    mac.sync().await;

    let report = phone.sync().await;
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(
        report.conflicts[0].outcome,
        ConflictOutcome::RestoredEditedRemotely
    );
    assert_eq!(phone.read("a.md").as_deref(), Some("valuable new text"));
    assert!(
        phone.syncer.state().conflict(&p("a.md")).unwrap().is_none(),
        "nothing left to decide"
    );
    assert!(phone.sync().await.is_quiet());
}

#[tokio::test]
async fn a_taken_conflict_copy_name_gets_a_number() {
    let (_world, _mac, mut phone) = diverged().await;
    // something already has the first choice of name
    phone.write("a.conflict-Phone-20261004-1234.md", "an unrelated file");
    phone.sync().await;
    assert_eq!(
        phone.read("a.conflict-Phone-20261004-1234.md").as_deref(),
        Some("an unrelated file"),
        "never overwritten"
    );
    assert_eq!(
        phone.read("a.conflict-Phone-20261004-1234-2.md").as_deref(),
        Some("edited on the Mac")
    );
}

#[tokio::test]
async fn resolving_something_that_is_not_in_conflict_is_an_error() {
    let world = World::new();
    let mut mac = world.device("Mac");
    assert!(matches!(
        mac.syncer.resolve(&p("a.md")).await,
        Err(crate::Error::NotInConflict(_))
    ));
}

// ---- what sync refuses to do --------------------------------------------------------------------

#[tokio::test]
async fn a_plaintext_file_is_never_uploaded() {
    let world = World::new();
    let mut mac = world.device("Mac");
    // dropped into the folder, not yet adopted (encrypted)
    mac.syncer
        .fs()
        .write_atomic(&p("secret.md"), b"# my private notes, in the clear")
        .unwrap();

    let report = mac.sync().await;
    assert_eq!(
        report.refused,
        [(p("secret.md"), Refusal::LocalNotCiphertext)]
    );
    assert!(report.uploaded.is_empty());
    assert_eq!(
        world.remote_paths().await,
        Vec::<String>::new(),
        "nothing reached the remote"
    );
    assert!(
        mac.syncer
            .state()
            .baseline(&p("secret.md"))
            .unwrap()
            .is_none()
    );

    // once adopted it goes up like anything else
    mac.enc.adopt_plaintext(std::time::Duration::ZERO).unwrap();
    assert_eq!(paths(&mac.sync().await.uploaded), ["secret.md"]);
}

#[tokio::test]
async fn a_remote_object_that_is_not_ciphertext_is_never_written_locally() {
    let world = World::new();
    let mut mac = world.device("Mac");
    world
        .remote()
        .put(
            &p("planted.md"),
            Bytes::from_static(b"# plaintext planted by a hostile server"),
            &flint_core::ContentHash::of(b"# plaintext planted by a hostile server"),
            Precondition::Unconditional,
        )
        .await
        .unwrap();

    let report = mac.sync().await;
    assert_eq!(
        report.refused,
        [(p("planted.md"), Refusal::RemoteNotCiphertext)]
    );
    assert!(report.downloaded.is_empty());
    assert!(
        !mac.syncer.fs().exists(&p("planted.md")).unwrap(),
        "so adoption can never mistake it for the user's own file"
    );
    assert!(
        mac.syncer
            .state()
            .baseline(&p("planted.md"))
            .unwrap()
            .is_none()
    );
    // and it is reported every time, not silently dropped
    assert_eq!(mac.sync().await.refused.len(), 1);
}

#[tokio::test]
async fn a_local_file_that_no_longer_decrypts_is_not_uploaded_over_the_good_copy() {
    let world = World::new();
    let mut mac = world.device("Mac");
    let mut phone = world.device("Phone");
    mac.write("a.md", "good text");
    mac.sync().await;
    phone.sync().await;

    // another program overwrote part of the encrypted file: it still starts with Flint's header
    let mut damaged = mac.syncer.fs().read(&p("a.md")).unwrap();
    let last = damaged.len() - 1;
    damaged[last] ^= 0xff;
    mac.syncer.fs().write_atomic(&p("a.md"), &damaged).unwrap();

    let report = mac.sync().await;
    assert_eq!(report.refused, [(p("a.md"), Refusal::LocalNotDecryptable)]);
    assert!(report.uploaded.is_empty());
    assert_eq!(
        phone.sync().await.downloaded,
        Vec::<flint_core::StorePath>::new()
    );
    assert_eq!(
        phone.read("a.md").as_deref(),
        Some("good text"),
        "the other device still has the good copy"
    );
    assert_eq!(
        mac.sync().await.refused.len(),
        1,
        "and it is reported every time"
    );
}

#[tokio::test]
async fn a_remote_object_that_does_not_decrypt_is_not_written_over_a_good_local_copy() {
    let world = World::new();
    let mut mac = world.device("Mac");
    mac.write("a.md", "good text");
    mac.sync().await;

    // ciphertext of another note put at this name: right header, but sealed under a different path
    let wrong = world
        .keys
        .seal(&p("other.md"), b"not for this path")
        .unwrap();
    world
        .remote()
        .put(
            &p("a.md"),
            Bytes::from(wrong.clone()),
            &flint_core::ContentHash::of(&wrong),
            Precondition::Unconditional,
        )
        .await
        .unwrap();

    let report = mac.sync().await;
    assert_eq!(report.refused, [(p("a.md"), Refusal::RemoteNotDecryptable)]);
    assert!(report.downloaded.is_empty());
    assert_eq!(
        mac.read("a.md").as_deref(),
        Some("good text"),
        "the local copy is untouched"
    );
}

#[tokio::test]
async fn a_store_whose_key_file_vanished_from_the_remote_is_left_alone_not_emptied() {
    let world = World::new();
    let mut mac = world.device("Mac");
    let mut phone = world.device("Phone");
    // the store has a key file, as every real store does
    mac.syncer
        .fs()
        .write_atomic(&p(".store/key.json"), world.key_file.to_json().as_bytes())
        .unwrap();
    mac.write("a.md", "my notes");
    mac.sync().await;
    phone.sync().await;
    assert!(phone.syncer.fs().exists(&p(".store/key.json")).unwrap());

    // the bucket is emptied (in a console, say)
    let remote = world.remote();
    for entry in remote.list().await.unwrap() {
        remote.delete(&entry.path, None).await.unwrap();
    }

    for device in [&mut mac, &mut phone] {
        let result = device.syncer.sync().await;
        assert!(
            matches!(result, Err(crate::Error::RemoteStoreMissing)),
            "{result:?}"
        );
        assert_eq!(
            device.read("a.md").as_deref(),
            Some("my notes"),
            "the notes are still here"
        );
        assert!(
            device.syncer.fs().exists(&p(".store/key.json")).unwrap(),
            "and so is the key file"
        );
    }
}

#[tokio::test]
async fn a_store_that_never_had_a_key_file_synced_is_not_stopped_by_its_absence() {
    // the guard is about a store that was synced before; tests, and a brand-new store, have no baseline for it
    let world = World::new();
    let mut mac = world.device("Mac");
    mac.write("a.md", "x");
    assert_eq!(mac.sync().await.uploaded, [p("a.md")]);
    assert!(mac.syncer.sync().await.is_ok());
}

#[tokio::test]
async fn a_download_that_does_not_match_its_recorded_hash_is_refused() {
    let world = World::new();
    let mut mac = world.device("Mac");
    let mut phone = world.device("Phone");
    mac.write("a.md", "genuine");
    mac.sync().await;
    // corrupt the stored bytes but leave the recorded SHA-256 of the original in place
    let remote = world.remote();
    let genuine = remote.get(&p("a.md")).await.unwrap();
    let mut damaged = genuine.bytes.to_vec();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    use object_store::{
        Attribute, AttributeValue, Attributes, ObjectStore, PutOptions, PutPayload, path::Path,
    };
    let mut attributes = Attributes::new();
    attributes.insert(
        Attribute::Metadata("sha256".into()),
        AttributeValue::from(genuine.entry.sha256.unwrap().to_string()),
    );
    world
        .store
        .put_opts(
            &Path::from("store/a.md"),
            PutPayload::from(damaged),
            PutOptions {
                attributes,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let report = phone.sync().await;
    assert!(
        matches!(report.refused.as_slice(), [(path, Refusal::HashMismatch { .. })] if path.as_str() == "a.md"),
        "{report:?}"
    );
    assert_eq!(phone.read("a.md"), None);
}

#[tokio::test]
async fn ciphertext_swapped_between_names_by_the_server_is_refused_not_downloaded() {
    // The path bound into every file's encryption is what catches it. Sync used to download it and leave the
    // app to fail when it opened the file; now sync checks that what it is about to write opens as that name.
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    mac.write("a.md", "contents of a");
    mac.write("b.md", "contents of b");
    mac.sync().await;
    let remote = world.remote();
    let b = remote.get(&p("b.md")).await.unwrap();
    remote
        .put(
            &p("a.md"),
            b.bytes.clone(),
            &b.entry.sha256.unwrap(),
            Precondition::Unconditional,
        )
        .await
        .unwrap();

    let report = phone.sync().await;
    assert_eq!(report.downloaded, [p("b.md")]);
    assert_eq!(report.refused, [(p("a.md"), Refusal::RemoteNotDecryptable)]);
    assert!(
        !phone.syncer.fs().exists(&p("a.md")).unwrap(),
        "the swapped file is not written at all"
    );
    assert_eq!(phone.read("b.md").as_deref(), Some("contents of b"));
}

#[tokio::test]
async fn a_local_edit_made_while_sync_is_running_is_not_overwritten_or_trashed() {
    let world = World::new();
    let spy = Spy::new(Arc::new(MemFs::new()));
    let mut mac = world.device_on("Mac", spy.clone(), world.remote());
    let mut phone = world.device("Phone");
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;
    // the phone deletes it, so the Mac's plan is "move to trash" (the local file is unchanged)
    phone.delete("a.md");
    phone.sync().await;

    // ...but the user edits the file after the plan was made, just before it is acted on (the guard hash right before trashing it; an unchanged file is not hashed during planning)
    let enc = flint_store_fs::EncryptedFs::new(spy.inner.clone(), world.keys.clone());
    spy.before_hash_call(0, move || {
        enc.write(&p("a.md"), b"edited a moment ago").unwrap()
    });
    let report = mac.sync().await;

    assert_eq!(
        report.refused,
        [(p("a.md"), Refusal::LocalChangedDuringSync)]
    );
    assert!(report.trashed_local.is_empty());
    assert_eq!(
        mac.read("a.md").as_deref(),
        Some("edited a moment ago"),
        "the edit survives"
    );
    // next time it is correctly seen as edited-here-deleted-there
    let next = mac.sync().await;
    assert_eq!(next.conflicts.len(), 1);
}

#[tokio::test]
async fn a_local_edit_made_while_a_download_is_in_flight_is_not_overwritten() {
    let world = World::new();
    let spy = Spy::new(Arc::new(MemFs::new()));
    let mut mac = world.device_on("Mac", spy.clone(), world.remote());
    let mut phone = world.device("Phone");
    phone.write("a.md", "v1");
    phone.sync().await;
    mac.sync().await;
    phone.write("a.md", "phone edit");
    phone.sync().await;

    // the Mac plans "download the phone's edit" (its own copy is unchanged); then the user edits the
    // file before the download is written over it
    let enc = flint_store_fs::EncryptedFs::new(spy.inner.clone(), world.keys.clone());
    spy.before_hash_call(0, move || {
        enc.write(&p("a.md"), b"typed on the Mac a moment ago")
            .unwrap()
    });
    let report = mac.sync().await;

    assert_eq!(
        report.refused,
        [(p("a.md"), Refusal::LocalChangedDuringSync)]
    );
    assert!(report.downloaded.is_empty());
    assert_eq!(
        mac.read("a.md").as_deref(),
        Some("typed on the Mac a moment ago")
    );
    // and the next cycle sees both sides changed and keeps both
    assert_eq!(mac.sync().await.conflicts.len(), 1);
}

// ---- failures -----------------------------------------------------------------------------------

#[tokio::test]
async fn a_lost_upload_reply_is_recognised_next_time_by_the_recorded_hash_and_never_conflicts() {
    let world = World::new();
    let mut mac = world.faulty_device("Mac");
    mac.write("a.md", "sent once");
    mac.syncer
        .remote()
        .inject(Op::Put, Some("a.md"), Fault::LoseReply, 1);

    let first = mac.sync().await;
    assert_eq!(first.failed.len(), 1, "the caller saw an error: {first:?}");
    assert!(
        mac.syncer.state().baseline(&p("a.md")).unwrap().is_none(),
        "no baseline until success is known"
    );
    assert_eq!(
        world.remote_plain("a.md").await.as_deref(),
        Some("sent once"),
        "but the write did land"
    );

    let second = mac.sync().await;
    assert_eq!(paths(&second.agreed), ["a.md"], "{second:?}");
    assert!(second.conflicts.is_empty() && second.uploaded.is_empty());
    assert!(mac.syncer.state().baseline(&p("a.md")).unwrap().is_some());
    assert!(mac.sync().await.is_quiet());
}

#[tokio::test]
async fn a_retry_refused_by_its_own_earlier_write_is_recognised_at_once() {
    let world = World::new();
    let mut mac = world.faulty_device("Mac");
    mac.write("a.md", "v1");
    mac.sync().await;
    mac.write("a.md", "v2");
    mac.syncer
        .remote()
        .inject(Op::Put, Some("a.md"), Fault::AppliedThenRefused, 1);

    let report = mac.sync().await;
    assert_eq!(paths(&report.agreed), ["a.md"], "{report:?}");
    assert!(report.failed.is_empty() && report.conflicts.is_empty());
    assert_eq!(world.remote_plain("a.md").await.as_deref(), Some("v2"));
    assert!(mac.sync().await.is_quiet());
}

#[tokio::test]
async fn a_transient_failure_changes_nothing_and_the_next_cycle_succeeds() {
    let world = World::new();
    let mut mac = world.faulty_device("Mac");
    mac.write("a.md", "x");
    mac.syncer.remote().inject(Op::Put, None, Fault::Fail, 1);
    let first = mac.sync().await;
    assert_eq!(first.failed.len(), 1);
    assert_eq!(mac.read("a.md").as_deref(), Some("x"));
    assert_eq!(paths(&mac.sync().await.uploaded), ["a.md"]);
}

#[tokio::test]
async fn failing_to_read_the_remote_stops_the_cycle_before_anything_is_touched() {
    let world = World::new();
    let mut mac = world.faulty_device("Mac");
    mac.write("a.md", "x");
    mac.syncer.remote().inject(Op::List, None, Fault::Fail, 1);
    assert!(mac.syncer.sync().await.is_err());
    assert_eq!(world.remote_paths().await, Vec::<String>::new());
    assert_eq!(paths(&mac.sync().await.uploaded), ["a.md"]);
}

#[tokio::test]
async fn a_failed_download_leaves_the_local_side_alone_and_is_retried() {
    let world = World::new();
    let mut mac = world.device("Mac");
    let mut phone = world.faulty_device("Phone");
    mac.write("a.md", "from the Mac");
    mac.sync().await;
    phone
        .syncer
        .remote()
        .inject(Op::Get, Some("a.md"), Fault::Fail, 1);
    let first = phone.sync().await;
    assert_eq!(first.failed.len(), 1);
    assert_eq!(phone.read("a.md"), None);
    assert_eq!(paths(&phone.sync().await.downloaded), ["a.md"]);
}

#[tokio::test]
async fn a_rival_write_between_planning_and_uploading_is_refused_and_becomes_a_conflict() {
    let world = World::new();
    let mut mac = world.device("Mac");
    let mut phone = world.faulty_device("Phone");
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;
    phone.write("a.md", "phone edit");

    // The phone plans "upload, if the remote is still v1". Just before its PUT arrives, the Mac's edit lands.
    let mac_edit = Bytes::from(world.keys.seal(&p("a.md"), b"mac edit").unwrap());
    phone
        .syncer
        .remote()
        .inject(Op::Put, Some("a.md"), Fault::RivalWrites(mac_edit), 1);

    let report = phone.sync().await;
    assert!(
        report.uploaded.is_empty() && report.failed.is_empty(),
        "{report:?}"
    );
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
    assert_eq!(
        world.remote_plain("a.md").await.as_deref(),
        Some("mac edit"),
        "the rival's write stands: it was not overwritten"
    );
    assert_eq!(
        phone.read("a.md").as_deref(),
        Some("phone edit"),
        "and the phone's edit is intact"
    );
    assert_eq!(
        phone.read("a.conflict-Phone-20261004-1234.md").as_deref(),
        Some("mac edit")
    );
}

#[tokio::test]
async fn a_server_that_refuses_every_conditional_write_ends_in_contended_with_nothing_lost() {
    // A well-behaved remote cannot do this (once its ETag changes the planner stops choosing an upload),
    // so the retry bound is only a safety valve: it must end the loop without touching either side.
    let world = World::new();
    let mut mac = world.device("Mac");
    let mut phone = world.faulty_device("Phone");
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;
    phone.write("a.md", "phone edit");
    phone
        .syncer
        .remote()
        .inject(Op::Put, Some("a.md"), Fault::Refuse, 1000);

    let report = phone.sync().await;
    assert_eq!(report.refused, [(p("a.md"), Refusal::Contended)]);
    assert!(report.uploaded.is_empty() && report.conflicts.is_empty());
    assert_eq!(phone.read("a.md").as_deref(), Some("phone edit"));
    assert_eq!(world.remote_plain("a.md").await.as_deref(), Some("v1"));
}

#[tokio::test]
async fn deleting_a_file_that_was_edited_elsewhere_in_the_meantime_is_refused_and_the_edit_restored()
 {
    let world = World::new();
    let mut mac = world.device("Mac");
    let mut phone = world.faulty_device("Phone");
    mac.write("a.md", "v1");
    mac.sync().await;
    phone.sync().await;
    phone.delete("a.md");

    // The phone plans "delete it remotely, if it is still v1". Just before the DELETE arrives, the Mac's edit lands.
    let late_edit = Bytes::from(world.keys.seal(&p("a.md"), b"late edit").unwrap());
    phone
        .syncer
        .remote()
        .inject(Op::Delete, Some("a.md"), Fault::RivalWrites(late_edit), 1);

    let report = phone.sync().await;
    assert!(report.deleted_remote.is_empty(), "{report:?}");
    assert_eq!(
        world.remote_plain("a.md").await.as_deref(),
        Some("late edit"),
        "the edit was not deleted"
    );
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(
        report.conflicts[0].outcome,
        ConflictOutcome::RestoredEditedRemotely
    );
    assert_eq!(
        phone.read("a.md").as_deref(),
        Some("late edit"),
        "and the phone got it back"
    );
}

#[tokio::test]
async fn key_json_syncs_as_plaintext_and_a_divergence_is_refused_without_a_copy() {
    let world = World::new();
    let mut mac = world.device("Mac");
    let mut phone = world.device("Phone");
    mac.syncer
        .fs()
        .write_atomic(&p(KEY_FILE), br#"{"version":1,"key_id":1,"wrapped":"A"}"#)
        .unwrap();
    assert_eq!(paths(&mac.sync().await.uploaded), [KEY_FILE]);
    assert_eq!(paths(&phone.sync().await.downloaded), [KEY_FILE]);
    assert_eq!(
        phone.syncer.fs().read(&p(KEY_FILE)).unwrap(),
        br#"{"version":1,"key_id":1,"wrapped":"A"}"#
    );

    // both devices change the password at once
    mac.syncer
        .fs()
        .write_atomic(
            &p(KEY_FILE),
            br#"{"version":1,"key_id":1,"wrapped":"B-mac"}"#,
        )
        .unwrap();
    phone
        .syncer
        .fs()
        .write_atomic(
            &p(KEY_FILE),
            br#"{"version":1,"key_id":1,"wrapped":"B-phone"}"#,
        )
        .unwrap();
    mac.sync().await;
    let report = phone.sync().await;
    assert_eq!(report.refused, [(p(KEY_FILE), Refusal::KeyFileDiverged)]);
    assert!(phone.copies().is_empty(), "key.json is never copied around");
    assert_eq!(
        phone.syncer.fs().read(&p(KEY_FILE)).unwrap(),
        br#"{"version":1,"key_id":1,"wrapped":"B-phone"}"#,
        "neither side overwritten"
    );
}

// ---- scenarios from the plan -------------------------------------------------------------------

#[tokio::test]
async fn a_device_that_was_offline_for_a_long_time_merges_cleanly_with_changes_to_other_files() {
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));
    for name in ["one.md", "two.md", "three.md", "four.md"] {
        mac.write(name, &format!("{name} v1"));
    }
    mac.sync().await;
    phone.sync().await;

    // the phone goes offline and edits two files and adds one; meanwhile the Mac edits the other two,
    // deletes one of them, and adds one
    phone.write("one.md", "one edited on the phone");
    phone.write("two.md", "two edited on the phone");
    phone.write("phone-new.md", "new on the phone");
    mac.write("three.md", "three edited on the Mac");
    mac.delete("four.md");
    mac.write("mac-new.md", "new on the Mac");
    mac.sync().await;

    // reconnecting: no conflicts, because nothing was touched on both sides
    let report = phone.sync().await;
    assert!(
        report.conflicts.is_empty() && report.failed.is_empty() && report.refused.is_empty(),
        "{report:?}"
    );
    mac.sync().await;
    for device in [&phone, &mac] {
        assert_eq!(
            device.read("one.md").as_deref(),
            Some("one edited on the phone")
        );
        assert_eq!(
            device.read("two.md").as_deref(),
            Some("two edited on the phone")
        );
        assert_eq!(
            device.read("three.md").as_deref(),
            Some("three edited on the Mac")
        );
        assert_eq!(device.read("four.md"), None);
        assert_eq!(
            device.read("phone-new.md").as_deref(),
            Some("new on the phone")
        );
        assert_eq!(device.read("mac-new.md").as_deref(), Some("new on the Mac"));
    }
}

#[tokio::test]
async fn a_password_change_on_one_device_reaches_the_other_without_re_encrypting_any_file() {
    use flint_crypto::{KdfParams, KeyFile};
    let fast = KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    };
    let world = World::new();
    let (mut mac, mut phone) = (world.device("Mac"), world.device("Phone"));

    // the store's key file, as create-store would have written it, and some notes
    let key_file = world.key_file.clone();
    mac.syncer
        .fs()
        .write_atomic(&p(KEY_FILE), key_file.to_json().as_bytes())
        .unwrap();
    mac.write("a.md", "a note");
    mac.sync().await;
    phone.sync().await;
    let sealed_before = phone.syncer.fs().read(&p("a.md")).unwrap();

    // the Mac changes the password: only key.json changes (the data key is re-wrapped, nothing re-encrypted)
    let changed = key_file
        .change_password("pw", "new password", fast)
        .unwrap();
    mac.syncer
        .fs()
        .write_atomic(&p(KEY_FILE), changed.to_json().as_bytes())
        .unwrap();
    let report = mac.sync().await;
    assert_eq!(paths(&report.uploaded), [KEY_FILE], "{report:?}");

    let report = phone.sync().await;
    assert_eq!(
        paths(&report.downloaded),
        [KEY_FILE],
        "the new key file arrives; no note moves: {report:?}"
    );
    assert_eq!(
        phone.syncer.fs().read(&p("a.md")).unwrap(),
        sealed_before,
        "the note on the phone was not touched"
    );
    // the phone can now unlock with the new password, and gets the very same data key
    let arrived = KeyFile::from_json(
        &String::from_utf8(phone.syncer.fs().read(&p(KEY_FILE)).unwrap()).unwrap(),
    )
    .unwrap();
    assert!(arrived.unlock("pw").is_err());
    assert_eq!(
        arrived.unlock("new password").unwrap().dek().as_bytes(),
        world.keys.dek().as_bytes()
    );
    assert_eq!(phone.read("a.md").as_deref(), Some("a note"));
}

// ---- cost ---------------------------------------------------------------------------------------

#[tokio::test]
async fn files_that_visibly_have_not_changed_are_not_rehashed() {
    let world = World::new();
    let spy = Spy::new(Arc::new(MemFs::new()));
    let mut mac = world.device_on("Mac", spy.clone(), world.remote());
    for i in 0..20 {
        mac.write(&format!("n{i}.md"), &format!("note {i}"));
    }
    mac.sync().await;

    let before = spy.hashed();
    assert!(mac.sync().await.is_quiet());
    assert_eq!(
        spy.hashed(),
        before,
        "a quiet cycle reads no file contents to hash them"
    );

    mac.write("n3.md", "edited");
    let before = spy.hashed();
    assert_eq!(paths(&mac.sync().await.uploaded), ["n3.md"]);
    assert!(
        spy.hashed() - before <= 2,
        "only the edited file is hashed: {}",
        spy.hashed() - before
    );
}
