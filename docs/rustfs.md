# Running the remote contract tests against a real S3 server

`flint-remote`'s contract suite (17 tests, written once in `src/contract.rs`) runs on three backends:

| Backend | How | When |
|---|---|---|
| `object_store` in-memory | in-process | every `cargo test` |
| `s3s-fs` (S3 emulator) | in-process, random localhost port | every `cargo test` |
| **RustFS** (or MinIO, or AWS) | a real server | opt-in, below |

## RustFS, as a plain binary (no Docker)

```sh
scripts/rustfs-contract.sh
```

It downloads RustFS 1.0.1 into `.tools/` (git-ignored, about 100 MB), **verifies the SHA-256 pinned in the script**,
starts it on a random localhost port with throwaway credentials and a temp data folder, runs the suite, and stops it.
Nothing is installed system-wide. Supported: macOS on Apple silicon, Linux x86_64 and aarch64 (needs `curl` 7.75+,
`unzip`, `python3`). To move to a newer RustFS, change `VERSION` and the pinned hashes (from the release's `SHA256SUMS`).
Extra arguments go to the test binary, e.g. `scripts/rustfs-contract.sh --nocapture`.

## Any other S3 server

Set these and run the ignored tests (they write under a unique `flint-contract/...` prefix and leave their objects
behind, so use a scratch bucket):

```sh
export FLINT_S3_BUCKET=my-scratch-bucket FLINT_S3_ACCESS_KEY=... FLINT_S3_SECRET_KEY=...
export FLINT_S3_ENDPOINT=https://s3.example.com   # omit for AWS
export FLINT_S3_REGION=us-east-1                  # default
cargo test -p flint-remote backends::external -- --ignored
```

The endpoint must be `https` unless it is localhost (the crate refuses anything else when it is built).

## What the three backends taught us (2026-10-04)

- **RustFS 1.0.1 passes all 17**, including the concurrency test: with eight writers racing, exactly one create
  wins and exactly one update from the same ETag wins. Its listings include ETags (the same MD5-style values `s3s-fs`
  gives for the same bytes). This closes the M2 parity check.
- **`s3s-fs` is fine for everything except races.** It checks `If-Match` / `If-None-Match` *before* writing, not
  atomically, so the race test skips it. Its listings carry no ETags (a test pins this, so we notice if it changes),
  which is why `RemoteStore::list_with_etags` exists. It also creates a bucket folder on first write, unlike S3.
- **Path characters.** `object_store` percent-encodes characters it cannot put in a key verbatim (`#`, `%`, `?`, ...).
  This is consistent on every device and round-trips (a test covers 26 awkward names), but the S3 key for `100%.md`
  is `100%25.md`. Ordinary names, spaces, apostrophes and Unicode are stored as they are.
- **A gotcha fixed along the way.** In `object_store`, `with_client_options(...)` replaces the whole options value, so
  it silently undid an earlier `with_allow_http(true)`. `S3Config` now sets both in one place.
- **Failures.** A bad secret key is `AccessDenied` (not retried), and an unreachable server is a `Backend` error that
  gives up in bounded time. `object_store` reports a refused `list` as a generic error even for HTTP 403, so
  `flint-remote` looks at the response to classify it.
