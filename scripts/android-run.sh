#!/usr/bin/env bash
# Builds the phone app, installs it on the connected device, and starts it.
#   scripts/android-run.sh            release build (what you would use every day)
#   scripts/android-run.sh --debug    debug build (a few hundred MB: debug symbols, but easier to debug)
#   scripts/android-run.sh --logs     also follow the app's log afterwards (can be combined with --debug)
#
# The build and signing are scripts/android-apk.sh (which says which key signs it); this adds the install and start.
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

apk=$(scripts/android-apk.sh $([ "$mode" = debug ] && echo --debug) | tail -n 1)

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
