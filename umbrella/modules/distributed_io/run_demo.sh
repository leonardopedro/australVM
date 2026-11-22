#!/usr/bin/env bash
# umbrella/modules/distributed_io/run_demo.sh — D09 step 4.
#
# Runs the demo twice: granted, and with `rs_cap_write` removed from the
# manifest. The first run must write `note.txt` inside the instance state
# directory; the second must be refused with UK-4001 and write nothing.
#
# Explicit paths only: the module directory is this script's own directory, the
# state lands beside it, and the host binary is resolved from the repository.
#
# Trust assumptions: the grant in module.toml is what authorizes the call. The
# capability is Tier 1 coupling (the host issues handles), not isolation — a
# module that calls std::fs directly is outside it, and the VM tier is what
# contains that case.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
MODHOST="${MODHOST:-$ROOT/safestos/cranelift/target/release/modhost}"

if [ ! -x "$MODHOST" ]; then
    echo "run_demo.sh: $MODHOST is not built (run: make bridge)" >&2
    exit 2
fi

if [ ! -f "$HERE/module.cps" ]; then
    bash "$HERE/gen_cps.sh"
fi

echo "--- granted"
rm -rf "$HERE/.state"
"$MODHOST" host "$HERE" --call compute
echo "wrote: $(cat "$HERE/.state/note.txt")"

echo "--- ungranted (rs_cap_write removed)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cp "$HERE/module.cps" "$tmp/module.cps"
sed 's/^kernel = .*/kernel = ["rs_cap_dir_open"]/' "$HERE/module.toml" > "$tmp/module.toml"
if grep '^kernel' "$tmp/module.toml" | grep -q rs_cap_write; then
    echo "run_demo.sh: could not remove the grant" >&2
    exit 1
fi
if "$MODHOST" host "$tmp" --call compute; then
    echo "run_demo.sh: FAIL the ungranted run was not refused" >&2
    exit 1
fi
if [ -e "$tmp/.state/note.txt" ]; then
    echo "run_demo.sh: FAIL the ungranted run wrote a file" >&2
    exit 1
fi
echo "refused, and nothing was written"
