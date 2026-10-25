# Windows

Status: **being ported.** The build runs on GitHub Actions. Tauri's Windows installer is made by the NSIS bundler.

## Getting an installer

Tagged releases have it attached: [Releases](https://github.com/alexespencer/flint/releases) (how a release is made: [`releasing.md`](releasing.md)). Every push also builds one, in the `windows` job of `.github/workflows/build.yml`:

- it builds `Flint_<version>_x64-setup.exe` (download it from the run's **Artifacts**, kept 30 days), then runs `cargo test --release --workspace --no-fail-fast` on Windows. The tests run after the installer is uploaded, in the release profile, so they reuse what the installer build compiled (OpenSSL takes the longest); `flint-tauri` is left out of them.

The installer is unsigned: Windows SmartScreen says "Windows protected your PC", then **More info → Run anyway**. It installs the WebView2 runtime if the machine lacks it.

To build on the Windows machine itself: install Rust (rustup), [bun](https://bun.sh), NASM and the Visual Studio C++ build tools, then

```powershell
cd app; bun install; cd ..\crates\flint-tauri
bunx @tauri-apps/cli@2 build --bundles nsis       # target\release\bundle\nsis\
bunx @tauri-apps/cli@2 dev                         # run it with live reload
```

## What differs from macOS and Android

- **Secrets.** The S3 secret key and a store's unlocked data key are kept in the Windows Credential Manager, one credential per secret under the service `dev.flint` (Control Panel → Credential Manager → Windows Credentials), for the current Windows user. They survive restarts and reinstalls, and the app and the command line share them. `cargo test -p flint-registry -- --ignored` round-trips the real Credential Manager.
- **Paths.** A store path always uses `/` (it is the S3 key); a backslash in a name is refused, since on Windows it would be a separator. On macOS, Linux and Android a file with a backslash in its name is left out of the listing (not synced), rather than making the folder unreadable.
- **File names.** Windows refuses names macOS, Linux and Android allow (`: ? * " < > |`, a trailing dot or space, `CON`, `NUL`, `COM1`...). `StorePath::windows_problem` finds them; `DiskFs` refuses to touch such a file on Windows with an error naming it.
- **Saves.** The folder-flush after an atomic write is skipped (Windows cannot open a folder), and replacing a file that another program has open (a virus scanner, a sync client) is retried a few times.

## Still to do

- Skip-and-warn, instead of failing the whole listing, for notes whose names Windows cannot hold.
- Windows Hello (the Touch ID / fingerprint equivalent).
