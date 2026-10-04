# Testing against a real S3 bucket

`flint-remote`'s contract suite (17 tests, written once in `src/contract.rs`) runs on three backends:

| Backend | How | When |
|---|---|---|
| `object_store` in-memory | in-process, atomic | every `cargo test` |
| `s3s-fs` (S3 emulator) | in-process, random localhost port | every `cargo test` |
| **your real S3 bucket** | the `FLINT_S3_*` variables below | opt-in: before a release, and after changing the adapter |

The first two cover everything except one thing: `s3s-fs` checks `If-Match` / `If-None-Match` *before* writing, not
atomically, so the "eight writers race and exactly one wins" tests skip it. The in-memory backend is atomic, and the real
bucket is the authoritative check of how S3 itself behaves (error codes, ETags, paging).

## Running it

Use a **scratch prefix** you do not mind filling. The tests write under `flint-contract/<run>/` and the run makes a few
thousand tiny requests (the paging test alone creates 1,250 objects), which costs well under a cent. Keep the
credentials in your own shell (in Claude Code, type `! cargo test ...` so they never enter the conversation):

```sh
export FLINT_S3_BUCKET=your-bucket FLINT_S3_REGION=eu-west-2
export FLINT_S3_ACCESS_KEY=... FLINT_S3_SECRET_KEY=...
# FLINT_S3_ENDPOINT=https://...   only for a non-AWS server (must be https)
cargo test -p flint-remote backends::external -- --ignored
```

For Cloudflare R2 (the endpoint is the account host, **without** the bucket name; the region is literally `auto`):

```sh
export FLINT_S3_ENDPOINT=https://<account id>.r2.cloudflarestorage.com FLINT_S3_REGION=auto
export FLINT_S3_BUCKET=<bucket name> FLINT_S3_ACCESS_KEY=<key id> FLINT_S3_SECRET_KEY=<secret>
```

Create the credentials under R2, "Manage API tokens", with Object Read & Write permission on the bucket. The secret is shown
once.

Sync relies on the server refusing a write whose `If-Match` / `If-None-Match` condition fails, so the server must
support conditional writes (AWS S3, Cloudflare R2, MinIO do). If a run fails, this prints what the server does with each
operation, one line each:

```sh
cargo test -p flint-remote diagnose_the_backend -- --ignored --nocapture 2>&1 | grep DIAG
```

Afterwards, delete what the run left behind:

```sh
cargo test -p flint-remote delete_everything_the_contract_tests_left_behind -- --ignored
```

(or remove the `flint-contract/` prefix yourself, or give it a lifecycle rule that expires it after a day).

## What the three backends taught us (2026-10-04)

- **A real server passed all 17.** RustFS 1.0.1 (run once as a plain binary, then retired because AWS is the real
  target) passed everything, including the concurrency race: with eight writers racing, exactly one create wins and
  exactly one update from the same ETag wins. Its listings include ETags.
- **`s3s-fs` is fine for everything except races.** Its listings carry no ETags (a test pins this, so we notice if it
  changes), which is why `RemoteStore::list_with_etags` exists. It also creates a bucket folder on first write, unlike S3.
- **Path characters.** `object_store` percent-encodes characters it cannot put in a key verbatim (`#`, `%`, `?`, ...).
  This is consistent on every device and round-trips (a test covers 26 awkward names), but the S3 key for `100%.md`
  is `100%25.md`. Ordinary names, spaces, apostrophes and Unicode are stored as they are.
- **A gotcha fixed along the way.** In `object_store`, `with_client_options(...)` replaces the whole options value, so
  it silently undid an earlier `with_allow_http(true)`. `S3Config` now sets both in one place.
- **Failures.** A bad secret key is `AccessDenied` (not retried), and an unreachable server is a `Backend` error that
  gives up in bounded time. `object_store` reports a refused `list` as a generic error even for HTTP 403, so
  `flint-remote` looks at the response to classify it.
