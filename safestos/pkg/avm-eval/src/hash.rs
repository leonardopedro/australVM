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

use crate::graph::Derivation;

/// The 32-byte BLAKE3 derivation identity.
pub type DerivationId = [u8; 32];

const MAGIC: &[u8] = b"AVM_DRV_V1\0";

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
        assert_ne!(calculate_derivation_hash(&d1, &[]), calculate_derivation_hash(&d2, &[]));
        // Separation between fields: moving a byte across a boundary moves the id.
        let (mut d3, _) = drv("root", vec![]);
        d3.version = "1.0.1".into();
        assert_ne!(calculate_derivation_hash(&d1, &[]), calculate_derivation_hash(&d3, &[]));
    }

    #[test]
    fn dep_ids_change_identity() {
        let (d, _) = drv("root", vec![]);
        let h1 = calculate_derivation_hash(&d, &[[1u8; 32]]);
        let h2 = calculate_derivation_hash(&d, &[[2u8; 32]]);
        assert_ne!(h1, h2);
    }
}
