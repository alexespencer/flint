# Build notes

## SQLCipher (flint-index) on Android

`rusqlite` is built with `bundled-sqlcipher-vendored-openssl`. The plain `bundled-sqlcipher` feature builds on macOS (it uses Apple's CommonCrypto) but fails for Android with `'openssl/crypto.h' file not found`, because SQLCipher needs OpenSSL and Android has none. The `vendored-openssl` variant compiles OpenSSL from source for every target.

Checked 2026-10-04 by cross-compiling `flint-index`: **aarch64, armv7, x86_64 and i686 Android all build** (about 26 s each, from a clean target directory). It is real SQLCipher 4.14.0 on SQLite 3.51.3 with FTS5 and JSON.

What the build needs:

- the Android NDK (tested with r30, `30.0.14904198`),
- `perl` and `make` on the host (both ship with macOS),
- the NDK on `PATH` and `ANDROID_NDK_HOME` set, plus the per-target compiler variables, for example:

```sh
NDK=$HOME/Library/Android/sdk/ndk/30.0.14904198
BIN=$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin
export ANDROID_NDK_HOME=$NDK PATH="$BIN:$PATH"
export CC_aarch64_linux_android=$BIN/aarch64-linux-android30-clang
export AR_aarch64_linux_android=$BIN/llvm-ar
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$BIN/aarch64-linux-android30-clang
cargo build -p flint-index --target aarch64-linux-android
```

`cargo tauri android build` sets most of this up itself, so this only matters when building a crate on its own. Not yet checked: a release build, and the size SQLCipher + OpenSSL add to the APK.
