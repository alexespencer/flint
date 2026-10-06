use flint_core::StorePath;
use flint_crypto::StoreKeys;

/// What sync needs to know about file contents, which it otherwise treats as opaque bytes.
///
/// Every file's path is part of what is encrypted, so ciphertext cannot simply be copied to another
/// name: it has to be opened under the old path and sealed under the new one.
pub trait Codec {
    /// Whether two stored versions of `path` hold the same plaintext. Identical edits made on two devices
    /// produce different ciphertext (random nonces), so hashes cannot answer this.
    fn same_plaintext(
        &self,
        path: &StorePath,
        a: &[u8],
        b: &[u8],
    ) -> Result<bool, flint_crypto::Error>;

    /// Whether `sealed` really is ciphertext of `path` that these keys can open. Starting with Flint's header
    /// is not enough: a file damaged or overwritten in part by another program still starts with it.
    fn opens(&self, path: &StorePath, sealed: &[u8]) -> bool;

    /// Re-encrypts `sealed` (a stored version of `from`) so it is valid as the contents of `to`.
    fn reseal(
        &self,
        from: &StorePath,
        to: &StorePath,
        sealed: &[u8],
    ) -> Result<Vec<u8>, flint_crypto::Error>;
}

/// The real codec: the store's keys.
pub struct KeysCodec(pub StoreKeys);

impl Codec for KeysCodec {
    fn opens(&self, path: &StorePath, sealed: &[u8]) -> bool {
        self.0.open(path, sealed).is_ok()
    }

    fn same_plaintext(
        &self,
        path: &StorePath,
        a: &[u8],
        b: &[u8],
    ) -> Result<bool, flint_crypto::Error> {
        Ok(self.0.open(path, a)? == self.0.open(path, b)?)
    }

    fn reseal(
        &self,
        from: &StorePath,
        to: &StorePath,
        sealed: &[u8],
    ) -> Result<Vec<u8>, flint_crypto::Error> {
        self.0.seal(to, &self.0.open(from, sealed)?)
    }
}
