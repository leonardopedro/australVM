#!/usr/bin/env bash
#
# Translate → verify → package → execute, end to end.
#
# Each pair in this directory is one Nix expression and the AustralVM module
# it was translated to. The script (1) has dnx-verify reduce both sides to
# their canonical DeltaNets and refuse any pair that is not equivalent (one
# pair, `precedence`, is a *declared* mismatch: the manifest requires it to
# differ), (2) turns each correct module into a signed `.avm` cell in a real
# avm-pkg store, under a derivation whose `sources` field is the verified Nix
# net digest, (3) hydrates every cell back out and compares it byte for byte,
# and (4) loads each cell through `avm-pkg load --exec` — the Cedar gate, the
# relocation and the sandboxed executor — and checks the value `main` returns
# against the Nix value. `precedence` is verified but never packaged: it is
# the control that shows the verifier reports a real mismatch.
#
# Requirements: dnx-verify (../dnx), avm-pkg + austral + cps2cell (this
# repo), rustc (for the executor). Paths can be overridden with DNX_VERIFY,
# AVM_PKG, AVM_AUSTRAL, AVM_CPS2CELL, AVM_NIXPKG_WORK, AVM_NIXPKG_STORE.
#
#   examples/nix-pkg/run.sh
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
  echo "run.sh: no $what found (tried: $*)" >&2
  return 1
}

DNX_ROOT=${DNX_ROOT:-$(cd "$ROOT/.." && pwd)/dnx}
DNX_VERIFY=${DNX_VERIFY:-$(pick dnx-verify \
  "$DNX_ROOT/target/release/dnx-verify" \
  "$DNX_ROOT/target/debug/dnx-verify")}
AVM_PKG=${AVM_PKG:-$(pick avm-pkg \
  "$ROOT/safestos/pkg/target/release/avm-pkg" \
  "$ROOT/safestos/pkg/target/debug/avm-pkg")}
AVM_AUSTRAL=${AVM_AUSTRAL:-$(pick austral "$ROOT/austral")}
AVM_CPS2CELL=${AVM_CPS2CELL:-$(pick cps2cell \
  "$ROOT/safestos/cranelift/target/release/cps2cell" \
  "$ROOT/safestos/cranelift/target/debug/cps2cell")}

for tool in "$DNX_VERIFY" "$AVM_PKG" "$AVM_AUSTRAL" "$AVM_CPS2CELL"; do
  [ -x "$tool" ] || { echo "run.sh: not executable: $tool" >&2; exit 1; }
done

WORK=${AVM_NIXPKG_WORK:-$ROOT/target/nix-pkg}
STORE=${AVM_NIXPKG_STORE:-$WORK/store}
SEED=${AVM_NIXPKG_SIGN_SEED:-1111111111111111111111111111111111111111111111111111111111111111}
POLICY=$ROOT/safestos/pkg/policies/allow.cedar

export LD_LIBRARY_PATH=$ROOT/safestos/cranelift/target/release${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export AVM_AUSTRAL AVM_CPS2CELL

mkdir -p "$WORK/cells" "$WORK/hydrated" "$WORK/negative"

# Packages, in build order. Keep in step with pairs.json (which also carries
# `precedence`, the declared mismatch that is never packaged).
names=(arith apply bool float let version sub mul nested not and multilet neg)
nix_file() { case $1 in
  arith) echo arith.nix ;; apply) echo apply.nix ;; bool) echo bool.nix ;;
  float) echo float.nix ;; let) echo let.nix ;; version) echo version.nix ;;
  sub) echo sub.nix ;; mul) echo mul.nix ;; nested) echo nested.nix ;;
  not) echo not.nix ;; and) echo and.nix ;; multilet) echo multilet.nix ;;
  neg) echo neg.nix ;;
esac; }
aum_file() { case $1 in
  arith) echo Arith.aum ;; apply) echo Apply.aum ;; bool) echo BoolCheck.aum ;;
  float) echo FloatSum.aum ;; let) echo LetExpr.aum ;; version) echo Version.aum ;;
  sub) echo Sub.aum ;; mul) echo Mul.aum ;; nested) echo Add2.aum ;;
  not) echo Not.aum ;; and) echo And.aum ;; multilet) echo MultiLet.aum ;;
  neg) echo Neg.aum ;;
esac; }
# The value `main` must return. Int64/Bool are read from `rax=`; Float64 is
# carried as its IEEE bit pattern in the same word under the engine's
# one-word ABI, so the executor also prints `f64_bits=` and float is checked
# there.
expected() { case $1 in
  arith) echo 20 ;; apply) echo 42 ;; bool) echo 1 ;; let) echo 42 ;;
  version) echo 212 ;; sub) echo 6 ;; mul) echo 42 ;; nested) echo 7 ;;
  not) echo 0 ;; and) echo 1 ;; multilet) echo 3 ;; neg) echo -2 ;;
  float) echo 3.75 ;;
esac; }

echo "== equivalence (dnx-verify, shared kernel UNF serializer)"
"$DNX_VERIFY" check --manifest "$DIR/pairs.json"

# Negative control: the verifier must report a real mismatch, not an error.
sed 's/41/42/' "$DIR/Apply.aum" > "$WORK/negative/Apply-42.aum"
set +e
"$DNX_VERIFY" check --nix-file "$DIR/apply.nix" \
  --austral-file "$WORK/negative/Apply-42.aum" >"$WORK/negative/report.txt" 2>&1
neg=$?
set -e
if [ "$neg" -ne 1 ]; then
  echo "  [fail] negative control expected exit 1 (different), got $neg" >&2
  cat "$WORK/negative/report.txt" >&2
  exit 1
fi
echo "  [ok] negative control: 41 -> 42 is reported different (exit 1)"

echo "== derivations (sources = the verified Nix net digest)"
plan=$WORK/plan.json
{
  printf '{ "derivations": [\n'
  first=1
  for name in "${names[@]}"; do
    digest=$("$DNX_VERIFY" nix-hash --file "$DIR/$(nix_file "$name")" --json |
      sed -n 's/.*"unf_hash":"\([0-9a-f]\{64\}\)".*/\1/p')
    [ -n "$digest" ] || { echo "run.sh: no unf_hash for $name" >&2; exit 1; }
    [ "$first" = 1 ] || printf ',\n'
    first=0
    printf '    { "name": "%s", "version": "1.0.0", "builder_symbol": "austral.translate",\n' "$name"
    printf '      "dependencies": [], "sources": ["%s"], "env_vars": [], "totality": "proven" }' "$digest"
  done
  printf '\n  ] }\n'
} > "$plan"

echo "== images (avm-pkg build: Austral source -> signed cell in the store)"
"$AVM_PKG" --store "$STORE" init >/dev/null
for name in "${names[@]}"; do
  aum=$DIR/$(aum_file "$name")
  out=$("$AVM_PKG" --store "$STORE" build "$aum" \
    --plan "$plan" --for "$name" --name "$name" --version 1.0.0 \
    --sign "$SEED" -o "$WORK/cells/$name.avm")
  identity=$(printf '%s' "$out" | sed -n 's/.*"identity":"\([0-9a-f]\{64\}\)".*/\1/p')
  drv=$(printf '%s' "$out" | sed -n 's/.*"drv_hash":"\([0-9a-f]\{64\}\)".*/\1/p')
  bytes=$(printf '%s' "$out" | sed -n 's/.*"cell_bytes":\([0-9]*\).*/\1/p')
  [ -n "$identity" ] && [ -n "$drv" ] || { echo "run.sh: build output unparsed: $out" >&2; exit 1; }
  printf '  %-9s drv=%s  cell=%s (%s B)\n' "$name" "$drv" "$identity" "$bytes"

  "$AVM_PKG" --store "$STORE" get "$identity" -o "$WORK/hydrated/$name.avm" >/dev/null
  cmp -s "$WORK/cells/$name.avm" "$WORK/hydrated/$name.avm" ||
    { echo "run.sh: $name did not round-trip through the store" >&2; exit 1; }
done
echo "  [ok] all ${#names[@]} cells hydrated byte-identically"

echo "== execution (avm-pkg load --exec: gate, relocate, sandbox, call main)"
rustc -O -o "$WORK/exec-image" "$DIR/exec-image.rs"
for name in "${names[@]}"; do
  win=$(expected "$name")
  report=$("$AVM_PKG" --store "$STORE" load "$WORK/cells/$name.avm" \
    --policy "$POLICY" --base 0x10000000 \
    --resolve 'Austral.Pervasive::trappingAdd=0x20000000' \
    --resolve 'Austral.Pervasive::trappingMultiply=0x20000010' \
    --resolve 'au_fadd=0x20000020' --resolve 'au_fsub=0x20000040' \
    --resolve 'au_fmul=0x20000060' --resolve 'au_fdiv=0x20000080' \
    --resolve 'au_f64_add=0x20000020' --resolve 'au_f64_subtract=0x20000040' \
    --resolve 'au_f64_multiply=0x20000060' --resolve 'au_f64_divide=0x20000080' \
    --exec "$WORK/exec-image")
  if [ "$name" = float ]; then
    got=$(printf '%s' "$report" | grep -o 'f64_bits=[0-9.eE+-]*' | cut -d= -f2)
  else
    got=$(printf '%s' "$report" | sed -n 's/.*"stdout":"rax=\(-\{0,1\}[0-9]*\) f64_bits=.*/\1/p')
  fi
  [ -n "$got" ] || { echo "run.sh: $name produced no value in: $report" >&2; exit 1; }
  if [ "$got" != "$win" ]; then
    echo "  [fail] $name executed to $got, Nix value is $win" >&2
    printf '%s\n' "$report" >&2
    exit 1
  fi
  echo "  [ok]   $name executes to $got"
done

echo
echo "store: $STORE   plan: $plan   cells: $WORK/cells"
