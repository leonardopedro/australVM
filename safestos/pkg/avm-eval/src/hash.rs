//! The derivation input-hash calculus (Sprint 2, spec §3.3).
//!
//! `DerivationId = BLAKE3( MAGIC ∥ name ∥ version ∥ builder_symbol ∥
//! sorted dep ids ∥ sorted source store-paths ∥ sorted env )`
//!
//! Two deliberate extensions over the spec's formula, both strictly
//! order- or content-correct:
//!
//! * **sources are hashed too** — the spec's prose lists them as derivation
//!   inputs but its formula omits them; without them a source patch could
//!   not change a derivation's identity, which would break the storage
//!   acceptance criterion ("minor source patch reuses chunks" presumes the
//!   derivation *is* rebuilt, i.e. its id changed).
//! * **dep ids are the resolved Merkle ids** of the dependency
//!   derivations (their output paths, by content addressing), matching the
//!   formula's `DepID_i`.
//!
//! Everything is length-prefixed and sorted; nothing is ever hashed from a
//! hash-map's iteration order (spec §6.4).
//!
//! Deliberately *not* hashed: `dependencies` enter through their resolved
//! ids, and `totality` (the plan.json record) is a property of the
//! compilation invocation, not of the derivation's inputs — recording
//! `proven` must never flip a Merkle id.
//!
//! # Closure digest (freshness guard, §4.2)
//!
//! [`calculate_closure_digest`] hashes the *same information* as
//! [`calculate_derivation_hash`] — every local field plus, recursively,
//! every dependency — but combines dependencies as `(name, closure)`
//! pairs instead of resolved ids. That makes it computable by walking the
//! plan alone. Its contract is one-directional and that is all the memo
//! tier needs:
//!
//! > equal closure digests ⇒ equal derivation ids.
//!
//! (Induction: equal closures give equal local fields and equal
//! `(name, closure)` multisets; by the induction hypothesis the deps' ids
//! are equal, so the id's sorted dependency-id multiset is equal too.)
//! The converse is deliberately not claimed: two structurally different
//! DAGs may map to the same id multiset, and a stale closure must never be
//! *adopted* merely because an id matched.

use crate::graph::Derivation;

/// The 32-byte BLAKE3 derivation identity.
pub type DerivationId = [u8; 32];

const MAGIC: &[u8] = b"AVM_DRV_V1\0";
const CLOSURE_MAGIC: &[u8] = b"AVM_CLOSURE_V1\0";

/// Compute a derivation's identity given its *resolved* dependency ids.
/// The caller resolves; this function only canonicalizes and hashes.
pub fn calculate_derivation_hash(drv: &Derivation, dep_ids: &[DerivationId]) -> DerivationId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(MAGIC);
    for field in [&drv.name, &drv.version, &drv.builder_symbol] {
        hasher.update(&(field.len() as u32).to_le_bytes());
        hasher.update(field.as_bytes());
    }

    let mut sorted_deps = dep_ids.to_vec();
    sorted_deps.sort_unstable();
    hasher.update(&(sorted_deps.len() as u32).to_le_bytes());
    for id in &sorted_deps {
        hasher.update(id);
    }

    let mut sorted_sources = drv.sources.clone();
    sorted_sources.sort();
    hasher.update(&(sorted_sources.len() as u32).to_le_bytes());
    for s in &sorted_sources {
        hasher.update(&(s.len() as u32).to_le_bytes());
        hasher.update(s.as_bytes());
    }

    let mut sorted_env = drv.env_vars.clone();
    sorted_env.sort();
    hasher.update(&(sorted_env.len() as u32).to_le_bytes());
    for (k, v) in &sorted_env {
        hasher.update(&(k.len() as u32).to_le_bytes());
        hasher.update(k.as_bytes());
        hasher.update(&(v.len() as u32).to_le_bytes());
        hasher.update(v.as_bytes());
    }

    *hasher.finalize().as_bytes()
}

/// Hash a derivation's input *closure*: the same local fields as
/// [`calculate_derivation_hash`], with `(dependency name, dependency
/// closure)` pairs in place of resolved dependency ids. The caller passes
/// the pairs in any order; they are sorted here by name, then by digest,
/// so declaration order cannot move the digest.
///
/// Contract: `closure(a) == closure(b)` implies `id(a) == id(b)`. The
/// manifest records this digest so a name-keyed lookup can adopt a stored
/// build without recomputing the subtree's id chain.
pub fn calculate_closure_digest(
    drv: &Derivation,
    dep_closures: &[(String, DerivationId)],
) -> DerivationId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CLOSURE_MAGIC);
    for field in [&drv.name, &drv.version, &drv.builder_symbol] {
        hasher.update(&(field.len() as u32).to_le_bytes());
        hasher.update(field.as_bytes());
    }

    let mut sorted_deps = dep_closures.to_vec();
    sorted_deps.sort();
    hasher.update(&(sorted_deps.len() as u32).to_le_bytes());
    for (name, closure) in &sorted_deps {
        hasher.update(&(name.len() as u32).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(closure);
    }

    let mut sorted_sources = drv.sources.clone();
    sorted_sources.sort();
    hasher.update(&(sorted_sources.len() as u32).to_le_bytes());
    for s in &sorted_sources {
        hasher.update(&(s.len() as u32).to_le_bytes());
        hasher.update(s.as_bytes());
    }

    let mut sorted_env = drv.env_vars.clone();
    sorted_env.sort();
    hasher.update(&(sorted_env.len() as u32).to_le_bytes());
    for (k, v) in &sorted_env {
        hasher.update(&(k.len() as u32).to_le_bytes());
        hasher.update(k.as_bytes());
        hasher.update(&(v.len() as u32).to_le_bytes());
        hasher.update(v.as_bytes());
    }

    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drv(name: &str, deps: Vec<DerivationId>) -> (Derivation, Vec<DerivationId>) {
        let d = Derivation {
            name: name.into(),
            version: "1.0".into(),
            builder_symbol: "stdenv.build".into(),
            dependencies: vec!["ignored-by-hash".into()],
            sources: vec!["aa".into(), "bb".into()],
            env_vars: vec![("K".into(), "V".into())],
            totality: None,
        };
        (d, deps)
    }

    /// Task 2.2 acceptance: reordering independent dependencies must not
    /// change any id.
    #[test]
    fn order_independence_of_dependency_ids() {
        let a: DerivationId = [1u8; 32];
        let b: DerivationId = [2u8; 32];
        let (d, _) = drv("root", vec![]);
        let h1 = calculate_derivation_hash(&d, &[a, b]);
        let h2 = calculate_derivation_hash(&d, &[b, a]);
        assert_eq!(h1, h2, "dependency order must not affect the derivation id");
    }

    #[test]
    fn order_independence_of_sources_and_env() {
        let (mut d, _) = drv("root", vec![]);
        let h1 = calculate_derivation_hash(&d, &[]);
        d.sources.reverse();
        d.env_vars.reverse();
        let h2 = calculate_derivation_hash(&d, &[]);
        assert_eq!(h1, h2, "source/env order must not affect the derivation id");
    }

    #[test]
    fn content_changes_identity() {
        let (d1, _) = drv("root", vec![]);
        let (mut d2, _) = drv("root", vec![]);
        d2.sources[0] = "cc".into();
        assert_ne!(
            calculate_derivation_hash(&d1, &[]),
            calculate_derivation_hash(&d2, &[])
        );
        // Separation between fields: moving a byte across a boundary moves the id.
        let (mut d3, _) = drv("root", vec![]);
        d3.version = "1.0.1".into();
        assert_ne!(
            calculate_derivation_hash(&d1, &[]),
            calculate_derivation_hash(&d3, &[])
        );
    }

    /// The freshness guard's contract: dependency order cannot move the
    /// closure, and equal closures come from equal local fields — which is
    /// what lets the memo adopt a stored id without recomputing it.
    #[test]
    fn closure_is_order_independent_and_tracks_the_same_inputs() {
        let (d, _) = drv("root", vec![]);
        let a: DerivationId = [1u8; 32];
        let b: DerivationId = [2u8; 32];
        let c1 = calculate_closure_digest(&d, &[("x".into(), a), ("y".into(), b)]);
        let c2 = calculate_closure_digest(&d, &[("y".into(), b), ("x".into(), a)]);
        assert_eq!(c1, c2, "dependency order must not affect the closure");

        // The id it guards is order-independent too, so a matched closure
        // is consistent with the id the cold path would compute.
        assert_eq!(
            calculate_derivation_hash(&d, &[a, b]),
            calculate_derivation_hash(&d, &[b, a])
        );

        let (mut d2, _) = drv("root", vec![]);
        d2.sources[0] = "cc".into();
        assert_ne!(
            calculate_closure_digest(&d2, &[("x".into(), a), ("y".into(), b)]),
            c1,
            "a source patch must move the closure"
        );
        assert_ne!(
            calculate_derivation_hash(&d2, &[a, b]),
            calculate_derivation_hash(&d, &[a, b])
        );

        // A dependency's closure is part of the parent's.
        assert_ne!(
            calculate_closure_digest(&d, &[("x".into(), a), ("y".into(), [3u8; 32])]),
            c1
        );
    }

    #[test]
    fn dep_ids_change_identity() {
        let (d, _) = drv("root", vec![]);
        let h1 = calculate_derivation_hash(&d, &[[1u8; 32]]);
        let h2 = calculate_derivation_hash(&d, &[[2u8; 32]]);
        assert_ne!(h1, h2);
    }
}
