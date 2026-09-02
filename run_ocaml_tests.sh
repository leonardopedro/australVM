#!/usr/bin/env bash
# Build + run the australVM OCaml test suites inside the project's own nix
# dev shell — the same flow CI runs (`nix-shell --command 'make test'`).
#
# The flake (nixos-unstable) pins an OCaml + rustc pair on one glibc, so the
# cranelift bridge .so and the OCaml test binaries stay link-compatible; the
# Rust-side bridge must exist at safestos/cranelift/target/release (rebuild
# with `make bridge` after touching safestos/cranelift).
#
# Usage:  nix develop -c bash ./run_ocaml_tests.sh
set -e
cd "$(dirname "$0")"

BRIDGE_DIR="$(pwd)/safestos/cranelift/target/release"
if [ ! -f "$BRIDGE_DIR/libaustral_cranelift_bridge.so" ]; then
  echo "bridge .so missing at $BRIDGE_DIR — run: nix develop -c 'make bridge'" >&2
  exit 1
fi

echo "=== build ==="
make 2>&1 | tail -6
echo "=== test ==="
# --force: re-run every suite even when dune considers the results cached
LD_LIBRARY_PATH="$BRIDGE_DIR" AUSTRAL_BRIDGE_DIR="$BRIDGE_DIR" \
  dune runtest --force 2>&1 | grep -E "^Ran:|^OK$|^FAILED|: PASS|: FAIL|Error:" | tail -30