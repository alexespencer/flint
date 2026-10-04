#!/bin/bash
# Runs flint-remote's opt-in tests against the real bucket described in `.env`.
#
#   scripts/s3-tests.sh              diagnose, then the contract suite, then clean up   (default)
#   scripts/s3-tests.sh diagnose     one line per operation showing what the server does
#   scripts/s3-tests.sh contract     the 17-test contract suite (writes under flint-contract/)
#   scripts/s3-tests.sh paging       just the 1,250-object listing test
#   scripts/s3-tests.sh cleanup      deletes everything under flint-contract/
#
# The secrets are read from `.env` (git-ignored) and are never printed.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f .env ] || { echo "no .env: copy .env.example to .env and fill it in" >&2; exit 1; }
set -a
# shellcheck disable=SC1091
. ./.env
set +a

missing=""
for name in FLINT_S3_ENDPOINT FLINT_S3_BUCKET FLINT_S3_REGION FLINT_S3_ACCESS_KEY FLINT_S3_SECRET_KEY; do
  [ -n "${!name:-}" ] || missing="$missing $name"
done
[ -z "$missing" ] || { echo "empty in .env:$missing" >&2; exit 1; }
echo "bucket: $FLINT_S3_BUCKET at $FLINT_S3_ENDPOINT (region $FLINT_S3_REGION); credentials loaded"

run() { cargo test -q -p flint-remote "$@" -- --ignored --nocapture; }

case "${1:-all}" in
  diagnose) run diagnose_the_backend 2>&1 | grep -E "DIAG|test result|panicked|error" ;;
  contract) run backends::external 2>&1 | grep -E "^test |test result|panicked|assert|listed|uploads failed" ;;
  paging)   run list_pages_through_more_than_one_page_of_objects 2>&1 | tail -25 ;;
  cleanup)  run delete_everything_the_contract_tests_left_behind 2>&1 | grep -E "deleted|test result|panicked" ;;
  all)
    "$0" diagnose
    "$0" contract || true
    "$0" cleanup
    ;;
  *) echo "usage: $0 [diagnose|contract|paging|cleanup|all]" >&2; exit 2 ;;
esac
