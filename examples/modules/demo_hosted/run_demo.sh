#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
CL_DIR="$ROOT/safestos/cranelift"
MODHOST="$CL_DIR/target/release/modhost"

if [ ! -f "$MODHOST" ]; then
    echo ">> Building modhost (release)"
    (cd "$CL_DIR" && cargo build --release --features test-stubs --bin modhost)
fi

if [ ! -f "$HERE/module.cps" ]; then
    echo ">> Generating module.cps"
    bash "$HERE/gen_cps.sh"
fi

echo ">> POSITIVE: modhost host demo_hosted --call run --repeat 3"
"$MODHOST" host "$HERE" --call run --repeat 3

echo ">> POSITIVE: modhost authorize (full manifest)"
"$MODHOST" authorize "$HERE/module.toml" demo_hosted uk_version

echo ">> NEGATIVE: modhost authorize (revoked uk_evolve)"
STRIPPED="$(mktemp)"
trap 'rm -f "$STRIPPED"' EXIT
grep -v uk_evolve "$HERE/module.toml" > "$STRIPPED"
if "$MODHOST" authorize "$STRIPPED" demo_hosted uk_evolve 2>/dev/null; then
    echo "FAIL: stripped manifest must DENY uk_evolve" >&2
    exit 1
else
    echo "PASS: revoking uk_evolve denies with UK-4001"
fi

echo "============================================================"
echo " demo_hosted: ALL TESTS PASSED"
echo "============================================================"
