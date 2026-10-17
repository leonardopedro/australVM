//! Ed25519 manifest signatures (Sprint 1, Task 1.3).
//!
//! Every root manifest carries the builder's Ed25519 signature over its
//! canonical *unsigned* encoding (and names the verifying key), so a
//! poisoned remote cache cannot forge a cell. Replaces the symmetric
//! password model the spec retires: verification is public, forgery needs
//! the builder's seed.
//!
//! Keys are raw 32-byte seeds (`SigningKey::from_bytes`); tests derive them
//! from BLAKE3 so no RNG dependency exists anywhere in the store.

use crate::engine::Manifest;
use crate::CasError;
use ed25519_dalek::{Signer, Signature, SigningKey, Verifier, VerifyingKey};

/// Build a signing key from a raw 32-byte seed.
pub fn signing_key_from_seed(seed: &[u8; 32]) -> SigningKey {
    SigningKey::from_bytes(seed)
}

/// Sign `manifest` in place: records the verifying key and signature over
/// the canonical unsigned encoding (the manifest's identity digest).
pub fn sign_manifest(manifest: &mut Manifest, key: &SigningKey) {
    let digest = manifest.identity_digest();
    let sig: Signature = key.sign(&digest);
    manifest.pubkey = Some(key.verifying_key().to_bytes());
    manifest.signature = Some(sig.to_bytes());
}

/// Verify a manifest's embedded signature. Unsigned manifests, manifests
/// whose key field is missing, and mismatched signatures all fail with a
/// `Signature` error — fail-closed, no "unsigned is fine" path.
pub fn verify_manifest(manifest: &Manifest) -> Result<(), CasError> {
    let Some(pk_bytes) = manifest.pubkey else {
        return Err(CasError::Signature("manifest carries no public key".into()));
    };
    let Some(sig_bytes) = manifest.signature else {
        return Err(CasError::Signature("manifest is unsigned".into()));
    };
    let vk = VerifyingKey::from_bytes(&pk_bytes)
        .map_err(|e| CasError::Signature(format!("invalid public key: {e}")))?;
    let sig = Signature::from_bytes(&sig_bytes);
    let digest = manifest.identity_digest();
    vk.verify(&digest, &sig)
        .map_err(|e| CasError::Signature(format!("signature mismatch: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Manifest;

    fn seed(n: u8) -> [u8; 32] {
        let mut s = [0u8; 32];
        s[0] = n;
        s[31] = 0xA5;
        s
    }

    #[test]
    fn sign_verify_roundtrip() {
        let key = signing_key_from_seed(&seed(1));
        let mut m = Manifest::new("demo", "0.1.0");
        m.total_size = 1234;
        sign_manifest(&mut m, &key);
        verify_manifest(&m).expect("fresh signature must verify");
        // Identity is signature-independent: signing does not move the hash.
        let before = Manifest::new("demo", "0.1.0");
        let mut signed = before.clone();
        sign_manifest(&mut signed, &key);
        assert_eq!(signed.identity_digest(), before.identity_digest());
    }

    #[test]
    fn tampered_manifest_fails() {
        let key = signing_key_from_seed(&seed(2));
        let mut m = Manifest::new("demo", "0.1.0");
        sign_manifest(&mut m, &key);
        m.total_size += 1; // tamper after signing
        assert!(matches!(verify_manifest(&m), Err(CasError::Signature(_))));
    }

    #[test]
    fn unsigned_manifest_fails_closed() {
        let m = Manifest::new("demo", "0.1.0");
        assert!(matches!(verify_manifest(&m), Err(CasError::Signature(_))));
    }

    #[test]
    fn wrong_key_fails() {
        let mut m = Manifest::new("demo", "0.1.0");
        sign_manifest(&mut m, &signing_key_from_seed(&seed(3)));
        // Rewrite the claimed key with a different one.
        m.pubkey = Some(signing_key_from_seed(&seed(4)).verifying_key().to_bytes());
        assert!(matches!(verify_manifest(&m), Err(CasError::Signature(_))));
    }
}
