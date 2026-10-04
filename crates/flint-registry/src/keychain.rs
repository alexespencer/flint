//! Secrets in the macOS login keychain.
//!
//! Everything lives in **one** keychain item (a JSON map), because macOS asks the user for permission
//! once per item for a program it does not recognise, and an unsigned program is a new one after every
//! rebuild. One item means one prompt per run, however many stores there are and however often the user
//! switches between them. The item is read at most once per process.
//!
//! An older version kept one item per secret; those are folded into the single item the first time it is
//! needed, then deleted.

use std::collections::BTreeMap;
use std::sync::Mutex;

use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use crate::{Error, Result, SecretStore};

/// The account name of the single item.
const VAULT: &str = "vault";

/// `errSecItemNotFound`
const ITEM_NOT_FOUND: i32 = -25300;

/// Secret name -> value as hex (JSON has no byte strings).
type Vault = BTreeMap<String, String>;

pub struct KeychainSecrets {
    service: String,
    /// What was read from the keychain, or last written to it.
    cached: Mutex<Option<Vault>>,
}

impl KeychainSecrets {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            cached: Mutex::new(None),
        }
    }

    /// Reads the item from the keychain (the only place that can make macOS ask the user).
    fn load(&self) -> Result<Vault> {
        match get_generic_password(&self.service, VAULT) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::Secrets(format!("the keychain item is damaged: {e}"))),
            Err(e) if e.code() == ITEM_NOT_FOUND => self.migrate_old_items(),
            Err(e) => Err(Error::Secrets(e.to_string())),
        }
    }

    fn save(&self, vault: &Vault) -> Result<()> {
        let json = serde_json::to_vec(vault).expect("plain data");
        set_generic_password(&self.service, VAULT, &json).map_err(|e| Error::Secrets(e.to_string()))
    }

    /// Gathers the one-item-per-secret entries of the earlier layout into a new vault, saves it, and
    /// deletes them. Finds nothing, and writes nothing, on a Mac that never had them.
    fn migrate_old_items(&self) -> Result<Vault> {
        let mut vault = Vault::new();
        let found = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(&self.service)
            .load_attributes(true)
            .limit(Limit::All)
            .search();
        let accounts: Vec<String> = match found {
            Ok(items) => items
                .iter()
                .filter_map(|item| item.simplify_dict()?.get("acct").cloned())
                .collect(),
            Err(e) if e.code() == ITEM_NOT_FOUND => Vec::new(),
            Err(e) => return Err(Error::Secrets(e.to_string())),
        };
        for account in accounts.iter().filter(|a| a.as_str() != VAULT) {
            let value = get_generic_password(&self.service, account)
                .map_err(|e| Error::Secrets(e.to_string()))?;
            vault.insert(account.clone(), to_hex(&value));
        }
        if !vault.is_empty() {
            self.save(&vault)?;
            for account in vault.keys() {
                let _ = delete_generic_password(&self.service, account);
            }
        }
        Ok(vault)
    }

    /// Applies `change` to the stored vault and saves it. Starts from what is in the keychain now, not
    /// from what this process remembers, so a change made meanwhile by another Flint process (the CLI,
    /// say) is not overwritten.
    fn update(&self, change: impl FnOnce(&mut Vault) -> bool) -> Result<()> {
        let mut cached = self.cached.lock().expect("not poisoned");
        let mut vault = self.load()?;
        if change(&mut vault) {
            self.save(&vault)?;
        }
        *cached = Some(vault);
        Ok(())
    }
}

impl SecretStore for KeychainSecrets {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let mut cached = self.cached.lock().expect("not poisoned");
        if cached.is_none() {
            *cached = Some(self.load()?);
        }
        let value = cached.as_ref().and_then(|vault| vault.get(key));
        value
            .map(|hex| {
                from_hex(hex).ok_or_else(|| Error::Secrets("a saved secret is damaged".into()))
            })
            .transpose()
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.update(|vault| {
            vault.insert(key.to_owned(), to_hex(value));
            true
        })
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.update(|vault| vault.remove(key).is_some())
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_nonsense() {
        let bytes = [0u8, 1, 0x7f, 0x80, 0xff];
        assert_eq!(from_hex(&to_hex(&bytes)).unwrap(), bytes);
        assert_eq!(from_hex("").unwrap(), Vec::<u8>::new());
        for bad in ["abc", "zz", "é1"] {
            assert!(from_hex(bad).is_none(), "{bad}");
        }
    }

    /// Touch the real login keychain, so they only run when asked:
    /// `cargo test -p flint-registry -- --ignored --test-threads=1`.
    fn clean(service: &str) {
        for account in [VAULT, "test-item", "old/one", "old/two"] {
            let _ = delete_generic_password(service, account);
        }
    }

    #[test]
    #[ignore = "uses the real login keychain"]
    fn the_keychain_round_trips_and_forgets() {
        let service = "dev.flint.test";
        clean(service);
        let secrets = KeychainSecrets::new(service);
        assert_eq!(secrets.get("test-item").unwrap(), None);
        assert!(!secrets.exists("test-item").unwrap());
        secrets.set("test-item", &[0, 1, 2, 255]).unwrap();
        assert_eq!(secrets.get("test-item").unwrap(), Some(vec![0, 1, 2, 255]));
        secrets.set("test-item", b"replaced").unwrap();

        // a second handle, as another process would be, sees it, and its own change keeps the first's
        let other = KeychainSecrets::new(service);
        assert_eq!(other.get("test-item").unwrap(), Some(b"replaced".to_vec()));
        other.set("another", b"x").unwrap();
        secrets.set("third", b"y").unwrap();
        let third = KeychainSecrets::new(service);
        assert_eq!(third.get("another").unwrap(), Some(b"x".to_vec()));
        assert_eq!(third.get("third").unwrap(), Some(b"y".to_vec()));

        secrets.delete("test-item").unwrap();
        secrets.delete("test-item").unwrap();
        assert_eq!(
            KeychainSecrets::new(service).get("test-item").unwrap(),
            None
        );
        for key in ["another", "third"] {
            secrets.delete(key).unwrap();
        }
        clean(service);
    }

    #[test]
    #[ignore = "uses the real login keychain"]
    fn items_from_the_one_item_per_secret_layout_are_folded_into_the_vault() {
        let service = "dev.flint.test-migration";
        clean(service);
        set_generic_password(service, "old/one", b"first").unwrap();
        set_generic_password(service, "old/two", &[9, 8, 7]).unwrap();

        let secrets = KeychainSecrets::new(service);
        assert_eq!(secrets.get("old/one").unwrap(), Some(b"first".to_vec()));
        assert_eq!(secrets.get("old/two").unwrap(), Some(vec![9, 8, 7]));
        assert!(
            get_generic_password(service, "old/one").is_err(),
            "the old items are gone"
        );
        assert_eq!(
            KeychainSecrets::new(service).get("old/one").unwrap(),
            Some(b"first".to_vec()),
            "and what they held is in the vault"
        );
        clean(service);
    }
}
