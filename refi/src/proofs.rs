//! The proof ledger: mm0 developments the exchange submitted to `mm0web`,
//! verified there (mm0-rs, then mm0-c) and published to Zenodo, with the DOI
//! each one earned.
//!
//! This is a second ledger beside the certificate ledger, and deliberately a
//! separate one: proofs are evidence, not money. Nothing here can mint, spend
//! or move a certificate — a published DOI is a citation, and the exchange's
//! policy kernel never sees it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::mm0::{Client, PublishReply};
use crate::util::{now_unix, sha256_hex};

/// The largest mm0/mm1 text the service forwards. The transport caps a whole
/// request body at 256 kB, so this is what fits end to end (mm0web itself would
/// take up to 4 MiB per field).
pub const MAX_FIELD: usize = 256 * 1024;

/// How often the worker asks mm0web whether a queued deposit has a DOI yet.
const REFRESH: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Handed to mm0web; its background worker is building the deposit.
    Queued,
    /// Verified and stored by mm0web, but not archived (no Zenodo key on the
    /// backend, or the archive step has not run).
    Verified,
    /// Archived: the deposit carries a DOI.
    Published,
    /// mm0web refused the development — a proof hole or a malformed spec.
    Rejected,
    /// Never verified, or the queue refused it; `detail` says why.
    Failed,
}

impl State {
    pub fn as_str(&self) -> &'static str {
        match self {
            State::Queued => "queued",
            State::Verified => "verified",
            State::Published => "published",
            State::Rejected => "rejected",
            State::Failed => "failed",
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, State::Queued | State::Verified)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proof {
    /// Exchange-side id: a digest of title + both texts, so re-submitting the
    /// same development updates one row instead of adding a second.
    pub id: String,
    pub project: String,
    pub title: String,
    pub sha256: String,
    pub state: State,
    /// mm0web's content-addressed proof id, once verification ran.
    #[serde(default)]
    pub mm0_proof: Option<String>,
    #[serde(default)]
    pub zenodo_record: Option<u64>,
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// What the checkers said: the `mm0-rs` and `mm0-c` output. Kept even
    /// when the record is archived, so an operator can see the evidence that
    /// justified the DOI.
    #[serde(default)]
    pub log: Option<String>,
    /// What went wrong, when something did: a rejection reason or the
    /// backend's error.
    #[serde(default)]
    pub detail: Option<String>,
    pub submitted_by: String,
    pub submitted_at: u64,
    pub updated_at: u64,
}

impl Proof {
    pub fn doi_url(&self) -> Option<String> {
        self.doi.as_ref().map(|d| format!("https://doi.org/{d}"))
    }

    /// Only an archived proof — one that actually earned a DOI — can back a
    /// certificate. Verified-without-a-DOI is evidence of checking, not of an
    /// archive, and a certificate wants a citation it can carry.
    pub fn can_mint(&self) -> bool {
        self.state == State::Published && self.doi.is_some()
    }

    /// The whole record as an operator sees it: the proof, the backend it ran
    /// on, and the certificates minted against its DOI.
    pub fn detail(&self, certificates: Vec<Value>, backend: Option<String>) -> ProofDetail {
        ProofDetail {
            proof: self.clone(),
            backend,
            certificates,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "title": self.title,
            "sha256": self.sha256,
            "state": self.state.as_str(),
            "proof": self.mm0_proof,
            "zenodo_record": self.zenodo_record,
            "doi": self.doi,
            "url": self.url,
            "doi_url": self.doi_url(),
            "log": self.log,
            "detail": self.detail,
            "submitted_by": self.submitted_by,
            "submitted_at": self.submitted_at,
            "updated_at": self.updated_at,
        })
    }
}

pub struct ProofDetail {
    pub proof: Proof,
    pub backend: Option<String>,
    pub certificates: Vec<Value>,
}

impl ProofDetail {
    pub fn to_json(&self) -> Value {
        // The proof's own fields stay at the top level so a client that only
        // reads a proof keeps working; the certificate layer is additive.
        let mut value = self.proof.to_json();
        value["backend"] = json!(self.backend);
        value["certificates"] = json!(self.certificates);
        value
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Store {
    #[serde(default = "format_version")]
    pub version: u32,
    #[serde(default)]
    pub proofs: Vec<Proof>,
}

fn format_version() -> u32 {
    1
}

impl Default for Store {
    fn default() -> Self {
        Self {
            version: format_version(),
            proofs: Vec::new(),
        }
    }
}

impl Store {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let store: Self = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("proof ledger {} is not valid: {e}", path.display()))?;
                if store.version != format_version() {
                    return Err(format!(
                        "proof ledger {} has version {} (this build writes {})",
                        path.display(),
                        store.version,
                        format_version()
                    ));
                }
                Ok(store)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| format!("encode proofs: {e}"))?;
        crate::util::write_atomic(path, &bytes, false)
    }

    pub fn by_id(&self, id: &str) -> Option<&Proof> {
        self.proofs.iter().find(|p| p.id == id)
    }

    /// Newest first, replacing any earlier row with the same id.
    pub fn put(&mut self, proof: Proof) {
        self.proofs.retain(|p| p.id != proof.id);
        self.proofs.insert(0, proof);
    }
}

pub fn proofs_path(state_dir: &Path) -> PathBuf {
    state_dir.join("proofs.json")
}

/// The exchange-side id: the same title and development always lands on the
/// same row, whatever the operator's clock or the backend's queue does.
pub fn record_id(title: &str, mm0: &str, mm1: &str) -> String {
    sha256_hex(format!("{title}\n{mm0}\n{mm1}").as_bytes())
}

pub struct Ledger {
    path: PathBuf,
    store: Store,
    client: Option<Client>,
}

impl Ledger {
    pub fn load(path: PathBuf, mm0web: Option<&str>) -> Result<Self, String> {
        let client = match mm0web {
            Some(base) => Some(Client::new(base)?),
            None => None,
        };
        Ok(Self {
            store: Store::load(&path)?,
            path,
            client,
        })
    }

    pub fn configured(&self) -> bool {
        self.client.is_some()
    }

    pub fn base(&self) -> Option<String> {
        self.client.as_ref().map(|c| c.base())
    }

    pub fn list(&self) -> &[Proof] {
        &self.store.proofs
    }

    pub fn by_id(&self, id: &str) -> Option<&Proof> {
        self.store.by_id(id)
    }

    /// (published, pending, failed-or-rejected)
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut published = 0;
        let mut pending = 0;
        let mut failed = 0;
        for p in &self.store.proofs {
            match p.state {
                State::Published => published += 1,
                State::Queued | State::Verified => pending += 1,
                State::Rejected | State::Failed => failed += 1,
            }
        }
        (published, pending, failed)
    }

    /// Submit a development. Validated by the caller (title/project/size); the
    /// only errors here are the backend's. An already-published development is
    /// returned as-is: the id is content-addressed, so that is the same proof.
    pub fn publish(
        &mut self,
        actor: &str,
        project: &str,
        title: &str,
        mm0: &str,
        mm1: &str,
    ) -> Result<Proof, String> {
        let id = record_id(title, mm0, mm1);
        if let Some(existing) = self.store.by_id(&id)
            && existing.state == State::Published
        {
            return Ok(existing.clone());
        }
        let Some(client) = self.client.as_ref() else {
            return Err("proof publishing is not configured on this exchange".to_string());
        };
        let reply = client.publish(project, title, mm0, mm1)?;
        let proof = new_proof(id, actor, project, title, mm0, mm1, reply);
        self.store.put(proof.clone());
        self.store.save(&self.path)?;
        Ok(proof)
    }

    /// Ask mm0web about every pending record and fold in whatever it learned:
    /// the DOI once the deposit is archived, or the reason it was not.
    pub fn refresh(&mut self) -> Result<usize, String> {
        let Some(client) = self.client.as_ref() else {
            return Ok(0);
        };
        let pending: Vec<(String, String, String)> = self
            .store
            .proofs
            .iter()
            .filter(|p| p.state.is_pending())
            .filter_map(|p| {
                p.mm0_proof
                    .clone()
                    .map(|mm0_proof| (p.id.clone(), p.project.clone(), mm0_proof))
            })
            .collect();
        let mut changed = 0;
        for (id, project, mm0_proof) in pending {
            // A backend that is down right now is not an error worth failing a
            // request over: the next tick asks again.
            let Ok(reply) = client.proof(&project, &mm0_proof) else {
                continue;
            };
            let Some(p) = self.store.proofs.iter_mut().find(|p| p.id == id) else {
                continue;
            };
            let mut touched = false;
            let next = match reply.state.as_str() {
                "published" => Some(State::Published),
                "publish-failed" => Some(State::Failed),
                "verified" => Some(State::Verified),
                _ => None, // still queued
            };
            if let Some(state) = next
                && p.state != state
            {
                p.state = state;
                touched = true;
            }
            if reply.doi.is_some() && p.doi != reply.doi {
                p.doi = reply.doi;
                touched = true;
            }
            if reply.record.is_some() && p.zenodo_record != reply.record {
                p.zenodo_record = reply.record;
                touched = true;
            }
            if reply.url.is_some() && p.url != reply.url {
                p.url = reply.url;
                touched = true;
            }
            if reply.log.is_some() && p.log != reply.log {
                p.log = reply.log;
                touched = true;
            }
            if touched {
                p.updated_at = now_unix();
                changed += 1;
            }
        }
        if changed > 0 {
            self.store.save(&self.path)?;
        }
        Ok(changed)
    }
}

fn new_proof(
    id: String,
    actor: &str,
    project: &str,
    title: &str,
    mm0: &str,
    mm1: &str,
    reply: PublishReply,
) -> Proof {
    let now = now_unix();
    let (state, detail) = match reply.status {
        // A proof hole or a malformed spec: mm0web answered 400 with the stage
        // it died at, and put the checker output under `log` (which we keep as
        // the verification log).
        400 => (
            State::Rejected,
            reply
                .stage
                .as_ref()
                .map(|s| format!("rejected at {s}"))
                .or_else(|| reply.error.clone()),
        ),
        // Verified, stored, and the deposit already exists (idempotent re-run).
        200 if reply.doi.is_some() => (State::Published, None),
        200 => (
            State::Verified,
            Some("mm0web verified the development but reported no deposit".to_string()),
        ),
        // Verification passed and the publication is queued on the backend.
        202 => (State::Queued, None),
        // Verified and stored, but the backend has no Zenodo credentials, so
        // nothing was archived. The proof is real; the DOI is what is missing.
        502 => (State::Verified, reply.error.clone()),
        other => (
            State::Failed,
            reply
                .error
                .clone()
                .or_else(|| Some(format!("mm0web answered {other}"))),
        ),
    };
    let sha256 = sha256_hex(format!("{mm0}\n{mm1}").as_bytes());
    Proof {
        id,
        project: project.to_string(),
        title: title.to_string(),
        sha256,
        state,
        mm0_proof: reply.proof,
        zenodo_record: reply.record,
        doi: reply.doi,
        url: reply.url,
        log: reply.log,
        detail,
        submitted_by: actor.to_string(),
        submitted_at: now,
        updated_at: now,
    }
}

/// Keep the ledger's pending rows current. One thread for the process, woken
/// every [`REFRESH`]; it stops when the service is dropped.
pub fn spawn_worker(ledger: Arc<Mutex<Ledger>>, stop: Arc<AtomicBool>) {
    let spawned = std::thread::Builder::new()
        .name("refi-proofs".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(REFRESH);
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(mut ledger) = ledger.lock()
                    && let Err(e) = ledger.refresh()
                {
                    tracing::warn!(error = %e, "proof ledger refresh failed");
                }
            }
        });
    if spawned.is_err() {
        tracing::warn!("could not start the proof ledger refresher");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16) -> PublishReply {
        PublishReply {
            status,
            accepted: status == 200 || status == 202,
            ..Default::default()
        }
    }

    #[test]
    fn ids_are_content_addressed() {
        let a = record_id("claim", "spec", "proof");
        assert_eq!(a.len(), 64);
        assert_eq!(a, record_id("claim", "spec", "proof"));
        assert_ne!(a, record_id("claim", "spec", "other"));
        assert_ne!(a, record_id("other", "spec", "proof"));
    }

    #[test]
    fn replies_map_onto_states() {
        let made = |status| new_proof("id".into(), "admin", "p", "t", "m0", "m1", reply(status));
        assert_eq!(made(400).state, State::Rejected);
        assert_eq!(made(202).state, State::Queued);
        assert_eq!(made(502).state, State::Verified);
        assert_eq!(made(503).state, State::Failed);
        assert_eq!(made(429).state, State::Failed);

        // The verification log survives; it is the evidence a DOI cites.
        let mut accepted = reply(202);
        accepted.log = Some("mm0-rs: ok\nmm0-c: ok".to_string());
        let p = new_proof("id".into(), "admin", "p", "t", "m0", "m1", accepted);
        assert_eq!(p.log.as_deref(), Some("mm0-rs: ok\nmm0-c: ok"));
        assert_eq!(p.detail, None);

        let mut published = reply(200);
        published.doi = Some("10.5281/zenodo.1".to_string());
        let p = new_proof("id".into(), "admin", "p", "t", "m0", "m1", published);
        assert_eq!(p.state, State::Published);
        assert_eq!(p.doi_url().as_deref(), Some("https://doi.org/10.5281/zenodo.1"));
    }

    #[test]
    fn only_published_proofs_mint() {
        let mut p = new_proof("id".into(), "admin", "p", "t", "m0", "m1", reply(202));
        assert!(!p.can_mint(), "queued is not an archive");
        p.state = State::Verified;
        p.doi = Some("10.5281/zenodo.9".to_string());
        assert!(!p.can_mint(), "verified without a DOI is not an archive");
        p.state = State::Published;
        assert!(p.can_mint(), "published with a DOI can back a certificate");
    }

    #[test]
    fn the_store_round_trips_and_replaces_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proofs.json");
        let mut store = Store::load(&path).unwrap();
        assert!(store.proofs.is_empty());

        let p = new_proof("abc".into(), "admin", "p", "t", "m0", "m1", reply(202));
        store.put(p.clone());
        store.put(p.clone());
        assert_eq!(store.proofs.len(), 1, "the same development is one row");
        store.save(&path).unwrap();

        let back = Store::load(&path).unwrap();
        assert_eq!(back.proofs.len(), 1);
        assert_eq!(back.proofs[0].id, "abc");
        assert_eq!(back.proofs[0].state, State::Queued);
        assert_eq!(back.version, 1);
    }

    #[test]
    fn a_ledger_without_a_backend_refuses_to_publish() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(proofs_path(dir.path()), None).unwrap();
        assert!(!ledger.configured());
        assert!(ledger.publish("admin", "p", "t", "m0", "m1").is_err());
        assert_eq!(ledger.refresh().unwrap(), 0);
    }
}
