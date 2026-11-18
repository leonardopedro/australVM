#!/usr/bin/env bash
#
# Multi-cell images: the owner cell calls a function that lives in a separate
# provider cell, and the loader links the two at load time.
#
# `run.sh` links one cell at a time against runtime stubs. Here `DoubleApp`
# imports `double` from `MathLib`: its cell carries that import and one call
# relocation, so loading it alone must fail with an unresolved symbol. Loading
# it with `--provider mathlib.avm` resolves the call to the provider's real
# image base (owner base + 0x01000000), maps both texts in the executor, and
# returns 84.
#
# The modules are compiled separately on purpose. The CPS emitter writes the
# functions of one module per output, so building the provider and the owner
# from separate `.cps` files is what puts `double` in a different cell; a
# single compile of both would inline/carry it in the owner.
#
# Requirements: avm-pkg + austral + cps2cell (this repo), rustc (executor).
# Paths can be overridden with AVM_PKG, AVM_AUSTRAL, AVM_CPS2CELL,
# AVM_NIXPKG_WORK, AVM_NIXPKG_STORE.
#
#   examples/nix-pkg/multi-cell.sh
#
set -euo pipefail

DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$DIR/../.." && pwd)

pick() { # name, then candidates; print the first that exists
  local what=$1
  shift
  for p in "$@"; do
    if [ -x "$p" ]; then
      printf '%s\n' "$p"
      return 0
    fi
  done
  echo "multi-cell.sh: no $what found (tried: $*)" >&2
  return 1
}

AVM_PKG=${AVM_PKG:-$(pick avm-pkg \
  "$ROOT/safestos/pkg/target/release/avm-pkg" \
  "$ROOT/safestos/pkg/target/debug/avm-pkg")}
AVM_AUSTRAL=${AVM_AUSTRAL:-$(pick austral "$ROOT/austral")}
AVM_CPS2CELL=${AVM_CPS2CELL:-$(pick cps2cell \
  "$ROOT/safestos/cranelift/target/release/cps2cell" \
  "$ROOT/safestos/cranelift/target/debug/cps2cell")}
for tool in "$AVM_PKG" "$AVM_AUSTRAL" "$AVM_CPS2CELL"; do
  [ -x "$tool" ] || { echo "multi-cell.sh: not executable: $tool" >&2; exit 1; }
done

WORK=${AVM_NIXPKG_WORK:-$ROOT/target/nix-pkg}/multi-cell
STORE=${AVM_NIXPKG_STORE:-$WORK/store}
SEED=${AVM_NIXPKG_SIGN_SEED:-1111111111111111111111111111111111111111111111111111111111111111}
POLICY=$ROOT/safestos/pkg/policies/allow.cedar
BASE=0x10000000
PROVIDER_BASE=0x11000000
# The provider's `x * 2` calls the Pervasive helper; the owner cell's import
# of `double` is resolved by the provider, not by the runtime.
RESOLVE_MUL='Austral.Pervasive::trappingMultiply=0x20000010'

export LD_LIBRARY_PATH=$ROOT/safestos/cranelift/target/release${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export AVM_CPS2CELL

mkdir -p "$WORK"

echo "== compile (two modules, two CPS images)"
"$AVM_AUSTRAL" compile "$DIR/MathLib.aui,$DIR/MathLib.aum" \
  --target-type=c --no-entrypoint --output="$WORK/math.c" \
  --use-cps-jit --allow-all --emit-cps="$WORK/math.cps" >/dev/null
"$AVM_AUSTRAL" compile "$DIR/MathLib.aui,$DIR/MathLib.aum" "$DIR/DoubleApp.aum" \
  --target-type=c --no-entrypoint --output="$WORK/app.c" \
  --use-cps-jit --allow-all --emit-cps="$WORK/app.cps" >/dev/null
echo "  [ok] math.cps (provider) and app.cps (owner)"

echo "== derivations (owner depends on the provider)"
{
  printf '{ "derivations": [\n'
  printf '  { "name": "mathlib", "version": "1.0.0", "builder_symbol": "austral.translate",\n'
  printf '    "dependencies": [], "sources": ["%s"], "env_vars": [], "totality": "proven" },\n' \
    "$(sha256sum "$DIR/MathLib.aum" | cut -d' ' -f1)"
  printf '  { "name": "doubleapp", "version": "1.0.0", "builder_symbol": "austral.translate",\n'
  printf '    "dependencies": ["mathlib"], "sources": ["%s"], "env_vars": [], "totality": "proven" }\n' \
    "$(sha256sum "$DIR/DoubleApp.aum" | cut -d' ' -f1)"
  printf '] }\n'
} > "$WORK/plan.json"

echo "== images (avm-pkg build from each CPS image)"
"$AVM_PKG" --store "$STORE" init >/dev/null
"$AVM_PKG" --store "$STORE" build "$WORK/math.cps" \
  --plan "$WORK/plan.json" --for mathlib --sign "$SEED" \
  -o "$WORK/mathlib.avm" >/dev/null
"$AVM_PKG" --store "$STORE" build "$WORK/app.cps" \
  --plan "$WORK/plan.json" --for doubleapp --sign "$SEED" \
  -o "$WORK/app.avm" >/dev/null
echo "  [ok] mathlib.avm (provider) and app.avm (owner)"

rustc -O -o "$WORK/exec-image" "$DIR/exec-image.rs"

echo "== negative control (the owner alone must not link)"
set +e
alone=$("$AVM_PKG" --store "$STORE" load "$WORK/app.avm" \
  --policy "$POLICY" --base "$BASE" --resolve "$RESOLVE_MUL" \
  --exec "$WORK/exec-image" 2>&1)
alone_rc=$?
set -e
if [ "$alone_rc" -eq 0 ] || ! printf '%s' "$alone" | grep -q 'unresolved symbol "double"'; then
  echo "  [fail] owner without its provider did not fail on the import (exit $alone_rc):" >&2
  printf '%s\n' "$alone" >&2
  exit 1
fi
echo "  [ok] without --provider: unresolved symbol \"double\" (exit $alone_rc)"

echo "== execution (owner + provider cell, linked at load)"
report=$("$AVM_PKG" --store "$STORE" load "$WORK/app.avm" \
  --policy "$POLICY" --base "$BASE" \
  --provider "$WORK/mathlib.avm" \
  --resolve "$RESOLVE_MUL" \
  --exec "$WORK/exec-image")
got=$(printf '%s' "$report" | sed -n 's/.*"stdout":"rax=\(-\{0,1\}[0-9]*\) f64_bits=.*/\1/p')
[ -n "$got" ] || { echo "multi-cell.sh: no value in: $report" >&2; exit 1; }
if [ "$got" != 84 ]; then
  echo "  [fail] owner calls provider's double(42) and should get 84, got $got" >&2
  printf '%s\n' "$report" >&2
  exit 1
fi
printf '%s' "$report" | grep -q "\"base\":\"$PROVIDER_BASE\"" ||
  { echo "  [fail] provider was not linked at $PROVIDER_BASE: $report" >&2; exit 1; }
echo "  [ok] double(42) resolved in the provider image (base $PROVIDER_BASE) and returned 84"

echo
echo "store: $STORE   cells: $WORK/{mathlib,app}.avm"
