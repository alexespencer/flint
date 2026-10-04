#!/usr/bin/env bash
# Builds the debug APK for a phone (arm64), installs it on the connected device, and starts it.
#   scripts/android-run.sh            build, install, launch
#   scripts/android-run.sh --logs     the same, then follow the app's log
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/android-env.sh

(cd app && bun install --frozen-lockfile >/dev/null)
(cd crates/flint-tauri && cargo tauri android build --debug --apk --target aarch64)

apk=crates/flint-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb install -r "$apk"
adb shell am start -n dev.flint.app/.MainActivity

if [ "${1:-}" = "--logs" ]; then
  adb logcat -c
  adb logcat --pid="$(adb shell pidof dev.flint.app)" -v time
fi
