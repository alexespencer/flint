# Releasing

A release is a git tag. Pushing one builds the Windows installer, the macOS disk image and the Android APK on GitHub Actions (`.github/workflows/build.yml`, free on a public repo), and a final job makes one GitHub release with all three attached and generated notes. The release is **immutable**: its files cannot be added to or replaced afterwards, so a mistake is fixed by releasing the next version, not by editing this one.

## Making a release

1. Set the version in `crates/flint-tauri/tauri.conf.json` (it names the installers; the Android version code is made from it, so it must go up every time) and commit.
2. `git tag v0.1.1 && git push origin v0.1.1`.
3. Watch the run in the Actions tab. The tag must match the version in `tauri.conf.json`, or the build stops at once and says so. The release appears once all three builds and the tests have passed; if one job fails, fix it and re-run the failed jobs.

## The installers

- **Windows** (`Flint_<version>_x64-setup.exe`): not signed, so SmartScreen says "Windows protected your PC": **More info → Run anyway**.
- **macOS** (`Flint_<version>_aarch64.dmg`, Apple Silicon only): signed only *ad hoc* (`signingIdentity` is `-` in `tauri.conf.json`), not with a developer identity or notarised (that needs a paid Apple Developer account). An Apple Silicon Mac refuses a download with no signature at all as "damaged", so the ad-hoc signature is needed; with it, macOS instead says it cannot verify the developer. Drag Flint to Applications, try to open it, then **System Settings → Privacy & Security → Open Anyway**. If macOS still calls it damaged, remove the download flag in a terminal: `xattr -dr com.apple.quarantine /Applications/Flint.app`.
- **Android** (`Flint_<version>_android-arm64.apk`): signed with the project's release key (below), so it installs over the previous release.

## Android: the release key, and updates

An Android phone only installs an update if it is signed with the **same key** as the app already on it. So the release APKs are signed with one key that lives in GitHub secrets, and that key must never change and must not be lost: keep a copy of the keystore somewhere safe (a password manager). Without it, the next version cannot be installed over the old one.

Make the key once:

```sh
keytool -genkeypair -v -keystore flint-release.keystore -alias flint -keyalg RSA -keysize 4096 -validity 36500
base64 -i flint-release.keystore | pbcopy        # macOS: the keystore, as text, on the clipboard
```

Then in the repo's **Settings → Secrets and variables → Actions**, add four secrets:

| Secret | Value |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | the base64 text of the keystore |
| `ANDROID_KEYSTORE_PASSWORD` | the keystore password you chose |
| `ANDROID_KEY_ALIAS` | `flint` |
| `ANDROID_KEY_PASSWORD` | the key's password (the same as the keystore's if you pressed Enter) |

Without `ANDROID_KEYSTORE_BASE64` a tag build stops with an error, rather than publishing an APK signed with a throwaway key. Builds from ordinary pushes are signed with a debug key and are for trying out only.

A phone that has an APK signed with the debug key (every build from `scripts/android-run.sh`) cannot be updated to a release-key APK: Android refuses it. Uninstall once (`adb uninstall dev.flint.app`), install the release APK, and join your stores again from S3 (the app's data on the phone, including saved secrets, goes with the uninstall; nothing on S3 does).

### Automatic updates with Obtainium

[Obtainium](https://obtainium.imranr.dev) installs apps from their GitHub releases and updates them when a new one appears. In it: **Add App**, paste `https://github.com/alexespencer/flint`, and it finds the APK in the latest release. Nothing is built into Flint for this. (Tauri's own updater does not support Android.)

## Not done yet

- **Intel Macs:** the disk image is Apple Silicon only.
- **Desktop self-update:** the Mac and Windows apps do not update themselves; download the next release.
- **Signing:** Windows (a certificate) and macOS (an Apple Developer account) cost money each year, so for now they stay unsigned.
