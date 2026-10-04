use std::sync::Arc;

use flint_crypto::{Dek, StoreKeys};
use tempfile::TempDir;

use super::*;
use crate::{Error, MemorySecrets, RemoteSettings};

fn record(name: &str, folder: &str) -> StoreRecord {
    StoreRecord {
        id: StoreId::generate(),
        name: name.into(),
        folder: folder.into(),
        remote: RemoteSettings {
            bucket: "bucket".into(),
            prefix: name.into(),
            region: "auto".into(),
            endpoint: Some("https://example.com".into()),
            access_key_id: "key".into(),
        },
    }
}

fn open(dir: &TempDir, secrets: &Arc<MemorySecrets>) -> Registry<Arc<MemorySecrets>> {
    Registry::open(dir.path(), secrets.clone()).unwrap()
}

#[test]
fn an_empty_registry_has_no_stores_and_nothing_active() {
    let dir = TempDir::new().unwrap();
    let registry = open(&dir, &Arc::default());
    assert!(registry.list().is_empty());
    assert!(registry.active().is_none());
}

#[test]
fn stores_and_the_active_one_survive_a_restart() {
    let dir = TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::new());
    let (a, b) = (record("Work", "/f/work"), record("Home", "/f/home"));
    {
        let mut registry = open(&dir, &secrets);
        registry.add(a.clone()).unwrap();
        registry.add(b.clone()).unwrap();
        assert_eq!(
            registry.active().unwrap().id,
            a.id,
            "the first store added is active"
        );
        registry.set_active(&b.id).unwrap();
    }
    let registry = open(&dir, &secrets);
    assert_eq!(registry.list(), [a, b.clone()]);
    assert_eq!(registry.active().unwrap().id, b.id);
}

#[test]
fn names_are_unique_ignoring_case_and_cannot_be_empty() {
    let dir = TempDir::new().unwrap();
    let mut registry = open(&dir, &Arc::default());
    registry.add(record("Work", "/f/work")).unwrap();
    assert!(matches!(
        registry.add(record("work", "/f/other")),
        Err(Error::NameTaken(_))
    ));
    assert!(matches!(
        registry.add(record("  ", "/f/blank")),
        Err(Error::EmptyName)
    ));
    assert_eq!(registry.list().len(), 1);
}

#[test]
fn stores_cannot_be_nested_or_use_a_relative_folder() {
    let dir = TempDir::new().unwrap();
    let mut registry = open(&dir, &Arc::default());
    registry.add(record("Work", "/f/work")).unwrap();
    for folder in ["/f/work", "/f/work/inner", "/f"] {
        assert!(
            matches!(
                registry.add(record("Other", folder)),
                Err(Error::NestedStore { .. })
            ),
            "{folder}"
        );
    }
    assert!(matches!(
        registry.add(record("Other", "relative/dir")),
        Err(Error::RelativeFolder(_))
    ));
    registry.add(record("Sibling", "/f/work-notes")).unwrap(); // a shared name prefix is not nesting
}

#[test]
fn a_moved_store_can_be_pointed_at_its_new_folder() {
    let dir = TempDir::new().unwrap();
    let mut registry = open(&dir, &Arc::default());
    let work = record("Work", "/f/work");
    registry.add(work.clone()).unwrap();
    registry.add(record("Home", "/f/home")).unwrap();
    registry.set_folder(&work.id, "/g/work").unwrap();
    assert_eq!(registry.get(&work.id).unwrap().folder, Path::new("/g/work"));
    assert!(matches!(
        registry.set_folder(&work.id, "/f/home/x"),
        Err(Error::NestedStore { .. })
    ));
    registry.set_folder(&work.id, "/g/work").unwrap(); // its own current folder is fine
}

#[test]
fn unlocked_keys_come_back_exactly_and_lock_forgets_them() {
    let dir = TempDir::new().unwrap();
    let mut registry = open(&dir, &Arc::default());
    let work = record("Work", "/f/work");
    registry.add(work.clone()).unwrap();
    assert!(!registry.is_unlocked(&work.id).unwrap());

    let keys = StoreKeys::new(7, Dek::from_bytes([9; 32]));
    registry.unlock(&work.id, &keys).unwrap();
    let back = registry.unlocked_keys(&work.id).unwrap().unwrap();
    assert_eq!(back.key_id(), 7);
    assert_eq!(back.dek().as_bytes(), keys.dek().as_bytes());

    registry.lock(&work.id).unwrap();
    assert!(!registry.is_unlocked(&work.id).unwrap());
    registry.lock(&work.id).unwrap(); // locking a locked store is fine
}

#[test]
fn damaged_cached_keys_count_as_locked() {
    let dir = TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::new());
    let mut registry = open(&dir, &secrets);
    let work = record("Work", "/f/work");
    registry.add(work.clone()).unwrap();
    secrets.set(&format!("{}/keys", work.id), b"short").unwrap();
    assert!(registry.unlocked_keys(&work.id).unwrap().is_none());
}

#[test]
fn nothing_secret_is_written_to_the_registry_file() {
    let dir = TempDir::new().unwrap();
    let mut registry = open(&dir, &Arc::default());
    let work = record("Work", "/f/work");
    registry.add(work.clone()).unwrap();
    registry
        .set_s3_secret(&work.id, "SECRET-ACCESS-KEY")
        .unwrap();
    registry
        .unlock(&work.id, &StoreKeys::new(1, Dek::from_bytes([0xAB; 32])))
        .unwrap();
    let text = std::fs::read_to_string(dir.path().join("stores.json")).unwrap();
    assert!(!text.contains("SECRET-ACCESS-KEY"));
    assert!(!text.contains("abab"));
    assert_eq!(
        registry.s3_secret(&work.id).unwrap().as_deref(),
        Some("SECRET-ACCESS-KEY")
    );
}

#[test]
fn removing_a_store_forgets_its_secrets_state_and_active_status() {
    let dir = TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::new());
    let mut registry = open(&dir, &secrets);
    let (work, home) = (record("Work", "/f/work"), record("Home", "/f/home"));
    registry.add(work.clone()).unwrap();
    registry.add(home.clone()).unwrap();
    registry.set_s3_secret(&work.id, "s").unwrap();
    registry
        .unlock(&work.id, &StoreKeys::new(1, Dek::from_bytes([1; 32])))
        .unwrap();
    let state = registry.state_dir(&work.id).unwrap();
    assert!(state.is_dir());

    registry.remove(&work.id).unwrap();
    assert!(matches!(
        registry.get(&work.id),
        Err(Error::UnknownStore(_))
    ));
    assert_eq!(
        registry.active().unwrap().id,
        home.id,
        "another store takes over"
    );
    assert!(
        secrets
            .get(&format!("{}/s3-secret", work.id))
            .unwrap()
            .is_none()
    );
    assert!(secrets.get(&format!("{}/keys", work.id)).unwrap().is_none());
    assert!(!state.exists());
}

#[test]
fn a_damaged_or_newer_registry_file_is_refused_and_left_alone() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("stores.json");
    std::fs::write(&path, "not json").unwrap();
    assert!(matches!(
        Registry::open(dir.path(), MemorySecrets::new()),
        Err(Error::Corrupt { .. })
    ));
    std::fs::write(&path, r#"{"format":99,"active":null,"stores":[]}"#).unwrap();
    assert!(matches!(
        Registry::open(dir.path(), MemorySecrets::new()),
        Err(Error::UnsupportedVersion { found: 99, .. })
    ));
    assert!(std::fs::read_to_string(&path).unwrap().contains("99"));
}

#[test]
fn store_ids_are_random_and_only_valid_ones_parse() {
    let id = StoreId::generate();
    assert_ne!(id, StoreId::generate());
    assert_eq!(StoreId::parse(id.as_str()), Some(id));
    for bad in ["", "../etc", "ABCDEF0123456789", "abc"] {
        assert!(StoreId::parse(bad).is_none(), "{bad}");
    }
}
