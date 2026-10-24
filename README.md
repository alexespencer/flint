# Flint

Encrypted markdown notes for macOS, Android and Windows with automated S3 sync. A folder of `.md` files with `[[links]]`, backlinks and search, like Obsidian. Every file is encrypted with a password only you know, and each store syncs to an S3 bucket you own. Conflicts are kept, never merged: a changed-on-two-devices note shows up with both versions side by side.

The core is Rust, shared by the CLI and a [Tauri 2](https://tauri.app) app with a Svelte frontend.

<!-- TODO: screenshot of the app (a note open, with the sidebar and backlinks) -->

**Status:** in development, and **not independently audited** (see [SECURITY.md](SECURITY.md)). The core, sync, command line, and the macOS, Android and Windows apps are built; nothing is signed yet. Windows has an installer (built by CI, see [`docs/windows.md`](docs/windows.md)). For macOS and Android, build from source for now (below); a `.dmg` installer and an Android app that updates itself are planned. What is built and what is left: [docs/roadmap.md](docs/roadmap.md).

## What it does

- **Notes** are plain markdown files in a folder, with `[[wikilinks]]`, backlinks, tags and full-text search.
- **The editor** shows markdown as you write it: headings, task-list checkboxes, inline code, fenced code blocks with syntax highlighting, and images (paste or drop one in and it is saved encrypted). The source of a line shows while the cursor is on it.
- **Sync** is automatic, to your own S3 bucket, and a conflict is kept as two versions to resolve rather than merged.
- **Light, dark or follow the system**, on every device.

<!-- TODO: second screenshot, e.g. the conflict view or the dark theme -->

## What you need

| For | You need |
| --- | --- |
| everything | Rust (the toolchain is pinned in `rust-toolchain.toml` and installed by `rustup` on first build) |
| the app | [bun](https://bun.sh), and the Tauri CLI: `cargo install tauri-cli --version "^2" --locked` |
| the phone | JDK 17, Android SDK + NDK `30.0.14904198`, `adb`, a phone with USB debugging on |
| the icons | `rsvg-convert` (`brew install librsvg`), only to regenerate them |
| syncing | **S3-compatible storage**: a bucket, and an access key with read and write on it (see below) |

**S3-compatible storage.** Flint syncs through any S3-compatible service, with your own bucket and key; nothing goes through a server of mine. One store is one bucket plus a folder (prefix) in it, and several stores can share a bucket. The service must support **conditional writes** (`If-Match` / `If-None-Match` on a PUT): that is how a second device's change is detected instead of silently overwriting. **Cloudflare R2** is what Flint is tested against, and its free tier is plenty for notes. AWS S3 supports conditional writes too, but I have not tried it. To check a service before trusting it with notes, run the contract tests against a scratch bucket with `scripts/s3-tests.sh` ([`docs/s3-testing.md`](docs/s3-testing.md)); they exercise exactly the behaviour sync depends on.

## Run the app on the Mac

```sh
cd app && bun install          # once
cd ../crates/flint-tauri
cargo tauri dev                # starts Vite and the app; frontend edits reload live
```

First launch: **New store** (a name, a folder, your S3 details, a password) or **Join existing** (the same S3 details and the store's password). Notes dropped into the store's folder are encrypted in place. The folder defaults to `~/Flint/<name>`.

To build a real `.app`:

```sh
cd crates/flint-tauri
cargo tauri build --debug --bundles app      # target/debug/bundle/macos/Flint.app (drop --debug for a release build)
open ../../target/debug/bundle/macos/Flint.app
```

It is unsigned, so macOS asks for permission to read the keychain once per run of a freshly built binary (click "Always Allow"). Touch ID unlock is offered after the first password unlock.

## Build and push to the phone

Plug the phone in, allow USB debugging, then from the repo root:

```sh
scripts/android-run.sh            # build the debug APK (arm64), install it, start it
scripts/android-run.sh --logs     # the same, then follow the app's log
```

That script sources `scripts/android-env.sh`, which sets up `JAVA_HOME`, the SDK and the NDK (SQLCipher builds OpenSSL from source and needs the NDK's tools on `PATH`). The first build takes a few minutes. On the phone: **Join existing**, sync, and the store's notes arrive (store folders live in the app's private storage). Details and the Android design notes are in [`docs/app.md`](docs/app.md).

## The command line

The same app service without a window:

```sh
cargo run -p flint-cli -- --help
cargo run -p flint-cli -- create-store Notes --bucket my-bucket --access-key-id <id> --endpoint https://...
```

See [`docs/cli.md`](docs/cli.md). The app and the CLI share the same list of stores and keychain entries.

## Tests

If you have [just](https://github.com/casey/just) installed, `just` lists the common tasks (`just test`, `just e2e`, `just dev`, `just android`, ...) and each one is a thin wrapper over the commands below.

```sh
cargo test --workspace         # everything that needs no network or real keys
cargo clippy --workspace --all-targets
cd app && bun run check && bun run test
```

Opt-in tests (they touch real things, so they are `#[ignore]`d):

- `scripts/s3-tests.sh` runs the remote contract suite against **your real bucket** (keys in a git-ignored `.env`; see [`docs/s3-testing.md`](docs/s3-testing.md)). Run it only when changing the storage layer.
- `cargo test -p flint-registry -- --ignored --test-threads=1` round-trips the macOS login keychain.
- `FLINT_SAMPLE_VAULT=<folder> cargo test -p flint-app-service -- --ignored` syncs a copy of a real vault between two simulated devices, in memory.

The sync engine is also checked by a property-based simulation of three devices with injected failures ([`docs/sync-testing.md`](docs/sync-testing.md)).

## How it fits together

```
app/                     Svelte + CodeMirror frontend
crates/
  flint-tauri            Tauri commands and events; no logic of its own
  flint-cli              headless shell
  flint-app-service      what every shell calls: create/join/unlock/sync/resolve, notes, search
  flint-registry         stores on this device, secrets (keychain / Android Keystore)
  flint-sync, -state     three-way sync planner and executor, baselines and conflicts
  flint-remote           S3 access (object_store), conditional writes
  flint-index            SQLCipher index: links, backlinks, search
  flint-store-fs         the store folder; encryption of files, adoption of dropped-in files
  flint-crypto           Argon2id, key wrapping, XChaCha20-Poly1305 file format
  flint-md-parse, -core  markdown links/tags/frontmatter, shared types
  tauri-plugin-keystore  Android Keystore and fingerprint, in Kotlin
```

Nothing below the shells imports Tauri, so the whole stack, encryption included, runs under `cargo test`.

## Docs

- [`docs/roadmap.md`](docs/roadmap.md): what is built and what is left
- [`SECURITY.md`](SECURITY.md): what Flint protects and what it does not, and how to report a problem
- [`docs/app.md`](docs/app.md): the desktop and Android apps, where things are stored
- [`docs/windows.md`](docs/windows.md): the Windows build and what differs there
- [`docs/cli.md`](docs/cli.md): the command line
- [`docs/s3-testing.md`](docs/s3-testing.md), [`docs/sync-testing.md`](docs/sync-testing.md): how storage and sync are tested
- [`docs/build-notes.md`](docs/build-notes.md): SQLCipher on Android
- [`docs/m0-findings.md`](docs/m0-findings.md): what the early spikes found (biometrics, Argon2 timing, Android)

## Security, in short

Each store has its own password. The password is stretched with Argon2id into a key that unwraps a random data key kept in `.store/key.json`; every file's contents are encrypted with it (XChaCha20-Poly1305, the file's path authenticated), so S3 and a stolen device hold only ciphertext. File names and sizes are visible. Folders starting with `.` (such as `.git`) are never synced or encrypted. There is no password recovery.

## License

[MIT](LICENSE).
