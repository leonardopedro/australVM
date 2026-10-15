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

## Scripts & gates

Everything under `scripts/`; `python3` is not on `PATH` in a bare NixOS
shell, so the `nix-shell -p python3 --run …` wrapper is part of each command:

- `scripts/verify-invariants` — the repo's invariant gate (symbol census vs
  `SYMBOL_REGISTRY`, cross-repo sync, …). Runs in CI
  (`.github/workflows/build-and-test.yml`); a missing interpreter is reported
  as an explained gap, never as a false failure.
- `scripts/check-links` — relative markdown links resolve
  (CI: `.github/workflows/linkcheck.yml`).
- `scripts/check-todo-ledger` — code files (`*.rs`/`*.sh`/`*.py`/`*.toml`)
  carry no `TODO`/`FIXME`/`HACK`/`XXX` marker unless the file is reviewed in
  `docs/TODO_LEDGER.md`; a ledger entry whose file no longer carries a
  marker is reported `[stale]` so it leaves with the work. Markdown
  historical status prose is out of scan scope by policy.
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
