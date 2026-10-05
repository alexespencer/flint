# Windows

Status: **being ported.** The build runs on GitHub Actions. Tauri's Windows installer is made by the NSIS bundler.

## Getting an installer

`.github/workflows/windows.yml` runs on every push and on every tag `v*`, and by hand from the Actions tab:

- the **test** job runs `cargo test --workspace` on Windows;
- the **installer** job builds `Flint_<version>_x64-setup.exe`; download it from the run's **Artifacts** (kept 30 days);
- for a tag (`git tag v0.1.0 && git push --tags`) the installer is also attached to the GitHub release.

The installer is unsigned: Windows SmartScreen says "Windows protected your PC", then **More info → Run anyway**. It
installs the WebView2 runtime if the machine lacks it.

To build on the Windows machine itself: install Rust (rustup), [bun](https://bun.sh), NASM and the Visual Studio C++ build
tools, then

```powershell
cd app; bun install; cd ..\crates\flint-tauri
bunx @tauri-apps/cli@2 build --bundles nsis       # target\release\bundle\nsis\
bunx @tauri-apps/cli@2 dev                         # run it with live reload
```

## What differs from macOS and Android

- **Paths.** A store path always uses `/` (it is the S3 key); a backslash in a name is refused, since on Windows it
  would be a separator. On macOS, Linux and Android a file with a backslash in its name is left out of the listing
  (not synced), rather than making the folder unreadable.
- **File names.** Windows refuses names macOS, Linux and Android allow (`: ? * " < > |`, a trailing dot or space,
  `CON`, `NUL`, `COM1`...). `StorePath::windows_problem` finds them; `DiskFs` refuses to touch such a file on Windows
  with an error naming it.
- **Saves.** The folder-flush after an atomic write is skipped (Windows cannot open a folder), and replacing a file that
  another program has open (a virus scanner, a sync client) is retried a few times.

## Still to do

- Saved secrets: only macOS and Android persist them; on Windows the S3 secret key and unlocked keys are kept in
  memory, so every launch asks again. Needs a Credential Manager store behind `SecretStore`.
- Skip-and-warn, instead of failing the whole listing, for notes whose names Windows cannot hold.
- Windows Hello (the Touch ID / fingerprint equivalent).
