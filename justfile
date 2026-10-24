# Common tasks. `just` lists them; `just <name>` runs one. The scripts they wrap are in scripts/.

default:
    @just --list

# --- run and build ---

# Run the desktop app with live reload (needs `bun install` in app/ once)
dev:
    cd crates/flint-tauri && cargo tauri dev

# Build Flint.app (release; `just app --debug` for a faster debug build)
app *flags:
    cd crates/flint-tauri && cargo tauri build {{ flags }} --bundles app

# Run the command line: just cli -- --help
cli *args:
    cargo run -p flint-cli -- {{ args }}

# Build, install and start the phone app on a connected device (`just android --debug`, `just android --logs`)
android *flags:
    scripts/android-run.sh {{ flags }}

# Regenerate every app icon from crates/flint-tauri/icon-src
icons:
    scripts/make-icons.sh

# --- checks ---

# Everything that needs no network or real keys: Rust tests and lints, then the frontend's checks and tests
test: rust frontend

# Rust: tests and clippy
rust:
    cargo test --workspace
    cargo clippy --workspace --all-targets

# Frontend: type check and unit tests
frontend:
    cd app && bun run check && bun run test

# Browser tests of the frontend against a faked Tauri side (first time: `just browsers`)
e2e:
    cd app && bun run e2e

# Download the browser the e2e tests use (once per machine)
browsers:
    cd app && bunx playwright install chromium

# Regenerate the README's pictures in docs/img
screenshots:
    cd app && bun run screenshots

# Unused Rust dependencies
machete:
    cargo machete

# --- against real things (see docs/s3-testing.md) ---

# S3 contract tests against the bucket in .env; pass diagnose, contract, paging or cleanup to run one part
s3 *part:
    scripts/s3-tests.sh {{ part }}

# Round-trip the real login keychain (macOS)
keychain:
    cargo test -p flint-registry -- --ignored --test-threads=1
