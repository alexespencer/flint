//! What happens when a note file is changed by something other than the app: the app is not the only thing that
//! can touch the store folder (a script, an editor, a copy from Finder).

use std::fs;
use std::time::Duration;

use flint_app_service::{AppService, InMemoryConnect, NewStore};
use flint_core::StorePath;
use flint_crypto::KdfParams;
use flint_registry::{MemorySecrets, Registry, RemoteSettings, StoreId};
use flint_sync::{DeviceName, Refusal};
use tempfile::TempDir;

const FAST: KdfParams = KdfParams {
    m_kib: 8,
    t: 1,
    p: 1,
};

struct Dev {
    svc: AppService<MemorySecrets, InMemoryConnect>,
    dir: std::path::PathBuf,
    _t: (TempDir, TempDir),
}

impl Dev {
    fn id(&self) -> StoreId {
        self.svc.registry().list()[0].id.clone()
    }
    fn read(&self, path: &str) -> Result<String, flint_app_service::Error> {
        self.svc.read_note(&self.id(), &p(path))
    }
}

fn p(s: &str) -> StorePath {
    StorePath::new(s).unwrap()
}

/// Two devices on one store, with `n.md` ("original") synced to both.
async fn fresh() -> (Dev, Dev) {
    let bucket = InMemoryConnect::new();
    let make = |name: &str| {
        let (cfg, folder) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let reg = Registry::open(cfg.path(), MemorySecrets::new()).unwrap();
        let svc = AppService::new(reg, bucket.clone(), DeviceName::new(name))
            .with_kdf(FAST)
            .with_quiet_period(Duration::ZERO);
        let dir = folder.path().join("N");
        Dev {
            svc,
            dir,
            _t: (cfg, folder),
        }
    };
    let (mut a, mut b) = (make("A"), make("B"));
    let new = |d: &Dev| NewStore {
        name: "N".into(),
        folder: d.dir.clone(),
        remote: RemoteSettings {
            bucket: "b".into(),
            prefix: "v".into(),
            region: "auto".into(),
            endpoint: None,
            access_key_id: "k".into(),
        },
        s3_secret: "s".into(),
        password: "pw".into(),
    };
    a.svc.create_store(new(&a)).await.unwrap();
    b.svc.join_store(new(&b)).await.unwrap();
    a.svc.save_note(&a.id(), &p("n.md"), "original").unwrap();
    a.svc.sync(&a.id()).await.unwrap();
    b.svc.sync(&b.id()).await.unwrap();
    (a, b)
}

#[tokio::test]
async fn a_file_replaced_with_plaintext_is_encrypted_and_synced() {
    let (a, b) = fresh().await;
    fs::write(a.dir.join("n.md"), "replaced by an outside program").unwrap();
    let outcome = a.svc.sync(&a.id()).await.unwrap();
    assert_eq!(outcome.adopted.adopted, [p("n.md")]);
    b.svc.sync(&b.id()).await.unwrap();
    assert_eq!(b.read("n.md").unwrap(), "replaced by an outside program");
    assert!(
        !fs::read(a.dir.join("n.md"))
            .unwrap()
            .starts_with(b"replaced"),
        "and no longer readable on disk"
    );
}

#[tokio::test]
async fn a_new_plaintext_file_dropped_in_is_encrypted_and_synced() {
    let (a, b) = fresh().await;
    fs::write(a.dir.join("dropped.md"), "dropped in").unwrap();
    let outcome = a.svc.sync(&a.id()).await.unwrap();
    assert_eq!(outcome.report.uploaded, [p("dropped.md")]);
    b.svc.sync(&b.id()).await.unwrap();
    assert_eq!(b.read("dropped.md").unwrap(), "dropped in");
}

#[tokio::test]
async fn an_emptied_file_becomes_an_empty_note_everywhere() {
    let (a, b) = fresh().await;
    fs::write(a.dir.join("n.md"), "").unwrap();
    a.svc.sync(&a.id()).await.unwrap();
    b.svc.sync(&b.id()).await.unwrap();
    assert_eq!(b.read("n.md").unwrap(), "");
}

#[tokio::test]
async fn a_damaged_encrypted_file_is_not_uploaded_and_the_other_device_keeps_the_good_copy() {
    for damage in ["flip the last byte", "append plaintext"] {
        let (a, b) = fresh().await;
        let mut bytes = fs::read(a.dir.join("n.md")).unwrap();
        if damage == "flip the last byte" {
            let last = bytes.len() - 1;
            bytes[last] ^= 0xff;
        } else {
            bytes.extend_from_slice(b"\nappended line");
        }
        fs::write(a.dir.join("n.md"), bytes).unwrap();

        let outcome = a.svc.sync(&a.id()).await.unwrap();
        assert_eq!(
            outcome.report.refused,
            [(p("n.md"), Refusal::LocalNotDecryptable)],
            "{damage}"
        );
        assert!(outcome.report.uploaded.is_empty(), "{damage}");

        b.svc.sync(&b.id()).await.unwrap();
        assert_eq!(
            b.read("n.md").unwrap(),
            "original",
            "{damage}: the good copy survives on the other device"
        );
        assert!(
            a.read("n.md").is_err(),
            "{damage}: here the file stays as it is, and is reported on every sync"
        );
    }
}
