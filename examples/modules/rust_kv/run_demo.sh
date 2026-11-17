#!/usr/bin/env bash
# rust_kv capability demo — PLAN_umbrella_production.md U01.
#
# There is no compiled rust_kv module yet, so the capability stages below are
# reported as UNSUPPORTED and the script exits 3. This script has no success
# banner: U22 replaces it with the real pipeline. The regression guard is
# `scripts/test-umbrella --case HAR02`, which rejects any variant that prints
# PASS without performing the operation.
#
# Exit: 0 is unreachable until U22; 1 a checked manifest fact is wrong;
#       3 the capability pipeline is not implemented.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNCOVERED=0

say() { printf '%s\n' "$*"; }

say "============================================================"
say " rust_kv demo — capability pipeline status"
say "============================================================"

say ">> manifest (checked, not executed)"
for want in 'name = "rust_kv"' 'archetype = "rust_capstd"' 'entry = "run"' \
            'fs = ["data/"]'; do
    if ! grep -Fqx "$want" "$HERE/module.toml"; then
        say "FAIL: module.toml does not declare: $want"
        exit 1
    fi
    say "   declared: $want"
done
say "   note: parsing the manifest is not a capability operation."

say ""
say ">> capability execution (fs read, traversal rejection, swap-grant gate)"
uncovered_capability_execution() {
    say "UNSUPPORTED: capability execution is not implemented"
    say "   uncovered path: examples/modules/rust_kv has no Rust cdylib, so no"
    say "   read of data/store.jsonl, no RESOLVE_BENEATH traversal refusal and no"
    say "   hot-swap grant decision was performed."
    say "   U22 supplies the real pipeline; until then this demo cannot pass."
    return 1
}
if ! uncovered_capability_execution; then
    UNCOVERED=$((UNCOVERED + 1))
fi

say ""
say ">> existing real tests (references only — this demo does not run them)"
say "   safestos/cranelift/tests/capstd_tests.rs unit-tests CapFs reads and the"
say "   RESOLVE_BENEATH traversal refusal with the capstd feature enabled:"
say "   cargo test --features test-stubs,capstd --test capstd_tests"
say "   The manifest swap-grant set comparison is unit-tested too, but it does"
say "   not enforce IO, so it does not stand in for demo execution."

say ""
say "============================================================"
say " rust_kv: DEMO NOT IMPLEMENTED — ${UNCOVERED} uncovered stage (capability execution)"
say "============================================================"
exit 3
