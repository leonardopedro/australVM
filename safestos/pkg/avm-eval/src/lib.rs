//! `avm-eval` — the pure memoization and DAG evaluation engine
//! (PLAN_avm_pkg.md Sprint 2).
//!
//! Package recipes are pure functions over the total subset; this crate
//! only ever sees their *derivations*: deterministic records hashed into a
//! Merkle DAG. Evaluation is referentially transparent — the same inputs
//! always yield the same derivation IDs — so the only interesting state is
//! the memo/cache layer, which can only ever skip work, never change it.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod graph;
pub mod hash;
pub mod memo;

pub use graph::{Derivation, EvalError, Plan, ResolvedGraph};
pub use hash::{calculate_derivation_hash, DerivationId};
pub use memo::{EvalReport, MemoTable, Presence};
