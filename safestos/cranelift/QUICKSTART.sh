#!/bin/bash
# Quick Start for Cranelift Bridge
# Session 4 Complete

echo "=== Cranelift Bridge Quick Commands ==="
echo

cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/australVM/safestos/cranelift

echo "1. Check build status"
ls -lh target/release/libaustral_cranelift_bridge.so 2>/dev/null && echo "✓ Built" || echo "✗ Not built"

echo
echo "2. View architecture"
ls -lh src/*.rs

echo
echo "3. Verify symbols (name mangling removed)"
nm -D target/release/libaustral_cranelift_bridge.so 2>/dev/null | grep " T " | head -6

echo
echo "4. Read summary"
head -50 SESSION_4_COMPLETE.md

echo
echo "=== To rebuild ==="
echo "cargo clean && cargo build --release"
echo
echo "=== To document ==="
echo "cat BUILD_SUCCESS.md"
echo
echo "=== Next Action ==="
echo "Implement 0x08 (If) in src/cps.rs OR Create OCaml CpsGen"
