#!/usr/bin/env bash
# U22 compiled-module capability workflow.
#
# This runs the *Austral module* in this directory through the ordinary path:
# `CapFs.aum` was compiled by the stock front end to `module.cps`, and `modhost`
# loads that CPS under `module.toml`'s grants, installs the instance's
# capability set and invokes `compute` through the JIT.
#
# Measured here:
#   1. the granted read succeeds and the granted write produces exactly the
#      bytes an independent reference program produces;
#   2. the module's attempt to read `../outside/sentinel.txt` is refused by the
#      host, and the outside sentinel is byte-identical afterwards;
#   3. perturbing the input changes the output to the perturbed reference, so a
#      canned result cannot survive;
#   4. removing `rs_cap_write` from the manifest makes the load fail with
#      UK-4001 and creates no output file.
#
# Exit: 0 all four hold, 1 a check failed (the reason is printed), 2 a required
# tool is absent.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
CL_DIR="$ROOT/safestos/cranelift"
MODHOST="$CL_DIR/target/release/modhost"
FEATURES="${AVM_FEATURES:---no-default-features --features cedar,tier1-rs}"
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/cap_fs.XXXXXX")"
KEEP="${KEEP:-0}"
cleanup() {
    if [ "$KEEP" = "1" ]; then
        echo "cap_fs: scratch kept at $SCRATCH"
    else
        rm -rf "$SCRATCH"
    fi
}
trap cleanup EXIT

fail() {
    echo "cap_fs: $*" >&2
    exit 1
}

if [ ! -f "$HERE/module.cps" ]; then
    bash "$HERE/gen_cps.sh" || fail "module.cps is missing and could not be generated"
fi

if [ ! -x "$MODHOST" ]; then
    echo ">> building modhost (release)"
    # shellcheck disable=SC2086
    (cd "$CL_DIR" && cargo build --release --offline $FEATURES -j 2 --bin modhost) \
        || fail "modhost could not be built"
fi

# The reference is compiled on every run, like scripts/toy-witness: no cargo, no
# shared build directory, and a missing rustc is an explained absence.
command -v rustc >/dev/null 2>&1 || fail "rustc is not on PATH (the reference oracle needs it)"
rustc -O "$HERE/reference.rs" -o "$SCRATCH/reference" || fail "the reference oracle did not compile"

stage() {
    local dir="$1"
    mkdir -p "$dir/.state/data" "$dir/.state/outside"
    cp "$HERE/module.cps" "$dir/module.cps"
    cp "$HERE/module.toml" "$dir/module.toml"
    cp "$HERE/outside/sentinel.txt" "$dir/.state/outside/sentinel.txt"
}

# --- 1. the granted path -----------------------------------------------------
GRANTED="$SCRATCH/granted"
stage "$GRANTED"
cp "$HERE/data/records.txt" "$GRANTED/.state/data/records.txt"

out="$("$MODHOST" host "$GRANTED" --call compute 2>"$SCRATCH/granted.log")" || {
    sed -n '1,20p' "$SCRATCH/granted.log" >&2
    fail "the granted run did not exit 0"
}
case "$out" in
    *"compute: 23"*) ;;
    *) fail "the granted run reported '$out', expected 'compute: 23'" ;;
esac

"$SCRATCH/reference" "$GRANTED/.state/data/records.txt" "$SCRATCH/expected.txt" \
    || fail "the reference oracle failed"
cmp -s "$SCRATCH/expected.txt" "$GRANTED/.state/out/result.txt" \
    || fail "the capability output differs from the independent reference"
echo "granted: read data/records.txt, wrote out/result.txt (23 bytes) matching the reference"

cmp -s "$HERE/outside/sentinel.txt" "$GRANTED/.state/outside/sentinel.txt" \
    || fail "the outside sentinel changed across the granted run"
echo "denied traversal: ../outside/sentinel.txt refused while the module ran; sentinel unchanged"

# --- 2. perturbed input ------------------------------------------------------
printf 'alpha=9\nbeta=8\ngamma=7\ndelta=6\n' > "$GRANTED/.state/data/records.txt"
"$SCRATCH/reference" "$GRANTED/.state/data/records.txt" "$SCRATCH/perturbed-expected.txt" \
    || fail "the reference oracle failed on the perturbed input"
rm -f "$GRANTED/.state/out/result.txt"
out="$("$MODHOST" host "$GRANTED" --call compute 2>"$SCRATCH/perturbed.log")" || {
    sed -n '1,20p' "$SCRATCH/perturbed.log" >&2
    fail "the perturbed run did not exit 0"
}
case "$out" in
    *"compute: 31"*) ;;
    *) fail "the perturbed run reported '$out', expected 'compute: 31'" ;;
esac
cmp -s "$SCRATCH/perturbed-expected.txt" "$GRANTED/.state/out/result.txt" \
    || fail "the perturbed output differs from the perturbed reference"
cmp -s "$SCRATCH/expected.txt" "$GRANTED/.state/out/result.txt" \
    && fail "the output did not change when the input did (a canned result survives)"
echo "perturbation: changed input produced the changed reference output (31 bytes)"

# --- 3. denied grant ---------------------------------------------------------
DENIED="$SCRATCH/denied"
stage "$DENIED"
cp "$HERE/data/records.txt" "$DENIED/.state/data/records.txt"
# Remove only the write grant; every other line stays as committed.
grep -v '"rs_cap_write",' "$HERE/module.toml" > "$DENIED/module.toml" \
    || fail "could not build the ungranted manifest"
if [ "$(grep -c 'rs_cap_write' "$DENIED/module.toml")" != "0" ]; then
    fail "the ungranted manifest still names rs_cap_write"
fi
set +e
"$MODHOST" host "$DENIED" --call compute >"$SCRATCH/denied.out" 2>"$SCRATCH/denied.log"
status=$?
set -e
[ "$status" = "1" ] || fail "the ungranted run exited $status, expected 1"
grep -q "UK-4001" "$SCRATCH/denied.log" \
    || fail "the ungranted run did not report UK-4001"
grep -q "rs_cap_write" "$SCRATCH/denied.log" \
    || fail "the ungranted run did not name rs_cap_write"
[ ! -e "$DENIED/.state/out/result.txt" ] \
    || fail "the ungranted run still created result.txt"
echo "denied: without the rs_cap_write grant the load is refused (UK-4001) and no file is created"

echo "cap_fs workflow: granted read/write matched the reference, traversal refused, perturbation tracked, ungranted call denied"
