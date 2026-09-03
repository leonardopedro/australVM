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

#[cfg(feature = "federation")]
pub fn create_consensus_node() -> ConsensusNode {
    ConsensusNode::new(Box::new(LocalConsensus::new()))
}

#[cfg(not(feature = "federation"))]
pub fn artifact_cid(cps_data: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    cps_data.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
}
