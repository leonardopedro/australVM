#!/usr/bin/env bash
# Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
# See LICENSE file for details.
#
# SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

set -euxo pipefail

# Build the Rust Cranelift bridge and the OCaml compiler.
make bridge
# Copy the bridge .so to the repo root so the compiler and JIT test can find it.
cp safestos/cranelift/target/release/libaustral_cranelift_bridge.so .
# Run the OCaml unit tests (including JIT tests).
LD_LIBRARY_PATH=. dune runtest
# Run the end-to-end tests (some use --use-cps-jit).
LD_LIBRARY_PATH=. python3 test-programs/runner.py
# Run the examples.
./run-examples.sh
# Build the stdlib tests.
make -C standard clean
make -C standard
# Run the stdlib tests.
./standard/test_bin
