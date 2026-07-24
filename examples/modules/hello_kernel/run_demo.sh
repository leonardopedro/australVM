#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
CL_DIR="$ROOT/safestos/cranelift"
MODHOST="$CL_DIR/target/release/modhost"

if [ ! -f "$MODHOST" ]; then
    echo ">> Building modhost (release, tidepool feature)"
    (cd "$CL_DIR" && cargo build --release --features "test-stubs,tidepool" --bin modhost)
fi

echo ">> POSITIVE: modhost host hello_kernel --call main"
"$MODHOST" host "$HERE" --call main

echo ">> NEGATIVE: grant removal → UK-4001"
STRIPPED="$(mktemp)"
trap 'rm -f "$STRIPPED"' EXIT
grep -v 'effects' "$HERE/module.toml" > "$STRIPPED"
if "$MODHOST" host "$HERE" --call main 2>/dev/null; then
    echo "FAIL: module without effects grant should fail" >&2
    exit 1
else
    echo "PASS: missing effects grant denied"
fi

echo "============================================================"
echo " hello_kernel: ALL TESTS PASSED"
echo "============================================================"
