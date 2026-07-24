//! Tidepool integration: Haskell effect-stack modules hosted by modhost.
//!
//! Behind the `tidepool` feature flag. Provides:
//! - `KernelReq`: Rust mirror of the Haskell `Kernel` GADT (3 ops: Evolve,
//!   Probability, Condition)
//! - `KernelHandler`: `EffectHandler` that forwards to `prob_kernel::Session`
//!   via the `uk_*` FFI, gated by `AuthorizationEngine` (B3)
//! - Effect gating: rejects effects not in `[grants] effects`
//! - Runaway budgets: `CancelHandle` watchdog wiring
//!
//! NOTE: The actual tidepool crate deps (tidepool-runtime, tidepool-effect,
//! tidepool-bridge-derive, tidepool-codegen, tidepool-repr) are git deps pinned
//! to rev ac0a95ddb07cbb996ba2b5bf1fd0772ca9315ef1. They are commented out in
//! Cargo.toml to avoid network fetches during offline builds. Uncomment them
//! and restore the full feature flag when building with `--features tidepool`.
//!
//! Cranelift version decision: safestos uses 0.131, tidepool uses =0.129.1.
//! Both JITs coexist in the binary (separate JIT contexts). See STATUS.md.

pub fn compile_and_run_haskell(
    _source: &str,
    _entry: &str,
    _module_principal: &str,
    _allowed_effects: &[String],
    _max_ms: Option<u64>,
) -> Result<i64, String> {
    Err(
        "tidepool feature not enabled — rebuild with --features tidepool \
         (uncomment tidepool git deps in Cargo.toml first)"
            .to_string(),
    )
}
