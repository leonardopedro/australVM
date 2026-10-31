[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/leonardopedro/australVM)

<!-- status: verified | tests: 161 dune / 18 suites + 108 safestos/pkg + 129 bridge cargo (excl. umbrella demo) | last_verified: 2026-10-25 -->

## Verification

The `status:` line above is a claim with a command behind it, not decoration:
the commands in the fenced block below are the ones that printed those counts,
with the date they were measured. No script re-reads the line itself — the gates
this repo actually runs are `scripts/verify-invariants` (symbol census, symbol
sync, the `AuthorizationEngine` default and the archetype surface) and
`scripts/check-todo-ledger`; the counts move only when a suite grows, and are
bumped by hand together with the run that measured them.

Everything below runs through the flake devshell — `nix develop -c` supplies
dune/ocaml/`python3`, which a bare NixOS shell does not have:

```sh
nix develop -c bash -c '
export LD_LIBRARY_PATH="$HOME/.local/lib:$LD_LIBRARY_PATH"   # Cranelift bridge .so
export UNFER_LOGOS_BIN=/path/to/unfer/target/debug/logos    # deltanet UNF gate

dune build @install --profile release
dune runtest --force          # 161 tests, 18 suites, all OK
cargo test --manifest-path safestos/pkg/Cargo.toml --workspace   # 108 tests, all OK
cargo test --manifest-path safestos/cranelift/Cargo.toml         # 129 tests, all OK
./run-examples.sh
python3 test-programs/runner.py   # "All tests passed" (311 programs)
'
```

Both environment variables are load-bearing and neither is discoverable from the
failure: without the first, every binary dies with
`libaustral_cranelift_bridge.so: cannot open shared object file`; without the
second, the deltanet gate has no reference reducer to compare against. An empty
`FAILURES.txt` in the repo root means a run failed — it is not checked in.

What "verified" does **not** mean: the unikernel/Mirage packaging under
`unikernel/` needs the optional Mirage toolchain and is not covered by the
counts above.

Every test compiles into the *same* process-global JIT module, so each function
needs a distinct name — `test/JitTest.ml` threads a `fname` through
`compile_named` (`run_0`, `run_add`, `run_fact`, `run_match`, …) for exactly that
reason. Reusing a name with a different signature fails the whole compile
(`DuplicateDefinition` / `IncompatibleSignature`). An earlier note here pointed at
an `AGENTS.md` §S36b that no longer existed; the live statement of the rule is
the comment at `test/JitTest.ml:11`. See also `../unfer/docs/CHECKLIST_EVIDENCE.md`
(S36b) and `../unfer/docs/CHECKLIST_DETAIL_ARCHIVE.md`.

# Austral Policy-Driven VM (SafestOS Extension)

A high-performance, secure runtime for Austral (extended with Tail Call Optimization) based on **Cranelift JIT** and **AWS Cedar**, inspired by the **Theseus OS** architecture.

## 🚀 Current Status: Phase 11 (Policy-Driven OS VM)
The project has successfully integrated a multi-tier security model combining compile-time linear type checks with JIT-time Cedar policy enforcement.

### Key Accomplishments (Phase 11)
- [x] **AWS Cedar Integration**: JIT-time static analysis blocks unauthorized calls.
- [x] **Multi-Tier Capabilities**: Linear tokens for Network, Memory, and Hot-Swapping.
- [x] **SafestOS Runtime Linkage**: Linked C-based scheduler and cell loader into the JIT bridge.
- [x] **Hot-Swappable Cells**: Metadata generation for `CellDescriptor` is operational.

### Since Phase 11
Phase 11 is the last *phased* milestone; newer work ships per-plan, and the
`status:` line at the top is the machine-checked claim (`scripts/check-status`):
- **Valen-inspired group borrows** — parser + typechecker (`PLAN_valen.md` P1, 2026-10-16).
- **`extern rust` interop, tier 1** — kernel-boundary gate with an `rs_*` namespace and `modhost authorize` (`PLAN_valen.md` §4 P2, 2026-10-17).
- **`umbrella/` prototype** — mixed Austral/Rust module bundles; a plain subfolder since 2026-10-18 (`PLAN_valen.md` P3).
- **avm-pkg content-addressed package engine** — `safestos/pkg/` crates (`avm-cas`, `avm-eval`, `avm-virtio`, `avm-vmm`, `avm-cli`), see `PLAN_avm_pkg.md`.
- **SpectrumOS-derived driver-domain direction** — design plan only, in `docs/spectrum_plan.md`.

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

The Rust bridge (`safestos/cranelift/`) pins **1.99.0** via its
`rust-toolchain.toml`, and the nix flake provides the same rustc for
`make bridge` — single stable toolchain shared with the unfer/
dynamic-arctic/velysterm repos (no nightly anywhere). Bump deliberately.
