# Agent Guidelines: australVM (Module Runtime)

australVM is the **module runtime** for the unfer modular probability kernel.
Austral cells (`.aui`/`.aum`) are linear-typechecked, lowered to CPS binary IR,
and JIT-compiled by the safestos cranelift bridge. Modules call the unfer kernel
in-process via native `uk_*` symbols registered in the JIT.

## Layout that matters

- `lib/` — the OCaml Austral compiler. `Compiler.ml` (`use_cps_jit` path),
  `Compiler_cps.ml` (Mtast → CPS IR), `CamlCompiler_rust_bridge.ml` (FFI to the
  Rust JIT). Build with `dune build lib/ bin/ test/`.
- `safestos/cranelift/` — the Rust JIT bridge (`austral_cranelift_bridge`).
  - `src/lib.rs` `cranelift_init()` registers `au_*` runtime symbols and, under
    feature `unfer-kernel`, all 18 `uk_*` + 5 `uz_*` kernel symbols from
    `unfer_ffi` (data-driven `UNFER_SYMBOLS`/`ZENODO_SYMBOLS` tables).
  - `src/auth.rs` — `AuthorizationEngine` trait, `ManifestAuthEngine` (TOML
    grants), `AllowAll`, and `safestos_load_auth_manifest()`. Cedar is an
    **optional** backend (`--features cedar`, on by default).
  - `src/cps.rs` `check_call_permission()` gates every call: `__`/`au_`/self are
    free; everything else (including `uk_*`) goes through `auth::check`.
  - `src/bin/modhost.rs` — loads a `module.toml` manifest and answers the
    kernel-call authorization question (UK-4001 enforcement point).
- `examples/kernel/UnferKernel.aui/.aum` — typed Austral bindings for `uk_*`.
  Byte buffers are `Address[Nat8]` + `Int64` length; handles are `Int64`.

## Build / feature notes

- `cargo build` (default features) → Cedar engine is the default authorizer.
- `cargo build --no-default-features --features unfer-kernel` → `AllowAll`
  default authorizer (used by `demo_module/run_demo.sh` for the live JIT
  execution demo); the manifest gate is then exercised explicitly via `modhost`.
- `crate-type` includes `rlib` so in-crate bins (`modhost`) link the bridge; the
  OCaml side still links the `cdylib`/`staticlib` at
  `safestos/cranelift/target/release/libaustral_cranelift_bridge.{a,so}` — so
  **rebuild that release artifact** after touching the bridge, or the OCaml
  `austral` binary will use a stale lib (and may miss `uk_*` symbols).
  Since A15 this is enforced, not just advised: `make all` and `make test`
  run `check-bridge-fresh`, which refuses to link when any bridge source
  (`src/*.rs`, `Cargo.toml`, `Cargo.lock`) is newer than the `.so` and tells
  you to run `make bridge`. Check it alone with `make check-bridge-fresh`.
- **Shell discipline (measured 2026-10-21).** Run every `dune` command
  inside `nix develop`, never a bare `nix-shell`. The two environments
  carry different glibcs (devshell 2.42, bare nix-shell 2.37 on this
  host), and dune relinks its test executables and ppx driver binaries
  to whichever environment is active — a `nix-shell … dune runtest`
  poisons `_build` with 2.37-linked exes that then cannot satisfy the
  bridge's `GLIBC_2.39`, and the failure surfaces as a loader error
  far from the command that caused it. Symptom: `version GLIBC_2.39 not
  found` (or a dead `ppx.exe` under `_build/default/.ppx/`). Remedy:
  re-run the command under `nix develop` — dune re-executes the link in
  the current environment and `_build` heals (stale ppx drivers are
  regenerable cache: `rm -rf _build/default/.ppx/<hash>` is safe).
  `../scripts/check-elf-interpreters` (workspace gate) catches the dead-
  loader half of this class across all repos in one scan.
- **When `nix develop` itself will not build (measured 2026-11-13, aarch64).**
  This channel's `alt-ergo` fails inside its own build (`ocamlopt: I/O error:
  as --debug-prefix-map …` — no assembler reachable in that sandbox), and since
  `devShells.default` carries the prover, the whole shell is unbuildable and no
  `dune` command can run at all. `nix develop .#ocaml` is the same nixpkgs and the
  same OCaml/rust/python inputs without why3/alt-ergo/z3; it is enough for
  `make all` and `make test` (measured: 161 tests, 18 suites, 0 failures on a
  clean `_build`) and **not** enough for anything that must *prove* —
  `LiquidWhy3.prove` then takes its structured-skip path. Use `.#ocaml` to build,
  `default` to prove. Address the flake as `git+file://$PWD#ocaml`: a plain path
  makes nix copy the whole worktree (14 GB of `refi/target`) into the store
  before it starts.

## Scripts & gates

Everything under `scripts/`; `python3` is not on `PATH` in a bare NixOS
shell, so the `nix-shell -p python3 --run …` wrapper is part of each command:

- `scripts/verify-invariants` — the repo's invariant gate (symbol census vs
  `SYMBOL_REGISTRY`, cross-repo sync, …). Runs in CI
  (`.github/workflows/build-and-test.yml`); a missing interpreter is reported
  as an explained gap, never as a false failure.
- `scripts/check-links` — relative markdown links resolve
  (CI: `.github/workflows/linkcheck.yml`).
- `scripts/au2mm0` — reflects an Austral kernel's obligations to integer
  polynomials and decides them twice (symbolic expansion + exact sampling),
  refusing `div`/`float` as a gap. `emit` writes
  `mm0/<stem>_certificate.{mm0,mm1}` over `docs/kernel_alg.mm0` and re-checks it
  with `mm0-rs` + `mm0-c` when they are installed; a false obligation is refused
  before any file is written.
- `scripts/mm0_ring.py` — the W2 prover behind `emit`: a proof-producing
  normalizer whose steps are ring-law applications from `docs/kernel_alg.mm0`
  and whose coefficient arithmetic is grounded as visible `cf_` facts. Its
  trust split is pinned in `BUDGETS` in `scripts/au2mm0`.
- `mm0/flux1d_certificate.{mm0,mm1}` — the first symbolic certificate (3
  obligations, `-- trust-split: structural=5 grounded=15 certificates=3`).
  Regenerated byte-identically by `verify-invariants`; its statements await
  approval (`mm0-comparator`) and are reported as a `[gap]` until then.
- `scripts/mm0-comparator` — approved MM0 statements, held out (approve /
  check / selftest). A generated statement is compared against
  `mm0/approved/<artifact>.stmts` plus a dependency-closure and theory hash;
  an artifact without an approval is reported as a `[gap]` by
  `verify-invariants`, never a silent pass.
- `scripts/check-todo-ledger` — code files (`*.rs`/`*.sh`/`*.py`/`*.toml`)
  carry no `TODO`/`FIXME`/`HACK`/`XXX` marker unless the file is reviewed in
  `docs/TODO_LEDGER.md`; a ledger entry whose file no longer carries a
  marker is reported `[stale]` so it leaves with the work. Markdown
  historical status prose is out of scan scope by policy.
- `scripts/cps-fingerprint <cps>` — the bridge's own sha256 of the bytes
  (`austral_cps_fingerprint`); `--object <cps>` gives the digest of the
  cranelift-emitted code capture plus `arch=`. Not JIT symbols: no manifest
  grant applies. The object digest is host-native and defined for the first
  compilation in a fresh process, so `verify-invariants` only compares it on a
  matching arch and reports `[gap]` otherwise.
- `scripts/isa-census <module.cps>` — diffs the JIT decoder's own opcode count
  (`crate::isa::note`, `ISA_REGISTRY` in `safestos/cranelift/src/isa.rs`), the
  `-- op 0xNN` table in `mm0/isa.mm0`, and what the module actually read; exits
  1 on any uncovered or overstated opcode. `--corpus` compiles the example
  modules whose CPS path produces a function and censuses the union (one module
  only measures one kernel). `scripts/isa-hypotheses <isa.mm0>` refuses a
  `-- gap-hypothesis:` whose `-- cites:` sha256 pin is stale or missing.
  `ISA_REGISTRY` is the instruction-side twin of `SYMBOL_REGISTRY`.
- `scripts/isa-encdec [--write]` — regenerates the `encdec` table and its 23
  proofs plus the three `step_refines` proofs in `mm0/isa.mm0`, and derives
  `mm0/isa.mm1` (the MM0 proof file) from it. MM0 splits the two:
  `mm0-rs compile mm0/isa.mm1 <mmb>` (output is positional, not `-o`) then
  `mm0-c <mmb> < mm0/isa.mm0`. Editing `isa.mm0` by hand requires `--write`
  afterwards, or the gate reports the proof file stale. A `theorem` in the
  `.mm0` with no entry in the script's proof table is a hard error, never a
  silent `sorry`.
  - MM0 has no lambda in proof terms and proof-position arguments are
    *hypotheses*, not terms: `(req_trans run_add_def (req_symm ir_add_sem))`
    works, `(req_trans (run_add_def s dst a b) ...)` fails with "too many
    arguments" — term arguments are inferred from the goal.
  - A `.mm0` statement's premises (`$ A $ > $ B $`) become named binder
    hypotheses designed by the script (`h1`, `h2`, …) in the `.mm1`, which is
    how a proof stays a plain term instead of needing a lambda.
- `lib/Cps_codec.ml` is the OCaml opcode table; `CpsGen` writes every opcode
  through it, and `test/CpsCodecTest.ml` holds it to `mm0/isa.mm0`.
- `examples/kernel/TestKernel.aum` is the smallest module that calls the kernel
  (`uk_version`); `isa-census --corpus` includes it. The corpus compiles with
  `--target-type=c` (no host linker, so `uk_*` need not exist in the `.so`) and
  passes `--allow-all` to both the compiler and the census — the census measures
  bytes in a capture module and never executes them, so the grant question stays
  with `modhost`.
- `scripts/obj-drift-control` — the W5 negative control: a semantics-preserving
  emitter edit moves `code.obj`, the gate refuses the certificate, and the tree
  is restored. It rebuilds the compiler twice, so it is not in
  `verify-invariants`; run it under `nix develop` (needs `dune`).
- Entry points outside `scripts/`: the `Makefile` (`make all` / `make test` /
  `make bridge`, all guarded by `check-bridge-fresh`), `run-examples.sh`,
  `test-programs/runner.py`, `demo_module/run_demo.sh`.

```bash
nix-shell -p python3 --run "bash scripts/verify-invariants"
nix-shell -p python3 --run "python3 scripts/check-links"
```

## Gotchas

- Cross-module foreign calls resolve to the *interface* decl_id, not the body's
  `External_Name`. `Compiler_cps.ml` keys foreign functions by **both** decl_id
  and source name so imported `uk_*` calls lower to the external symbol
  (e.g. `uk_version`) instead of a qualified Austral name the JIT can't resolve.
- The JIT authorization principal is the **module** name (A10): manifest load
  records `[module] name` as the deployment principal and
  `check_call_permission` attributes each gated `uk_*`/`uz_*` call to it
  (fallback: CPS header's module name, then the caller for legacy binaries).
  `modhost` checks the manifest decision directly, and the live gate itself
  is tested in `cps.rs::a10_live_grant_tests` (see `unfer/docs/MODULES.md` §6).

## Changelog

`CHANGELOG.md` is **generated** from conventional commits (git-cliff,
config in `cliff.toml`, pattern adapted from `emthin/cliff.toml`).
The repo's pre-conventional/vendored history is excluded by design
(`filter_unconventional`), so a large skip count in git-cliff's warning
is expected. Refresh with:

```bash
nix run nixpkgs#git-cliff -- -o CHANGELOG.md
```
