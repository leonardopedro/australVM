#!/usr/bin/env bash
# umbrella/modules/distributed_io/gen_cps.sh — D09 step 4.
#
# Compile the demo with the *ordinary* front-end and keep the CPS it emits:
#
#   ./austral compile DistributedIo.aum --entrypoint=DistributedIo:main \
#       --use-cps-jit --target-type=c --emit-cps=module.cps --emit-adapters=... \
#       --output=module.c
#
# `--allow-all` is used here, at compile time, only because this step's output is
# the CPS file: the authorization the demo is about happens later, when `modhost`
# loads `module.cps + module.toml` and compiles it under the instance's grants.
# Emitting the CPS is not a grant to call anything.
#
# Explicit paths only: the input is resolved next to this script, and both
# outputs land in this directory. Nothing here reads a developer-machine path.
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
    "$AUSTRAL" compile "$HERE/DistributedIo.aum" \
    --entrypoint=DistributedIo:main \
    --use-cps-jit --allow-all --target-type=c \
    --emit-cps="$HERE/module.cps" \
    --emit-adapters="$HERE/module.adapters" \
    --output="$tmp/module.c" > "$tmp/compile.log" 2>&1 || {
    echo "gen_cps.sh: the demo did not compile" >&2
    tail -20 "$tmp/compile.log" >&2
    exit 1
}

echo "Generated $HERE/module.cps ($(wc -c < "$HERE/module.cps") bytes)"
