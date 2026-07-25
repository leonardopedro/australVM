# SafestOS Implementation Status

**Date**: 2026-07-19  
**Status**: CPS JIT pipeline functional — `make bridge` builds the full stack (Rust bridge
→ OCaml compiler binary) in one command. Unfer path dep kept; build.rs guard verifies
sibling repo exists.

---

## Current State

### What Works
- Rust Cranelift bridge compiles (`cargo build --release`)
- `make bridge` builds bridge + OCaml compiler in one command
- `compile_to_function_named()` FFI works end-to-end
- CPS binary parsing with full three-pass compilation
- Opcode 0x08 (If/Select) implemented in Rust via `builder.ins().select()`
- Comparison opcodes (0x10, 0x13-0x19) working with borrow-checker fixes
- Tail call support via `return_call` when 0x07 follows 0x04
- Automatic import/stub detection for external function references
- `test_fib_math` runs and produces results
- Unfer path dep kept (not switched to git dep) for offline operation; `build.rs` guard
  verifies sibling `../unfer` repo's `Cargo.toml` exists at compile time and panics early
  with a clear message if missing (Stage B1)

### What Needs To Be Done
1. **Recompile OCaml** after `CpsGen.ml` 0x08 patch (`dune build`)
2. **Regenerate** `examples/fib/cps_Fib_only.bin` with patched compiler
3. **Verify** `fib(10) = 55` in `test_fib_math`
4. **Remove** debug `eprintln!`/`println!` statements from `cps.rs`
5. **Verify** comparison opcode mapping between OCaml and Rust
6. **Fix noreturn warnings** in `safestos/runtime/scheduler.c` (Stage B5)

---

## Build

```bash
# Full bridge + compiler rebuild (one command)
make bridge

# Rust bridge only
cd safestos/cranelift && cargo build --release
```

---

## Architecture

```
.austral source
    ↓ Austral compiler (OCaml)
    ↓ TailCallAnalysis + CpsGen.ml
CPS binary IR (0x43505331 magic)
    ↓ compile_to_function_named() [FFI]
    ↓ cps::compile_cps_to_clif()
Cranelift IR
    ↓ JITModule
Native code (function pointer)
```

---

## Path dep decision (Stage B1)

The `unfer_ffi` crate is kept as a **path dependency** (`path = "../../../unfer/unfer_ffi"`)
rather than switched to a git dep, because:

- The sibling repos are always deployed together (same top-level directory).
- Offline operation is common; a git dep would break without network.
- A `build.rs` guard verifies the sibling `unfer/Cargo.toml` exists at compile time
  and panics early with a clear message if it's missing or misaligned.
- `make bridge` runs the full build in one command.

The `lib/dune` file no longer contains machine-specific absolute paths; it uses the
`AUSTRAL_BRIDGE_DIR` environment variable (default: `../safestos/cranelift/target/release`).

---

## Cranelift version decision (Stage B9)

**safestos bridge**: cranelift `0.131`
**tidepool-codegen**: cranelift `=0.129.1` (pinned exact)

These diverge. **Decision: accept both JITs in the binary.** Correctness is fine —
they run in separate JIT contexts (safestos `JITModule` for Austral CPS modules,
tidepool `JitEffectMachine` for Haskell effect modules). The cost is binary size
(~2 Cranelift code caches) and two separate JIT initialization paths. Rebasing
one onto the other is not worth the maintenance burden given tidepool pins exact
versions (`=0.129.1`) and the safestos bridge tracks the latest stable.

Tidepool deps are added behind the `tidepool` feature flag (off by default) so
the base bridge binary stays lean.

---

## Egison pattern matching (Stage B9b)

**Path decision: (a) sweet-egison TH quasiquoters** — confirmed working on GHC 9.10.3.

`sweet-egison` (shallow embedding) is used instead of miniEgison (deep embedding):
- TH quasiquoters (`[mc| ... |]`) expand at GHC compile time, before `tidepool-extract`
  serializes Core — the JIT never sees TH.
- ~40x faster than miniEgison on benchmarks (comb2, perm2, CDCL).
- Available from nixpkgs-unstable binary cache (no source builds needed).

**GHC environment**: `nix build` of `haskellPackages.ghcWithPackages` with
`egison`, `egison-pattern-src`, `egison-pattern-src-th-mode`, `sweet-egison`.
Cached at `/nix/store/1ir89h874mwag82kkryrrp52f10sc7y9-ghc-9.10.3-with-packages`.

**Matchers available**: `Something`, `Eql`, `List`, `Multiset`, `Set`, `(m1, m2)` pairs.
**Eager safety**: `matchAll dfs` + `take` for bounded enumeration; `matchAll bfs` for
fair enumeration of infinite targets (twin primes).

**Fock-space module** (`examples/modules/fock_match/`):
- Operator strings as `[(Mode, Create | Annihilate)]`
- Multiset matcher finds contraction pairs for Wick's theorem
- Normal-orders a₁a₁†a₂a₂† → vacuum + a₂†a₂ + a₁†a₁ + a₁†a₂†a₁a₂
- JIT-path tests: twin primes (bfs), poker hands (Multiset), unordered pairs
