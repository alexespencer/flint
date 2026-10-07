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
    bucket: InMemoryConnect,
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
            bucket: bucket.clone(),
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

/// A PNG's signature, then bytes that look like nothing in particular: stands in for a real image.
fn fake_png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend((0..=255u8).cycle().take(5000));
    bytes
}

#[tokio::test]
async fn an_image_is_encrypted_on_disk_and_in_s3_and_comes_back_byte_for_byte() {
    let (a, b) = fresh().await;
    let image = fake_png();
    fs::create_dir_all(a.dir.join("Images")).unwrap();
    fs::write(a.dir.join("Images/Pasted image 1.png"), &image).unwrap(); // dropped into the folder
    let outcome = a.svc.sync(&a.id()).await.unwrap();
    assert_eq!(outcome.report.uploaded, [p("Images/Pasted image 1.png")]);

    // the file on disk is no longer a PNG: it starts with Flint's header, and the picture's bytes are not in it
    let on_disk = fs::read(a.dir.join("Images/Pasted image 1.png")).unwrap();
    assert!(on_disk.starts_with(b"FLNT") && !on_disk.starts_with(b"\x89PNG"));
    assert!(
        !on_disk.windows(8).any(|w| w == &image[8..16]),
        "no run of the image's bytes is readable"
    );

    // ... and so is what reached S3
    let remote = flint_app_service::Connect::connect(
        &a.bucket,
        &flint_registry::RemoteSettings {
            bucket: "b".into(),
            prefix: "v".into(),
            region: "auto".into(),
            endpoint: None,
            access_key_id: "k".into(),
        },
        "",
    )
    .unwrap();
    use flint_remote::RemoteStore;
    let stored = remote
        .get(&p("Images/Pasted image 1.png"))
        .await
        .unwrap()
        .bytes;
    assert!(stored.starts_with(b"FLNT") && !stored.starts_with(b"\x89PNG"));
    assert!(!stored.windows(8).any(|w| w == &image[8..16]));

    // each device can read it back exactly, through the app
    assert_eq!(
        a.svc
            .read_file(&a.id(), &p("Images/Pasted image 1.png"))
            .unwrap(),
        image
    );
    b.svc.sync(&b.id()).await.unwrap();
    assert_eq!(
        b.svc
            .read_file(&b.id(), &p("Images/Pasted image 1.png"))
            .unwrap(),
        image
    );
    assert!(matches!(
        b.svc.read_file(&b.id(), &p("Images/missing.png")),
        Err(flint_app_service::Error::NothingAt(_))
    ));
}
