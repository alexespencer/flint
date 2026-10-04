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
