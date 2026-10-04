# The macOS app

Svelte 5 + TypeScript frontend in `app/`, Tauri 2 shell in `crates/flint-tauri`. All logic is in the Rust crates below it.

## Run it (development)

```sh
cd app && bun install          # once
cd ../crates/flint-tauri
cargo tauri dev                # starts Vite and the app; the frontend reloads as you edit
```

## Build and open a real app

```sh
cd crates/flint-tauri
cargo tauri build --bundles app           # release build: target/release/bundle/macos/Flint.app
cargo tauri build --debug --bundles app   # faster: target/debug/bundle/macos/Flint.app
open ../../target/debug/bundle/macos/Flint.app
```

The app is not signed or notarised (no Apple Developer account), so a copy moved to another Mac needs
right-click → Open the first time. Built on this Mac it opens normally.

## Checks

```sh
cd app && bun run check && bun run test   # svelte-check, vitest
cargo test --workspace
```

## Where things are

- Stores list: `~/Library/Application Support/Flint/stores.json`, shared with the CLI (`docs/cli.md`).
- S3 secret key and unlocked data keys: login keychain, service `dev.flint`.
- Store folders: `~/Flint/<name>` by default.

## Android

```sh
scripts/android-run.sh          # build the debug APK (arm64), install it on the connected phone, start it
scripts/android-run.sh --logs   # the same, then follow the app's log
```

`scripts/android-env.sh` (sourced by the script) sets up the JDK 17, the SDK and the NDK, and puts the NDK's tools on
`PATH`; SQLCipher builds OpenSSL from source and needs them (see `docs/build-notes.md`). Needs `adb` and USB debugging.

- Store folders live in the app's private storage, so there is no folder picker; a new store's notes arrive by syncing from S3.
- Saved secrets (the S3 secret key) are encrypted by a hardware-backed Android Keystore key
  (`crates/tauri-plugin-keystore`, Kotlin). A store's unlocked data key is held in memory only, so a store is locked
  again when the app is closed, and the app locks it when it goes to the background.
- Fingerprint unlock: after the first password unlock, the data key is wrapped under a Keystore key that needs a
  fingerprint for every use. Adding or removing a fingerprint invalidates it and the password is asked for again.
- TLS trusts the bundled Mozilla roots (the system verifier needs a Java-side setup); desktop uses the system roots.
- App backup is off (`allowBackup=false`), so `sync.db` and the index never go to Google Drive.
