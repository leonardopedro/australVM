//! Sandbox accounts: a DID, an ed25519 key the service holds on the account's
//! behalf (a *custodial* wallet, appropriate for the sandbox settlement mode
//! this service runs in), and a bearer token stored only as a SHA-256 hash.
//!
//! The account file is written with mode 0600 because it contains the sandbox
//! signing secrets. A non-custodial deployment would keep only the DID and a
//! public key here and take signed transactions from the client instead; that
//! is deliberately out of scope for v1 and is called out in the README.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use unfer_consensus::signing::Keypair;

use crate::util::{now_unix, random_token, sha256_hex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Can hold a reserve, withdraw, deposit and receive e-coins.
    Customer,
    /// Can receive deposits and peg fiat out.
    Merchant,
    /// Everything, including denominations, minting, wire settlement and
    /// content management.
    Admin,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Customer => "customer",
            Role::Merchant => "merchant",
            Role::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "customer" => Some(Role::Customer),
            "merchant" => Some(Role::Merchant),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub did: String,
    pub name: String,
    pub role: Role,
    /// Hex-encoded 32-byte ed25519 seed. Custodial sandbox wallet only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub secret: String,
    /// SHA-256 of the bearer token; the token itself is shown once.
    pub token_sha256: String,
    pub created_at: u64,
}

impl Account {
    pub fn keypair(&self) -> Option<Keypair> {
        let bytes = hex::decode(&self.secret).ok()?;
        let seed: [u8; 32] = bytes.as_slice().try_into().ok()?;
        Some(Keypair::from_bytes(&seed))
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    /// The public view (never includes the secret or the token hash).
    pub fn public(&self, reserves: &[(String, u64)], coins: u64, merchant_balance: u64) -> serde_json::Value {
        serde_json::json!({
            "did": self.did,
            "name": self.name,
            "role": self.role.as_str(),
            "created_at": self.created_at,
            "reserves": reserves
                .iter()
                .map(|(id, balance)| serde_json::json!({"reserve_id": id, "balance": balance}))
                .collect::<Vec<_>>(),
            "certificates": coins,
            "merchant_balance": merchant_balance,
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AccountStore {
    #[serde(default)]
    pub accounts: Vec<Account>,
}

impl AccountStore {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("accounts file {} is not valid: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| format!("encode accounts: {e}"))?;
        crate::util::write_atomic(path, &bytes, true)
    }

    /// Create a fresh account and return it with its one-time bearer token.
    pub fn create(&mut self, name: &str, role: Role) -> (Account, String) {
        let (kp, seed) = seed_keypair();
        let token = random_token("refi_");
        let account = Account {
            did: kp.did(),
            name: name.to_string(),
            role,
            secret: hex::encode(seed),
            token_sha256: sha256_hex(token.as_bytes()),
            created_at: now_unix(),
        };
        self.accounts.push(account.clone());
        (account, token)
    }

    pub fn by_did(&self, did: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.did == did)
    }

    /// Resolve a bearer token to its account, comparing hashes in constant
    /// time. Tokens are never stored in the clear.
    pub fn by_token(&self, token: &str) -> Option<&Account> {
        let probe = sha256_hex(token.as_bytes());
        self.accounts
            .iter()
            .find(|a| crate::util::ct_eq(a.token_sha256.as_bytes(), probe.as_bytes()))
    }

    pub fn count(&self) -> usize {
        self.accounts.len()
    }
}

/// Generate a keypair from a fresh random seed and keep both halves.
///
/// `Keypair` deliberately has no secret exporter (the library keeps signing
/// material opaque), so the sandbox wallet is created from a seed the host
/// generates and records itself.
pub fn seed_keypair() -> (Keypair, [u8; 32]) {
    let seed: [u8; 32] = rand::random();
    (Keypair::from_bytes(&seed), seed)
}

/// The account file lives beside the exchange state.
pub fn accounts_path(state_dir: &Path) -> PathBuf {
    state_dir.join("accounts.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_hashed_and_resolve_to_the_right_account() {
        let (kp, seed) = seed_keypair();
        let mut store = AccountStore::default();
        let token = random_token("refi_");
        store.accounts.push(Account {
            did: kp.did(),
            name: "alice".into(),
            role: Role::Customer,
            secret: hex::encode(seed),
            token_sha256: sha256_hex(token.as_bytes()),
            created_at: 1,
        });
        let found = store.by_token(&token).unwrap();
        assert_eq!(found.name, "alice");
        assert!(store.by_token("refi_nope").is_none());
        assert!(!found.token_sha256.contains(&token[..8]));
        assert_eq!(found.keypair().unwrap().did(), kp.did());
    }

    #[test]
    fn accounts_round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = accounts_path(dir.path());
        let mut store = AccountStore::default();
        let (_a, _t) = store.create("bob", Role::Merchant);
        store.save(&path).unwrap();
        let loaded = AccountStore::load(&path).unwrap();
        assert_eq!(loaded.count(), 1);
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o600, "accounts file must not be world-readable");
    }
}
