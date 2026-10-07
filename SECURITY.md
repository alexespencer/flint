# Security

Flint is a **personal project and has not been independently audited.** It is built from well-known, audited building blocks and no custom cryptography, but nobody other than its author has reviewed how they are put together. Use it for notes you can afford to lose or have read, and keep your own backups. There is no warranty (see [LICENSE](LICENSE)).

## What Flint protects, and how

- Every note and attachment is encrypted before it is written to disk or uploaded: XChaCha20-Poly1305 with a random nonce per file, and the file's **path authenticated**, so a file moved or swapped under another name fails to open.
- A random 256-bit data key encrypts everything. It is stored only wrapped under a key derived from the store's password with **Argon2id** (256 MiB, 2 passes), in `.store/key.json`. Changing the password re-wraps the data key; no file is re-encrypted.
- The local search and link index is an encrypted SQLCipher database, keyed from the data key.
- Sync refuses to upload plaintext, and refuses to write a downloaded file that does not decrypt as the name it arrived under, so a hostile or damaged server cannot plant plaintext notes or replace a good local note with garbage.
- The primitives are the RustCrypto crates (`argon2`, `chacha20poly1305`, `hkdf`, `sha2`) and SQLCipher.

## What it does **not** protect

- **File names, folder structure, sizes and timing** are visible to whoever can read the bucket or the folder. Only file _contents_ are encrypted.
- **A password cannot be recovered.** Nobody, including the author, can read a store without it.
- **A store with no password** (an option when creating one) is still encrypted, but with a key derived from the empty password, so anyone who can read the bucket or the folder can read it. It is for convenience, not secrecy.
- **Folders whose names start with a dot** (`.git`, `.obsidian`, ...) are neither encrypted nor synced; they stay readable in the store folder.
- **A device that is unlocked, or malware running as you,** can read what the app can. Touch ID and fingerprint unlock gate _the app's_ use of a stored key; on macOS the operating system does not enforce that gate (Android's hardware-backed key does). Keys are removed from the keychain on lock but not yet zeroed in the app's memory.
- **Plaintext files dropped into the store folder** are readable until the app encrypts them in place (at the next sync, a moment after they stop changing).
- **Windows** does not yet keep secrets between launches (see the [roadmap](docs/roadmap.md)).
- Nothing here defends against someone who can modify the app itself.

## Reporting a problem

Please report a vulnerability **privately**: use the repository's **Security** tab → **Report a vulnerability** (GitHub private vulnerability reporting), rather than a public issue. This is a one-person project, so replies are best-effort.
