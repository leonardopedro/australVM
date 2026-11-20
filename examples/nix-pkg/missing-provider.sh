#!/usr/bin/env bash
#
# A plan closure is looked up in the store by the derivation id the plan
# resolved. This control names a dependency (`absent`) that was never built
# and requires `load` to refuse the image, naming the derivation and its id —
# not link a smaller image, and not fall back to reading a directory (there is
# no `--cells` flag any more).
#
# It needs no compiler: the refusal happens during provider resolution, before
# the owner cell is read, so the positional path is deliberately a name that
# does not exist.
#
#   examples/nix-pkg/missing-provider.sh
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
  echo "missing-provider.sh: no $what found (tried: $*)" >&2
  return 1
}

AVM_PKG=${AVM_PKG:-$(pick avm-pkg \
  "$ROOT/safestos/pkg/target/release/avm-pkg" \
  "$ROOT/safestos/pkg/target/debug/avm-pkg")}
[ -x "$AVM_PKG" ] || { echo "missing-provider.sh: not executable: $AVM_PKG" >&2; exit 1; }

WORK=${AVM_NIXPKG_WORK:-$ROOT/target/nix-pkg}/missing-provider
STORE=$WORK/store
POLICY=$ROOT/safestos/pkg/policies/allow.cedar
SEED=1111111111111111111111111111111111111111111111111111111111111111
# A well-formed key: it parses, and the store lookup (which happens first)
# never reaches the signature check because the dependency is missing.
KEY=$SEED

mkdir -p "$WORK"
"$AVM_PKG" --store "$STORE" init >/dev/null

cat > "$WORK/plan.json" <<'EOF'
{ "derivations": [
  { "name": "absent", "version": "1", "builder_symbol": "austral.translate",
    "dependencies": [], "sources": [], "env_vars": [], "totality": "proven" },
  { "name": "app", "version": "1", "builder_symbol": "austral.translate",
    "dependencies": ["absent"], "sources": [], "env_vars": [], "totality": "proven" } ] }
EOF

ID=$("$AVM_PKG" drv-hash "$WORK/plan.json" absent |
  sed -n 's/.*"target":"\([0-9a-f]\{64\}\)".*/\1/p')
[ "${#ID}" -eq 64 ] || {
  echo "missing-provider.sh: drv-hash did not report `absent`'s id" >&2
  exit 1
}

echo "== load a plan whose dependency was never built"
set +e
out=$("$AVM_PKG" --store "$STORE" load "$WORK/never-built.avm" \
  --policy "$POLICY" \
  --plan "$WORK/plan.json" --for app \
  --key "$KEY" 2>&1)
rc=$?
set -e

if [ "$rc" -eq 0 ]; then
  echo "  [fail] a never-built dependency was accepted (exit 0):" >&2
  printf '%s\n' "$out" >&2
  exit 1
fi
if ! printf '%s' "$out" | grep -q 'no cell for derivation `absent`'; then
  echo "  [fail] the refusal does not name the derivation:" >&2
  printf '%s\n' "$out" >&2
  exit 1
fi
if ! printf '%s' "$out" | grep -q "$ID"; then
  echo "  [fail] the refusal does not name the derivation id the plan resolved:" >&2
  printf '%s\n' "$out" >&2
  exit 1
fi
echo "  [ok] load refused derivation \`absent\` ($ID) before reading any owner cell"

echo
echo "store: $STORE   plan: $WORK/plan.json"
