use std::fmt;
use std::str::FromStr;

use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// SHA-256 of some bytes. Sync compares these (over the stored ciphertext) instead of timestamps.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Lowercase hex, the form stored in sync.db and in `x-amz-meta-sha256`.
impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

impl FromStr for ContentHash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s, &mut bytes)?;
        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Published SHA-256 test vectors (FIPS 180-2 / NIST).
    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn matches_published_sha256_vectors() {
        assert_eq!(ContentHash::of(b"").to_string(), EMPTY);
        assert_eq!(ContentHash::of(b"abc").to_string(), ABC);
    }

    #[test]
    fn hex_round_trips() {
        let hash = ContentHash::of(b"abc");
        assert_eq!(hash.to_string().parse::<ContentHash>().unwrap(), hash);
        assert_eq!(ABC.to_uppercase().parse::<ContentHash>().unwrap(), hash);
        assert_eq!(ContentHash::from_bytes(*hash.as_bytes()), hash);
    }

    #[test]
    fn rejects_malformed_hex() {
        for bad in [
            "",
            "abc",
            &ABC[..63],
            &format!("{ABC}00"),
            &format!("{}g", &ABC[..63]),
        ] {
            assert!(
                matches!(bad.parse::<ContentHash>(), Err(Error::InvalidHash(_))),
                "input {bad:?}"
            );
        }
    }

    #[test]
    fn different_bytes_give_different_hashes() {
        assert_ne!(ContentHash::of(b"a"), ContentHash::of(b"b"));
    }

    #[test]
    fn debug_shows_the_hex() {
        assert_eq!(
            format!("{:?}", ContentHash::of(b"abc")),
            format!("ContentHash({ABC})")
        );
    }
}
