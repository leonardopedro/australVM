//! `arctic_authority` — a threshold-signed collective `AuthorizationEngine` backend for
//! australVM (`../australVM/safestos/cranelift/src/auth.rs`), built on the Arctic
//! deterministic two-round threshold Schnorr scheme (`../dynamic-arctic`, MIT).
//!
//! Where `ManifestAuthEngine` grants a module capabilities by static TOML manifest and
//! `CedarVmEngine` evaluates a single Cedar policy set, `ArcticAuthEngine` authorizes a
//! sensitive kernel call only against a `DelegationCertificate` that an *n-of-t* threshold
//! signing coalition has jointly signed with the group's Arctic key — no single party (not
//! even a compromised authority node below the threshold) can forge a grant. This is the
//! collective analogue of Cedar's single-policy decision: `t` distinct signers must agree
//! before a principal is authorized to call a resource.
//!
//! `arctic_authority` does not implement the Arctic protocol itself (that stays in
//! `../dynamic-arctic`); it only consumes `arctic_core::verify` plus the
//! `DelegationCertificate`/`DelegationRequest` wire types already defined there, and adds
//! the authorization-decision layer: certificate registration, expiry, and per-capability
//! matching against a `(principal, action, resource)` triple.

use arctic::arctic_core::{self, PubKey, Signature};
use arctic::types::DelegationCertificate;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthzResult {
    Allow,
    Deny,
}

#[derive(Debug, thiserror::Error)]
pub enum ArcticAuthError {
    #[error("delegation certificate failed threshold-signature verification")]
    BadSignature,
    #[error("group public key bytes are not a valid Ristretto point")]
    BadGroupKey,
}

/// Decompress a 32-byte Ristretto point (e.g. the `master_public_key` published in a
/// `did:web` document's `verificationMethod`) into the `PubKey` type Arctic's `verify`
/// expects.
pub fn group_pk_from_bytes(bytes: &[u8; 32]) -> Result<PubKey, ArcticAuthError> {
    curve25519_dalek::ristretto::CompressedRistretto(*bytes)
        .decompress()
        .ok_or(ArcticAuthError::BadGroupKey)
}

/// A threshold-signed collective `AuthorizationEngine` backend.
///
/// Holds the Arctic authority's group public key and a registry of delegation
/// certificates that have been verified against it. `authorize`/`check` never performs
/// signature verification on the hot path — that happens once, at `register_certificate`
/// time — so authorization checks stay cheap even though issuing a certificate required a
/// full 2-round threshold signing ceremony among `t` of `n` authority nodes.
pub struct ArcticAuthEngine {
    group_pk: PubKey,
    // keyed by the delegatee's public key (as carried in `DelegationCertificate::delegatee_pk`)
    certs: HashMap<String, DelegationCertificate>,
}

impl ArcticAuthEngine {
    pub fn new(group_pk: PubKey) -> Self {
        Self { group_pk, certs: HashMap::new() }
    }

    /// Verify `sig` against this engine's group public key for the canonical JSON
    /// encoding of `cert`, and if valid, register it so future `check`/`authorize` calls
    /// for `cert.delegatee_pk` are evaluated against it.
    ///
    /// The message signed is the JSON-serialized certificate itself — this must match
    /// exactly what the authority (`../dynamic-arctic/src/main.rs::handle_delegate`)
    /// signs, so the wire format is not renegotiated: an unfer-side verifier and the
    /// arctic authority server agree on `serde_json::to_vec(&cert)`.
    pub fn register_certificate(
        &mut self,
        cert: DelegationCertificate,
        sig: &Signature,
    ) -> Result<(), ArcticAuthError> {
        let cert_bytes = serde_json::to_vec(&cert).expect("DelegationCertificate serializes");
        if !arctic_core::verify(&self.group_pk, &cert_bytes, sig) {
            return Err(ArcticAuthError::BadSignature);
        }
        self.certs.insert(cert.delegatee_pk.clone(), cert);
        Ok(())
    }

    pub fn revoke(&mut self, delegatee_pk: &str) {
        self.certs.remove(delegatee_pk);
    }

    /// `principal` is the delegatee's public key (as registered via
    /// `register_certificate`); `action` mirrors the `AuthorizationEngine` convention
    /// (only `"Call"` is meaningful here); `resource` is the capability name, e.g. a
    /// `uk_*` symbol such as `"uk_observe"`. A certificate authorizes `resource` if its
    /// `capabilities` list contains that name verbatim or the wildcard `"*"`, and it has
    /// not expired.
    pub fn check(&self, principal: &str, action: &str, resource: &str) -> AuthzResult {
        if action != "Call" {
            return AuthzResult::Deny;
        }
        let Some(cert) = self.certs.get(principal) else {
            return AuthzResult::Deny;
        };
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        if now >= cert.expires_at {
            return AuthzResult::Deny;
        }
        if cert.capabilities.iter().any(|c| c == resource || c == "*") {
            AuthzResult::Allow
        } else {
            AuthzResult::Deny
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arctic::arctic_core::{combine, keygen, sign1, sign2};

    // Mirrors ../dynamic-arctic/src/arctic_core.rs::test_arctic_good's (n, t) — a
    // known-good, already-tested Arctic parameter pair — to build a real threshold
    // signature over a `DelegationCertificate` without standing up the axum HTTP
    // authority server from main.rs.
    fn sign_certificate(cert: &DelegationCertificate) -> (PubKey, Signature) {
        let n = 7u32;
        let t = 4u32;
        let (group_pk, _player_pks, seckeys) = keygen(n, t);
        let coalition: Vec<u32> = (1..=n).collect();
        let cert_bytes = serde_json::to_vec(cert).unwrap();

        let r1_outputs: Vec<_> = seckeys.iter().map(|k| sign1(k, &coalition, &cert_bytes)).collect();
        let sigshares: Vec<_> = seckeys
            .iter()
            .map(|k| sign2(&group_pk, k, &coalition, &cert_bytes, &r1_outputs).unwrap())
            .collect();
        let sig = combine(&group_pk, t, &coalition, &cert_bytes, &r1_outputs, &sigshares).unwrap();
        (group_pk, sig)
    }

    fn far_future() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600
    }

    #[test]
    fn valid_certificate_authorizes_granted_capability() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: far_future(),
            capabilities: vec!["uk_observe".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);
        let mut engine = ArcticAuthEngine::new(group_pk);
        engine.register_certificate(cert, &sig).expect("valid signature registers");

        assert_eq!(engine.check("z6MkHotKey", "Call", "uk_observe"), AuthzResult::Allow);
    }

    #[test]
    fn ungranted_capability_is_denied() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: far_future(),
            capabilities: vec!["uk_observe".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);
        let mut engine = ArcticAuthEngine::new(group_pk);
        engine.register_certificate(cert, &sig).unwrap();

        assert_eq!(engine.check("z6MkHotKey", "Call", "uk_set_hamiltonian"), AuthzResult::Deny);
    }

    #[test]
    fn wildcard_capability_authorizes_anything() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: far_future(),
            capabilities: vec!["*".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);
        let mut engine = ArcticAuthEngine::new(group_pk);
        engine.register_certificate(cert, &sig).unwrap();

        assert_eq!(engine.check("z6MkHotKey", "Call", "uk_bayesian_update"), AuthzResult::Allow);
    }

    #[test]
    fn expired_certificate_is_denied() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: 1, // 1970, long expired
            capabilities: vec!["*".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);
        let mut engine = ArcticAuthEngine::new(group_pk);
        engine.register_certificate(cert, &sig).unwrap();

        assert_eq!(engine.check("z6MkHotKey", "Call", "uk_bayesian_update"), AuthzResult::Deny);
    }

    #[test]
    fn tampered_certificate_fails_signature_verification() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: far_future(),
            capabilities: vec!["uk_observe".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);

        // A malicious relay bumps its own capabilities after the fact, without
        // re-running the threshold ceremony.
        let mut tampered = cert;
        tampered.capabilities = vec!["*".to_string()];

        let mut engine = ArcticAuthEngine::new(group_pk);
        let result = engine.register_certificate(tampered, &sig);
        assert!(matches!(result, Err(ArcticAuthError::BadSignature)));
    }

    #[test]
    fn unregistered_principal_is_denied() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: far_future(),
            capabilities: vec!["*".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);
        let mut engine = ArcticAuthEngine::new(group_pk);
        engine.register_certificate(cert, &sig).unwrap();

        assert_eq!(engine.check("someone_else", "Call", "uk_observe"), AuthzResult::Deny);
    }

    #[test]
    fn non_call_action_is_denied() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: far_future(),
            capabilities: vec!["*".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);
        let mut engine = ArcticAuthEngine::new(group_pk);
        engine.register_certificate(cert, &sig).unwrap();

        assert_eq!(engine.check("z6MkHotKey", "Write", "uk_observe"), AuthzResult::Deny);
    }

    #[test]
    fn revoke_removes_authorization() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: far_future(),
            capabilities: vec!["*".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);
        let mut engine = ArcticAuthEngine::new(group_pk);
        engine.register_certificate(cert, &sig).unwrap();
        assert_eq!(engine.check("z6MkHotKey", "Call", "uk_observe"), AuthzResult::Allow);

        engine.revoke("z6MkHotKey");
        assert_eq!(engine.check("z6MkHotKey", "Call", "uk_observe"), AuthzResult::Deny);
    }

    #[test]
    fn group_pk_from_bytes_round_trips_a_valid_point() {
        let (group_pk, _, _) = keygen(7, 4);
        let bytes = group_pk.compress().to_bytes();
        let recovered = group_pk_from_bytes(&bytes).expect("valid compressed Ristretto point");
        assert_eq!(recovered.compress().to_bytes(), bytes);
    }
}
