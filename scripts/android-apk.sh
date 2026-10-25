#!/usr/bin/env bash
# Builds the phone app (arm64) and signs it. The APK's path is the last line printed.
#   scripts/android-apk.sh            release build (what you would use every day)
#   scripts/android-apk.sh --debug    debug build (a few hundred MB: debug symbols, but easier to debug)
#
# The release APK comes out of the build unsigned, and Android will not install an unsigned APK, so it is
# aligned and signed here. The key is, in order of preference:
#   FLINT_KEYSTORE (+ FLINT_KEYSTORE_PASS, FLINT_KEY_ALIAS, FLINT_KEY_PASS)   a real release key; see docs/releasing.md
#   otherwise the standard debug key (~/.android/debug.keystore, made if missing), which is only for your own
#   phone: the same key the debug builds use, so release and debug builds install over each other and keep
#   the store data. An APK for anyone else needs a real release key, and the same one every time, or phones
#   refuse the update.
# TAURI overrides the Tauri CLI command (default: cargo tauri).
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/android-env.sh
tauri=${TAURI:-cargo tauri}

mode=release
[ "${1:-}" = "--debug" ] && mode=debug

(cd app && bun install --frozen-lockfile >/dev/null)

if [ "$mode" = debug ]; then
  (cd crates/flint-tauri && $tauri android build --debug --apk --target aarch64) >&2
  echo crates/flint-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
  exit 0
fi

(cd crates/flint-tauri && $tauri android build --apk --target aarch64) >&2
out=crates/flint-tauri/gen/android/app/build/outputs/apk/universal/release
tools="$ANDROID_HOME/build-tools/$(ls "$ANDROID_HOME/build-tools" | sort -V | tail -1)"

keystore=${FLINT_KEYSTORE:-$HOME/.android/debug.keystore}
pass=${FLINT_KEYSTORE_PASS:-android}
if [ ! -f "$keystore" ]; then
  [ -z "${FLINT_KEYSTORE:-}" ] || { echo "FLINT_KEYSTORE is set but $keystore does not exist" >&2; exit 1; }
  mkdir -p "$(dirname "$keystore")"
  keytool -genkeypair -keystore "$keystore" -storepass android -keypass android -alias androiddebugkey \
    -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US" >&2
fi

"$tools/zipalign" -p -f 4 "$out/app-universal-release-unsigned.apk" "$out/aligned.apk" >&2
alias_args=()
[ -z "${FLINT_KEY_ALIAS:-}" ] || alias_args=(--ks-key-alias "$FLINT_KEY_ALIAS")
"$tools/apksigner" sign --ks "$keystore" --ks-pass "pass:$pass" --key-pass "pass:${FLINT_KEY_PASS:-$pass}" \
  "${alias_args[@]+"${alias_args[@]}"}" --out "$out/flint-release.apk" "$out/aligned.apk" >&2
echo "$out/flint-release.apk"
