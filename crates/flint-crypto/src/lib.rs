//! Key derivation, key wrapping and the encrypted file format for Flint. Pure: no I/O.
//!
//! The key hierarchy:
//!
//! ```text
//! password --Argon2id(salt, params)--> KEK --unwraps--> DEK (random 256-bit, stored wrapped in key.json)
//! DEK --HKDF--> content key (encrypts files)  and  index key (keys the local SQLCipher index)
//! ```
//!
//! Changing the password re-wraps the DEK under a new KEK; no file is re-encrypted. Each file is
//! XChaCha20-Poly1305 with a random 24-byte nonce, and its [`StorePath`](flint_core::StorePath) is
//! authenticated, so a file moved or swapped under another name fails to open.
//!
//! Only audited RustCrypto crates are used; there are no custom primitives.

mod error;
mod file;
mod kdf;
mod keyfile;
mod keys;

pub use error::{Error, Result};
pub use kdf::KdfParams;
pub use keyfile::KeyFile;
pub use keys::{Dek, StoreKeys};
