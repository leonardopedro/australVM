#!/usr/bin/env bash
# examples/modules/cap_fs/gen_cps.sh — regenerate module.cps from CapFs.aum.
#
# The ordinary front-end is the only compiler used; `--allow-all` is a
# *compile-time* flag here because the output of this step is the CPS file.
# The authorization the demo measures happens later, when `modhost` loads
# `module.cps` under `module.toml`'s grants. Emitting CPS is not a grant.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
AUSTRAL="${AUSTRAL:-$ROOT/austral}"
BRIDGE="${BRIDGE:-$ROOT/safestos/cranelift/target/release/libaustral_cranelift_bridge.so}"

if [ ! -x "$AUSTRAL" ]; then
    echo "gen_cps.sh: $AUSTRAL is not built (run: make all)" >&2
    exit 2
fi
if [ ! -f "$BRIDGE" ]; then
    echo "gen_cps.sh: $BRIDGE is not built (run: make bridge)" >&2
    exit 2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

LD_LIBRARY_PATH="$(dirname "$BRIDGE"):$ROOT${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    "$AUSTRAL" compile "$HERE/CapFs.aum" \
    --entrypoint=CapFs:main \
    --use-cps-jit --allow-all --target-type=c \
    --emit-cps="$HERE/module.cps" \
    --emit-adapters="$HERE/module.adapters" \
    --output="$tmp/module.c" > "$tmp/compile.log" 2>&1 || {
    echo "gen_cps.sh: CapFs.aum did not compile" >&2
    tail -20 "$tmp/compile.log" >&2
    exit 1
}

echo "Generated $HERE/module.cps ($(wc -c < "$HERE/module.cps") bytes)"
