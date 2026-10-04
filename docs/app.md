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
