#!/usr/bin/env bash
# Builds the phone app, installs it on the connected device, and starts it.
#   scripts/android-run.sh            release build (what you would use every day)
#   scripts/android-run.sh --debug    debug build (a few hundred MB: debug symbols, but easier to debug)
#   scripts/android-run.sh --logs     also follow the app's log afterwards (can be combined with --debug)
#
# The release APK comes out of the build unsigned, and Android will not install an unsigned APK, so it is
# aligned and signed here with the standard debug key (~/.android/debug.keystore). That is only for your own
# phone: it is the same key the debug builds use, so release and debug builds install over each other, and
# keep the store data. A build for anyone else needs a real release key.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/android-env.sh

mode=release logs=no
for arg in "$@"; do
  case "$arg" in
    --debug) mode=debug ;;
    --logs) logs=yes ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

(cd app && bun install --frozen-lockfile >/dev/null)

if [ "$mode" = debug ]; then
  (cd crates/flint-tauri && cargo tauri android build --debug --apk --target aarch64)
  apk=crates/flint-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
else
  (cd crates/flint-tauri && cargo tauri android build --apk --target aarch64)
  out=crates/flint-tauri/gen/android/app/build/outputs/apk/universal/release
  tools="$ANDROID_HOME/build-tools/$(ls "$ANDROID_HOME/build-tools" | sort -V | tail -1)"
  "$tools/zipalign" -p -f 4 "$out/app-universal-release-unsigned.apk" "$out/aligned.apk"
  "$tools/apksigner" sign --ks "$HOME/.android/debug.keystore" --ks-pass pass:android --key-pass pass:android \
    --out "$out/flint-release.apk" "$out/aligned.apk"
  apk="$out/flint-release.apk"
fi

echo "installing $apk ($(du -h "$apk" | cut -f1))"
if ! result=$(adb install -r "$apk" 2>&1); then
  echo "$result" >&2
  if echo "$result" | grep -q "UPDATE_INCOMPATIBLE\|signatures do not match"; then
    echo "The installed app was signed with a different key. Uninstalling it would delete the app's data on the phone" >&2
    echo "(stores are re-joined from S3), so it is not done for you: run  adb uninstall dev.flint.app  if you want that." >&2
  fi
  exit 1
fi
echo "$result"
adb shell am start -n dev.flint.app/.MainActivity

if [ "$logs" = yes ]; then
  adb logcat -c
  adb logcat --pid="$(adb shell pidof dev.flint.app)" -v time
fi
