#!/bin/sh
# Rebuild the Tauri Android spike, install it on the connected phone and launch it.
#
#   ./rebuild.sh            build + install + launch
#   ./rebuild.sh --no-run   build + install only
#
# Needs: node/npm, ANDROID_HOME (SDK + NDK), a JDK, `cargo tauri`, and one device visible to `adb devices`.
set -eu
cd "$(dirname "$0")/spikes/tauri-android"

: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
export ANDROID_HOME
export NDK_HOME="${NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | tail -1)}"
export JAVA_HOME="${JAVA_HOME:-$(/usr/libexec/java_home 2>/dev/null || true)}"
ADB="$ANDROID_HOME/platform-tools/adb"
APP_ID=dev.flint.spike
APK=src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk

DEVICE=$("$ADB" devices | awk 'NR>1 && $2=="device" {print $1}')
case $(printf '%s\n' "$DEVICE" | grep -c .) in
  0) echo "no device: plug in the phone, unlock it, and accept the USB debugging prompt" >&2; exit 1 ;;
  1) ;;
  *) echo "more than one device attached; set ANDROID_SERIAL to pick one:" >&2; printf '%s\n' "$DEVICE" >&2; exit 1 ;;
esac

[ -d node_modules ] || npm install --silent
npm run --silent build   # bundles src-web/ (CodeMirror) into dist/editor.js

cargo tauri android build --debug --apk --target aarch64
"$ADB" -s "$DEVICE" install -r "$APK"

if [ "${1:-}" != "--no-run" ]; then
  "$ADB" -s "$DEVICE" shell am force-stop "$APP_ID"
  "$ADB" -s "$DEVICE" shell am start -n "$APP_ID/.MainActivity"
fi
