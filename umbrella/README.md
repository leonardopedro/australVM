# umbrella

Umbrella is australVM's Rust-interop and capability-runtime orchestrator.
All planning for Valen-inspired Rust interop, versioned adapters, capability
enforcement, and production hardening is consolidated in
[`../PLAN_umbrella_production.md`](../PLAN_umbrella_production.md), alongside the
[source review](../docs/UMBRELLA_PRODUCTION_REVIEW.md),
[acceptance matrix](../docs/umbrella/TEST_MATRIX.md), and
[execution record](../docs/umbrella/EXECUTION.md).

> **Paused work:** Bend-language integration
> ([`../PLAN_umbrella_bend.md`](../PLAN_umbrella_bend.md)) is paused and out of
> scope for current Umbrella development.

Historically derived from [`../PLAN_valen.md`](../PLAN_valen.md) (`P3`,
integrated out of its submodule form on 2026-10-18), `umbrella` treats
australVM as (a) a library (`austral_cranelift_bridge`) and (b) a compile-only
front-end subprocess (`austral --use-cps-jit --emit-cps-only --emit-cps-bundle`),
while owning Cargo builds, generated Rust adapters, explicit `RTLD_LOCAL`
provider admission, and host capability injection.

## Current pipeline (2026-11-27, U00–U22)

1. **Strict project preflight (`project.rs`, `policy.rs`):** `--project <json>`
   (`schema: 1`, `deny_unknown_fields`) separates build, load, and call
   authorizations, refuses `untrusted-native` (pending U26), and validates
   capability roots and network policies before spawning any tool.
2. **Exact Cargo artifact build (`cargo.rs`, `trust.rs`):** `cargo metadata --no-deps`
   and `--message-format=json` select the exact `(package_id, target)` cdylib;
   `trusted` pins check `Cargo.lock` sha256 and requested features before Cargo
   runs, and `library_sha256` + `nm` exports before `dlopen`.
3. **Compile-only CPS bundle (`frontend.rs`, `staging.rs`, `cache.rs`):**
   Reads module imports, detects cycles/duplicates, derives leaf-first Kahn
   build order, runs `austral` with `--emit-cps-only --emit-cps-bundle` in a
   private staging directory (no `--allow-all`, no `CPS_JIT_ADVISORY`), and
   verifies `trusted.signatures` and `descriptor_sha256`. A content-addressed
   `--cache-dir` (defaulting to `<project dir>/.umbrella-cache` for `--project`)
   caches front-end, provider, and adapter artifacts.
4. **Explicit provider & adapter linkage (`provider.rs`, `interface.rs`, `adapters.rs`):**
   Admits the provider `RTLD_LOCAL` (`ProviderSet::load`), renders/compiles
   checked wrappers (`abi scalar 1`, `abi buffer 1/2`, `abi handle 1`,
   `Option`/`Either`/`Result` `__checked` + `__free`, and `abi text 1` via
   `--descriptor-document`), links the provider by absolute `DT_NEEDED`, and
   drains/retires older generations on reload.
5. **Instance-scoped JIT & capabilities (`main.rs`):** Binds an immutable
   `InstanceContext` + caller scope, injects rooted `openat`/`O_NOFOLLOW`
   filesystem grants and `NetPolicy` (`rustls`) network grants, compiles the
   CPS bundle with `compile_entry_i64`, and executes the entry.

## Run

```sh
# Granted path: Result should be RESULT -1781872834555808882
cargo run --release -- \
    --manifest module.toml \
    modules/Demo.aum

# Denied target-call path: exits 1; stderr names UK-4001
cargo run --release -- modules/Demo.aum
```

Key flags: `--project <json>` (strict deployment document, mutually exclusive
with `--manifest`), `--manifest <toml>`, `--entry <name>` (default `compute`),
`--austral <bin>` (defaults to `../austral` or `AUSTRAL_BIN`), `--cratedir <dir>`
(default `modules/rscrate`), `--cargo-package <name>`, `--target-dir <dir>`,
`--profile dev|release`, `--features <list>` (repeatable), `--locked`,
`--offline`, `--liquid-required`, `--cache-dir <dir>`, `--emit-trust-record <json>`,
`--descriptor-document <file>` (`--emit-wrappers <dir>` | `--build-wrappers <dir> --provider <lib>`),
`--expect <i64>`, `--json`.

## Known remaining limits (see `../PLAN_umbrella_production.md` §3 & §6)

- **Adapter link bloat (`U10b` / UR15):** Generated adapter crates currently
  link `austral_cranelift_bridge` (~585 MB per generation) until
  `avm-adapter-rt` is extracted.
- **In-process execution (`U23` / UR20):** No supervised worker process or hard
  native execution deadline yet; legacy global manifest/symbol registry state
  remains in-process.
- **Unfinished driver & supervision wiring (`U06b`, `U15`, `U21`):** `cargo_build`
  and `adapters::build_at` still use raw `Command::output()`; `main.rs` does
  not yet inspect `cranelift_last_error()`/`JIT_PANIC` after `execute_function`;
  `TextAdapter` is not yet wired into the module run path; call-grant refusal
  at `main.rs:761` still exits `2` instead of `1`.
- **Capability demo & isolation (`U22`, `U26`):** `../examples/modules/rust_kv/`
  remains stubbed at exit 3 (`HAR02`); `untrusted-native` remains refused until
  U26 escape controls pass.
