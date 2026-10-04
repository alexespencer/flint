#!/bin/bash
# Runs flint-remote's contract suite against a real RustFS server, started as a plain binary (no Docker).
#
#   scripts/rustfs-contract.sh
#
# Downloads a pinned RustFS release into .tools/ (git-ignored), checks its SHA-256 against the value
# pinned below, starts it on a random localhost port with throwaway credentials and a temp data
# folder, runs the suite, and stops it again. Nothing is installed system-wide.
#
# Supported: macOS on Apple silicon, Linux on x86_64 and aarch64 (needs curl >= 7.75, unzip, python3).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=1.0.1
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)   ASSET=rustfs-macos-aarch64-v$VERSION.zip;    SHA256=18aac7101c3484b98f93de64a0609aaa8e02aba072ba7927172c4b2eae7d4843 ;;
  Linux-x86_64)   ASSET=rustfs-linux-x86_64-gnu-v$VERSION.zip; SHA256=9742e183cb0dfb0d48fa909797af572fe47b7f827569036baa4156e0d3958ee5 ;;
  Linux-aarch64)  ASSET=rustfs-linux-aarch64-gnu-v$VERSION.zip; SHA256=1dcfb6ae59702b8e90394157e57450ed219515a1cc63525f730d4e120e9be7b9 ;;
  *) echo "no pinned RustFS binary for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

DIR=.tools/rustfs/$VERSION
BIN=$DIR/rustfs
if [ ! -x "$BIN" ]; then
  mkdir -p "$DIR"
  echo "downloading $ASSET (about 100 MB)..."
  curl -fsSL -o "$DIR/$ASSET" "https://github.com/rustfs/rustfs/releases/download/$VERSION/$ASSET"
  echo "$SHA256  $DIR/$ASSET" | shasum -a 256 -c - >/dev/null || { echo "checksum mismatch for $ASSET; deleting it" >&2; rm -f "$DIR/$ASSET"; exit 1; }
  unzip -oq "$DIR/$ASSET" -d "$DIR"
  chmod +x "$BIN"
fi

PORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
ACCESS_KEY=flint-test-key
SECRET_KEY=flint-test-secret-0123456789
BUCKET=flint-test
DATA=$(mktemp -d)
LOG=$DATA/rustfs.log

cleanup() {
  [ -n "${PID:-}" ] && kill "$PID" 2>/dev/null && wait "$PID" 2>/dev/null || true
  rm -rf "$DATA"
}
trap cleanup EXIT

mkdir "$DATA/volume"
"$BIN" server --address "127.0.0.1:$PORT" --access-key "$ACCESS_KEY" --secret-key "$SECRET_KEY" "$DATA/volume" >"$LOG" 2>&1 &
PID=$!

# the bucket has to exist; creating it doubles as the readiness check
for _ in $(seq 1 60); do
  CODE=$(curl -s -o /dev/null -w '%{http_code}' --aws-sigv4 "aws:amz:us-east-1:s3" --user "$ACCESS_KEY:$SECRET_KEY" \
         -X PUT "http://127.0.0.1:$PORT/$BUCKET" 2>/dev/null || true)
  case "$CODE" in 200|409) break ;; esac
  kill -0 "$PID" 2>/dev/null || { echo "RustFS exited early:" >&2; tail -20 "$LOG" >&2; exit 1; }
  sleep 0.5
done
case "${CODE:-}" in 200|409) ;; *) echo "RustFS did not come up (last status ${CODE:-none}):" >&2; tail -20 "$LOG" >&2; exit 1 ;; esac
echo "RustFS $VERSION is up on 127.0.0.1:$PORT"

export FLINT_S3_ENDPOINT="http://127.0.0.1:$PORT" FLINT_S3_BUCKET=$BUCKET FLINT_S3_REGION=us-east-1
export FLINT_S3_ACCESS_KEY=$ACCESS_KEY FLINT_S3_SECRET_KEY=$SECRET_KEY
cargo test -p flint-remote backends::external -- --ignored "$@"
