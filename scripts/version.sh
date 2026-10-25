#!/usr/bin/env bash
# Prints the app's version (from crates/flint-tauri/tauri.conf.json), which names the installers.
#   scripts/version.sh            prints it
#   scripts/version.sh --tag v0.1.2   fails unless that tag matches it (a release is cut from a tag)
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(grep -m1 '"version"' crates/flint-tauri/tauri.conf.json | sed -E 's/.*"version": *"([^"]+)".*/\1/')
if [ "${1:-}" = "--tag" ]; then
  if [ "${2#v}" != "$version" ]; then
    echo "the tag ${2} does not match the version in tauri.conf.json ($version): bump it, commit, and tag again" >&2
    exit 1
  fi
else
  echo "$version"
fi
