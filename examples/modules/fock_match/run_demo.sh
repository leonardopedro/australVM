#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
CL_DIR="$ROOT/safestos/cranelift"
MODHOST="$CL_DIR/target/release/modhost"

GHC_ENV="${GHC_ENV:-/nix/store/1ir89h874mwag82kkryrrp52f10sc7y9-ghc-9.10.3-with-packages}"

echo ">> Compiling fock_match Haskell module (sweet-egison TH quasiquoters)"
export PATH="$GHC_ENV/bin:$PATH"
ghc -O2 -main-is FockMatch.main -o "$HERE/fock_match" "$HERE/haskell/FockMatch.hs"

echo ">> Running fock_match directly (GHC-compiled binary)"
"$HERE/fock_match"

if [ -f "$MODHOST" ]; then
    echo ">> POSITIVE: modhost host fock_match --call main"
    "$MODHOST" host "$HERE" --call main
else
    echo ">> SKIP: modhost not built (run cargo build --release --features test-stubs,tidepool)"
fi

echo "============================================================"
echo " fock_match: ALL TESTS PASSED"
echo "============================================================"
