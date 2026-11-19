#!/usr/bin/env bash
#
# Multi-cell images: the owner cell calls a function that lives in a separate
# provider cell, and the loader links the two at load time.
#
# `run.sh` links one cell at a time against runtime stubs. Here `DoubleApp`
# imports `double` from `MathLib`: its cell carries that import and one call
# relocation, so loading it alone must fail with an unresolved symbol. Loading
# resolving the owner's plan dependency (`--plan … --for doubleapp`) reads
# `mathlib`'s cell from the store by the derivation id the plan resolved,
# verifies it against the builder key from the build report, and the call
# resolves to the provider's real image base (owner
# base + 0x01000000); both texts are mapped in the executor and it returns 84.
#
# A provider without `--key`, and one signed by another builder, are refused.
#
# The second half compiles a mutually-referencing pair: the owner's `tick` is
# called back from the provider's `compute`, so loading either cell alone
# misses a symbol and only the linked image returns 53.
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
# The build report names the public half of the signing key. `load` verifies
# provider cells against it, so a provider cannot be swapped by layout alone.
math_out=$("$AVM_PKG" --store "$STORE" build "$WORK/math.cps" \
  --plan "$WORK/plan.json" --for mathlib --sign "$SEED" \
  -o "$WORK/mathlib.avm")
KEY=$(printf '%s' "$math_out" | sed -n 's/.*"pubkey":"\([0-9a-f]\{64\}\)".*/\1/p')
MATH_DRV=$(printf '%s' "$math_out" | sed -n 's/.*"drv_hash":"\([0-9a-f]\{64\}\)".*/\1/p')
[ "${#KEY}" -eq 64 ] && [ "${#MATH_DRV}" -eq 64 ] || {
  echo "multi-cell.sh: build did not report a pubkey/drv_hash: $math_out" >&2
  exit 1
}
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

# The provider set is the plan closure: `--for doubleapp` names `mathlib`,
# and its cell is looked up in the store by derivation id.
echo "== execution (owner + provider cell, resolved from the plan)"
report=$("$AVM_PKG" --store "$STORE" load "$WORK/app.avm" \
  --policy "$POLICY" --base "$BASE" \
  --plan "$WORK/plan.json" --for doubleapp \
  --key "$KEY" \
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
# The provider report is the evidence, so pin every field it promises rather
# than only the return value: origin, the derivation id the lookup used, the
# declared imports, the gated calls, and the address each call resolved to.
for want in \
  '"source":"store"' \
  '"path":"store:mathlib"' \
  '"derivation":"'"$MATH_DRV"'"' \
  '"imports":["Austral.Pervasive::trappingMultiply"]' \
  '"calls":["Austral.Pervasive::trappingMultiply"]' \
  '"resolved":[{"symbol":"Austral.Pervasive::trappingMultiply","address":"0x20000010"}]'; do
  printf '%s' "$report" | grep -qF -- "$want" || {
    echo "  [fail] provider report is missing $want: $report" >&2
    exit 1
  }
done
echo "  [ok] double(42) resolved in the provider image (base $PROVIDER_BASE) and returned 84"
echo "        [ok] report names the store-derived provider, its imports, calls and resolved address"

echo "== negative control (a provider without a key is refused)"
set +e
no_key=$("$AVM_PKG" --store "$STORE" load "$WORK/app.avm" \
  --policy "$POLICY" --base "$BASE" \
  --plan "$WORK/plan.json" --for doubleapp \
  --resolve "$RESOLVE_MUL" 2>&1)
no_key_rc=$?
set -e
if [ "$no_key_rc" -eq 0 ] || ! printf '%s' "$no_key" | grep -q -- '--key'; then
  echo "  [fail] a provider was accepted with no key (exit $no_key_rc):" >&2
  printf '%s\n' "$no_key" >&2
  exit 1
fi
echo "  [ok] provider without --key refused (exit $no_key_rc)"

echo "== negative control (a provider signed by another builder is refused)"
# A separate store holding the same derivation signed by a different builder:
# the lookup by id finds it, and the signature check refuses it.
OTHER=$WORK/other/store
"$AVM_PKG" --store "$OTHER" init >/dev/null
"$AVM_PKG" --store "$OTHER" build "$WORK/math.cps" \
  --plan "$WORK/plan.json" --for mathlib \
  --sign 2222222222222222222222222222222222222222222222222222222222222222 \
  >/dev/null
set +e
wrong_key=$("$AVM_PKG" --store "$OTHER" load "$WORK/app.avm" \
  --policy "$POLICY" --base "$BASE" \
  --plan "$WORK/plan.json" --for doubleapp \
  --key "$KEY" \
  --resolve "$RESOLVE_MUL" 2>&1)
wrong_key_rc=$?
set -e
if [ "$wrong_key_rc" -eq 0 ] || ! printf '%s' "$wrong_key" | grep -q 'signature'; then
  echo "  [fail] a provider signed by another builder was accepted (exit $wrong_key_rc):" >&2
  printf '%s\n' "$wrong_key" >&2
  exit 1
fi
echo "  [ok] provider signed by another builder refused (exit $wrong_key_rc)"

# ── callback: the provider calls back into the owner ─────────────────────
#
# Owner `Callback` defines `tick` and `main`, and calls `compute`; provider
# `Wave` defines `compute` and calls `tick`. They reference each other, so
# each cell's text is relocated against the other's symbols at load.
#
# Module-level imports have to be acyclic, so each side declares its callee as
# a foreign symbol (`@@ Foreign_Import`) with the symbol name the other cell
# exports, and each cell is compiled on its own. C code generation binds a
# symbol the JIT cannot resolve to a trap thunk (`set_link_time_imports`): the
# import is emitted in the CPS and resolved by the loader, and nothing can run
# through the unresolved address. The script checks that no unresolved-symbol
# diagnostic is logged.
CB=$WORK/callback
mkdir -p "$CB"
echo "== callback: compiling the mutually-referencing pair"
"$AVM_AUSTRAL" compile "$DIR/Callback.aui,$DIR/Callback.aum" \
  --target-type=c --no-entrypoint --output="$CB/owner.c" \
  --use-cps-jit --allow-all --emit-cps="$CB/owner.cps" >/dev/null 2>"$CB/owner.err"
"$AVM_AUSTRAL" compile "$DIR/Wave.aui,$DIR/Wave.aum" \
  --target-type=c --no-entrypoint --output="$CB/wave.c" \
  --use-cps-jit --allow-all --emit-cps="$CB/wave.cps" >/dev/null 2>"$CB/wave.err"
for f in "$CB/owner.cps" "$CB/wave.cps"; do
  [ -s "$f" ] || { echo "  [fail] no CPS emitted at $f" >&2; exit 1; }
done
if grep -q "can't resolve symbol" "$CB/owner.err" "$CB/wave.err"; then
  echo "  [fail] the compiler still logs an unresolved-symbol diagnostic:" >&2
  grep -h "can't resolve symbol" "$CB/owner.err" "$CB/wave.err" >&2
  exit 1
fi
printf '  [ok] owner.cps (imports compute) and wave.cps (imports tick)\n'
printf '        [ok] cross-cell imports bound at compile time, no diagnostic logged\n'

{
  printf '{ "derivations": [\n'
  printf '  { "name": "wave", "version": "1.0.0", "builder_symbol": "austral.translate",\n'
  printf '    "dependencies": [], "sources": [], "env_vars": [], "totality": "proven" },\n'
  printf '  { "name": "callback", "version": "1.0.0", "builder_symbol": "austral.translate",\n'
  printf '    "dependencies": ["wave"], "sources": [], "env_vars": [], "totality": "proven" }\n'
  printf '] }\n'
} > "$CB/plan.json"

"$AVM_PKG" --store "$STORE" build "$CB/wave.cps" \
  --plan "$CB/plan.json" --for wave --sign "$SEED" \
  -o "$CB/wave.avm" >/dev/null
"$AVM_PKG" --store "$STORE" build "$CB/owner.cps" \
  --plan "$CB/plan.json" --for callback --sign "$SEED" \
  -o "$CB/callback.avm" >/dev/null

echo "== callback negative control (owner alone misses compute)"
set +e
cb_alone=$("$AVM_PKG" --store "$STORE" load "$CB/callback.avm" \
  --policy "$POLICY" --base "$BASE" --key "$KEY" \
  --resolve 'Austral.Pervasive::trappingAdd=0x20000000' 2>&1)
cb_alone_rc=$?
set -e
if [ "$cb_alone_rc" -eq 0 ] || ! printf '%s' "$cb_alone" | grep -q 'unresolved symbol "compute"'; then
  echo "  [fail] owner without the provider did not miss compute (exit $cb_alone_rc):" >&2
  printf '%s\n' "$cb_alone" >&2
  exit 1
fi
echo "  [ok] without the provider: unresolved symbol \"compute\" (exit $cb_alone_rc)"

echo "== callback execution (owner -> provider -> owner)"
cb=$("$AVM_PKG" --store "$STORE" load "$CB/callback.avm" \
  --policy "$POLICY" --base "$BASE" \
  --plan "$CB/plan.json" --for callback --key "$KEY" \
  --resolve 'Austral.Pervasive::trappingAdd=0x20000000' \
  --exec "$WORK/exec-image")
cb_got=$(printf '%s' "$cb" | sed -n 's/.*"stdout":"rax=\(-\{0,1\}[0-9]*\) f64_bits=.*/\1/p')
[ -n "$cb_got" ] || { echo "multi-cell.sh: no value in: $cb" >&2; exit 1; }
if [ "$cb_got" != 53 ]; then
  echo "  [fail] tick(42)=52 in the owner and +1 in the provider should be 53, got $cb_got" >&2
  printf '%s\n' "$cb" >&2
  exit 1
fi
echo "  [ok] provider's compute(42) called owner's tick and returned 53"

echo
echo "store: $STORE   cells: $WORK/{mathlib,app}.avm and $CB/{wave,callback}.avm"
