# M0 findings

Results of the four spikes in `spikes/`. The spike code is throwaway and is deleted when M0 closes;
this file is what survives. Status as of 2026-10-03.

| Spike | Status |
|---|---|
| 1. Tauri 2 Android hello-world | **Done**: runs on a Galaxy S24-class phone (SM-S921B, Android 16); editor check passed |
| 2. Conditional PUT on S3 emulators | Passes on s3s-fs; parity with a real server done in M2 (`docs/s3-testing.md`) |
| 3. Biometric key storage | macOS **done** (free LAContext gate, no signing); Android **done** (StrongBox, fingerprint per use, cancel path verified); enrolment-change path untested |
| 4. Argon2id timing | **Done**: Mac and phone measured; params chosen |

The M0 gate ("all four work on real hardware") is met: all four run on real hardware (this Mac and an SM-S921B phone). Two
deviations from the original plan are recorded below: RustFS moved to the M2 gate, and macOS uses LAContext + a login-keychain item
instead of a biometric keychain access control.

## 2. Conditional PUT (`spikes/s3-conditional`, `cargo test -p spike-s3-conditional`)

object_store 0.14.2 against s3s-fs 0.17.0, in-process. Confirmed:

- `PutMode::Create` is exclusive: the second create returns `Error::AlreadyExists`, and the first write is intact.
- `PutMode::Update(etag)` with the current ETag succeeds and returns a new ETag.
- `PutMode::Update(stale_etag)` returns `Error::Precondition` (412) and does not overwrite the newer write.
- User metadata (`x-amz-meta-sha256`, via `Attribute::Metadata`) round-trips through GET.
- The ETag from PUT equals the ETag from GET and HEAD.

Findings that change the plan:

1. **s3s-fs does not return ETags in LIST.** Real S3 does. The plan's "Snapshot remote: LIST gives key + ETag"
   holds on AWS but not on the default CI emulator. `remote` needs either a HEAD fallback for entries
   with no ETag, or the contract suite must tolerate it. The test asserts this so we notice if s3s-fs fixes it.
2. **The plan's `S3ConditionalPut::ETagMatch` warning is outdated.** In object_store 0.14 `ETagMatch` is the default.
   Only an explicit `Disabled` gives `NotImplemented`. Set it explicitly anyway so intent is visible.
3. **s3s 0.17 requires rustc 1.96.** The repo pins 1.99.0 in `rust-toolchain.toml` (installed side by side;
   the global default is untouched).
4. object_store 0.14 moved `get`, `head`, `put` etc. to the `ObjectStoreExt` trait; import it.

RustFS parity was done in M2, with the RustFS binary and not Docker, including the race tests (see `docs/s3-testing.md`; RustFS was later retired in favour of the real AWS bucket): s3s-fs checks preconditions non-atomically, so races are tested on RustFS.

## 4. Argon2id timing (`spikes/kdf-timing`)

`cargo run --release -p spike-kdf-timing`. Median of 5 runs, Apple Silicon Mac:

| mem MiB | t | p | ms |
|---|---|---|---|
| 19 | 2 | 1 | 13 |
| 32 | 3 | 1 | 29 |
| 64 | 2 | 1 | 44 |
| 64 | 3 | 1 | 67 |
| 64 | 3 | 4 | 67 |
| 128 | 3 | 1 | 142 |
| 256 | 2 | 1 | 196 |

Same binary on the phone (SM-S921B, arm64, `adb shell`, median of 5):

| mem MiB | t | p | ms |
|---|---|---|---|
| 19 | 2 | 1 | 37 |
| 32 | 3 | 1 | 87 |
| 64 | 2 | 1 | 128 |
| 64 | 3 | 1 | 178 |
| 64 | 3 | 4 | 179 |
| 128 | 3 | 1 | 365 |
| 256 | 2 | 1 | 607 |

The phone is about 2.7x slower than the Mac.

**Chosen parameters: Argon2id, m = 256 MiB, t = 2, p = 1** (about 0.6 s on the phone, 0.2 s on the Mac).
It sits in the middle of the 0.5-1 s target with headroom for thermal throttling and well above OWASP's minimum and
RFC 9106's second recommendation. Memory is preferred over extra passes because memory is what limits GPU cracking.

- `p = 1`: four lanes took the same time as one, so the `argon2` crate runs lanes serially without its `parallel` feature.
- The parameters are stored per store in `key.json`, so they can be raised later without breaking existing stores.
- A store created on the Mac must unlock on the weakest phone, so do not go above 256 MiB without testing a low-end device.
- Only one phone was measured, and thermals were not stressed.

## 3. Biometric key storage (`spikes/keychain`)

**Decision: on macOS the key lives in an ordinary login-keychain item, gated in-app by Touch ID via
`LAContext`. No Apple Developer account, signing identity or entitlement is needed.**

What was tried, in order:

1. Keychain item with a biometric access control (`BIOMETRY_CURRENT_SET`, data-protection keychain) from an
   unsigned binary: **OSStatus -34018** (errSecMissingEntitlement).
2. The same, inside a bundle signed with a free Apple Development certificate plus a Xcode-generated profile
   with the `keychain-access-groups` entitlement: **works**, Touch ID prompt shown. Rejected anyway: the
   free-team profile expires after 7 days and is tied to one Mac, so shipping this means the paid
   Developer Program (US$99/yr).
3. **Chosen:** `set_generic_password` (login keychain, no access control) + `LAContext.evaluatePolicy` with
   `DeviceOwnerAuthentication`, in an ad-hoc-signed `cargo build` binary. Verified by hand on this Mac:
   - Touch ID prompt appears; success then reads the key.
   - "Use Password" fallback works.
   - Cancelling the prompt refuses, and the key is not read.
   - No keychain "allow access" dialog appeared (not re-tested after a rebuild).

Trade-off: the OS does not enforce the biometric check, the app does. Code running as the same user could read the
keychain item directly. For a single-user notes app whose threat is a stolen or powered-off Mac, accepted; FileVault
plus ciphertext on disk carry that case. The `SecretStore` trait keeps this swappable if the app is ever signed.

Crates: `security-framework` 3, `objc2-local-authentication` 0.3, `block2` 0.6. Run: `cargo run -p spike-keychain -- store|unlock|forget`.

### Android half (`spikes/tauri-android/plugins/tauri-plugin-keystore`)

A small Tauri 2 plugin in Kotlin (scaffolded with `cargo tauri plugin new`, package `dev.flint.keystore`; no third-party
plugin needed). Commands: `status`, `wrap`, `unwrap`, `forget`. It creates an AES-256-GCM key in the Android Keystore with
`setUserAuthenticationRequired(true)`, `AUTH_BIOMETRIC_STRONG` with timeout 0 (a fingerprint for **every** use, via
`BiometricPrompt` + `CryptoObject`), `setInvalidatedByBiometricEnrollment(true)`, and StrongBox when available. The store's
DEK would be kept only as `{iv, ciphertext}`; the key never leaves the secure hardware. Costs nothing, so Android keeps
real OS-enforced biometrics. Uses `androidx.biometric` 1.1.0; the Tauri activity is already a `FragmentActivity`.

Verified on the SM-S921B:
- `status`: `insideSecureHardware: true`, **`securityLevel: strongbox`** (the dedicated security chip, not just the TEE).
- `wrap` then `unwrap` with a fingerprint each time: the key round-trips byte for byte.
- A non-enrolled finger does not unlock; the prompt stays open.
- Tapping "Use password" on the sheet returns `auth_error_13` and no key. This is the app's fall-back-to-password path.

Not verified:
- Enrolling a new fingerprint invalidates the key (`key_invalidated`), so the app must fall back to the password and
  re-wrap the DEK afterwards. Untested on the device (skipped by choice); written from the documented Android behaviour.
  Test it before relying on it, in M6.

Design notes for the real app: rejection codes the app should handle are `auth_error_10` (cancelled), `auth_error_13`
("Use password" button), lockout codes 7 and 9, and `key_invalidated`. After `key_invalidated`, call `forget`, ask for the
password, unwrap the DEK with it, and `wrap` again.

## 1. Tauri 2 Android (`spikes/tauri-android`)

A Rust command `storage_probe` writes then reads `probe.md` in `app_data_dir()` and returns the path. The debug APK builds
(`cargo tauri android build --debug --apk --target aarch64`, needs `NDK_HOME` and `JAVA_HOME`; Tauri rejects
version `0.0.0` on Android). Tauri CLI 2.10.1, NDK 30.0.14904198, JDK 17.

Verified on the phone (SM-S921B, Android 16 / API 36): installed with `adb install -r`, pressed the button, the app
reported `ok`. `adb shell run-as dev.flint.spike cat probe.md` shows the file Rust wrote at
`/data/user/0/dev.flint.spike/probe.md`.

Findings:
- **On Android `app_data_dir()` is the app's root directory**, not `files/`. It is app-private (`drwx------`, owned by the app uid).
- **Auto-backup is on by default.** The generated manifest has no `android:allowBackup` attribute, so Android may back
  app data up to Google Drive. Stores are ciphertext, but `sync.db` and other state would go too. Set `allowBackup="false"`
  (or keep stores under `no_backup/`) when the real Android app is configured.
- The debug APK is 136 MB (all targets' debug symbols, universal); irrelevant for the spike.

**Editor performance (the plan's 5,000-line check): passed.** CodeMirror 6 with markdown highlighting and line wrapping,
in the Android WebView on the SM-S921B. A 20,000-line note (4x the target) felt smooth with no lag; worst keystroke-to-next-frame
latency was **49 ms** (measured on-device with a `beforeinput` to double-`requestAnimationFrame` probe, shown in the spike UI).
The 5,000-line and plain-text variants were not separately recorded, as they are strictly lighter. Not tested: very long single
lines, huge pasted blocks, or low-end phones.

`./rebuild.sh` rebuilds the spike (bundles CodeMirror with esbuild from `spikes/tauri-android/src-web/`), installs and launches it.

## To close M0

1. ~~Install the APK, run the KDF binary, and run the 5,000-line editor check on the phone.~~ Done.
2. ~~Run the conditional-PUT tests against RustFS.~~ Deferred to M2: RustFS will run as a plain binary (no Docker,
   so the repo stays portable), and the RustFS parity check becomes part of the M2 gate.
3. ~~macOS biometric key storage.~~ Done: LAContext + plain login keychain (see section 3).
4. ~~Build the Android Keystore spike.~~ Done (see section 3). The new-fingerprint invalidation check is deferred to M6.

**M0 status: closed**, apart from the RustFS parity check, which moved to the M2 gate. Next step is deleting `spikes/` and starting M1.
