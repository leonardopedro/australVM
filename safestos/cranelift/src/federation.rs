//! Federation-aware module hosting (B11).
//!
//! Behind the `federation` feature flag. Provides:
//! - Module identity: DID creation on first load via `unfer_identity`
//! - Content-addressed artifacts: CID computation via `unfer_data`
//! - Federation effect types: DidCreate, ContentPublish, ConsensusSync
//!
//! Modules participate in the QuePaxa federation under their own DID.
//! The module principal maps to a DID for consensus operations.

#[cfg(feature = "federation")]
use unfer_consensus::{ConsensusNode, Keypair, LocalConsensus};
#[cfg(feature = "federation")]
use unfer_data::compute_cid;
#[cfg(feature = "federation")]
use unfer_identity::DidManager;

#[cfg(feature = "federation")]
pub struct ModuleIdentity {
    pub did: String,
    pub keypair: Keypair,
}

#[cfg(feature = "federation")]
impl ModuleIdentity {
    /// A DID created here is a real identity, not the non-federated stub.
    pub fn is_stub(&self) -> bool {
        false
    }

    pub fn create(node: &mut ConsensusNode, module_name: &str) -> Result<Self, String> {
        let keypair = Keypair::generate();
        let mut mgr = DidManager::new(node);
        let did = mgr
            .create_did(&keypair, Some(format!("modhost://{module_name}")))
            .map_err(|e| format!("DID creation failed: {e}"))?;
        Ok(Self { did, keypair })
    }
}

#[cfg(feature = "federation")]
pub fn artifact_cid(cps_data: &[u8]) -> String {
    compute_cid(cps_data)
}

/// D18: strict distributed mode needs a cryptographic artifact identity.
/// Federation CIDs qualify; an opaque or short id does not.
#[cfg(feature = "federation")]
pub fn artifact_id_is_cryptographic(id: &str) -> bool {
    crate::verification::cryptographic_artifact_id(id) || id.starts_with("bafy")
}

#[cfg(feature = "federation")]
pub fn create_consensus_node() -> ConsensusNode {
    ConsensusNode::new(Box::new(LocalConsensus::new()))
}

/// D18: strict distributed mode must not silently inherit the in-process
/// `LocalConsensus` node. This constructor names that refusal instead of
/// returning a node whose deployment identity is implicit. A real network
/// deployment is constructed by its own configuration path.
#[cfg(feature = "federation")]
pub fn create_consensus_node_strict() -> Result<ConsensusNode, String> {
    Err(
        "strict distributed profile refuses an implicit LocalConsensus deployment; \
         configure the network consensus node explicitly"
            .to_string(),
    )
}

/// D18: the non-federated fallback is still a cryptographic digest. It used to
/// be `DefaultHasher`, whose output is neither collision-resistant nor stable
/// across Rust releases, and could not name a network artifact.
#[cfg(not(feature = "federation"))]
pub fn artifact_cid(cps_data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(cps_data);
    let mut text = String::with_capacity(7 + digest.len() * 2);
    text.push_str("sha256:");
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

#[cfg(not(feature = "federation"))]
pub struct ModuleIdentity {
    pub did: String,
}

#[cfg(not(feature = "federation"))]
impl ModuleIdentity {
    pub fn stub(module_name: &str) -> Self {
        Self {
            did: format!("did:unfer:stub:{module_name}"),
        }
    }

    /// A non-federated identity is always a stub: it is useful for local
    /// development and is refused by a strict distributed profile.
    pub fn is_stub(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_ids_are_cryptographic() {
        let id = artifact_cid(b"module bytes");
        assert!(
            crate::verification::cryptographic_artifact_id(&id) || id.starts_with("bafy"),
            "artifact id must be a cryptographic digest: {id}"
        );
    }

    #[test]
    fn the_non_federated_identity_is_labelled_a_stub() {
        #[cfg(not(feature = "federation"))]
        {
            let id = ModuleIdentity::stub("M");
            assert!(id.is_stub());
            assert!(id.did.starts_with("did:unfer:stub:"));
        }
    }

    #[test]
    fn strict_mode_refuses_local_consensus() {
        #[cfg(feature = "federation")]
        {
            let e = create_consensus_node_strict().unwrap_err();
            assert!(e.contains("LocalConsensus"), "{e}");
        }
    }
}
