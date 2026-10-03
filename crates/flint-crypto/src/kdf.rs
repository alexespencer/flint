use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::{Error, Result};

pub(crate) const KEK_LEN: usize = 32;
pub(crate) const SALT_LEN: usize = 16;

// Upper bounds, so a hostile key.json cannot make unlock allocate gigabytes or spin for minutes.
const MAX_M_KIB: u32 = 1 << 20; // 1 GiB
const MAX_T: u32 = 16;
const MAX_P: u32 = 8;

/// Argon2id cost parameters. Stored per store in `key.json`, so they can be raised later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    /// Passes over memory.
    pub t: u32,
    /// Lanes. The `argon2` crate runs lanes serially, so more than 1 only adds work.
    pub p: u32,
}

impl KdfParams {
    /// 256 MiB, 2 passes, 1 lane: about 0.6 s on the Galaxy S24-class phone and 0.2 s on an M-series
    /// Mac (M0 spike). Memory is preferred over passes because memory is what limits GPU cracking.
    pub const DEFAULT: Self = Self {
        m_kib: 256 * 1024,
        t: 2,
        p: 1,
    };

    pub fn validate(&self) -> Result<()> {
        if !(1..=MAX_P).contains(&self.p) {
            return Err(Error::KdfParams("lanes (p) out of range"));
        }
        if !(1..=MAX_T).contains(&self.t) {
            return Err(Error::KdfParams("passes (t) out of range"));
        }
        if !(Params::MIN_M_COST.max(8 * self.p)..=MAX_M_KIB).contains(&self.m_kib) {
            return Err(Error::KdfParams("memory (m_kib) out of range"));
        }
        Ok(())
    }

    fn argon2(&self) -> Result<Argon2<'static>> {
        self.validate()?;
        let params = Params::new(self.m_kib, self.t, self.p, Some(KEK_LEN))
            .map_err(|e| Error::Kdf(e.to_string()))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

/// Derives the key-encryption key. The password is Unicode-normalised (NFKC) first, so the same
/// password typed on a Mac and on Android (which may compose accents differently) gives the same key.
pub(crate) fn derive_kek(
    password: &str,
    salt: &[u8],
    params: &KdfParams,
) -> Result<Zeroizing<[u8; KEK_LEN]>> {
    let normalised: Zeroizing<String> = Zeroizing::new(password.nfkc().collect());
    let mut kek = Zeroizing::new([0u8; KEK_LEN]);
    params
        .argon2()?
        .hash_password_into(normalised.as_bytes(), salt, &mut *kek)
        .map_err(|e| Error::Kdf(e.to_string()))?;
    Ok(kek)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Cheapest parameters Argon2 accepts, so tests that derive keys stay fast. Never use outside tests.
    pub(crate) const FAST: KdfParams = KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    };

    #[test]
    fn default_params_are_valid() {
        KdfParams::DEFAULT.validate().unwrap();
    }

    #[test]
    fn rejects_out_of_range_params() {
        for bad in [
            KdfParams { m_kib: 7, ..FAST },
            KdfParams { m_kib: 0, ..FAST },
            KdfParams {
                m_kib: (1 << 20) + 1,
                ..FAST
            },
            KdfParams { t: 0, ..FAST },
            KdfParams { t: 17, ..FAST },
            KdfParams { p: 0, ..FAST },
            KdfParams { p: 9, ..FAST },
            KdfParams {
                m_kib: 8,
                p: 2,
                ..FAST
            }, // memory must be at least 8 KiB per lane
        ] {
            assert!(
                matches!(bad.validate(), Err(Error::KdfParams(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn same_inputs_same_key_and_any_change_changes_it() {
        let salt = [1u8; SALT_LEN];
        let base = derive_kek("hunter2", &salt, &FAST).unwrap();
        assert_eq!(*base, *derive_kek("hunter2", &salt, &FAST).unwrap());
        assert_ne!(*base, *derive_kek("hunter3", &salt, &FAST).unwrap());
        assert_ne!(
            *base,
            *derive_kek("hunter2", &[2u8; SALT_LEN], &FAST).unwrap()
        );
        assert_ne!(
            *base,
            *derive_kek("hunter2", &salt, &KdfParams { t: 2, ..FAST }).unwrap()
        );
    }

    #[test]
    fn password_is_unicode_normalised() {
        let salt = [1u8; SALT_LEN];
        // "é" as one code point versus "e" + combining acute: the same password to a human.
        let composed = derive_kek("caf\u{e9}", &salt, &FAST).unwrap();
        let decomposed = derive_kek("cafe\u{301}", &salt, &FAST).unwrap();
        assert_eq!(*composed, *decomposed);
        assert_ne!(*composed, *derive_kek("cafe", &salt, &FAST).unwrap());
    }
}
