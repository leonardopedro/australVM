//! The service: authentication, rate limiting, routing, JSON API, site pages
//! and the admin console. Transport-independent — `server.rs` adapts sockets to
//! `handle`, and the tests call it directly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};

use crate::accounts::{Account, AccountStore, Role, accounts_path};
use crate::content::{ContentStore, content_path};
use crate::engine::{Cmd, Engine, Read, default_reserve_id, parse_id};
use crate::proofs::{Ledger, MAX_FIELD, Proof, State, proofs_path};
use crate::site::{self, Chrome};
use crate::util::{
    ApiError, HttpRequest, HttpResponse, ct_eq, escape_html, form_get, is_doi, now_unix,
    parse_form, sha256_hex,
};

/// Deployment configuration (from CLI flags / env).
#[derive(Debug, Clone)]
pub struct Config {
    pub state_dir: PathBuf,
    pub base_url: String,
    /// Allow anyone to self-register a customer account.
    pub open_registration: bool,
    /// Mark cookies `Secure` (set when serving over TLS).
    pub secure_cookies: bool,
    /// Base URL of the `mm0web` proof service, when proof publishing is
    /// configured. Unset means the exchange runs without a proof backend and
    /// refuses submissions instead of pretending.
    pub mm0web: Option<String>,
    /// Sandbox settlement banner. This build always runs the simulated wire
    /// gateway; the flag exists so the label cannot silently disappear.
    pub sandbox: bool,
}

pub struct Metrics {
    pub requests: AtomicU64,
    pub errors: AtomicU64,
    pub mutations: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            mutations: AtomicU64::new(0),
        }
    }
}

#[derive(Default)]
struct RateLimiter {
    buckets: HashMap<String, (u64, u32)>,
}

impl RateLimiter {
    fn allow(&mut self, key: &str, per_minute: u32) -> bool {
        let minute = now_unix() / 60;
        let entry = self.buckets.entry(key.to_string()).or_insert((minute, 0));
        if entry.0 != minute {
            *entry = (minute, 0);
        }
        if entry.1 >= per_minute {
            return false;
        }
        entry.1 += 1;
        true
    }
}

enum Caller {
    Admin,
    Account(Account),
}

impl Caller {
    fn account(&self) -> Option<&Account> {
        match self {
            Caller::Account(a) => Some(a),
            Caller::Admin => None,
        }
    }

    fn did(&self) -> Option<&str> {
        self.account().map(|a| a.did.as_str())
    }

    fn is_admin(&self) -> bool {
        matches!(self, Caller::Admin) || self.account().map(|a| a.is_admin()).unwrap_or(false)
    }
}

pub struct Service {
    pub engine: Engine,
    accounts: Mutex<AccountStore>,
    content: Mutex<ContentStore>,
    /// Proofs published through `mm0web`, and the DOI each one earned. Shared
    /// with the refresher thread, which folds in DOIs as the backend's
    /// publishing worker finishes them.
    ledger: Arc<Mutex<Ledger>>,
    ledger_stop: Arc<AtomicBool>,
    accounts_path: PathBuf,
    content_path: PathBuf,
    admin_hash: String,
    pub config: Config,
    started: Instant,
    pub metrics: Metrics,
    limiter: Mutex<RateLimiter>,
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop_ledger_worker();
    }
}

impl Service {
    pub fn new(
        config: Config,
        engine: Engine,
        admin_token: &str,
    ) -> Result<Self, String> {
        let accounts_file = accounts_path(&config.state_dir);
        let mut accounts = AccountStore::load(&accounts_file)?;
        let content_file = content_path(&config.state_dir);
        let mut content = ContentStore::load(&content_file)?;
        if !content.seeded {
            content.seed_if_empty();
            content.save(&content_file)?;
        }
        // Guarantee at least one admin account matching the bootstrap token, so
        // the console has a DID to attribute edits to.
        if !accounts.accounts.iter().any(|a| a.is_admin()) {
            let admin = Account {
                did: "did:unfer:bootstrap-admin".to_string(),
                name: "bootstrap".to_string(),
                role: Role::Admin,
                secret: String::new(),
                token_sha256: sha256_hex(admin_token.as_bytes()),
                created_at: now_unix(),
            };
            accounts.accounts.push(admin);
            accounts.save(&accounts_file)?;
        }
        let ledger = Ledger::load(proofs_path(&config.state_dir), config.mm0web.as_deref())?;
        if ledger.configured() {
            tracing::info!(backend = ?ledger.base(), "proof publishing is configured");
        } else {
            tracing::info!("proof publishing is not configured (no --mm0web); submissions are refused");
        }
        let ledger = Arc::new(Mutex::new(ledger));
        let ledger_stop = Arc::new(AtomicBool::new(false));
        crate::proofs::spawn_worker(Arc::clone(&ledger), Arc::clone(&ledger_stop));
        Ok(Self {
            engine,
            accounts: Mutex::new(accounts),
            content: Mutex::new(content),
            ledger,
            ledger_stop,
            accounts_path: accounts_file,
            content_path: content_file,
            admin_hash: sha256_hex(admin_token.as_bytes()),
            config,
            started: Instant::now(),
            metrics: Metrics::default(),
            limiter: Mutex::new(RateLimiter::default()),
        })
    }

    /// Stop the proof refresher when the service goes away, so a dropped
    /// service (tests, a restart) leaves no thread polling a dead backend.
    fn stop_ledger_worker(&self) {
        self.ledger_stop.store(true, Ordering::Relaxed);
    }

    pub fn handle(&self, req: HttpRequest) -> HttpResponse {
        self.metrics.requests.fetch_add(1, Ordering::Relaxed);
        let resp = match self.route(&req) {
            Ok(resp) => resp,
            Err(e) => e.to_response(),
        };
        if resp.status >= 400 {
            self.metrics.errors.fetch_add(1, Ordering::Relaxed);
        }
        self.decorate(resp)
    }

    fn decorate(&self, mut resp: HttpResponse) -> HttpResponse {
        resp.headers.push((
            "X-Content-Type-Options".to_string(),
            "nosniff".to_string(),
        ));
        resp.headers
            .push(("Referrer-Policy".to_string(), "no-referrer".to_string()));
        resp.headers.push((
            "Content-Security-Policy".to_string(),
            "default-src 'none'; style-src 'self'; img-src 'self' data:; form-action 'self'; \
             base-uri 'none'; frame-ancestors 'none'"
                .to_string(),
        ));
        resp.headers.push((
            "Permissions-Policy".to_string(),
            "camera=(), microphone=(), geolocation=()".to_string(),
        ));
        if self.config.secure_cookies {
            resp.headers.push((
                "Strict-Transport-Security".to_string(),
                "max-age=31536000".to_string(),
            ));
        }
        resp
    }

    // ── routing ────────────────────────────────────────────────────────

    fn route(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        let path = req.path.trim_end_matches('/');
        let path = if path.is_empty() { "/" } else { path };
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let method = req.method.as_str();

        // Unauthenticated, unthrottled operational endpoints.
        if method == "GET" && path == "/healthz" {
            return self.healthz();
        }
        if method == "GET" && path == "/metrics" {
            return self.metrics_page();
        }
        if method == "GET" && path == "/style.css" {
            return Ok(HttpResponse {
                status: 200,
                content_type: "text/css; charset=utf-8".to_string(),
                body: site::style_css().as_bytes().to_vec(),
                headers: vec![("Cache-Control".to_string(), "public, max-age=300".to_string())],
            });
        }

        // Global per-client throttle for everything else. A missing peer (a
        // local test) is grouped as "local".
        let peer = if req.peer.is_empty() { "local" } else { req.peer.as_str() };
        if !self
            .limiter
            .lock()
            .map(|mut l| l.allow(peer, 240))
            .unwrap_or(false)
        {
            return Err(ApiError::limit("too many requests from this client; slow down"));
        }

        match (method, segs.as_slice()) {
            ("GET", []) => self.site_home(),
            ("GET", ["registry"]) => self.site_registry(),
            ("GET", ["api"]) => self.site_api(),
            ("GET", ["pages", slug]) => self.site_page(slug),
            ("GET", ["proofs"]) => self.site_proofs(),
            ("GET", ["proofs", id]) => self.site_proof(id),
            ("GET", ["admin"]) => self.admin_home(req),
            ("GET", ["admin", "login"]) => Ok(site::login(&self.chrome(req), None).into()),
            ("POST", ["admin", "login"]) => self.admin_login(req),
            ("POST", ["admin", "logout"]) => Ok(self.admin_logout()),
            ("GET", ["admin", "pages", slug]) => self.admin_editor(req, slug),
            ("POST", ["admin", "pages"]) => self.admin_save_page(req),
            ("POST", ["admin", "pages", slug, action]) => self.admin_page_action(req, slug, action),
            ("POST", ["admin", "accounts"]) => self.admin_create_account_form(req),
            ("GET", ["admin", "proofs"]) => self.admin_proofs(req),
            ("POST", ["admin", "proofs"]) => self.admin_publish_proof(req),
            ("GET", ["admin", "proofs", id]) => self.admin_proof_detail(req, id),
            ("POST", ["admin", "proofs", id, "mint"]) => self.admin_mint_from_proof(req, id),
            ("POST", ["admin", "denominations"]) => self.admin_issue_denom_form(req),
            _ if segs.first() == Some(&"api") => self.route_api(req, &segs),
            _ => {
                // An unknown path is a real 404, not a 200 page that happens to
                // say "not found": crawlers, monitors and clients branch on the
                // status code.
                let html = self.with_chrome(|c| site::not_found(c))?;
                Ok(HttpResponse::html(404, html))
            }
        }
    }

    fn route_api(&self, req: &HttpRequest, segs: &[&str]) -> Result<HttpResponse, ApiError> {
        let method = req.method.as_str();
        let path = segs;

        // Public reads.
        if method == "GET" && path == ["api", "v1", "status"] {
            return Ok(HttpResponse::json(200, self.engine.read(Read::Status)?));
        }
        if method == "GET" && path == ["api", "v1", "registry"] {
            let limit = req.query_get("limit").and_then(|v| v.parse().ok()).unwrap_or(200);
            let kind = req.query_get("kind").map(|k| k.to_string());
            return Ok(HttpResponse::json(200, self.engine.read(Read::Registry { limit, kind })?));
        }
        if method == "GET" && path == ["api", "v1", "denominations"] {
            return Ok(HttpResponse::json(200, self.engine.read(Read::Denominations)?));
        }
        if method == "GET" && path == ["api", "v1", "ops"] {
            let limit = req.query_get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
            return Ok(HttpResponse::json(200, self.engine.read(Read::Ops { limit })?));
        }
        if method == "GET" && path.len() == 4 && path[..3] == ["api", "v1", "reserves"] {
            let id = parse_id(path[3]).ok_or_else(|| ApiError::bad_request("reserve id must be 64 hex characters"))?;
            let mut value = self.engine.read(Read::Reserve { id })?;
            let owner = value["owner"].as_str().unwrap_or("").to_string();
            // A reserve balance is private to its owner (or an admin).
            let caller = self.authenticate(req, false)?;
            if !caller.is_admin() && caller.did() != Some(owner.as_str()) {
                return Err(ApiError::unauthorized("this reserve belongs to another account"));
            }
            value["sandbox"] = json!(self.config.sandbox);
            return Ok(HttpResponse::json(200, value));
        }
        if method == "GET" && path.len() == 4 && path[..3] == ["api", "v1", "merchants"] {
            return Ok(HttpResponse::json(
                200,
                self.engine.read(Read::Merchant { did: path[3].to_string() })?,
            ));
        }

        // Account creation (rate-limited separately from the general bucket).
        if method == "POST" && path == ["api", "v1", "accounts"] {
            return self.api_register(req);
        }

        // Proofs are public evidence: the registry can be read without an
        // account, exactly like the certificate registry.
        if method == "GET" && path == ["api", "v1", "proofs"] {
            return self.api_proofs();
        }
        if method == "GET" && path.len() == 4 && path[..3] == ["api", "v1", "proofs"] {
            return self.api_proof(path[3]);
        }

        // Everything below is authenticated. Admin-only endpoints are
        // resolved before an account DID is required, because the bootstrap
        // admin token is not an account token.
        let caller = self.authenticate(req, false)?;

        // Mint a certificate whose provenance is a published proof's DOI — the
        // join between the two ledgers. Admin only, and only an archived proof
        // may name an archive.
        if method == "POST"
            && path.len() == 5
            && path[..3] == ["api", "v1", "proofs"]
            && path[4] == "mint"
        {
            if !caller.is_admin() {
                return Err(ApiError::unauthorized("admin token required"));
            }
            let body = req.json().unwrap_or(Value::Null);
            let out = self.mint_from_proof(
                caller.did().unwrap_or("bootstrap-admin"),
                path[3],
                body["amount"].as_u64().unwrap_or(0),
                body["owner"].as_str().unwrap_or(""),
            )?;
            return Ok(HttpResponse::json(201, out));
        }
        if path.len() > 2 && path[..3] == ["api", "v1", "admin"] {
            return self.route_admin_api(req, path, &caller);
        }
        if method == "GET" && path == ["api", "v1", "audit"] {
            if !caller.is_admin() {
                return Err(ApiError::unauthorized("admin token required"));
            }
            return Ok(HttpResponse::json(200, self.engine.read(Read::Audit)?));
        }

        if method == "GET" && path == ["api", "v1", "me"] {
            return self.api_me(&caller);
        }
        // The admin token may still perform operator actions (confirm a wire);
        // it is attributed to the bootstrap admin DID.
        let did = caller
            .did()
            .unwrap_or("did:unfer:bootstrap-admin")
            .to_string();
        if method == "POST" && path == ["api", "v1", "reserves"] {
            let body = req.json().unwrap_or(Value::Null);
            let reserve = match body["reserve_id"].as_str() {
                Some(s) => parse_id(s).ok_or_else(|| ApiError::bad_request("reserve_id must be 64 hex characters"))?,
                None => default_reserve_id(&did),
            };
            let out = self.engine.call(Cmd::OpenReserve { actor: did.clone(), reserve })?;
            self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
            return Ok(HttpResponse::json(200, out));
        }
        if method == "POST" && path.len() == 5 && path[..3] == ["api", "v1", "reserves"] {
            let id = parse_id(path[3]).ok_or_else(|| ApiError::bad_request("reserve id must be 64 hex characters"))?;
            let amount = self.amount(req)?;
            let action = path[4];
            let out = match action {
                "peg-in" => self.engine.call(Cmd::PegIn { actor: did.clone(), reserve: id, amount })?,
                "withdraw" => self.engine.call(Cmd::Withdraw { actor: did.clone(), reserve: id, amount })?,
                _ => return Err(ApiError::not_found("unknown reserve action")),
            };
            self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
            return Ok(HttpResponse::json(200, out));
        }
        if method == "POST" && path == ["api", "v1", "deposits"] {
            return self.api_deposit(req, &caller);
        }
        if method == "POST" && path == ["api", "v1", "transfers"] {
            return self.api_transfer(req, &caller);
        }
        if method == "POST" && path == ["api", "v1", "peg-outs"] {
            let body = req.json().unwrap_or(Value::Null);
            let amount = self.amount(req)?;
            let bank = body["bank_account"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            if bank.is_empty() || bank.len() > 64 || !bank.chars().all(|c| c.is_ascii_alphanumeric() || " -/:".contains(c)) {
                return Err(ApiError::bad_request("bank_account must be 1..=64 letters, digits, spaces or -/ :"));
            }
            let out = self.engine.call(Cmd::PegOut {
                actor: did.clone(),
                amount,
                bank,
                admin: caller.is_admin(),
            })?;
            self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
            return Ok(HttpResponse::json(200, out));
        }
        if method == "POST" && path.len() == 5 && path[..3] == ["api", "v1", "peg-outs"] {
            let wire = path[3].to_string();
            let action = path[4];
            let out = match action {
                "confirm" => {
                    if !caller.is_admin() {
                        return Err(ApiError::unauthorized("only an operator may confirm a wire"));
                    }
                    self.engine.call(Cmd::ConfirmPegOut { actor: did.clone(), wire, admin: true })?
                }
                "cancel" => self.engine.call(Cmd::CancelPegOut {
                    actor: did.clone(),
                    wire,
                    admin: caller.is_admin(),
                })?,
                _ => return Err(ApiError::not_found("unknown peg-out action")),
            };
            self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
            return Ok(HttpResponse::json(200, out));
        }
        Err(ApiError::not_found("unknown API endpoint"))
    }

    fn route_admin_api(
        &self,
        req: &HttpRequest,
        path: &[&str],
        caller: &Caller,
    ) -> Result<HttpResponse, ApiError> {
        if !caller.is_admin() {
            return Err(ApiError::unauthorized("admin token required"));
        }
        let method = req.method.as_str();
        if method == "GET" && path == ["api", "v1", "admin", "accounts"] {
            let store = self.accounts.lock().map_err(|_| ApiError::internal("accounts lock"))?;
            let rows: Vec<Value> = store
                .accounts
                .iter()
                .map(|a| json!({"did": a.did, "name": a.name, "role": a.role.as_str(), "created_at": a.created_at}))
                .collect();
            return Ok(HttpResponse::json(200, json!({"accounts": rows})));
        }
        if method == "POST" && path == ["api", "v1", "admin", "accounts"] {
            let body = req.json().unwrap_or(Value::Null);
            let name = body["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() || name.len() > 64 {
                return Err(ApiError::bad_request("name must be 1..=64 characters"));
            }
            let role = Role::parse(body["role"].as_str().unwrap_or("customer"))
                .ok_or_else(|| ApiError::bad_request("role must be customer, merchant or admin"))?;
            let (account, token) = {
                let mut store = self.accounts.lock().map_err(|_| ApiError::internal("accounts lock"))?;
                let created = store.create(&name, role);
                store.save(&self.accounts_path).map_err(ApiError::internal)?;
                created
            };
            return Ok(HttpResponse::json(
                201,
                json!({
                    "did": account.did,
                    "name": account.name,
                    "role": account.role.as_str(),
                    "token": token,
                    "warning": "store this token now; it is not shown again",
                }),
            ));
        }
        if method == "POST" && path == ["api", "v1", "admin", "proofs"] {
            let body = req.json().unwrap_or(Value::Null);
            let proof = self.publish_proof(
                caller.did().unwrap_or("bootstrap-admin"),
                body["project"].as_str().unwrap_or("refi"),
                body["title"].as_str().unwrap_or(""),
                body["mm0"].as_str().unwrap_or(""),
                body["mm1"].as_str().unwrap_or(""),
            )?;
            return Ok(HttpResponse::json(submission_status(proof.state), proof.to_json()));
        }
        if method == "POST" && path == ["api", "v1", "admin", "denominations"] {
            let body = req.json().unwrap_or(Value::Null);
            let value = body["value"].as_u64().ok_or_else(|| ApiError::bad_request("value is required"))?;
            let expires_seq = body["expires_seq"].as_u64();
            let out = self.engine.call(Cmd::IssueDenom {
                actor: caller.did().unwrap_or("admin").to_string(),
                value,
                expires_seq,
                admin: true,
            })?;
            self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
            return Ok(HttpResponse::json(200, out));
        }
        if method == "POST" && path == ["api", "v1", "admin", "mint"] {
            let body = req.json().unwrap_or(Value::Null);
            let amount = body["amount"].as_u64().ok_or_else(|| ApiError::bad_request("amount is required"))?;
            let owner = body["owner"].as_str().unwrap_or("").trim().to_string();
            let source = body["source"].as_str().unwrap_or("").trim().to_string();
            if !crate::engine::is_did(&owner) {
                return Err(ApiError::bad_request("owner must be a did:unfer:<64 hex> DID"));
            }
            // Two provenance forms: a UNFCCC order (a registry reference) or
            // a DOI (the archive that holds a published proof). Anything else
            // is not something a certificate may claim.
            let provenance_ok = source.len() <= 200
                && (source.starts_with("unfccc:vc:") || is_doi(&source));
            if !provenance_ok {
                return Err(ApiError::bad_request(
                    "source must be a UNFCCC provenance reference (unfccc:vc:<order>, max 200 \
                     chars) or a DOI (10.<registrant>/<suffix>)",
                ));
            }
            let out = self.engine.call(Cmd::Mint {
                actor: caller.did().unwrap_or("admin").to_string(),
                amount,
                owner,
                source,
                admin: true,
            })?;
            self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
            return Ok(HttpResponse::json(201, out));
        }
        Err(ApiError::not_found("unknown admin endpoint"))
    }

    fn api_register(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        if !self.config.open_registration {
            return Err(ApiError::new(403, "UK-4001", "RegistrationClosed", "self-registration is disabled; ask an operator for an account"));
        }
        let peer = if req.peer.is_empty() { "local" } else { req.peer.as_str() };
        if !self
            .limiter
            .lock()
            .map(|mut l| l.allow(&format!("register:{peer}"), 10))
            .unwrap_or(false)
        {
            return Err(ApiError::limit("too many account registrations from this client"));
        }
        let body = req.json().unwrap_or(Value::Null);
        let name = body["name"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() || name.len() > 64 {
            return Err(ApiError::bad_request("name must be 1..=64 characters"));
        }
        let (account, token) = {
            let mut store = self.accounts.lock().map_err(|_| ApiError::internal("accounts lock"))?;
            let created = store.create(&name, Role::Customer);
            store.save(&self.accounts_path).map_err(ApiError::internal)?;
            created
        };
        Ok(HttpResponse::json(
            201,
            json!({
                "did": account.did,
                "name": account.name,
                "role": account.role.as_str(),
                "token": token,
                "warning": "store this token now; it is not shown again",
                "sandbox": self.config.sandbox,
            }),
        ))
    }

    fn api_me(&self, caller: &Caller) -> Result<HttpResponse, ApiError> {
        let account = caller
            .account()
            .ok_or_else(|| ApiError::unauthorized("this endpoint needs an account token"))?;
        let reserves = self.engine.read(Read::Reserves { owner: account.did.clone() })?;
        let coins = self.engine.read(Read::Coins { owner: account.did.clone() })?;
        let merchant = self.engine.read(Read::Merchant { did: account.did.clone() })?;
        let reserves_vec = reserves["reserves"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                (
                    r["reserve_id"].as_str().unwrap_or("").to_string(),
                    r["balance"].as_u64().unwrap_or(0),
                )
            })
            .collect::<Vec<_>>();
        let mut view = account.public(
            &reserves_vec,
            coins["total"].as_u64().unwrap_or(0),
            merchant["balance"].as_u64().unwrap_or(0),
        );
        view["certificates_detail"] = coins;
        view["sandbox"] = json!(self.config.sandbox);
        Ok(HttpResponse::json(200, view))
    }

    fn api_deposit(&self, req: &HttpRequest, caller: &Caller) -> Result<HttpResponse, ApiError> {
        let account = caller
            .account()
            .ok_or_else(|| ApiError::unauthorized("a customer token is required"))?;
        let body = req.json().unwrap_or(Value::Null);
        let merchant = body["merchant"].as_str().unwrap_or("").trim().to_string();
        if !crate::engine::is_did(&merchant) {
            return Err(ApiError::bad_request("merchant must be a did:unfer:<64 hex> DID"));
        }
        let ids = body["coins"]
            .as_array()
            .ok_or_else(|| ApiError::bad_request("coins must be an array of certificate ids"))?;
        if ids.is_empty() || ids.len() > 16 {
            return Err(ApiError::bad_request("deposit 1..=16 certificates at a time"));
        }
        let mut coins = Vec::with_capacity(ids.len());
        for id in ids {
            let s = id.as_str().ok_or_else(|| ApiError::bad_request("coin ids are strings"))?;
            coins.push(parse_id(s).ok_or_else(|| ApiError::bad_request("coin id must be 64 hex characters"))?);
        }
        // The engine needs the signing seed; keep it out of request bodies and
        // take it from the stored account.
        let seed = hex::decode(&account.secret)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ApiError::internal("the sandbox wallet key is malformed"))?;
        let out = self.engine.call(Cmd::Deposit {
            actor: account.did.clone(),
            secret: seed,
            coins,
            merchant,
        })?;
        self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
        Ok(HttpResponse::json(200, out))
    }

    fn api_transfer(&self, req: &HttpRequest, caller: &Caller) -> Result<HttpResponse, ApiError> {
        let account = caller
            .account()
            .ok_or_else(|| ApiError::unauthorized("an account token is required"))?;
        let body = req.json().unwrap_or(Value::Null);
        let inputs = body["inputs"]
            .as_array()
            .ok_or_else(|| ApiError::bad_request("inputs must be an array of certificate ids"))?;
        if inputs.is_empty() || inputs.len() > 16 {
            return Err(ApiError::bad_request("transfer 1..=16 inputs at a time"));
        }
        let mut ids = Vec::with_capacity(inputs.len());
        for id in inputs {
            let s = id.as_str().ok_or_else(|| ApiError::bad_request("input ids are strings"))?;
            ids.push(parse_id(s).ok_or_else(|| ApiError::bad_request("input id must be 64 hex characters"))?);
        }
        let outputs = body["outputs"]
            .as_array()
            .ok_or_else(|| ApiError::bad_request("outputs must be an array of {amount,to}"))?;
        if outputs.is_empty() || outputs.len() > 16 {
            return Err(ApiError::bad_request("1..=16 outputs"));
        }
        let mut outs = Vec::with_capacity(outputs.len());
        for o in outputs {
            let amount = o["amount"].as_u64().ok_or_else(|| ApiError::bad_request("output amount is required"))?;
            let to = o["to"].as_str().unwrap_or("").to_string();
            if !crate::engine::is_did(&to) {
                return Err(ApiError::bad_request("output recipient must be a did:unfer: DID"));
            }
            outs.push((amount, to));
        }
        let seed = hex::decode(&account.secret)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ApiError::internal("the sandbox wallet key is malformed"))?;
        let out = self.engine.call(Cmd::Transfer {
            actor: account.did.clone(),
            secret: seed,
            inputs: ids,
            outputs: outs,
        })?;
        self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
        Ok(HttpResponse::json(200, out))
    }

    fn amount(&self, req: &HttpRequest) -> Result<u64, ApiError> {
        let body = req.json().unwrap_or(Value::Null);
        body["amount"]
            .as_u64()
            .filter(|a| *a > 0)
            .ok_or_else(|| ApiError::bad_request("amount must be a positive integer"))
    }

    // ── auth ───────────────────────────────────────────────────────────

    fn bearer(req: &HttpRequest) -> Option<&str> {
        if let Some(v) = req.header("authorization") {
            if let Some(rest) = v.strip_prefix("Bearer ") {
                return Some(rest.trim());
            }
        }
        req.header("x-api-key").map(|s| s.trim())
    }

    fn cookie(req: &HttpRequest, name: &str) -> Option<String> {
        let raw = req.header("cookie")?;
        for part in raw.split(';') {
            let part = part.trim();
            if let Some((k, v)) = part.split_once('=') {
                if k.trim() == name {
                    return Some(v.trim().to_string());
                }
            }
        }
        None
    }

    fn token_is_admin(&self, token: &str) -> bool {
        ct_eq(sha256_hex(token.as_bytes()).as_bytes(), self.admin_hash.as_bytes())
    }

    fn authenticate(&self, req: &HttpRequest, allow_cookie: bool) -> Result<Caller, ApiError> {
        let token = Self::bearer(req)
            .map(|s| s.to_string())
            .or_else(|| if allow_cookie { Self::cookie(req, "refi_admin") } else { None })
            .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        if self.token_is_admin(&token) {
            return Ok(Caller::Admin);
        }
        let store = self
            .accounts
            .lock()
            .map_err(|_| ApiError::internal("accounts lock"))?;
        let account = store
            .by_token(&token)
            .cloned()
            .ok_or_else(|| ApiError::unauthorized("unknown token"))?;
        Ok(Caller::Account(account))
    }

    fn require_admin(&self, req: &HttpRequest, form_token: Option<&str>) -> Result<Caller, ApiError> {
        if let Some(t) = form_token {
            if !self.token_is_admin(t) {
                return Err(ApiError::unauthorized("the form's admin token is not valid"));
            }
            return Ok(Caller::Admin);
        }
        let caller = self.authenticate(req, true)?;
        if !caller.is_admin() {
            return Err(ApiError::unauthorized("admin sign-in required"));
        }
        Ok(caller)
    }

    // ── site ───────────────────────────────────────────────────────────

    fn chrome(&self, _req: &HttpRequest) -> Chrome<'_> {
        // The nav list is owned by the content store; `chrome` borrows it only
        // for the duration of a render, so callers take the guard first via
        // `with_content`. This helper exists for pages that need no nav data.
        Chrome {
            pages: &[],
            sandbox: self.config.sandbox,
            base_url: &self.config.base_url,
        }
    }

    fn with_chrome<T>(&self, render: impl FnOnce(&Chrome) -> T) -> Result<T, ApiError> {
        let content = self.content.lock().map_err(|_| ApiError::internal("content lock"))?;
        let pages = content.public_slugs();
        let chrome = Chrome {
            pages: &pages,
            sandbox: self.config.sandbox,
            base_url: &self.config.base_url,
        };
        Ok(render(&chrome))
    }

    fn site_home(&self) -> Result<HttpResponse, ApiError> {
        let status = self.engine.read(Read::Status)?;
        let registry = self.engine.read(Read::Registry { limit: 8, kind: None })?;
        let html = self.with_chrome(|chrome| site::home(chrome, &status, &registry))?;
        Ok(HttpResponse::html(200, html))
    }

    fn site_registry(&self) -> Result<HttpResponse, ApiError> {
        let registry = self.engine.read(Read::Registry { limit: 200, kind: None })?;
        let proofs = self.proof_index();
        let html = self.with_chrome(|chrome| site::registry(chrome, &registry, &proofs))?;
        Ok(HttpResponse::html(200, html))
    }

    /// (doi, id, title) for every archived proof — what the registry needs to
    /// name the proof behind an `attested` certificate. A copy, so rendering
    /// never holds the ledger lock.
    fn proof_index(&self) -> Vec<(String, String, String)> {
        let Ok(ledger) = self.ledger.lock() else {
            return Vec::new();
        };
        ledger
            .list()
            .iter()
            .filter_map(|p| Some((p.doi.clone()?, p.id.clone(), p.title.clone())))
            .collect()
    }

    /// The public page for one proof: the other end of the registry's
    /// "backed by" link. An unknown id is the site's 404 page, not a JSON
    /// error — it is a page a person followed.
    fn site_proof(&self, id: &str) -> Result<HttpResponse, ApiError> {
        let detail = match self.proof_detail(id) {
            Ok(detail) => detail,
            Err(e) if e.status == 404 => {
                let html = self.with_chrome(|c| site::not_found(c))?;
                return Ok(HttpResponse::html(404, html));
            }
            Err(e) => return Err(e),
        };
        let html = self.with_chrome(|chrome| site::proof_page(chrome, &detail))?;
        Ok(HttpResponse::html(200, html))
    }

    fn site_api(&self) -> Result<HttpResponse, ApiError> {
        let html = self.with_chrome(|chrome| site::api_docs(chrome, &self.config.base_url))?;
        Ok(HttpResponse::html(200, html))
    }

    fn site_page(&self, slug: &str) -> Result<HttpResponse, ApiError> {
        let content = self.content.lock().map_err(|_| ApiError::internal("content lock"))?;
        let Some(page) = content.by_slug(slug).filter(|p| p.published) else {
            drop(content);
            let html = self.with_chrome(|c| site::not_found(c))?;
            return Ok(HttpResponse::html(404, html));
        };
        let pages = content.public_slugs();
        let chrome = Chrome {
            pages: &pages,
            sandbox: self.config.sandbox,
            base_url: &self.config.base_url,
        };
        Ok(HttpResponse::html(200, site::page_view(&chrome, page)))
    }

    // ── admin console ──────────────────────────────────────────────────

    fn admin_home(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        let caller = match self.authenticate(req, true) {
            Ok(c) => c,
            Err(_) => {
                let html = self.with_chrome(|c| site::login(c, None))?;
                return Ok(HttpResponse::html(200, html));
            }
        };
        if !caller.is_admin() {
            let html = self.with_chrome(|c| site::login(c, Some("this account is not an admin")))?;
            return Ok(HttpResponse::html(403, html));
        }
        let status = self.engine.read(Read::Status)?;
        let audit = self.engine.read(Read::Audit)?;
        // The forms carry the admin token itself (double-submit against the
        // session cookie), so the console needs the cookie's value.
        let csrf = Self::cookie(req, "refi_admin").unwrap_or_default();
        let content = self.content.lock().map_err(|_| ApiError::internal("content lock"))?;
        let accounts = self.accounts.lock().map_err(|_| ApiError::internal("accounts lock"))?;
        let pages = content.public_slugs();
        let chrome = Chrome {
            pages: &pages,
            sandbox: self.config.sandbox,
            base_url: &self.config.base_url,
        };
        let html = site::admin_dashboard(&chrome, &content.pages, &accounts.accounts, &status, &audit, &csrf);
        Ok(HttpResponse::html(200, html))
    }

    fn admin_login(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        let form = parse_form(&req.body);
        let token = form_get(&form, "token").unwrap_or("");
        let peer = if req.peer.is_empty() { "local" } else { req.peer.as_str() };
        if !self
            .limiter
            .lock()
            .map(|mut l| l.allow(&format!("login:{peer}"), 10))
            .unwrap_or(false)
        {
            return Err(ApiError::limit("too many sign-in attempts"));
        }
        if !self.token_is_admin(token) {
            let html = self.with_chrome(|c| site::login(c, Some("invalid admin token")))?;
            return Ok(HttpResponse::html(401, html));
        }
        let mut resp = HttpResponse::html(303, String::new()).with_header("Location", "/admin");
        let mut cookie = format!(
            "refi_admin={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200"
        );
        if self.config.secure_cookies {
            cookie.push_str("; Secure");
        }
        resp.headers.push(("Set-Cookie".to_string(), cookie));
        Ok(resp)
    }

    fn admin_logout(&self) -> HttpResponse {
        HttpResponse::html(303, String::new())
            .with_header("Location", "/")
            .with_header("Set-Cookie", "refi_admin=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
    }

    fn admin_editor(&self, req: &HttpRequest, slug: &str) -> Result<HttpResponse, ApiError> {
        self.require_admin(req, None)?;
        let content = self.content.lock().map_err(|_| ApiError::internal("content lock"))?;
        let page = if slug == "new" { None } else { content.by_slug(slug) };
        if slug != "new" && page.is_none() {
            return Err(ApiError::not_found("no such page"));
        }
        let csrf = Self::cookie(req, "refi_admin").unwrap_or_default();
        let pages = content.public_slugs();
        let chrome = Chrome {
            pages: &pages,
            sandbox: self.config.sandbox,
            base_url: &self.config.base_url,
        };
        Ok(HttpResponse::html(
            200,
            site::editor(&chrome, page, &csrf, None),
        ))
    }

    fn admin_save_page(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        let form = parse_form(&req.body);
        let token = form_get(&form, "token").unwrap_or("");
        let caller = self.require_admin(req, Some(token))?;
        let title = form_get(&form, "title").unwrap_or("").trim().to_string();
        if title.is_empty() || title.len() > 160 {
            return Err(ApiError::bad_request("title must be 1..=160 characters"));
        }
        let body = form_get(&form, "body").unwrap_or("").to_string();
        if body.len() > 100_000 {
            return Err(ApiError::bad_request("page body is limited to 100 kB"));
        }
        let slug = form_get(&form, "slug").unwrap_or("").trim().to_string();
        let published = form_get(&form, "published").is_some();
        let author = caller
            .did()
            .map(|d| d.to_string())
            .unwrap_or_else(|| "admin".to_string());
        let mut content = self.content.lock().map_err(|_| ApiError::internal("content lock"))?;
        let saved = content.put(
            if slug.is_empty() { None } else { Some(&slug) },
            &title,
            &body,
            published,
            &author,
        );
        content.save(&self.content_path).map_err(ApiError::internal)?;
        Ok(HttpResponse::html(303, String::new())
            .with_header("Location", &format!("/admin/pages/{}", saved.slug)))
    }

    fn admin_page_action(&self, req: &HttpRequest, slug: &str, action: &str) -> Result<HttpResponse, ApiError> {
        let form = parse_form(&req.body);
        let token = form_get(&form, "token").unwrap_or("");
        self.require_admin(req, Some(token))?;
        let mut content = self.content.lock().map_err(|_| ApiError::internal("content lock"))?;
        match action {
            "publish" => {
                let Some(page) = content.pages.iter_mut().find(|p| p.slug == slug) else {
                    return Err(ApiError::not_found("no such page"));
                };
                page.published = !page.published;
                page.updated_at = now_unix();
                page.revision += 1;
            }
            "delete" => {
                if !content.delete(slug) {
                    return Err(ApiError::not_found("no such page"));
                }
            }
            _ => return Err(ApiError::not_found("unknown page action")),
        }
        content.save(&self.content_path).map_err(ApiError::internal)?;
        Ok(HttpResponse::html(303, String::new()).with_header("Location", "/admin"))
    }

    fn admin_create_account_form(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        let form = parse_form(&req.body);
        let token = form_get(&form, "token").unwrap_or("");
        self.require_admin(req, Some(token))?;
        let name = form_get(&form, "name").unwrap_or("").trim().to_string();
        if name.is_empty() || name.len() > 64 {
            return Err(ApiError::bad_request("name must be 1..=64 characters"));
        }
        let role = Role::parse(form_get(&form, "role").unwrap_or("customer"))
            .ok_or_else(|| ApiError::bad_request("unknown role"))?;
        let (account, account_token) = {
            let mut store = self.accounts.lock().map_err(|_| ApiError::internal("accounts lock"))?;
            let created = store.create(&name, role);
            store.save(&self.accounts_path).map_err(ApiError::internal)?;
            created
        };
        let html = format!(
            "<section class=\"panel\"><div class=\"panel-head\"><h2>Account created</h2>\
             <a class=\"small\" href=\"/admin\">← back</a></div>\
             <p>Give this token to <strong>{name}</strong> now — it is shown once.</p>\
             <pre><code>{token}</code></pre>\
             <div class=\"kv\"><div><span>DID</span><code>{did}</code></div>\
             <div><span>Role</span><code>{role}</code></div></div></section>",
            name = escape_html(&account.name),
            token = escape_html(&account_token),
            did = escape_html(&account.did),
            role = account.role.as_str(),
        );
        // A bare styled page is fine here: the token is one-time and the page
        // is not navigable afterwards.
        Ok(HttpResponse::html(200, format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Account created</title>\
             <link rel=\"stylesheet\" href=\"/style.css\"></head><body><main>{html}\
             <p><a href=\"/admin\">Back to admin</a></p></main></body></html>"
        )))
    }

    fn admin_issue_denom_form(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        let form = parse_form(&req.body);
        let token = form_get(&form, "token").unwrap_or("");
        self.require_admin(req, Some(token))?;
        let value = form_get(&form, "value")
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .ok_or_else(|| ApiError::bad_request("value must be a positive integer"))?;
        let expires = form_get(&form, "expires_seq").and_then(|v| v.parse::<u64>().ok());
        self.engine.call(Cmd::IssueDenom {
            actor: "admin".to_string(),
            value,
            expires_seq: expires,
            admin: true,
        })?;
        self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
        Ok(HttpResponse::html(303, String::new()).with_header("Location", "/admin"))
    }

    // ── proofs ─────────────────────────────────────────────────────────

    /// Validate a development and hand it to `mm0web`. Shared by the admin API
    /// and the console form; only the response shape differs.
    fn publish_proof(
        &self,
        actor: &str,
        project: &str,
        title: &str,
        mm0: &str,
        mm1: &str,
    ) -> Result<Proof, ApiError> {
        let title = title.trim();
        let project = project.trim();
        let project = if project.is_empty() { "refi" } else { project };
        if title.is_empty() || title.len() > 200 {
            return Err(ApiError::bad_request("title must be 1..=200 characters"));
        }
        if project.len() > 64 {
            return Err(ApiError::bad_request("project must be at most 64 characters"));
        }
        if mm0.trim().is_empty() {
            return Err(ApiError::bad_request("mm0 spec is required"));
        }
        if mm0.len() > MAX_FIELD || mm1.len() > MAX_FIELD {
            return Err(ApiError::new(
                413,
                "UK-1003",
                "PayloadTooLarge",
                "each of mm0 and mm1 must be at most 256 kB",
            ));
        }
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| ApiError::internal("proof ledger lock"))?;
        if !ledger.configured() {
            return Err(ApiError::unavailable(
                "proof publishing is not configured; start the service with --mm0web <url>",
            ));
        }
        let proof = ledger
            .publish(actor, project, title, mm0, mm1)
            .map_err(|e| ApiError::new(502, "UK-5002", "ProofBackend", e))?;
        drop(ledger);
        self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
        Ok(proof)
    }

    fn api_proofs(&self) -> Result<HttpResponse, ApiError> {
        let ledger = self
            .ledger
            .lock()
            .map_err(|_| ApiError::internal("proof ledger lock"))?;
        let (published, pending, failed) = ledger.counts();
        let body = json!({
            "configured": ledger.configured(),
            "backend": ledger.base(),
            "counts": {"published": published, "pending": pending, "failed": failed},
            "proofs": ledger.list().iter().map(|p| p.to_json()).collect::<Vec<_>>(),
        });
        Ok(HttpResponse::json(200, body))
    }

    fn api_proof(&self, id: &str) -> Result<HttpResponse, ApiError> {
        let detail = self.proof_detail(id)?;
        Ok(HttpResponse::json(200, detail.to_json()))
    }

    /// The proof, the backend it ran on, and every certificate minted against
    /// its DOI — one read for the detail view.
    fn proof_detail(&self, id: &str) -> Result<crate::proofs::ProofDetail, ApiError> {
        let (proof, backend) = {
            let ledger = self
                .ledger
                .lock()
                .map_err(|_| ApiError::internal("proof ledger lock"))?;
            (ledger.by_id(id).cloned(), ledger.base())
        };
        let Some(proof) = proof else {
            return Err(ApiError::not_found("no such proof"));
        };
        let certificates = proof
            .doi
            .as_deref()
            .map(|doi| self.certificates_for(doi))
            .unwrap_or_default();
        Ok(proof.detail(certificates, backend))
    }

    /// Registry rows whose provenance is exactly this proof's DOI.
    fn certificates_for(&self, doi: &str) -> Vec<Value> {
        let Ok(registry) = self.engine.read(Read::Registry { limit: 500, kind: None }) else {
            return Vec::new();
        };
        registry["certificates"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c["source"].as_str() == Some(doi))
            .collect()
    }

    /// Mint a certificate whose provenance is this proof's DOI. The policy
    /// kernel still gates it (admin role, amount bounds, post-op audit); what
    /// this adds is the join: the certificate cites the archive that holds the
    /// proof.
    fn mint_from_proof(
        &self,
        actor: &str,
        id: &str,
        amount: u64,
        owner: &str,
    ) -> Result<Value, ApiError> {
        let proof = {
            let ledger = self
                .ledger
                .lock()
                .map_err(|_| ApiError::internal("proof ledger lock"))?;
            ledger.by_id(id).cloned()
        }
        .ok_or_else(|| ApiError::not_found("no such proof"))?;
        if !proof.can_mint() {
            return Err(ApiError::bad_request(
                "only a published proof with a DOI can back a certificate",
            ));
        }
        if !crate::engine::is_did(owner) {
            return Err(ApiError::bad_request("owner must be a did:unfer:<64 hex> DID"));
        }
        if amount == 0 {
            return Err(ApiError::bad_request("amount must be positive"));
        }
        let doi = proof.doi.clone().unwrap_or_default();
        let out = self.engine.call(Cmd::Mint {
            actor: actor.to_string(),
            amount,
            owner: owner.to_string(),
            source: doi,
            admin: true,
        })?;
        self.metrics.mutations.fetch_add(1, Ordering::Relaxed);
        Ok(out)
    }

    /// A copy of the ledger's public face: the refresher thread also holds this
    /// mutex, so views never keep it while rendering (a slow render must not
    /// postpone a DOI arriving).
    fn ledger_view(&self) -> Result<(Vec<Proof>, (usize, usize, usize), bool, Option<String>), ApiError> {
        let ledger = self
            .ledger
            .lock()
            .map_err(|_| ApiError::internal("proof ledger lock"))?;
        Ok((
            ledger.list().to_vec(),
            ledger.counts(),
            ledger.configured(),
            ledger.base(),
        ))
    }

    fn site_proofs(&self) -> Result<HttpResponse, ApiError> {
        let (proofs, counts, configured, backend) = self.ledger_view()?;
        let html = self.with_chrome(|chrome| {
            site::proofs(chrome, &proofs, counts, configured, backend.as_deref())
        })?;
        Ok(HttpResponse::html(200, html))
    }

    fn admin_proofs(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        let caller = match self.authenticate(req, true) {
            Ok(c) => c,
            Err(_) => {
                let html = self.with_chrome(|c| site::login(c, None))?;
                return Ok(HttpResponse::html(200, html));
            }
        };
        if !caller.is_admin() {
            let html = self.with_chrome(|c| site::login(c, Some("this account is not an admin")))?;
            return Ok(HttpResponse::html(403, html));
        }
        let csrf = Self::cookie(req, "refi_admin").unwrap_or_default();
        let (proofs, counts, configured, backend) = self.ledger_view()?;
        let html = self.with_chrome(|chrome| {
            site::admin_proofs(chrome, &proofs, counts, configured, backend.as_deref(), &csrf)
        })?;
        Ok(HttpResponse::html(200, html))
    }

    fn admin_publish_proof(&self, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
        let form = parse_form(&req.body);
        let token = form_get(&form, "token").unwrap_or("");
        self.require_admin(req, Some(token))?;
        let proof = self.publish_proof(
            "bootstrap-admin",
            form_get(&form, "project").unwrap_or("refi"),
            form_get(&form, "title").unwrap_or(""),
            form_get(&form, "mm0").unwrap_or(""),
            form_get(&form, "mm1").unwrap_or(""),
        )?;
        let html = self.with_chrome(|chrome| site::proof_result(chrome, &proof))?;
        Ok(HttpResponse::html(200, html))
    }

    /// One proof in the console: what the checkers said, where the bytes are
    /// archived, and what has been minted against them.
    fn admin_proof_detail(&self, req: &HttpRequest, id: &str) -> Result<HttpResponse, ApiError> {
        let caller = match self.authenticate(req, true) {
            Ok(c) => c,
            Err(_) => {
                let html = self.with_chrome(|c| site::login(c, None))?;
                return Ok(HttpResponse::html(200, html));
            }
        };
        if !caller.is_admin() {
            let html = self.with_chrome(|c| site::login(c, Some("this account is not an admin")))?;
            return Ok(HttpResponse::html(403, html));
        }
        let csrf = Self::cookie(req, "refi_admin").unwrap_or_default();
        let detail = self.proof_detail(id)?;
        let html =
            self.with_chrome(|chrome| site::admin_proof_detail(chrome, &detail, &csrf))?;
        Ok(HttpResponse::html(200, html))
    }

    /// The console's mint form: mint, then land back on the detail page, where
    /// the new certificate is already in the list (the proof's own section is
    /// the confirmation — no second page to render).
    fn admin_mint_from_proof(&self, req: &HttpRequest, id: &str) -> Result<HttpResponse, ApiError> {
        let form = parse_form(&req.body);
        let token = form_get(&form, "token").unwrap_or("");
        self.require_admin(req, Some(token))?;
        let amount = form_get(&form, "amount")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let owner = form_get(&form, "owner").unwrap_or("");
        self.mint_from_proof("bootstrap-admin", id, amount, owner)?;
        Ok(HttpResponse::html(303, String::new()).with_header(
            "Location",
            &format!("/admin/proofs/{id}"),
        ))
    }

    // ── operational ────────────────────────────────────────────────────

    fn healthz(&self) -> Result<HttpResponse, ApiError> {
        let audit = self.engine.read(Read::Audit)?;
        let ok = audit["ok"] == true;
        let body = json!({
            "status": if ok { "ok" } else { "degraded" },
            "audit": if ok { "ok" } else { "failed" },
            "policy": "loaded",
            "uptime_s": self.started.elapsed().as_secs(),
            "sandbox": self.config.sandbox,
        });
        Ok(HttpResponse::json(if ok { 200 } else { 503 }, body))
    }

    fn metrics_page(&self) -> Result<HttpResponse, ApiError> {
        let status = self.engine.read(Read::Status).unwrap_or(Value::Null);
        let accounts = self.accounts.lock().map(|a| a.count()).unwrap_or(0);
        let text = format!(
            "# HELP refi_requests_total HTTP requests handled\n\
             # TYPE refi_requests_total counter\n\
             refi_requests_total {}\n\
             # HELP refi_errors_total responses with status >= 400\n\
             # TYPE refi_errors_total counter\n\
             refi_errors_total {}\n\
             # HELP refi_mutations_total accepted state transitions\n\
             # TYPE refi_mutations_total counter\n\
             refi_mutations_total {}\n\
             # HELP refi_uptime_seconds process uptime\n\
             # TYPE refi_uptime_seconds gauge\n\
             refi_uptime_seconds {}\n\
             # HELP refi_accounts registered accounts\n\
             # TYPE refi_accounts gauge\n\
             refi_accounts {}\n\
             # HELP refi_ledger_supply certificates in circulation\n\
             # TYPE refi_ledger_supply gauge\n\
             refi_ledger_supply {}\n\
             # HELP refi_reserves_total reserve liabilities\n\
             # TYPE refi_reserves_total gauge\n\
             refi_reserves_total {}\n\
             # HELP refi_merchants_total merchant fiat balances\n\
             # TYPE refi_merchants_total gauge\n\
             refi_merchants_total {}\n\
             # HELP refi_policy_loaded Austral policy kernel loaded\n\
             # TYPE refi_policy_loaded gauge\n\
             refi_policy_loaded 1\n",
            self.metrics.requests.load(Ordering::Relaxed),
            self.metrics.errors.load(Ordering::Relaxed),
            self.metrics.mutations.load(Ordering::Relaxed),
            self.started.elapsed().as_secs(),
            accounts,
            status["ledger"]["total_supply"].as_u64().unwrap_or(0),
            status["reserves_total"].as_u64().unwrap_or(0),
            status["merchants_total"].as_u64().unwrap_or(0),
        );
        Ok(HttpResponse::text(200, text))
    }
}

impl From<String> for HttpResponse {
    fn from(html: String) -> Self {
        HttpResponse::html(200, html)
    }
}

/// A convenience for tests: the paths a caller may use to probe the service.
pub fn state_paths(state_dir: &Path) -> (PathBuf, PathBuf) {
    (accounts_path(state_dir), content_path(state_dir))
}

/// The status a submission answers with, so a caller can tell "archived" from
/// "queued" from "refused" without reading the body.
fn submission_status(state: State) -> u16 {
    match state {
        State::Published => 200,
        State::Queued | State::Verified => 202,
        State::Rejected => 422,
        State::Failed => 502,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_refills_each_minute() {
        let mut l = RateLimiter::default();
        for _ in 0..3 {
            assert!(l.allow("ip", 3));
        }
        assert!(!l.allow("ip", 3));
        assert!(l.allow("other", 3));
    }

    #[test]
    fn tokens_are_admin_only_when_hashed_equal() {
        let hash = sha256_hex(b"secret");
        assert!(ct_eq(sha256_hex(b"secret").as_bytes(), hash.as_bytes()));
        assert!(!ct_eq(sha256_hex(b"secreu").as_bytes(), hash.as_bytes()));
    }
}
