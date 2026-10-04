[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/leonardopedro/australVM)

<!-- status: verified | tests: 110 dune / 14 suites | last_verified: 2026-10-04 -->

## Verification

The `status:` line above is a claim with a command behind it, not decoration.
`scripts/check-status` (workspace root) re-reads it and fails on drift.

```sh
# from a nix dev shell
export LD_LIBRARY_PATH="$HOME/.local/lib:$LD_LIBRARY_PATH"   # Cranelift bridge .so
export UNFER_LOGOS_BIN=/path/to/unfer/target/debug/logos    # deltanet UNF gate

dune build @install --profile release
dune runtest --force          # 110 tests, 14 suites, all OK
./run-examples.sh
python3 test-programs/runner.py   # "All tests passed"
```

Both environment variables are load-bearing and neither is discoverable from the
failure: without the first, every binary dies with
`libaustral_cranelift_bridge.so: cannot open shared object file`; without the
second, the deltanet gate has no reference reducer to compare against. An empty
`FAILURES.txt` in the repo root means a run failed — it is not checked in.

What "verified" does **not** mean: the unikernel/Mirage packaging under
`unikernel/` needs the optional Mirage toolchain and is not covered by the
counts above, and `AGENTS.md` §S36b records a known JIT-symbol collision on the
Rust path that is separate from this.

# Austral Policy-Driven VM (SafestOS Extension)

A high-performance, secure runtime for Austral (extended with Tail Call Optimization) based on **Cranelift JIT** and **AWS Cedar**, inspired by the **Theseus OS** architecture.

## 🚀 Current Status: Phase 11 (Policy-Driven OS VM)
The project has successfully integrated a multi-tier security model combining compile-time linear type checks with JIT-time Cedar policy enforcement.

### Key Accomplishments (Phase 11)
- [x] **AWS Cedar Integration**: JIT-time static analysis blocks unauthorized calls.
- [x] **Multi-Tier Capabilities**: Linear tokens for Network, Memory, and Hot-Swapping.
- [x] **SafestOS Runtime Linkage**: Linked C-based scheduler and cell loader into the JIT bridge.
- [x] **Hot-Swappable Cells**: Metadata generation for `CellDescriptor` is operational.

### 🏛 Architecture
1. **Frontend (OCaml)**: Compiles Austral to monomorphized CPS IR.
2. **Bridge (C/Rust)**: 
    *   **Cedar Engine (Rust)**: Manages `PolicySet` and `Entities`.
    *   **Cranelift JIT (Rust)**: Translates CPS to machine code, performing Cedar checks on every `App` (Application) node.
3. **Runtime (C)**: Provides the lock-free scheduler, cell loader, and memory management (derived from SafestOS).

## 🛠 Features
- **Static Policy Enforcement**: Cedar queries during JIT compilation provide zero-runtime overhead security.
- **Linear Type Safety**: Capabilities are unforgeable tokens that must be consumed to perform privileged operations.
- **Hot-Swapping**: Structural type-safety checks allow replacing modules without restarting the VM.
- **Cranelift Optimized**: Native machine code generation for x86_64.

## 📂 Directory Structure
- `lib/`: OCaml compiler frontend and CPS generator.
- `safestos/cranelift/`: Rust bridge containing Cedar and Cranelift logic.
- `safestos/runtime/`: C runtime providing the VM execution environment.
- `test_programs/`: Austral examples for capabilities and hot-swapping.

## 🦀 Rust toolchain

The Rust bridge (`safestos/cranelift/`) pins **1.97.1** via its
`rust-toolchain.toml`, and the nix flake provides the same rustc for
`make bridge` — single stable toolchain shared with the unfer/
dynamic-arctic/velysterm repos (no nightly anywhere). Bump deliberately.
