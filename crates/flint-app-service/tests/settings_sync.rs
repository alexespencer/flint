//! A store's shared settings: kept on each device, synced through S3 as an encrypted `.store/settings.json`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use flint_app_service::{AppService, Connect, Error, InMemoryConnect, NewStore, SettingsChange};
use flint_core::{ContentHash, StorePath};
use flint_crypto::KdfParams;
use flint_registry::{MemorySecrets, Registry, RemoteSettings, StoreId};
use flint_remote::{Capabilities, ETag, Precondition, RemoteEntry, RemoteObject, RemoteStore};
use flint_sync::DeviceName;
use tempfile::TempDir;

const FAST: KdfParams = KdfParams {
    m_kib: 8,
    t: 1,
    p: 1,
};
const SETTINGS_PATH: &str = ".store/settings.json";

fn p(s: &str) -> StorePath {
    StorePath::new(s).unwrap()
}

fn remote_settings() -> RemoteSettings {
    RemoteSettings {
        bucket: "b".into(),
        prefix: "v".into(),
        region: "auto".into(),
        endpoint: None,
        access_key_id: "k".into(),
    }
}

/// A bucket that, when armed, lets "another device" write the settings object just before ours is written.
#[derive(Clone)]
struct RivalBucket {
    inner: InMemoryConnect,
    /// Sealed settings text the rival writes (set by the test), and whether it still has to.
    rival: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    interfered: Arc<AtomicBool>,
}

struct RivalRemote {
    inner: <InMemoryConnect as Connect>::Remote,
    owner: RivalBucket,
}

impl Connect for RivalBucket {
    type Remote = RivalRemote;
    fn connect(&self, s: &RemoteSettings, secret: &str) -> flint_remote::Result<RivalRemote> {
        Ok(RivalRemote {
            inner: self.inner.connect(s, secret)?,
            owner: self.clone(),
        })
    }
}

impl RemoteStore for RivalRemote {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn list(&self) -> flint_remote::Result<Vec<RemoteEntry>> {
        self.inner.list().await
    }
    async fn head(&self, path: &StorePath) -> flint_remote::Result<Option<RemoteEntry>> {
        self.inner.head(path).await
    }
    async fn get(&self, path: &StorePath) -> flint_remote::Result<RemoteObject> {
        self.inner.get(path).await
    }
    async fn put(
        &self,
        path: &StorePath,
        bytes: Bytes,
        sha256: &ContentHash,
        precondition: Precondition,
    ) -> flint_remote::Result<RemoteEntry> {
        if path.as_str() == SETTINGS_PATH {
            let rival = self.owner.rival.lock().unwrap().take();
            if let Some(sealed) = rival {
                // another device writes first: this write was based on what was there before, so it is refused
                self.owner.interfered.store(true, Ordering::SeqCst);
                self.inner
                    .put(
                        path,
                        Bytes::from(sealed.clone()),
                        &ContentHash::of(&sealed),
                        Precondition::Unconditional,
                    )
                    .await?;
            }
        }
        self.inner.put(path, bytes, sha256, precondition).await
    }
    async fn delete(&self, path: &StorePath, expect: Option<&ETag>) -> flint_remote::Result<()> {
        self.inner.delete(path, expect).await
    }
}

struct Device<C: Connect> {
    svc: AppService<MemorySecrets, C>,
    dir: std::path::PathBuf,
    clock: Arc<AtomicU64>,
    _t: (TempDir, TempDir),
}

impl<C: Connect> Device<C> {
    fn new(name: &str, connect: C, start_ms: u64) -> Self {
        let (cfg, folder) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let clock = Arc::new(AtomicU64::new(start_ms));
        let ticking = clock.clone();
        let svc = AppService::new(
            Registry::open(cfg.path(), MemorySecrets::new()).unwrap(),
            connect,
            DeviceName::new(name),
        )
        .with_kdf(FAST)
        .with_quiet_period(Duration::ZERO)
        .with_clock_ms(move || ticking.load(Ordering::SeqCst));
        let dir = folder.path().join("N");
        Self {
            svc,
            dir,
            clock,
            _t: (cfg, folder),
        }
    }

    fn new_store(&self) -> NewStore {
        NewStore {
            name: "N".into(),
            folder: self.dir.clone(),
            remote: remote_settings(),
            s3_secret: "s".into(),
            password: "pw".into(),
        }
    }

    fn id(&self) -> StoreId {
        self.svc.registry().list()[0].id.clone()
    }

    fn set(&self, attachments: Option<&str>, notes: Option<&str>) {
        self.svc
            .set_settings(
                &self.id(),
                SettingsChange {
                    attachment_folder: attachments.map(str::to_owned),
                    new_note_folder: notes.map(str::to_owned),
                },
            )
            .unwrap();
    }

    fn pictures(&self) -> String {
        self.svc
            .settings(&self.id())
            .unwrap()
            .settings
            .attachment_folder
    }

    fn notes(&self) -> String {
        self.svc
            .settings(&self.id())
            .unwrap()
            .settings
            .new_note_folder
    }

    fn at(&self, ms: u64) {
        self.clock.store(ms, Ordering::SeqCst);
    }
}

async fn two_devices() -> (
    Device<InMemoryConnect>,
    Device<InMemoryConnect>,
    InMemoryConnect,
) {
    let bucket = InMemoryConnect::new();
    let mut a = Device::new("A", bucket.clone(), 1_000);
    let mut b = Device::new("B", bucket.clone(), 1_000);
    a.svc.create_store(a.new_store()).await.unwrap();
    b.svc.join_store(b.new_store()).await.unwrap();
    (a, b, bucket)
}

#[tokio::test]
async fn a_fresh_store_has_the_defaults_and_nothing_is_written_to_s3_until_something_is_set() {
    let (a, _, bucket) = two_devices().await;
    assert_eq!((a.pictures().as_str(), a.notes().as_str()), ("Images", ""));
    let outcome = a.svc.sync(&a.id()).await.unwrap();
    assert_eq!(outcome.settings, Default::default());
    let remote = bucket.connect(&remote_settings(), "").unwrap();
    assert!(
        remote.get(&p(SETTINGS_PATH)).await.is_err(),
        "no object yet"
    );
}

#[tokio::test]
async fn a_setting_made_on_one_device_reaches_the_other_and_a_joining_device_starts_with_it() {
    let (a, b, bucket) = two_devices().await;
    a.set(Some("Pics/pasted"), Some("Inbox"));
    let pushed = a.svc.sync(&a.id()).await.unwrap().settings;
    assert!(
        pushed.shared && !pushed.changed_here && pushed.error.is_none(),
        "{pushed:?}"
    );

    let pulled = b.svc.sync(&b.id()).await.unwrap().settings;
    assert!(pulled.changed_here && !pulled.shared, "{pulled:?}");
    assert_eq!(
        (b.pictures().as_str(), b.notes().as_str()),
        ("Pics/pasted", "Inbox")
    );
    // a second sync has nothing more to do
    assert_eq!(
        b.svc.sync(&b.id()).await.unwrap().settings,
        Default::default()
    );

    // a device joining later starts with them
    let mut c = Device::new("C", bucket, 5_000);
    c.svc.join_store(c.new_store()).await.unwrap();
    c.svc.sync(&c.id()).await.unwrap();
    assert_eq!(
        (c.pictures().as_str(), c.notes().as_str()),
        ("Pics/pasted", "Inbox")
    );
}

#[tokio::test]
async fn settings_changed_on_two_devices_are_all_kept_and_the_same_one_goes_to_the_later_change() {
    let (a, b, _) = two_devices().await;
    // offline, both change things: A the pictures folder (twice, the second time later than B's change), B both
    a.at(2_000);
    a.set(Some("APics"), None);
    b.at(3_000);
    b.set(Some("BPics"), Some("BInbox"));
    a.at(4_000);
    a.set(Some("APics2"), None);

    for _ in 0..2 {
        a.svc.sync(&a.id()).await.unwrap();
        b.svc.sync(&b.id()).await.unwrap();
    }
    for device in [&a, &b] {
        assert_eq!(
            device.pictures(),
            "APics2",
            "A's later change wins the setting both changed"
        );
        assert_eq!(
            device.notes(),
            "BInbox",
            "B's change to the other setting is not lost"
        );
    }
}

#[tokio::test]
async fn a_folder_is_tidied_and_an_unusable_one_is_refused_without_changing_anything() {
    let (a, _, _) = two_devices().await;
    a.set(Some(" /Pics/pasted/ "), None);
    assert_eq!(a.pictures(), "Pics/pasted");
    a.set(Some(""), None);
    assert_eq!(a.pictures(), "", "the top level, chosen on purpose");

    for bad in ["../up", ".hidden", "a\\b", "what?"] {
        let refused = a.svc.set_settings(
            &a.id(),
            SettingsChange {
                attachment_folder: Some("Kept".into()),
                new_note_folder: Some(bad.into()),
            },
        );
        assert!(
            matches!(refused, Err(Error::Settings(_))),
            "{bad}: {refused:?}"
        );
        assert_eq!(
            a.pictures(),
            "",
            "{bad}: nothing from the refused change was applied"
        );
    }
}

#[tokio::test]
async fn the_shared_copy_is_encrypted_and_is_not_one_of_the_notes() {
    let (a, b, bucket) = two_devices().await;
    a.set(Some("Secret pictures folder"), None);
    let outcome = a.svc.sync(&a.id()).await.unwrap();
    assert!(
        !outcome
            .report
            .uploaded
            .iter()
            .any(|path| path.as_str() == SETTINGS_PATH),
        "not synced as a file"
    );

    let remote = bucket.connect(&remote_settings(), "").unwrap();
    let stored = remote.get(&p(SETTINGS_PATH)).await.unwrap().bytes;
    assert!(
        stored.starts_with(b"FLNT"),
        "encrypted like everything else"
    );
    assert!(
        !String::from_utf8_lossy(&stored).contains("Secret pictures"),
        "the folder name is not readable in S3"
    );

    b.svc.sync(&b.id()).await.unwrap();
    assert!(
        b.svc.files(&b.id()).unwrap().is_empty(),
        "and it does not show up among the files"
    );
    assert!(!b.dir.join(".store/settings.json").exists());
}

#[tokio::test]
async fn settings_from_a_newer_flint_are_left_alone_and_the_device_says_so() {
    let (a, b, bucket) = two_devices().await;
    // a newer Flint (as far as this one can tell) wrote the shared settings, sealed under the store's key
    let keys = a.svc.registry().unlocked_keys(&a.id()).unwrap().unwrap();
    let newer = r#"{"version": 99, "settings": {"attachment_folder": {"value": "FromTheFuture", "at": 9}}, "new_shape": [1]}"#;
    let sealed = keys.seal(&p(SETTINGS_PATH), newer.as_bytes()).unwrap();
    let remote = bucket.connect(&remote_settings(), "").unwrap();
    remote
        .put(
            &p(SETTINGS_PATH),
            Bytes::from(sealed.clone()),
            &ContentHash::of(&sealed),
            Precondition::Unconditional,
        )
        .await
        .unwrap();

    let outcome = b.svc.sync(&b.id()).await.unwrap();
    assert_eq!(outcome.settings.newer_version, Some(99));
    assert!(!outcome.settings.shared && !outcome.settings.changed_here);
    let view = b.svc.settings(&b.id()).unwrap();
    assert_eq!(view.newer_version, Some(99));
    assert_eq!(
        view.settings.attachment_folder, "Images",
        "its own defaults are used"
    );
    assert!(matches!(
        b.svc.set_settings(
            &b.id(),
            SettingsChange {
                attachment_folder: Some("X".into()),
                ..Default::default()
            }
        ),
        Err(Error::SettingsFromNewerVersion(99))
    ));

    // the shared copy is exactly as it was, however often this device syncs, and notes sync as usual
    b.svc
        .save_note(&b.id(), &p("n.md"), "still syncing")
        .unwrap();
    b.svc.sync(&b.id()).await.unwrap();
    assert_eq!(
        remote.get(&p(SETTINGS_PATH)).await.unwrap().bytes.as_ref(),
        sealed.as_slice()
    );
    a.svc.sync(&a.id()).await.unwrap();
    assert_eq!(
        a.svc.read_note(&a.id(), &p("n.md")).unwrap(),
        "still syncing"
    );
}

#[tokio::test]
async fn a_damaged_shared_copy_is_replaced_by_this_devices_settings() {
    let (a, _, bucket) = two_devices().await;
    let remote = bucket.connect(&remote_settings(), "").unwrap();
    remote
        .put(
            &p(SETTINGS_PATH),
            Bytes::from_static(b"not even encrypted"),
            &ContentHash::of(b"not even encrypted"),
            Precondition::Unconditional,
        )
        .await
        .unwrap();
    a.set(Some("Pics"), None);
    let outcome = a.svc.sync(&a.id()).await.unwrap().settings;
    assert!(outcome.shared && outcome.error.is_none(), "{outcome:?}");
    let keys = a.svc.registry().unlocked_keys(&a.id()).unwrap().unwrap();
    let repaired = keys
        .open(
            &p(SETTINGS_PATH),
            &remote.get(&p(SETTINGS_PATH)).await.unwrap().bytes,
        )
        .unwrap();
    assert!(String::from_utf8(repaired).unwrap().contains("Pics"));
}

#[tokio::test]
async fn a_rival_write_in_the_middle_of_a_push_is_merged_not_overwritten() {
    let bucket = RivalBucket {
        inner: InMemoryConnect::new(),
        rival: Arc::default(),
        interfered: Arc::default(),
    };
    let mut a = Device::new("A", bucket.clone(), 1_000);
    a.svc.create_store(a.new_store()).await.unwrap();

    // the shared copy exists already (made by another device earlier)
    a.at(2_000);
    a.set(Some("First"), None);
    a.svc.sync(&a.id()).await.unwrap();

    // now A changes one setting and pushes; just before that write lands, another device changes the other
    a.at(4_000);
    a.set(None, Some("AInbox"));
    let keys = a.svc.registry().unlocked_keys(&a.id()).unwrap().unwrap();
    let rival =
        r#"{"version": 1, "settings": {"attachment_folder": {"value": "Rival", "at": 3000}}}"#;
    *bucket.rival.lock().unwrap() = Some(keys.seal(&p(SETTINGS_PATH), rival.as_bytes()).unwrap());

    let outcome = a.svc.sync(&a.id()).await.unwrap().settings;
    assert!(
        bucket.interfered.load(Ordering::SeqCst),
        "the rival write happened"
    );
    assert!(outcome.error.is_none() && outcome.shared, "{outcome:?}");
    // neither side's change was lost: the rival's later pictures folder, and A's notes folder
    assert_eq!(a.pictures(), "Rival");
    assert_eq!(a.notes(), "AInbox");
    let remote = bucket.connect(&remote_settings(), "").unwrap();
    let text = String::from_utf8(
        keys.open(
            &p(SETTINGS_PATH),
            &remote.get(&p(SETTINGS_PATH)).await.unwrap().bytes,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(text.contains("Rival") && text.contains("AInbox"), "{text}");
}

#[tokio::test]
async fn deleting_a_store_takes_its_shared_settings_with_it_and_a_removed_store_forgets_them_here()
{
    let (mut a, _, bucket) = two_devices().await;
    a.set(Some("Pics"), None);
    a.svc.sync(&a.id()).await.unwrap();
    let remote = bucket.connect(&remote_settings(), "").unwrap();
    assert!(remote.get(&p(SETTINGS_PATH)).await.is_ok());

    // removing a store from a device keeps it on S3, and joining again brings the settings back
    let id = a.id();
    a.svc.remove_store(&id, false).await.unwrap();
    a.svc.join_store(a.new_store()).await.unwrap();
    a.svc.sync(&a.id()).await.unwrap();
    assert_eq!(a.pictures(), "Pics");

    // deleting it everywhere removes the settings object too
    let id = a.id();
    a.svc.delete_store(&id, "N").await.unwrap();
    assert!(
        remote.list().await.unwrap().is_empty(),
        "nothing is left in S3"
    );
}
