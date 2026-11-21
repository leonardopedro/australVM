//! U13 — the host-managed typed handle registry.
//!
//! The table lives in the bridge (`austral_cranelift_bridge::handles`) because a
//! generated `__new`/`__dispose` wrapper is compiled with the bridge as its
//! only dependency and therefore cannot link this crate. This module is the one
//! name host code uses, so `umbrella::handles::HandleRegistry` and the bridge's
//! table are the same type rather than two tables that could disagree.

pub use austral_cranelift_bridge::handles::*;
