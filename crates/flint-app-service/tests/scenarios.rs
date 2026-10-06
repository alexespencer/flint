//! Two devices ("Mac" and "Phone"), each with its own config folder, secrets and store folder, sharing
//! one in-memory bucket. Everything goes through `AppService`, the way a shell would.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use flint_app_service::{AppService, Connect, Error, InMemoryConnect, NewStore};
use flint_core::StorePath;
use flint_crypto::KdfParams;
use flint_registry::{MemorySecrets, Registry, RemoteSettings, StoreId};
use flint_remote::RemoteStore;
use flint_sync::DeviceName;
use tempfile::TempDir;

const FAST: KdfParams = KdfParams {
    m_kib: 8,
    t: 1,
    p: 1,
};

fn p(s: &str) -> StorePath {
    StorePath::new(s).unwrap()
}

fn settings() -> RemoteSettings {
    RemoteSettings {
        bucket: "bucket".into(),
        prefix: "vault".into(),
        region: "auto".into(),
        endpoint: None,
        access_key_id: "key".into(),
    }
}

struct Device {
    service: AppService<MemorySecrets, InMemoryConnect>,
    folder: TempDir,
    _config: TempDir,
}

impl Device {
    fn new(name: &str, bucket: &InMemoryConnect) -> Self {
        let config = TempDir::new().unwrap();
        let registry = Registry::open(config.path(), MemorySecrets::new()).unwrap();
        let service = AppService::new(registry, bucket.clone(), DeviceName::new(name))
            .with_kdf(FAST)
            .with_quiet_period(Duration::ZERO);
        Self {
            service,
            folder: TempDir::new().unwrap(),
            _config: config,
        }
    }

    fn new_store(&self, password: &str) -> NewStore {
        NewStore {
            name: "Notes".into(),
            folder: self.folder.path().join("Notes"),
            remote: settings(),
            s3_secret: "secret".into(),
            password: password.into(),
        }
    }

    fn id(&self) -> StoreId {
        self.service.registry().list()[0].id.clone()
    }

    fn dir(&self) -> std::path::PathBuf {
        self.folder.path().join("Notes")
    }

    fn drop_in(&self, path: &str, text: &str) {
        let file = self.dir().join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }

    async fn sync(&self) -> flint_app_service::SyncOutcome {
        self.service.sync(&self.id()).await.unwrap()
    }

    fn read(&self, path: &str) -> String {
        self.service.read_note(&self.id(), &p(path)).unwrap()
    }
}

/// Creates the store on a new "Mac" and joins it from a new "Phone".
async fn two_devices() -> (Device, Device, InMemoryConnect) {
    let bucket = InMemoryConnect::new();
    let mut mac = Device::new("Mac", &bucket);
    mac.service.create_store(mac.new_store("pw")).await.unwrap();
    let mut phone = Device::new("Phone", &bucket);
    phone
        .service
        .join_store(phone.new_store("pw"))
        .await
        .unwrap();
    (mac, phone, bucket)
}

#[tokio::test]
async fn files_dropped_into_a_new_store_are_encrypted_and_reach_a_second_device() {
    let bucket = InMemoryConnect::new();
    let mut mac = Device::new("Mac", &bucket);
    mac.drop_in("Ideas.md", "# Ideas\nsee [[Plan]] #todo");
    mac.drop_in("Plan.md", "the plan");
    mac.drop_in("img/logo.png", "not really a png");
    mac.service.create_store(mac.new_store("pw")).await.unwrap();

    let outcome = mac.sync().await;
    assert_eq!(outcome.adopted.adopted.len(), 3);
    assert_eq!(outcome.report.uploaded.len(), 3);
    assert!(outcome.report.failed.is_empty() && outcome.report.refused.is_empty());

    // nothing readable on disk or in the bucket
    for file in ["Ideas.md", "Plan.md", "img/logo.png"] {
        assert!(
            !fs::read(mac.dir().join(file))
                .unwrap()
                .windows(4)
                .any(|w| w == b"Plan" || w == b"Idea")
        );
    }
    let remote = bucket.connect(&settings(), "").unwrap();
    for entry in remote.list().await.unwrap() {
        let bytes = remote.get(&entry.path).await.unwrap().bytes;
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("the plan") && !text.contains("Ideas"),
            "{}",
            entry.path
        );
    }

    let mut phone = Device::new("Phone", &bucket);
    phone
        .service
        .join_store(phone.new_store("pw"))
        .await
        .unwrap();
    let outcome = phone.sync().await;
    assert_eq!(outcome.report.downloaded.len(), 3);
    assert_eq!(phone.read("Ideas.md"), "# Ideas\nsee [[Plan]] #todo");

    // the phone's index knows the links the mac's notes make
    let backlinks = phone.service.backlinks(&phone.id(), &p("Plan.md")).unwrap();
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].from, p("Ideas.md"));
    assert_eq!(
        phone.service.search(&phone.id(), "plan", 10).unwrap().len(),
        2
    );
}

#[tokio::test]
async fn edits_flow_both_ways() {
    let (mac, phone, _) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "from the mac")
        .unwrap();
    mac.sync().await;
    phone.sync().await;
    assert_eq!(phone.read("a.md"), "from the mac");

    phone
        .service
        .save_note(&phone.id(), &p("a.md"), "edited on the phone")
        .unwrap();
    phone.sync().await;
    mac.sync().await;
    assert_eq!(mac.read("a.md"), "edited on the phone");
}

#[tokio::test]
async fn a_conflict_is_surfaced_kept_and_resolved() {
    let (mac, phone, _) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "start")
        .unwrap();
    mac.sync().await;
    phone.sync().await;

    mac.service
        .save_note(&mac.id(), &p("a.md"), "mac edit")
        .unwrap();
    phone
        .service
        .save_note(&phone.id(), &p("a.md"), "phone edit")
        .unwrap();
    mac.sync().await; // the mac gets there first
    let outcome = phone.sync().await;
    assert_eq!(outcome.report.conflicts.len(), 1);

    // the phone's own edit is untouched, the mac's version sits beside it, and the index sees both
    assert_eq!(phone.read("a.md"), "phone edit");
    let status = phone.service.status(&phone.id()).unwrap();
    assert_eq!(status.conflicts.len(), 1);
    assert_eq!(status.conflicts[0].path, p("a.md"));
    let copy = status.conflicts[0].copy.clone().unwrap();
    assert_eq!(phone.read(copy.as_str()), "mac edit");
    assert_eq!(
        phone.service.search(&phone.id(), "mac", 10).unwrap()[0].path,
        copy
    );

    // while held, sync leaves the path alone
    let held = phone.sync().await.report;
    assert!(!held.uploaded.contains(&p("a.md")) && held.downloaded.is_empty());
    assert!(
        held.conflicts.is_empty(),
        "no second conflict is raised for a held path"
    );
    assert_eq!(phone.read("a.md"), "phone edit");

    // the user merges, deletes the copy, and resolves
    phone
        .service
        .save_note(&phone.id(), &p("a.md"), "merged")
        .unwrap();
    fs::remove_file(phone.dir().join(copy.as_str())).unwrap();
    phone
        .service
        .resolve(&phone.id(), &p("a.md"))
        .await
        .unwrap();
    assert!(
        phone
            .service
            .status(&phone.id())
            .unwrap()
            .conflicts
            .is_empty()
    );
    phone.sync().await;
    mac.sync().await;
    assert_eq!(mac.read("a.md"), "merged");
}

#[tokio::test]
async fn resolving_moves_the_conflict_copy_to_the_trash_on_every_device() {
    let (mac, phone, _) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "start")
        .unwrap();
    mac.sync().await;
    phone.sync().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "mac edit")
        .unwrap();
    phone
        .service
        .save_note(&phone.id(), &p("a.md"), "phone edit")
        .unwrap();
    mac.sync().await;
    phone.sync().await; // the phone loses and holds the mac's version as a copy
    let copy = phone.service.status(&phone.id()).unwrap().conflicts[0]
        .copy
        .clone()
        .unwrap();

    // the copy is an ordinary file, so the mac (which won) gets it too
    phone.sync().await;
    mac.sync().await;
    assert!(mac.dir().join(copy.as_str()).exists());

    phone
        .service
        .save_note(&phone.id(), &p("a.md"), "merged")
        .unwrap();
    phone
        .service
        .resolve(&phone.id(), &p("a.md"))
        .await
        .unwrap();
    assert!(
        !phone.dir().join(copy.as_str()).exists(),
        "moved out of the folder"
    );
    assert!(
        phone.dir().join(".trash").join(copy.as_str()).exists(),
        "into the trash, not deleted"
    );
    assert!(!phone.service.files(&phone.id()).unwrap().contains(&copy));
    assert!(
        phone
            .service
            .search(&phone.id(), "mac edit", 5)
            .unwrap()
            .is_empty(),
        "and out of the index"
    );

    phone.sync().await;
    mac.sync().await;
    assert!(
        !mac.dir().join(copy.as_str()).exists(),
        "the other device's copy goes too"
    );
    assert!(mac.dir().join(".trash").join(copy.as_str()).exists());
    assert_eq!(mac.read("a.md"), "merged");
}

#[tokio::test]
async fn deleting_a_note_trashes_it_and_removes_it_from_the_other_device() {
    let (mac, phone, _) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("keep.md"), "keep")
        .unwrap();
    mac.service
        .save_note(&mac.id(), &p("old.md"), "needle")
        .unwrap();
    mac.sync().await;
    phone.sync().await;

    assert_eq!(mac.service.delete_path(&mac.id(), &p("old.md")).unwrap(), 1);
    assert!(!mac.dir().join("old.md").exists());
    assert!(
        mac.dir().join(".trash/old.md").exists(),
        "recoverable, not erased"
    );
    assert!(!mac.service.files(&mac.id()).unwrap().contains(&p("old.md")));
    assert!(
        mac.service
            .search(&mac.id(), "needle", 5)
            .unwrap()
            .is_empty()
    );

    mac.sync().await;
    phone.sync().await;
    assert!(!phone.dir().join("old.md").exists());
    assert_eq!(phone.read("keep.md"), "keep");
}

#[tokio::test]
async fn moving_a_note_keeps_its_text_and_arrives_at_the_new_path_on_the_other_device() {
    let (mac, phone, _) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("idea.md"), "# Idea\nbody")
        .unwrap();
    mac.sync().await;
    phone.sync().await;

    assert_eq!(
        mac.service
            .move_path(&mac.id(), &p("idea.md"), &p("Projects/idea.md"))
            .unwrap(),
        1
    );
    assert_eq!(
        mac.read("Projects/idea.md"),
        "# Idea\nbody",
        "re-encrypted under the new path, so it still opens"
    );
    assert!(!mac.dir().join("idea.md").exists());
    assert_eq!(
        mac.service.search(&mac.id(), "body", 5).unwrap()[0].path,
        p("Projects/idea.md")
    );

    mac.sync().await;
    phone.sync().await;
    assert_eq!(phone.read("Projects/idea.md"), "# Idea\nbody");
    assert!(!phone.dir().join("idea.md").exists());

    // a plain rename too
    mac.service
        .move_path(&mac.id(), &p("Projects/idea.md"), &p("Projects/plan.md"))
        .unwrap();
    mac.sync().await;
    phone.sync().await;
    assert_eq!(phone.read("Projects/plan.md"), "# Idea\nbody");
}

#[tokio::test]
async fn moving_and_deleting_a_folder_takes_everything_in_it() {
    let (mac, phone, _) = two_devices().await;
    for (path, text) in [
        ("a/one.md", "1"),
        ("a/deep/two.md", "2"),
        ("a/deep/three.md", "3"),
        ("b.md", "b"),
    ] {
        mac.service.save_note(&mac.id(), &p(path), text).unwrap();
    }
    mac.sync().await;
    phone.sync().await;

    assert_eq!(
        mac.service
            .move_path(&mac.id(), &p("a"), &p("Archive/a"))
            .unwrap(),
        3
    );
    assert_eq!(mac.read("Archive/a/deep/two.md"), "2");
    assert!(
        !mac.dir().join("a").exists(),
        "the emptied folders are gone"
    );
    mac.sync().await;
    phone.sync().await;
    assert_eq!(phone.read("Archive/a/one.md"), "1");
    assert_eq!(phone.read("Archive/a/deep/three.md"), "3");
    assert!(!phone.dir().join("a").exists());

    assert_eq!(
        phone
            .service
            .delete_path(&phone.id(), &p("Archive"))
            .unwrap(),
        3
    );
    assert!(!phone.dir().join("Archive").exists());
    assert!(phone.dir().join(".trash/Archive/a/one.md").exists());
    phone.sync().await;
    mac.sync().await;
    assert!(!mac.dir().join("Archive").exists());
    assert_eq!(mac.read("b.md"), "b", "the rest is untouched");
}

#[tokio::test]
async fn an_empty_folder_can_be_made_listed_moved_and_stays_on_this_device() {
    let (mac, phone, _) = two_devices().await;
    mac.service
        .create_folder(&mac.id(), &p("Ideas/Later"))
        .unwrap();
    assert_eq!(
        mac.service.folders(&mac.id()).unwrap(),
        [p("Ideas"), p("Ideas/Later")]
    );
    mac.sync().await;
    phone.sync().await;
    assert!(
        phone.service.folders(&phone.id()).unwrap().is_empty(),
        "a folder with no file is not synced"
    );

    mac.service
        .move_path(&mac.id(), &p("Ideas"), &p("Done"))
        .unwrap();
    assert_eq!(
        mac.service.folders(&mac.id()).unwrap(),
        [p("Done"), p("Done/Later")]
    );
    assert_eq!(mac.service.delete_path(&mac.id(), &p("Done")).unwrap(), 0);
    assert!(mac.service.folders(&mac.id()).unwrap().is_empty());
}

#[tokio::test]
async fn risky_moves_and_deletes_are_refused_and_change_nothing() {
    let (mac, phone, _) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "start")
        .unwrap();
    mac.service
        .save_note(&mac.id(), &p("b.md"), "other")
        .unwrap();
    mac.service
        .save_note(&mac.id(), &p("dir/x.md"), "x")
        .unwrap();
    mac.sync().await;
    phone.sync().await;
    let id = mac.id();

    let taken = mac.service.move_path(&id, &p("a.md"), &p("b.md"));
    assert!(
        matches!(
            taken,
            Err(Error::Local(flint_store_fs::Error::AlreadyExists(_)))
        ),
        "{taken:?}"
    );
    assert_eq!(mac.read("b.md"), "other", "the target is not overwritten");
    assert!(matches!(
        mac.service.move_path(&id, &p("dir"), &p("dir/inner")),
        Err(Error::InsideItself)
    ));

    // a folder is not merged into one that already exists
    mac.service.save_note(&id, &p("other/y.md"), "y").unwrap();
    let merge = mac.service.move_path(&id, &p("dir"), &p("other"));
    assert!(
        matches!(
            merge,
            Err(Error::Local(flint_store_fs::Error::AlreadyExists(_)))
        ),
        "{merge:?}"
    );
    assert_eq!(mac.read("dir/x.md"), "x", "nothing moved");
    assert_eq!(
        mac.service
            .files(&id)
            .unwrap()
            .iter()
            .filter(|f| f.as_str().starts_with("other/"))
            .count(),
        1
    );
    for hidden in [".trash/x", ".git/config", ".store/key.json"] {
        assert!(
            matches!(
                mac.service.delete_path(&id, &p(hidden)),
                Err(Error::NotANote(_))
            ),
            "{hidden}"
        );
        assert!(
            matches!(
                mac.service.move_path(&id, &p("a.md"), &p(hidden)),
                Err(Error::NotANote(_))
            ),
            "{hidden}"
        );
        assert!(
            matches!(
                mac.service.create_folder(&id, &p(hidden)),
                Err(Error::NotANote(_))
            ),
            "{hidden}"
        );
    }
    assert!(matches!(
        mac.service.delete_path(&id, &p("missing.md")),
        Err(Error::NothingAt(_))
    ));

    // a note in an unresolved conflict cannot be moved or deleted, nor can the folder holding it
    mac.service
        .save_note(&id, &p("dir/x.md"), "mac edit")
        .unwrap();
    phone
        .service
        .save_note(&phone.id(), &p("dir/x.md"), "phone edit")
        .unwrap();
    mac.sync().await;
    phone.sync().await;
    assert_eq!(
        phone.service.status(&phone.id()).unwrap().conflicts.len(),
        1
    );
    let pid = phone.id();
    assert!(matches!(
        phone.service.delete_path(&pid, &p("dir/x.md")),
        Err(Error::InConflict(_))
    ));
    assert!(matches!(
        phone.service.move_path(&pid, &p("dir"), &p("moved")),
        Err(Error::InConflict(_))
    ));
    assert_eq!(phone.read("dir/x.md"), "phone edit");
}

#[tokio::test]
async fn a_store_can_be_made_and_joined_without_a_password() {
    let bucket = InMemoryConnect::new();
    let mut mac = Device::new("Mac", &bucket);
    mac.service.create_store(mac.new_store("")).await.unwrap();
    assert!(mac.service.registry().list()[0].passwordless);
    mac.service
        .save_note(&mac.id(), &p("a.md"), "no secret here")
        .unwrap();
    mac.sync().await;

    // still encrypted on disk and in the bucket: only the key is easy to derive
    assert!(
        !fs::read(mac.dir().join("a.md"))
            .unwrap()
            .windows(6)
            .any(|w| w == b"secret")
    );
    let remote = bucket.connect(&settings(), "").unwrap();
    for entry in remote
        .list()
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.path.as_str() != ".store/key.json")
    {
        let bytes = remote.get(&entry.path).await.unwrap().bytes;
        assert!(bytes.starts_with(b"FLNT"), "{}", entry.path);
    }

    // another device joins with only the bucket details: no password to give
    let mut phone = Device::new("Phone", &bucket);
    phone.service.join_store(phone.new_store("")).await.unwrap();
    assert!(phone.service.registry().list()[0].passwordless);
    phone.sync().await;
    assert_eq!(phone.read("a.md"), "no secret here");

    // locking and unlocking need nothing; any other password does not work
    mac.service.lock_all().unwrap();
    assert!(matches!(
        mac.service.unlock(&mac.id(), "guess").await,
        Err(Error::WrongPassword)
    ));
    let id = mac.id();
    mac.service.unlock_noting(&id, "").await.unwrap();
    assert!(mac.service.status(&id).unwrap().unlocked);
}

#[tokio::test]
async fn the_empty_password_does_not_open_a_store_that_has_one_and_is_never_assumed() {
    let (mac, _, bucket) = two_devices().await; // password "pw"
    drop(mac);
    let mut tablet = Device::new("Tablet", &bucket);
    let joined = tablet.service.join_store(tablet.new_store("")).await;
    assert!(matches!(joined, Err(Error::WrongPassword)), "{joined:?}");
    assert!(tablet.service.registry().list().is_empty());
}

#[tokio::test]
async fn a_store_without_a_password_can_be_given_one_and_other_devices_notice() {
    let bucket = InMemoryConnect::new();
    let mut mac = Device::new("Mac", &bucket);
    mac.service.create_store(mac.new_store("")).await.unwrap();
    let mut phone = Device::new("Phone", &bucket);
    phone.service.join_store(phone.new_store("")).await.unwrap();
    let (mid, pid) = (mac.id(), phone.id());

    // the new password cannot be empty; the old one is the empty one
    assert!(matches!(
        mac.service.change_password_noting(&mid, "", "").await,
        Err(Error::EmptyPassword)
    ));
    mac.service
        .change_password_noting(&mid, "", "now protected")
        .await
        .unwrap();
    assert!(!mac.service.registry().list()[0].passwordless);

    // the phone learns it on its next sync, and then on the first unlock that fails with the empty password
    phone.sync().await;
    phone.service.lock_all().unwrap();
    assert!(
        phone.service.registry().list()[0].passwordless,
        "stale until it tries"
    );
    assert!(matches!(
        phone.service.unlock_noting(&pid, "").await,
        Err(Error::WrongPassword)
    ));
    assert!(
        !phone.service.registry().list()[0].passwordless,
        "now it asks for a password"
    );
    phone
        .service
        .unlock_noting(&pid, "now protected")
        .await
        .unwrap();
    assert!(!phone.service.registry().list()[0].passwordless);
}

async fn wipe_the_bucket(bucket: &InMemoryConnect) {
    let remote = bucket.connect(&settings(), "").unwrap();
    for entry in remote.list().await.unwrap() {
        remote.delete(&entry.path, None).await.unwrap();
    }
}

#[tokio::test]
async fn removing_a_store_keeps_its_folder_aside_and_joining_again_brings_everything_back() {
    let (mut mac, phone, bucket) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "kept on s3")
        .unwrap();
    mac.service
        .save_note(&mac.id(), &p("deep/b.md"), "also kept")
        .unwrap();
    let id = mac.id();

    let removed = mac.service.remove_store(&id, false).await.unwrap(); // syncs first, so nothing is left behind
    let aside = removed.folder_kept_as.unwrap();
    assert!(
        aside
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Notes.removed-"),
        "{aside:?}"
    );
    assert!(
        aside.join("a.md").is_file(),
        "the folder is kept, not deleted"
    );
    assert!(!mac.dir().exists());
    assert!(mac.service.registry().list().is_empty());

    // the notes were all on S3: the other device has them, and so does a fresh join
    phone.sync().await;
    assert_eq!(phone.read("a.md"), "kept on s3");
    mac.service.join_store(mac.new_store("pw")).await.unwrap();
    mac.sync().await;
    assert_eq!(mac.read("a.md"), "kept on s3");
    assert_eq!(mac.read("deep/b.md"), "also kept");
    drop(bucket);
}

#[tokio::test]
async fn removal_is_refused_while_something_is_not_on_s3_unless_forced() {
    let (mut mac, phone, _) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "start")
        .unwrap();
    mac.sync().await;
    phone.sync().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "mac edit")
        .unwrap();
    phone
        .service
        .save_note(&phone.id(), &p("a.md"), "phone edit")
        .unwrap();
    phone.sync().await;
    mac.sync().await; // the mac loses: a conflict that exists only here
    let id = mac.id();
    assert_eq!(mac.service.status(&id).unwrap().conflicts.len(), 1);

    let refused = mac.service.remove_store(&id, false).await;
    assert!(
        matches!(&refused, Err(Error::NotFullySynced(why)) if why.contains("conflict")),
        "{refused:?}"
    );
    assert_eq!(mac.service.registry().list().len(), 1, "still registered");
    assert!(mac.dir().join("a.md").exists(), "and nothing moved");

    // a locked store cannot be synced, so it needs force too; the folder is kept either way
    mac.service.lock_all().unwrap();
    assert!(matches!(
        mac.service.remove_store(&id, false).await,
        Err(Error::Locked(_))
    ));
    let removed = mac.service.remove_store(&id, true).await.unwrap();
    assert!(removed.folder_kept_as.unwrap().join("a.md").is_file());
    assert!(mac.service.registry().list().is_empty());
}

#[tokio::test]
async fn a_wiped_bucket_stops_sync_everywhere_and_one_device_can_put_the_store_back() {
    let (mac, phone, bucket) = two_devices().await;
    mac.service
        .save_note(&mac.id(), &p("a.md"), "precious")
        .unwrap();
    mac.sync().await;
    phone.sync().await;

    wipe_the_bucket(&bucket).await;

    // nothing is emptied or trashed on either device
    for device in [&mac, &phone] {
        let result = device.service.sync(&device.id()).await;
        assert!(
            matches!(result, Err(Error::StoreMissingOnRemote)),
            "{result:?}"
        );
        assert_eq!(device.read("a.md"), "precious");
        assert!(device.dir().join(".store/key.json").is_file());
    }
    // ...and they can still lock and unlock
    phone.service.lock_all().unwrap();
    phone.service.unlock(&phone.id(), "pw").await.unwrap();

    // the mac puts its copy back; the phone sees the store again and carries on
    let outcome = mac.service.reupload_store(&mac.id()).await.unwrap();
    assert!(
        outcome
            .report
            .uploaded
            .iter()
            .any(|path| path.as_str() == "a.md"),
        "{:?}",
        outcome.report.uploaded
    );
    phone.sync().await;
    assert_eq!(phone.read("a.md"), "precious");
    mac.service
        .save_note(&mac.id(), &p("b.md"), "after the restore")
        .unwrap();
    mac.sync().await;
    phone.sync().await;
    assert_eq!(phone.read("b.md"), "after the restore");
}

#[tokio::test]
async fn deleting_a_store_needs_its_name_and_empties_s3_for_every_device() {
    let (mut mac, mut phone, bucket) = two_devices().await;
    mac.service.save_note(&mac.id(), &p("a.md"), "x").unwrap();
    mac.sync().await;
    phone.sync().await;
    let id = mac.id();

    let wrong = mac.service.delete_store(&id, "notes").await; // the name is case-sensitive
    assert!(matches!(wrong, Err(Error::NameMismatch)), "{wrong:?}");
    let remote = bucket.connect(&settings(), "").unwrap();
    assert!(
        !remote.list().await.unwrap().is_empty(),
        "nothing was deleted"
    );
    assert_eq!(mac.service.registry().list().len(), 1);

    let deleted = mac.service.delete_store(&id, "Notes").await.unwrap();
    assert!(deleted >= 2, "the note and the key file: {deleted}");
    assert!(remote.list().await.unwrap().is_empty());
    assert!(mac.service.registry().list().is_empty());

    // the other device stops at once, keeps its notes, and can remove the store (the folder is kept)
    let pid = phone.id();
    assert!(matches!(
        phone.service.sync(&pid).await,
        Err(Error::StoreMissingOnRemote)
    ));
    assert_eq!(phone.read("a.md"), "x");
    let removed = phone.service.remove_store(&pid, true).await.unwrap();
    assert!(removed.folder_kept_as.unwrap().join("a.md").is_file());

    // a new store can now use the same bucket and folder name
    let fresh = mac.service.create_store(mac.new_store("another")).await;
    assert!(fresh.is_ok(), "{fresh:?}");
    let _ = &mut phone;
}

#[tokio::test]
async fn joining_needs_the_right_password_and_an_existing_store() {
    let bucket = InMemoryConnect::new();

    let mut early = Device::new("Phone", &bucket);
    let nothing = early.service.join_store(early.new_store("pw")).await;
    assert!(matches!(nothing, Err(Error::NoSuchStore)));
    assert!(early.service.registry().list().is_empty());

    let mut mac = Device::new("Mac", &bucket);
    mac.service.create_store(mac.new_store("pw")).await.unwrap();

    let mut phone = Device::new("Phone", &bucket);
    let wrong = phone
        .service
        .join_store(phone.new_store("not the password"))
        .await;
    assert!(matches!(wrong, Err(Error::WrongPassword)));
    assert!(
        phone.service.registry().list().is_empty(),
        "a failed join leaves nothing behind"
    );
    assert!(!phone.dir().join(".store/key.json").exists());

    let mut other = Device::new("Tablet", &bucket);
    let again = other.service.create_store(other.new_store("another")).await;
    assert!(
        matches!(again, Err(Error::StoreExists)),
        "an existing store is never overwritten"
    );
    assert!(other.service.registry().list().is_empty());
}

#[tokio::test]
async fn a_locked_store_does_nothing_until_the_password_is_given() {
    let (mac, _, _) = two_devices().await;
    mac.service.lock(&mac.id()).unwrap();
    assert!(matches!(
        mac.service.sync(&mac.id()).await,
        Err(Error::Locked(_))
    ));
    assert!(matches!(
        mac.service.read_note(&mac.id(), &p("a.md")),
        Err(Error::Locked(_))
    ));
    assert!(!mac.service.status(&mac.id()).unwrap().unlocked);

    assert!(matches!(
        mac.service.unlock(&mac.id(), "wrong").await,
        Err(Error::WrongPassword)
    ));
    assert!(!mac.service.status(&mac.id()).unwrap().unlocked);
    mac.service.unlock(&mac.id(), "pw").await.unwrap();
    mac.sync().await;
}

#[tokio::test]
async fn lock_all_locks_every_store_on_the_device() {
    let (mut mac, _, _) = two_devices().await;
    let mut second = mac.new_store("other");
    second.name = "Second".into();
    second.folder = mac.folder.path().join("Second");
    second.remote.prefix = "second".into();
    let second_id = mac.service.create_store(second).await.unwrap();
    let first_id = mac.id();
    assert!(mac.service.status(&first_id).unwrap().unlocked);
    assert!(mac.service.status(&second_id).unwrap().unlocked);

    mac.service.lock_all().unwrap();

    assert!(!mac.service.status(&first_id).unwrap().unlocked);
    assert!(!mac.service.status(&second_id).unwrap().unlocked);
    mac.service.unlock(&second_id, "other").await.unwrap();
    assert!(
        !mac.service.status(&first_id).unwrap().unlocked,
        "unlocking one leaves the other locked"
    );
}

#[tokio::test]
async fn a_password_change_on_one_device_keeps_the_other_working() {
    let (mac, phone, _) = two_devices().await;
    mac.service.save_note(&mac.id(), &p("a.md"), "one").unwrap();
    mac.sync().await;
    phone.sync().await;

    mac.service
        .change_password(&mac.id(), "pw", "new password")
        .await
        .unwrap();

    // the phone has not heard yet: it still syncs, with the key it has
    phone
        .service
        .save_note(&phone.id(), &p("b.md"), "from the phone")
        .unwrap();
    let outcome = phone.sync().await;
    assert!(
        outcome.report.failed.is_empty(),
        "{:?}",
        outcome.report.failed
    );
    assert!(outcome.report.refused.is_empty());
    mac.sync().await;
    assert_eq!(
        mac.read("b.md"),
        "from the phone",
        "no file was re-encrypted"
    );

    // after that sync the phone's key file is the new one: only the new password unlocks it
    phone.service.lock(&phone.id()).unwrap();
    assert!(matches!(
        phone.service.unlock(&phone.id(), "pw").await,
        Err(Error::WrongPassword)
    ));
    phone
        .service
        .unlock(&phone.id(), "new password")
        .await
        .unwrap();
    assert_eq!(phone.read("a.md"), "one");
}

#[tokio::test]
async fn the_new_password_works_on_a_device_that_has_not_synced_since() {
    let (mac, phone, _) = two_devices().await;
    mac.service
        .change_password(&mac.id(), "pw", "new password")
        .await
        .unwrap();
    phone.service.lock(&phone.id()).unwrap();
    // the phone's key file is still the old one; the new password is checked against S3
    phone
        .service
        .unlock(&phone.id(), "new password")
        .await
        .unwrap();
    // its key file is now the new one, so the old password no longer fits
    phone.service.lock(&phone.id()).unwrap();
    assert!(matches!(
        phone.service.unlock(&phone.id(), "pw").await,
        Err(Error::WrongPassword)
    ));
    phone
        .service
        .unlock(&phone.id(), "new password")
        .await
        .unwrap();
    phone.sync().await;
    assert!(phone.service.status(&phone.id()).unwrap().unlocked);
}

#[tokio::test]
async fn a_password_change_with_the_wrong_old_password_changes_nothing() {
    let (mac, phone, _) = two_devices().await;
    let before = fs::read(mac.dir().join(".store/key.json")).unwrap();
    let result = mac.service.change_password(&mac.id(), "wrong", "new").await;
    assert!(matches!(result, Err(Error::WrongPassword)));
    assert_eq!(fs::read(mac.dir().join(".store/key.json")).unwrap(), before);
    phone.service.lock(&phone.id()).unwrap();
    phone.service.unlock(&phone.id(), "pw").await.unwrap();
}

#[tokio::test]
async fn two_simultaneous_password_changes_do_not_overwrite_each_other() {
    let (mac, phone, _) = two_devices().await;
    // the phone changes first, and the mac, which has not heard, tries to as well
    phone
        .service
        .change_password(&phone.id(), "pw", "phone's password")
        .await
        .unwrap();
    // change_password syncs first, so the mac sees the phone's key file and builds on it:
    // its old password no longer fits
    let result = mac
        .service
        .change_password(&mac.id(), "pw", "mac's password")
        .await;
    assert!(matches!(result, Err(Error::WrongPassword)), "{result:?}");
    mac.service
        .change_password(&mac.id(), "phone's password", "mac's password")
        .await
        .unwrap();
    phone.sync().await;
    phone.service.lock(&phone.id()).unwrap();
    phone
        .service
        .unlock(&phone.id(), "mac's password")
        .await
        .unwrap();
}

/// A bucket that, once armed, lets "another device" change the password just before the second listing,
/// which is the moment between `change_password`'s two syncs.
#[derive(Clone)]
struct Meddling {
    bucket: InMemoryConnect,
    lists_since_armed: Arc<AtomicUsize>,
    armed: Arc<AtomicBool>,
}

struct MeddlingRemote {
    inner: <InMemoryConnect as Connect>::Remote,
    owner: Meddling,
}

impl Connect for Meddling {
    type Remote = MeddlingRemote;

    fn connect(&self, s: &RemoteSettings, secret: &str) -> flint_remote::Result<MeddlingRemote> {
        Ok(MeddlingRemote {
            inner: self.bucket.connect(s, secret)?,
            owner: self.clone(),
        })
    }
}

impl RemoteStore for MeddlingRemote {
    fn capabilities(&self) -> flint_remote::Capabilities {
        self.inner.capabilities()
    }

    async fn list(&self) -> flint_remote::Result<Vec<flint_remote::RemoteEntry>> {
        if self.owner.armed.load(Ordering::SeqCst)
            && self.owner.lists_since_armed.fetch_add(1, Ordering::SeqCst) == 1
        {
            let key = p(".store/key.json");
            let theirs = flint_crypto::KeyFile::from_json(
                std::str::from_utf8(&self.inner.get(&key).await?.bytes).unwrap(),
            )
            .unwrap()
            .change_password("pw", "elsewhere", FAST)
            .unwrap()
            .to_json();
            let hash = flint_core::ContentHash::of(theirs.as_bytes());
            self.inner
                .put(
                    &key,
                    theirs.into(),
                    &hash,
                    flint_remote::Precondition::Unconditional,
                )
                .await?;
        }
        self.inner.list().await
    }

    async fn head(
        &self,
        path: &StorePath,
    ) -> flint_remote::Result<Option<flint_remote::RemoteEntry>> {
        self.inner.head(path).await
    }

    async fn get(&self, path: &StorePath) -> flint_remote::Result<flint_remote::RemoteObject> {
        self.inner.get(path).await
    }

    async fn put(
        &self,
        path: &StorePath,
        bytes: bytes::Bytes,
        sha256: &flint_core::ContentHash,
        precondition: flint_remote::Precondition,
    ) -> flint_remote::Result<flint_remote::RemoteEntry> {
        self.inner.put(path, bytes, sha256, precondition).await
    }

    async fn delete(
        &self,
        path: &StorePath,
        expect: Option<&flint_remote::ETag>,
    ) -> flint_remote::Result<()> {
        self.inner.delete(path, expect).await
    }
}

#[tokio::test]
async fn a_password_change_that_loses_a_race_is_rolled_back() {
    let meddling = Meddling {
        bucket: InMemoryConnect::new(),
        lists_since_armed: Arc::default(),
        armed: Arc::default(),
    };
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let registry = Registry::open(config.path(), MemorySecrets::new()).unwrap();
    let mut mac = AppService::new(registry, meddling.clone(), DeviceName::new("Mac"))
        .with_kdf(FAST)
        .with_quiet_period(Duration::ZERO);
    let id = mac
        .create_store(NewStore {
            name: "Notes".into(),
            folder: folder.path().join("Notes"),
            remote: settings(),
            s3_secret: "secret".into(),
            password: "pw".into(),
        })
        .await
        .unwrap();
    let key_file = folder.path().join("Notes/.store/key.json");
    let before = fs::read(&key_file).unwrap();

    meddling.armed.store(true, Ordering::SeqCst);
    let result = mac.change_password(&id, "pw", "mac's password").await;
    assert!(
        matches!(result, Err(Error::PasswordChangedElsewhere)),
        "{result:?}"
    );
    assert_eq!(
        fs::read(&key_file).unwrap(),
        before,
        "the local key file is back as it was"
    );

    // nothing was lost: the next sync takes the other device's key file, and its password is the one that works
    meddling.armed.store(false, Ordering::SeqCst);
    mac.sync(&id).await.unwrap();
    mac.lock(&id).unwrap();
    assert!(matches!(
        mac.unlock(&id, "mac's password").await,
        Err(Error::WrongPassword)
    ));
    mac.unlock(&id, "elsewhere").await.unwrap();
}

#[tokio::test]
async fn status_lists_files_still_waiting_to_be_encrypted() {
    let (mac, _, _) = two_devices().await;
    mac.drop_in("new.md", "just dropped");
    let status = mac.service.status(&mac.id()).unwrap();
    assert_eq!(status.awaiting_encryption, [p("new.md")]);
    mac.service.adopt(&mac.id(), Duration::ZERO).unwrap();
    assert!(
        mac.service
            .status(&mac.id())
            .unwrap()
            .awaiting_encryption
            .is_empty()
    );
}

#[tokio::test]
async fn a_second_store_can_reuse_the_first_ones_s3_secret() {
    let (mut mac, _, _) = two_devices().await;
    let first = mac.id();
    let secret = mac.service.reuse_s3_secret(&first).unwrap();
    assert_eq!(secret, "secret");

    let mut second = mac.new_store("other password");
    second.name = "Second".into();
    second.folder = mac.folder.path().join("Second");
    second.remote.prefix = "second".into();
    second.s3_secret = secret;
    let id = mac.service.create_store(second).await.unwrap();
    assert_eq!(mac.service.reuse_s3_secret(&id).unwrap(), "secret");

    let nobody = StoreId::generate();
    assert!(matches!(
        mac.service.reuse_s3_secret(&nobody),
        Err(Error::Registry(flint_registry::Error::UnknownStore(_)))
    ));
}

#[tokio::test]
async fn stores_cannot_be_nested_and_names_are_unique() {
    let (mut mac, _, _) = two_devices().await;
    let mut inner = mac.new_store("pw");
    inner.name = "Inner".into();
    inner.folder = mac.dir().join("inside");
    inner.remote.prefix = "other".into();
    assert!(matches!(
        mac.service.create_store(inner).await,
        Err(Error::Registry(flint_registry::Error::NestedStore { .. }))
    ));
    assert_eq!(
        mac.service.registry().list().len(),
        1,
        "nothing half-made is left in the registry"
    );
    assert!(!Path::new(&mac.dir().join("inside/.store/key.json")).exists());
}

/// The real thing, on a copy of a real vault: set `FLINT_SAMPLE_VAULT`, then
/// `cargo test -p flint-app-service -- --ignored vault`.
#[tokio::test]
#[ignore = "needs FLINT_SAMPLE_VAULT"]
async fn a_copy_of_a_real_vault_survives_a_round_trip_between_two_devices() {
    let source = std::env::var("FLINT_SAMPLE_VAULT").expect("FLINT_SAMPLE_VAULT");
    let bucket = InMemoryConnect::new();
    let mut mac = Device::new("Mac", &bucket);
    flint_testkit::copy_dir_all(Path::new(&source), &mac.dir()).unwrap();
    mac.service.create_store(mac.new_store("pw")).await.unwrap();
    let up = mac.sync().await;
    assert!(up.report.failed.is_empty(), "{:?}", up.report.failed);

    let mut phone = Device::new("Phone", &bucket);
    phone
        .service
        .join_store(phone.new_store("pw"))
        .await
        .unwrap();
    let down = phone.sync().await;
    assert!(down.report.failed.is_empty(), "{:?}", down.report.failed);
    assert_eq!(
        phone.service.files(&phone.id()).unwrap(),
        mac.service.files(&mac.id()).unwrap()
    );
    for file in mac.service.files(&mac.id()).unwrap() {
        if file.as_str().ends_with(".md") {
            assert_eq!(mac.read(file.as_str()), phone.read(file.as_str()), "{file}");
        }
    }
    assert!(
        phone.sync().await.report.is_quiet(),
        "a second sync finds nothing to do"
    );
}
