//! Tests for federation-aware module hosting (B11).

#[test]
fn artifact_cid_deterministic() {
    let data = b"test module artifact bytes";
    let cid1 = austral_cranelift_bridge::federation::artifact_cid(data);
    let cid2 = austral_cranelift_bridge::federation::artifact_cid(data);
    assert_eq!(cid1, cid2);
    assert!(!cid1.is_empty());
}

#[test]
fn artifact_cid_different_for_different_data() {
    let cid1 = austral_cranelift_bridge::federation::artifact_cid(b"module A");
    let cid2 = austral_cranelift_bridge::federation::artifact_cid(b"module B");
    assert_ne!(cid1, cid2);
}

#[cfg(not(feature = "federation"))]
#[test]
fn module_identity_stub() {
    let id = austral_cranelift_bridge::federation::ModuleIdentity::stub("test_mod");
    assert!(id.did.contains("test_mod"));
    assert!(id.did.starts_with("did:unfer:stub:"));
}

#[cfg(feature = "federation")]
#[test]
fn module_identity_creates_did() {
    let mut node = austral_cranelift_bridge::federation::create_consensus_node();
    let id = austral_cranelift_bridge::federation::ModuleIdentity::create(
        &mut node,
        "test_mod",
    )
    .unwrap();
    assert!(id.did.starts_with("did:unfer:"));
}

#[cfg(feature = "federation")]
#[test]
fn artifact_cid_uses_sha256() {
    let cid = austral_cranelift_bridge::federation::artifact_cid(b"hello");
    assert_eq!(cid.len(), 64);
}
