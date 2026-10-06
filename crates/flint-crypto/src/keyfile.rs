//! `key.json`: the per-store file holding the Argon2 parameters, the salt and the wrapped DEK.
//!
//! This is the one `.store/` file that syncs, so a second device needs only the password. It holds
//! no secret in the clear. The DEK is wrapped with XChaCha20-Poly1305 under the password-derived
//! KEK, and every other field (version, key id, KDF parameters, salt) is authenticated by that
//! wrap, so weakening the KDF parameters or swapping the salt makes unlock fail.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::kdf::{KEK_LEN, KdfParams, SALT_LEN, derive_kek};
use crate::keys::{Dek, KEY_LEN, StoreKeys};
use crate::{Error, Result};

const VERSION: u32 = 1;
const ALGORITHM: &str = "argon2id";
const NONCE_LEN: usize = 24;
const WRAPPED_LEN: usize = KEY_LEN + 16;
const FIRST_KEY_ID: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyFile {
    version: u32,
    key_id: u32,
    kdf: KdfSection,
    wrapped_dek: WrappedDek,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KdfSection {
    algorithm: String,
    #[serde(flatten)]
    params: KdfParams,
    #[serde(with = "b64")]
    salt: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WrappedDek {
    #[serde(with = "b64")]
    nonce: Vec<u8>,
    #[serde(with = "b64")]
    ciphertext: Vec<u8>,
}

mod b64 {
    use super::*;

    #[allow(clippy::ptr_arg)] // serde's `with` passes `&Vec<u8>`
    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

impl KeyFile {
    /// Makes a new store key: a random DEK wrapped under `password`.
    pub fn create(password: &str, params: KdfParams) -> Result<(Self, StoreKeys)> {
        let dek = Dek::random()?;
        let key_file = Self::wrap(password, params, FIRST_KEY_ID, &dek)?;
        Ok((key_file, StoreKeys::new(FIRST_KEY_ID, dek)))
    }

    /// Derives the KEK from `password` and unwraps the DEK. Slow by design (Argon2id).
    pub fn unlock(&self, password: &str) -> Result<StoreKeys> {
        self.check_shape()?;
        let kek = derive_kek(password, &self.kdf.salt, &self.kdf.params)?;
        let nonce: [u8; NONCE_LEN] = self
            .wrapped_dek
            .nonce
            .as_slice()
            .try_into()
            .expect("checked by check_shape");
        let aad = wrap_aad(self.key_id, &self.kdf.params, &self.kdf.salt);
        let dek = XChaCha20Poly1305::new_from_slice(&*kek)
            .expect("KEK is 32 bytes")
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &self.wrapped_dek.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Unlock)?;
        let bytes: [u8; KEY_LEN] = dek.as_slice().try_into().map_err(|_| Error::Unlock)?;
        Ok(StoreKeys::new(self.key_id, Dek::from_bytes(bytes)))
    }

    /// Re-wraps the same DEK under a new password (fresh salt and nonce). No file is re-encrypted,
    /// and keys cached on other devices stay valid.
    pub fn change_password(&self, old: &str, new: &str, new_params: KdfParams) -> Result<Self> {
        let keys = self.unlock(old)?;
        Self::wrap(new, new_params, self.key_id, keys.dek())
    }

    pub fn key_id(&self) -> u32 {
        self.key_id
    }

    pub fn kdf_params(&self) -> KdfParams {
        self.kdf.params
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("KeyFile always serialises")
    }

    pub fn from_json(json: &str) -> Result<Self> {
        let key_file: Self =
            serde_json::from_str(json).map_err(|e| Error::KeyFile(e.to_string()))?;
        key_file.check_shape()?;
        Ok(key_file)
    }

    fn wrap(password: &str, params: KdfParams, key_id: u32, dek: &Dek) -> Result<Self> {
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut salt)?;
        getrandom::fill(&mut nonce)?;

        let kek = derive_kek(password, &salt, &params)?;
        let aad = wrap_aad(key_id, &params, &salt);
        let ciphertext = XChaCha20Poly1305::new_from_slice(&*kek)
            .expect("KEK is 32 bytes")
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: dek.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Encrypt)?;
        Ok(Self {
            version: VERSION,
            key_id,
            kdf: KdfSection {
                algorithm: ALGORITHM.into(),
                params,
                salt: salt.to_vec(),
            },
            wrapped_dek: WrappedDek {
                nonce: nonce.to_vec(),
                ciphertext,
            },
        })
    }

    /// Rejects anything unusable before spending 256 MiB of memory on it.
    fn check_shape(&self) -> Result<()> {
        let bad = |what: &str| Err(Error::KeyFile(what.to_string()));
        if self.version != VERSION {
            return bad(&format!("unsupported version {}", self.version));
        }
        if self.kdf.algorithm != ALGORITHM {
            return bad(&format!("unsupported KDF {:?}", self.kdf.algorithm));
        }
        if self.kdf.salt.len() != SALT_LEN {
            return bad("salt has the wrong length");
        }
        if self.wrapped_dek.nonce.len() != NONCE_LEN {
            return bad("nonce has the wrong length");
        }
        if self.wrapped_dek.ciphertext.len() != WRAPPED_LEN {
            return bad("wrapped key has the wrong length");
        }
        self.kdf.params.validate()
    }
}

/// Fixed-layout authenticated data for the wrap: everything in key.json except the wrapped key.
fn wrap_aad(key_id: u32, params: &KdfParams, salt: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(32 + salt.len());
    aad.extend_from_slice(b"flint key v1");
    for n in [key_id, params.m_kib, params.t, params.p] {
        aad.extend_from_slice(&n.to_be_bytes());
    }
    aad.extend_from_slice(salt);
    aad
}

const _: () = assert!(
    KEK_LEN == KEY_LEN,
    "the KEK is used directly as an XChaCha20-Poly1305 key"
);

#[cfg(test)]
mod tests {
    use flint_core::StorePath;
    use serde_json::{Value, json};

    use super::*;
    use crate::kdf::tests::FAST;

    #[test]
    fn the_empty_password_is_a_real_password() {
        // a store with "no password" is wrapped under the empty one: still encrypted, just easy to open
        let (key_file, keys) = KeyFile::create("", FAST).unwrap();
        let again = KeyFile::from_json(&key_file.to_json()).unwrap();
        assert_eq!(
            again.unlock("").unwrap().dek().as_bytes(),
            keys.dek().as_bytes()
        );
        assert!(matches!(again.unlock("anything"), Err(Error::Unlock)));
        // and a store with a password is not opened by the empty one
        let (protected, _) = KeyFile::create("pw", FAST).unwrap();
        assert!(matches!(protected.unlock(""), Err(Error::Unlock)));
        // the empty password can be upgraded to a real one, which no longer opens with ""
        let upgraded = key_file.change_password("", "pw", FAST).unwrap();
        assert!(upgraded.unlock("pw").is_ok());
        assert!(upgraded.unlock("").is_err());
    }

    // Generated by a separate Python implementation (cryptography's Argon2id + XChaCha20-Poly1305
    // built from HChaCha20), not by this code. Password "correct horse", DEK 0x11 x32, key id 3.
    const INDEPENDENT: &str = r#"{
      "version": 1, "key_id": 3,
      "kdf": {"algorithm": "argon2id", "m_kib": 8, "t": 1, "p": 1, "salt": "AAECAwQFBgcICQoLDA0ODw=="},
      "wrapped_dek": {"nonce": "ZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7",
                      "ciphertext": "/xxd0QHgbtQJlG0hdAHUd2KYPeBdyE+btQxy8mtpps0i/GY4uajZLCxsvjuEJAvZ"}
    }"#;
    // Same, but the password is "café ①" and NFKC folds the circled digit to "1".
    const INDEPENDENT_NFKC: &str = r#"{"version": 1, "key_id": 3, "kdf": {"algorithm": "argon2id", "m_kib": 8, "t": 1, "p": 1, "salt": "AAECAwQFBgcICQoLDA0ODw=="}, "wrapped_dek": {"nonce": "ZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7", "ciphertext": "J3zJVL5WaZNFteUrOtPGQn3JOLeywlk//leKafc4SErwqad5L9SBr8qu2fmO/r6i"}}"#;

    type Edit = Box<dyn Fn(&mut Value)>;

    fn path() -> StorePath {
        StorePath::new("notes/a.md").unwrap()
    }

    fn edited(key_file: &KeyFile, edit: impl FnOnce(&mut Value)) -> String {
        let mut value: Value = serde_json::from_str(&key_file.to_json()).unwrap();
        edit(&mut value);
        value.to_string()
    }

    #[test]
    fn create_then_unlock_gives_the_same_keys() {
        let (key_file, created) = KeyFile::create("hunter2", FAST).unwrap();
        let unlocked = key_file.unlock("hunter2").unwrap();
        assert_eq!(unlocked.key_id(), 1);
        assert_eq!(created.dek().as_bytes(), unlocked.dek().as_bytes());
        let sealed = created.seal(&path(), b"note").unwrap();
        assert_eq!(unlocked.open(&path(), &sealed).unwrap(), b"note");
    }

    #[test]
    fn wrong_password_is_refused() {
        let (key_file, _) = KeyFile::create("hunter2", FAST).unwrap();
        for wrong in ["hunter3", "", "Hunter2", "hunter2 "] {
            assert!(
                matches!(key_file.unlock(wrong), Err(Error::Unlock)),
                "{wrong:?}"
            );
        }
    }

    #[test]
    fn unlocks_a_key_file_made_by_an_independent_implementation() {
        let key_file = KeyFile::from_json(INDEPENDENT).unwrap();
        let keys = key_file.unlock("correct horse").unwrap();
        assert_eq!(keys.key_id(), 3);
        assert_eq!(keys.dek().as_bytes(), &[0x11; 32]);
        assert!(matches!(key_file.unlock("wrong horse"), Err(Error::Unlock)));
    }

    #[test]
    fn password_spellings_that_normalise_alike_unlock_the_same_file() {
        let key_file = KeyFile::from_json(INDEPENDENT_NFKC).unwrap();
        for spelling in [
            "caf\u{e9} \u{2460}",
            "cafe\u{301} \u{2460}",
            "caf\u{e9} 1",
            "cafe\u{301} 1",
        ] {
            let keys = key_file
                .unlock(spelling)
                .unwrap_or_else(|e| panic!("{spelling:?}: {e}"));
            assert_eq!(keys.dek().as_bytes(), &[0x11; 32]);
        }
        assert!(matches!(key_file.unlock("caf\u{e9} 2"), Err(Error::Unlock)));
    }

    #[test]
    fn json_round_trips() {
        let (key_file, _) = KeyFile::create("hunter2", FAST).unwrap();
        assert_eq!(KeyFile::from_json(&key_file.to_json()).unwrap(), key_file);
    }

    #[test]
    fn json_holds_no_secret_in_the_clear() {
        let (key_file, keys) = KeyFile::create("hunter2", FAST).unwrap();
        let json = key_file.to_json();
        assert!(!json.contains("hunter2"));
        assert!(!json.contains(&STANDARD.encode(keys.dek().as_bytes())));
        assert!(!json.contains(&hex::encode(keys.dek().as_bytes())));
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["kdf"]["algorithm"], "argon2id");
        assert_eq!(value["kdf"]["m_kib"], 8);
    }

    #[test]
    fn every_create_uses_a_fresh_dek_and_salt() {
        let (a, ka) = KeyFile::create("same", FAST).unwrap();
        let (b, kb) = KeyFile::create("same", FAST).unwrap();
        assert_ne!(ka.dek().as_bytes(), kb.dek().as_bytes());
        assert_ne!(a.kdf.salt, b.kdf.salt);
        assert_ne!(a.wrapped_dek.nonce, b.wrapped_dek.nonce);
    }

    #[test]
    fn changing_the_password_keeps_the_dek_and_every_existing_file_readable() {
        let (old, keys) = KeyFile::create("old password", FAST).unwrap();
        let sealed = keys.seal(&path(), b"written before the change").unwrap();

        let new = old
            .change_password("old password", "new password", FAST)
            .unwrap();

        assert!(matches!(new.unlock("old password"), Err(Error::Unlock)));
        let after = new.unlock("new password").unwrap();
        assert_eq!(after.dek().as_bytes(), keys.dek().as_bytes());
        assert_eq!(after.key_id(), keys.key_id());
        assert_eq!(
            after.open(&path(), &sealed).unwrap(),
            b"written before the change"
        );
        assert_ne!(old.kdf.salt, new.kdf.salt, "fresh salt");
        assert_ne!(old.wrapped_dek.ciphertext, new.wrapped_dek.ciphertext);
        // the old key file still works with the old password, e.g. on a device that has not synced yet
        assert!(old.unlock("old password").is_ok());
    }

    #[test]
    fn changing_the_password_can_change_the_kdf_cost() {
        let (old, _) = KeyFile::create("pw", FAST).unwrap();
        let stronger = KdfParams {
            m_kib: 64,
            t: 2,
            p: 1,
        };
        let new = old.change_password("pw", "pw2", stronger).unwrap();
        assert_eq!(new.kdf_params(), stronger);
        assert!(new.unlock("pw2").is_ok());
    }

    #[test]
    fn changing_the_password_needs_the_old_one() {
        let (old, _) = KeyFile::create("pw", FAST).unwrap();
        assert!(matches!(
            old.change_password("not pw", "new", FAST),
            Err(Error::Unlock)
        ));
    }

    #[test]
    fn tampering_with_any_authenticated_field_makes_unlock_fail() {
        let (key_file, _) = KeyFile::create("pw", FAST).unwrap();
        let tamperings: [(&str, Edit); 6] = [
            ("key id", Box::new(|v| v["key_id"] = json!(2))),
            ("memory", Box::new(|v| v["kdf"]["m_kib"] = json!(16))),
            ("passes", Box::new(|v| v["kdf"]["t"] = json!(2))),
            (
                "salt",
                Box::new(|v| v["kdf"]["salt"] = json!(STANDARD.encode([9u8; SALT_LEN]))),
            ),
            (
                "nonce",
                Box::new(|v| v["wrapped_dek"]["nonce"] = json!(STANDARD.encode([9u8; NONCE_LEN]))),
            ),
            (
                "wrapped key",
                Box::new(|v| {
                    v["wrapped_dek"]["ciphertext"] = json!(STANDARD.encode([9u8; WRAPPED_LEN]))
                }),
            ),
        ];
        for (name, tamper) in tamperings {
            let json = edited(&key_file, tamper);
            let parsed = KeyFile::from_json(&json).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                matches!(parsed.unlock("pw"), Err(Error::Unlock)),
                "{name} tampering was not detected"
            );
        }
    }

    #[test]
    fn flipping_any_bit_of_the_wrapped_key_makes_unlock_fail() {
        let (key_file, _) = KeyFile::create("pw", FAST).unwrap();
        for byte in 0..WRAPPED_LEN {
            let mut tampered = key_file.clone();
            tampered.wrapped_dek.ciphertext[byte] ^= 1;
            assert!(
                matches!(tampered.unlock("pw"), Err(Error::Unlock)),
                "byte {byte}"
            );
        }
    }

    #[test]
    fn unusable_key_files_are_rejected_before_any_expensive_work() {
        let (key_file, _) = KeyFile::create("pw", FAST).unwrap();
        let cases: [(&str, Edit); 9] = [
            ("future version", Box::new(|v| v["version"] = json!(2))),
            (
                "other KDF",
                Box::new(|v| v["kdf"]["algorithm"] = json!("argon2i")),
            ),
            (
                "short salt",
                Box::new(|v| v["kdf"]["salt"] = json!(STANDARD.encode([0u8; 8]))),
            ),
            (
                "short nonce",
                Box::new(|v| v["wrapped_dek"]["nonce"] = json!(STANDARD.encode([0u8; 12]))),
            ),
            (
                "short wrapped key",
                Box::new(|v| v["wrapped_dek"]["ciphertext"] = json!(STANDARD.encode([0u8; 40]))),
            ),
            (
                "not base64",
                Box::new(|v| v["kdf"]["salt"] = json!("***not base64***")),
            ),
            (
                "hostile memory",
                Box::new(|v| v["kdf"]["m_kib"] = json!(u32::MAX)),
            ),
            (
                "hostile passes",
                Box::new(|v| v["kdf"]["t"] = json!(1_000_000)),
            ),
            (
                "missing field",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("wrapped_dek");
                }),
            ),
        ];
        for (name, edit) in cases {
            let json = edited(&key_file, edit);
            assert!(
                matches!(
                    KeyFile::from_json(&json),
                    Err(Error::KeyFile(_) | Error::KdfParams(_))
                ),
                "{name} was accepted"
            );
        }
        for garbage in ["", "not json", "[]", "{}", "null"] {
            assert!(
                matches!(KeyFile::from_json(garbage), Err(Error::KeyFile(_))),
                "{garbage:?}"
            );
        }
    }
}
