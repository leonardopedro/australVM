//! Bridges `arctic_authority::ArcticAuthEngine` (a threshold-signed collective
//! authorization backend built on `../dynamic-arctic`'s Arctic scheme) into this crate's
//! `AuthorizationEngine` trait, the same way `policy.rs` bridges Cedar. Unlike
//! `ManifestAuthEngine` (static TOML grants) or Cedar (a single policy set), a call is
//! authorized here only if a *t-of-n* threshold signing coalition has jointly signed a
//! still-valid `DelegationCertificate` naming the caller's capability — see
//! `arctic_authority`'s crate docs for the full rationale.

use crate::auth::{AuthorizationEngine, Decision};
use arctic_authority::{ArcticAuthEngine, AuthzResult};

pub struct ArcticVmEngine(ArcticAuthEngine);

impl ArcticVmEngine {
    pub fn new(engine: ArcticAuthEngine) -> Self {
        Self(engine)
    }

    pub fn inner_mut(&mut self) -> &mut ArcticAuthEngine {
        &mut self.0
    }
}

/// Install `engine` as the process-wide `AuthorizationEngine`, the same way
/// `safestos_load_auth_manifest` installs a `ManifestAuthEngine`. From this point every
/// `check_call_permission` — including `uk_*`/`uz_*` kernel calls — is decided by whether
/// the caller holds a still-valid, threshold-signed `DelegationCertificate` for that
/// capability.
pub fn install(engine: ArcticAuthEngine) {
    crate::auth::set_auth_engine(Box::new(ArcticVmEngine::new(engine)));
}

impl AuthorizationEngine for ArcticVmEngine {
    fn authorize(&self, principal: &str, action: &str, resource: &str) -> Result<Decision, String> {
        match self.0.check(principal, action, resource) {
            AuthzResult::Allow => Ok(Decision::Allow),
            AuthzResult::Deny => Ok(Decision::Deny),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arctic::arctic_core::{combine, keygen, sign1, sign2};
    use arctic::types::DelegationCertificate;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sign_certificate(
        cert: &DelegationCertificate,
    ) -> (arctic::arctic_core::PubKey, arctic::arctic_core::Signature) {
        let n = 7u32;
        let t = 4u32;
        let (group_pk, _player_pks, seckeys) = keygen(n, t);
        let coalition: Vec<u32> = (1..=n).collect();
        let cert_bytes = serde_json::to_vec(cert).unwrap();

        let r1_outputs: Vec<_> = seckeys
            .iter()
            .map(|k| sign1(k, &coalition, &cert_bytes))
            .collect();
        let sigshares: Vec<_> = seckeys
            .iter()
            .map(|k| sign2(&group_pk, k, &coalition, &cert_bytes, &r1_outputs).unwrap())
            .collect();
        let sig = combine(
            &group_pk,
            t,
            &coalition,
            &cert_bytes,
            &r1_outputs,
            &sigshares,
        )
        .unwrap();
        (group_pk, sig)
    }

    #[test]
    fn arctic_vm_engine_authorizes_via_authorization_engine_trait() {
        let cert = DelegationCertificate {
            issuer_did: "did:web:authority.example".to_string(),
            delegatee_pk: "z6MkHotKey".to_string(),
            expires_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600,
            capabilities: vec!["uk_observe".to_string()],
        };
        let (group_pk, sig) = sign_certificate(&cert);
        let mut inner = ArcticAuthEngine::new(group_pk);
        inner.register_certificate(cert, &sig).unwrap();
        let engine = ArcticVmEngine::new(inner);

        assert_eq!(
            engine
                .authorize("z6MkHotKey", "Call", "uk_observe")
                .unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine
                .authorize("z6MkHotKey", "Call", "uk_set_hamiltonian")
                .unwrap(),
            Decision::Deny
        );
        assert_eq!(
            engine
                .authorize("unknown_principal", "Call", "uk_observe")
                .unwrap(),
            Decision::Deny
        );
    }
}
