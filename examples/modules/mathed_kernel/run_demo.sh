#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
CL_DIR="$ROOT/safestos/cranelift"
MODHOST="$CL_DIR/target/release/modhost"

GHC_ENV="${GHC_ENV:-/nix/store/1ir89h874mwag82kkryrrp52f10sc7y9-ghc-9.10.3-with-packages}"

echo ">> Compiling mathed_kernel Haskell module (fock_match-style)"
export PATH="$GHC_ENV/bin:$PATH"
ghc -O2 -main-is MathedKernel.main -o "$HERE/mathed_kernel" "$HERE/haskell/MathedKernel.hs"

echo ">> POSITIVE: the wire contract answers Jupyter-shaped outputs"
OUT="$("$HERE/mathed_kernel" <<'EOF'
{"module": "mathed_kernel", "language": "mathed", "code": "echo 2 + 2"}
EOF
)"
echo "$OUT"
case "$OUT" in
  *'ran on mathed_kernel: echo 2 + 2'*) echo "PASS: code echoed in a stdout stream output" ;;
  *) echo "FAIL: unexpected module output: $OUT" >&2; exit 1 ;;
esac

if [ -f "$MODHOST" ]; then
    echo ">> POSITIVE: manifest grants uk_model_create to 'mathed_kernel'"
    "$MODHOST" authorize "$HERE/module.toml" mathed_kernel uk_model_create

    echo ">> NEGATIVE: grant removal -> UK-4001 CallDenied"
    STRIPPED="$(mktemp)"
    trap 'rm -f "$STRIPPED"' EXIT
    sed '/uk_model_create/d' "$HERE/module.toml" > "$STRIPPED"
    if "$MODHOST" authorize "$STRIPPED" mathed_kernel uk_model_create >/dev/null 2>&1; then
        echo "FAIL: module without the grant should be denied" >&2
        exit 1
    else
        echo "PASS: missing grant denied (deny-by-default)"
    fi
else
    echo ">> SKIP: modhost not built (run: cargo build --release --features test-stubs,tidepool --bin modhost in $CL_DIR)"
fi

echo "============================================================"
echo " mathed_kernel: ALL TESTS PASSED"
echo "============================================================"
