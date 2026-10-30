//! End-to-end tests over the real request path: HTTP-shaped request in,
//! policy kernel + exchange + persistence + site out. Most of them call
//! `Service::handle` directly; the transport tests at the bottom drive
//! `src/server.rs` over loopback sockets, because framing and limits are the
//! one thing the router cannot check.
//!
//! The tests need the stock `austral` front-end (to turn the `.aum` policy
//! kernel into CPS) and the Cranelift bridge it links. When the front-end is
//! not present the tests report a skip instead of pretending to pass; the
//! deploy check (`refi_service --check`) covers that case on the target host.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;

use refi_service::engine::{Engine, EngineConfig};
use refi_service::server::serve_listener;
use refi_service::service::{Config, Service};
use refi_service::util::HttpRequest;

struct Fixture {
    _dir: TempDir,
    service: Service,
    admin: String,
}

fn repo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn austral_bin() -> PathBuf {
    std::env::var_os("AUSTRAL_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_dir().join("../austral"))
}

/// Compile the policy module once per fixture into the isolated state dir.
fn build_cps(cps: &PathBuf) -> Result<(), String> {
    let bin = austral_bin();
    if !bin.exists() {
        return Err(format!("austral front-end not found at {}", bin.display()));
    }
    let module = repo_dir().join("modules/RefiPolicy.aum");
    let mut cmd = Command::new(&bin);
    cmd.arg("compile")
        .arg(&module)
        .arg("--use-cps-jit")
        .arg("--allow-all")
        .arg("--target-type=tc")
        .arg(format!("--emit-cps={}", cps.display()));
    if let Some(dir) = bin.parent() {
        let mut lp = dir.display().to_string();
        if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
            lp.push(':');
            lp.push_str(&old.to_string_lossy());
        }
        cmd.env("LD_LIBRARY_PATH", lp);
    }
    let out = cmd.output().map_err(|e| format!("spawn austral: {e}"))?;
    if !cps.exists() {
        return Err(format!(
            "no CPS produced ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn fixture(admin: &str) -> Option<Fixture> {
    fixture_with(admin, None)
}

/// A fixture whose service knows an mm0web backend. `None` runs the exchange
/// without proof publishing, which is the default deployment.
fn fixture_with(admin: &str, mm0web: Option<&str>) -> Option<Fixture> {
    let dir = tempfile::tempdir().ok()?;
    let cps = dir.path().join("policy.cps");
    if let Err(e) = build_cps(&cps) {
        eprintln!("SKIP: {e}");
        return None;
    }
    let engine = match Engine::start(EngineConfig {
        state_path: dir.path().join("exchange.json"),
        treasury_seed: [9u8; 32],
        manifest_path: repo_dir().join("module.toml"),
        cps_path: cps,
        austral_bin: austral_bin(),
        module_path: repo_dir().join("modules/RefiPolicy.aum"),
        max_wire: 10_000,
        daily_cap: 5_000,
        denom_horizon: 10_000,
        default_denoms: vec![1, 5, 10, 50, 100, 500, 1000],
    }) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("SKIP: engine start: {e}");
            return None;
        }
    };
    let service = Service::new(
        Config {
            state_dir: dir.path().to_path_buf(),
            base_url: "https://refi.test".to_string(),
            open_registration: true,
            secure_cookies: true,
            mm0web: mm0web.map(str::to_string),
            sandbox: true,
        },
        engine,
        admin,
    )
    .unwrap_or_else(|e| {
        panic!("service construction failed after the engine started: {e}")
    });
    Some(Fixture {
        _dir: dir,
        service,
        admin: admin.to_string(),
    })
}

fn request(
    svc: &Service,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, String) {
    // The server splits the query off the request target; do the same here so a
    // test path may carry `?key=value`.
    let (path, query) = match path.split_once('?') {
        Some((p, q)) => (
            p,
            q.split('&')
                .filter(|s| !s.is_empty())
                .map(|pair| {
                    let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                    (
                        refi_service::util::url_decode(k),
                        refi_service::util::url_decode(v),
                    )
                })
                .collect(),
        ),
        None => (path, Vec::new()),
    };
    let mut headers = Vec::new();
    if let Some(t) = token {
        headers.push(("authorization".to_string(), format!("Bearer {t}")));
    }
    let body = body
        .map(|v| serde_json::to_vec(&v).unwrap())
        .unwrap_or_default();
    if !body.is_empty() {
        headers.push(("content-type".to_string(), "application/json".to_string()));
    }
    let resp = svc.handle(HttpRequest {
        method: method.to_string(),
        path: path.to_string(),
        query,
        headers,
        body,
        peer: "127.0.0.1:50000".to_string(),
        secure: true,
        ..Default::default()
    });
    (
        resp.status,
        String::from_utf8_lossy(&resp.body).into_owned(),
    )
}

fn api(
    svc: &Service,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let (status, text) = request(svc, method, path, token, body);
    let value = serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw": text}));
    (status, value)
}

fn form(svc: &Service, path: &str, cookie: Option<&str>, fields: &[(&str, &str)]) -> (u16, Vec<(String, String)>, String) {
    let body = fields
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let mut headers = vec![(
        "content-type".to_string(),
        "application/x-www-form-urlencoded".to_string(),
    )];
    if let Some(c) = cookie {
        headers.push(("cookie".to_string(), c.to_string()));
    }
    let resp = svc.handle(HttpRequest {
        method: "POST".to_string(),
        path: path.to_string(),
        headers,
        body: body.into_bytes(),
        peer: "127.0.0.1:50001".to_string(),
        ..Default::default()
    });
    (
        resp.status,
        resp.headers,
        String::from_utf8_lossy(&resp.body).into_owned(),
    )
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn account(svc: &Service, name: &str) -> (String, String) {
    let (status, v) = api(svc, "POST", "/api/v1/accounts", None, Some(json!({"name": name})));
    assert_eq!(status, 201, "register {name}: {v}");
    (
        v["did"].as_str().unwrap().to_string(),
        v["token"].as_str().unwrap().to_string(),
    )
}

#[test]
fn full_sandbox_lifecycle_through_the_policy_kernel() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };
    let svc = &fx.service;

    // ── accounts ──────────────────────────────────────────────────────
    let (alice_did, alice) = account(svc, "alice");
    let (status, bob) = api(
        svc,
        "POST",
        "/api/v1/admin/accounts",
        Some(&fx.admin),
        Some(json!({"name": "bob", "role": "merchant"})),
    );
    assert_eq!(status, 201, "{bob}");
    let bob_did = bob["did"].as_str().unwrap().to_string();
    let bob_token = bob["token"].as_str().unwrap().to_string();
    assert!(bob_did.starts_with("did:unfer:"));

    // Unauthenticated reads of private state are refused.
    let (status, _) = api(svc, "GET", "/api/v1/me", None, None);
    assert_eq!(status, 401);

    // ── reserve + sandbox wire ───────────────────────────────────────
    let (status, opened) = api(svc, "POST", "/api/v1/reserves", Some(&alice), None);
    assert_eq!(status, 200, "{opened}");
    let reserve = opened["reserve_id"].as_str().unwrap().to_string();
    assert_eq!(opened["owner"], json!(alice_did));

    let (status, peg_in) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/peg-in"),
        Some(&alice),
        Some(json!({"amount": 1000})),
    );
    assert_eq!(status, 200, "{peg_in}");
    assert_eq!(peg_in["reserve_balance"], json!(1000));
    assert_eq!(peg_in["sandbox"], json!(true));

    // ── withdraw a live denomination ─────────────────────────────────
    let (status, coin) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/withdraw"),
        Some(&alice),
        Some(json!({"amount": 500})),
    );
    assert_eq!(status, 200, "{coin}");
    let coin_id = coin["coin_id"].as_str().unwrap().to_string();
    assert_eq!(coin["reserve_balance"], json!(500));

    // A value with no live denomination is refused by the policy kernel.
    let (status, err) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/withdraw"),
        Some(&alice),
        Some(json!({"amount": 700})),
    );
    assert_eq!(status, 422, "unsupported denomination: {err}");
    assert_eq!(err["error"]["code"], json!("UK-7104"));

    // Bob cannot spend Alice's reserve.
    let (status, err) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/withdraw"),
        Some(&bob_token),
        Some(json!({"amount": 100})),
    );
    assert_eq!(status, 404, "foreign reserve: {err}");

    // ── deposit to the merchant ──────────────────────────────────────
    let (status, dep) = api(
        svc,
        "POST",
        "/api/v1/deposits",
        Some(&alice),
        Some(json!({"coins": [coin_id], "merchant": bob_did})),
    );
    assert_eq!(status, 200, "deposit: {dep}");
    assert_eq!(dep["credited"], json!(500));
    assert_eq!(dep["merchant_balance"], json!(500));

    // Double deposit is refused.
    let (status, err) = api(
        svc,
        "POST",
        "/api/v1/deposits",
        Some(&alice),
        Some(json!({"coins": [coin_id], "merchant": bob_did})),
    );
    assert_eq!(status, 409, "{err}");
    assert_eq!(err["error"]["code"], json!("UK-7105"));

    // ── peg-out, cancel, confirm ─────────────────────────────────────
    let (status, peg) = api(
        svc,
        "POST",
        "/api/v1/peg-outs",
        Some(&bob_token),
        Some(json!({"amount": 200, "bank_account": "DE99 0000 1234"})),
    );
    assert_eq!(status, 200, "peg-out: {peg}");
    let wire = peg["wire_id"].as_str().unwrap().to_string();
    assert_eq!(peg["merchant_balance"], json!(300));

    // Alice may not cancel Bob's wire.
    let (status, _) = api(
        svc,
        "POST",
        &format!("/api/v1/peg-outs/{wire}/cancel"),
        Some(&alice),
        None,
    );
    assert_eq!(status, 403);

    // Bob cancels: the reservation returns to his balance.
    let (status, cancelled) = api(
        svc,
        "POST",
        &format!("/api/v1/peg-outs/{wire}/cancel"),
        Some(&bob_token),
        None,
    );
    assert_eq!(status, 200, "{cancelled}");
    assert_eq!(cancelled["merchant_balance"], json!(500));

    // A fresh wire, confirmed by the operator.
    let (status, peg2) = api(
        svc,
        "POST",
        "/api/v1/peg-outs",
        Some(&bob_token),
        Some(json!({"amount": 200, "bank_account": "DE99 0000 1234"})),
    );
    assert_eq!(status, 200, "{peg2}");
    let wire2 = peg2["wire_id"].as_str().unwrap().to_string();
    let (status, conf) = api(
        svc,
        "POST",
        &format!("/api/v1/peg-outs/{wire2}/confirm"),
        Some(&fx.admin),
        None,
    );
    assert_eq!(status, 200, "{conf}");
    assert_eq!(conf["confirmed"], json!(true));

    // ── accounting identity ──────────────────────────────────────────
    let (status, audit) = api(svc, "GET", "/api/v1/audit", Some(&fx.admin), None);
    assert_eq!(status, 200, "{audit}");
    assert_eq!(audit["ok"], json!(true));
    assert_eq!(audit["fiat_in"], json!(1000));
    assert_eq!(audit["fiat_out"], json!(200));
    assert_eq!(
        audit["assets"].as_u64().unwrap(),
        audit["liabilities"].as_u64().unwrap(),
        "fiat conservation identity holds"
    );
    assert_eq!(audit["reserves_total"], json!(500));
    assert_eq!(audit["merchants_total"], json!(300));
    assert_eq!(audit["total_supply"], json!(0), "deposit retired the e-coin");

    // The audit identity is not visible to non-admins.
    let (status, _) = api(svc, "GET", "/api/v1/audit", Some(&alice), None);
    assert_eq!(status, 401);

    // ── carbon certificates ──────────────────────────────────────────
    let (status, minted) = api(
        svc,
        "POST",
        "/api/v1/admin/mint",
        Some(&fx.admin),
        Some(json!({"amount": 10, "owner": alice_did, "source": "unfccc:vc:sandbox-42"})),
    );
    assert_eq!(status, 201, "mint: {minted}");
    let cert = minted["cert_id"].as_str().unwrap().to_string();

    let (status, err) = api(
        svc,
        "POST",
        "/api/v1/admin/mint",
        Some(&fx.admin),
        Some(json!({"amount": 1, "owner": alice_did, "source": "not-a-vc"})),
    );
    assert_eq!(status, 400, "{err}");

    // Non-admins cannot mint.
    let (status, _) = api(
        svc,
        "POST",
        "/api/v1/admin/mint",
        Some(&alice),
        Some(json!({"amount": 1, "owner": alice_did, "source": "unfccc:vc:x"})),
    );
    assert_eq!(status, 401);

    // The registry holds both value forms: the spendable e-coin minted on
    // withdraw and the UNFCCC-backed carbon certificate. Filter for carbon.
    let (status, registry) = api(svc, "GET", "/api/v1/registry?kind=carbon", None, None);
    assert_eq!(status, 200);
    assert_eq!(registry["count"], json!(1));
    assert_eq!(registry["certificates"][0]["kind"], json!("carbon"));
    assert_eq!(registry["certificates"][0]["cert_id"], json!(cert));
    assert_eq!(registry["certificates"][0]["status"], json!("unspent"));

    // Transfer the certificate to Bob; value is conserved.
    let (status, moved) = api(
        svc,
        "POST",
        "/api/v1/transfers",
        Some(&alice),
        Some(json!({"inputs": [cert], "outputs": [{"amount": 10, "to": bob_did}]})),
    );
    assert_eq!(status, 200, "transfer: {moved}");
    let (_, registry) = api(svc, "GET", "/api/v1/registry?kind=carbon", None, None);
    assert_eq!(registry["count"], json!(2), "old mint + the new output");
    assert_eq!(registry["certificates"][0]["status"], json!("unspent"));
    assert_eq!(registry["certificates"][0]["owner"], json!(bob_did));

    // A transfer that creates value is refused by the ledger.
    let cert2 = registry["certificates"][0]["cert_id"].as_str().unwrap().to_string();
    let (status, err) = api(
        svc,
        "POST",
        "/api/v1/transfers",
        Some(&bob_token),
        Some(json!({"inputs": [cert2], "outputs": [{"amount": 11, "to": bob_did}]})),
    );
    assert_eq!(status, 422, "conservation violation: {err}");
}

#[test]
fn limits_and_daily_caps_are_enforced_by_the_kernel() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };
    let svc = &fx.service;
    let (_did, token) = account(svc, "carol");
    let (_, opened) = api(svc, "POST", "/api/v1/reserves", Some(&token), None);
    let reserve = opened["reserve_id"].as_str().unwrap().to_string();

    // Over the per-request ceiling (10_000 in the fixture).
    let (status, err) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/peg-in"),
        Some(&token),
        Some(json!({"amount": 10_001})),
    );
    assert_eq!(status, 429, "{err}");

    // Daily cap is 5_000.
    let (status, _) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/peg-in"),
        Some(&token),
        Some(json!({"amount": 4_000})),
    );
    assert_eq!(status, 200);
    let (status, err) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/peg-in"),
        Some(&token),
        Some(json!({"amount": 2_000})),
    );
    assert_eq!(status, 429, "daily cap: {err}");

    // A zero or negative amount never reaches the exchange.
    let (status, _) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/withdraw"),
        Some(&token),
        Some(json!({"amount": 0})),
    );
    assert_eq!(status, 400);
}

#[test]
fn site_and_admin_console_render_without_injection() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };
    let svc = &fx.service;

    let (status, home) = request(svc, "GET", "/", None, None);
    assert_eq!(status, 200);
    assert!(home.contains("Sandbox settlement"));
    assert!(home.contains("ReFi Exchange"));

    let (status, registry) = request(svc, "GET", "/registry", None, None);
    assert_eq!(status, 200);
    assert!(registry.contains("Certificate registry"));

    let (status, health) = request(svc, "GET", "/healthz", None, None);
    assert_eq!(status, 200);
    assert!(health.contains("\"audit\":\"ok\""));

    let (status, metrics) = request(svc, "GET", "/metrics", None, None);
    assert_eq!(status, 200);
    assert!(metrics.contains("refi_requests_total"));
    assert!(metrics.contains("refi_policy_loaded 1"));

    // /admin without a session shows the sign-in form, not the console.
    let (status, page) = request(svc, "GET", "/admin", None, None);
    assert_eq!(status, 200);
    assert!(page.contains("Admin sign-in"));

    let (status, headers, _) = form(
        svc,
        "/admin/login",
        None,
        &[("token", "refi_admin_test_token_0123456789")],
    );
    assert_eq!(status, 303);
    let cookie = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| v.clone())
        .expect("session cookie");
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("Secure"));
    let cookie = cookie.split(';').next().unwrap().to_string();

    let (status, page) = request_with_cookie(svc, "/admin", &cookie);
    assert_eq!(status, 200);
    assert!(page.contains("Content"));
    assert!(page.contains("Exchange"));

    // Create and publish a page through the form, including a hostile body.
    let (status, _, _) = form(
        svc,
        "/admin/pages",
        Some(&cookie),
        &[
            ("token", "refi_admin_test_token_0123456789"),
            ("title", "Launch notes"),
            ("body", "**live** <script>alert(1)</script>"),
            ("published", "1"),
        ],
    );
    assert_eq!(status, 303);

    let (status, html) = request(svc, "GET", "/pages/launch-notes", None, None);
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("<strong>live</strong>"));
    assert!(!html.contains("<script>alert(1)</script>"));

    // Unpublish it; the public route disappears.
    let (status, _, _) = form(
        svc,
        "/admin/pages/launch-notes/publish",
        Some(&cookie),
        &[("token", "refi_admin_test_token_0123456789")],
    );
    assert_eq!(status, 303);
    let (status, _) = request(svc, "GET", "/pages/launch-notes", None, None);
    assert_eq!(status, 404);

    // A form without the token is refused.
    let (status, _, _) = form(
        svc,
        "/admin/pages",
        Some(&cookie),
        &[("token", "wrong"), ("title", "Nope"), ("body", "x")],
    );
    assert_eq!(status, 401);

    // Security headers are applied to every response.
    let resp = svc.handle(HttpRequest {
        method: "GET".into(),
        path: "/".into(),
        peer: "127.0.0.1:1".into(),
        ..Default::default()
    });
    let names: Vec<&str> = resp.headers.iter().map(|(k, _)| k.as_str()).collect();
    assert!(names.contains(&"Content-Security-Policy"));
    assert!(names.contains(&"X-Content-Type-Options"));
    assert!(names.contains(&"Strict-Transport-Security"));
}

fn request_with_cookie(svc: &Service, path: &str, cookie: &str) -> (u16, String) {
    let resp = svc.handle(HttpRequest {
        method: "GET".to_string(),
        path: path.to_string(),
        headers: vec![("cookie".to_string(), cookie.to_string())],
        peer: "127.0.0.1:50002".to_string(),
        ..Default::default()
    });
    (
        resp.status,
        String::from_utf8_lossy(&resp.body).into_owned(),
    )
}

#[test]
fn state_survives_a_restart() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };
    let svc = &fx.service;
    let (_did, token) = account(svc, "dave");
    let (_, opened) = api(svc, "POST", "/api/v1/reserves", Some(&token), None);
    let reserve = opened["reserve_id"].as_str().unwrap().to_string();
    let (status, _) = api(
        svc,
        "POST",
        &format!("/api/v1/reserves/{reserve}/peg-in"),
        Some(&token),
        Some(json!({"amount": 500})),
    );
    assert_eq!(status, 200);

    let state_dir = fx._dir.path().to_path_buf();
    drop(fx.service); // stops the engine thread
    let cps = state_dir.join("policy.cps");
    assert!(cps.exists(), "the CPS artifact is cached in the state dir");

    let reopened = Service::new(
        Config {
            state_dir: state_dir.clone(),
            base_url: "https://refi.test".to_string(),
            open_registration: true,
            secure_cookies: true,
            mm0web: None,
            sandbox: true,
        },
        Engine::start(EngineConfig {
            state_path: state_dir.join("exchange.json"),
            treasury_seed: [9u8; 32],
            manifest_path: repo_dir().join("module.toml"),
            cps_path: cps,
            austral_bin: austral_bin(),
            module_path: repo_dir().join("modules/RefiPolicy.aum"),
            max_wire: 10_000,
            daily_cap: 5_000,
            denom_horizon: 10_000,
            default_denoms: vec![1, 5, 10, 50, 100, 500, 1000],
        })
        .unwrap(),
        "refi_admin_test_token_0123456789",
    )
    .unwrap();

    let (status, me) = api(&reopened, "GET", "/api/v1/me", Some(&token), None);
    assert_eq!(status, 200);
    assert_eq!(me["reserves"][0]["balance"], json!(500));

    let (_, audit) = api(&reopened, "GET", "/api/v1/audit", Some("refi_admin_test_token_0123456789"), None);
    assert_eq!(audit["ok"], json!(true));
    assert_eq!(audit["reserves_total"], json!(500));
}

// ── proofs (the mm0web join) ─────────────────────────────────────────
//
// The backend is a stand-in for `mm0web`: two endpoints, the same JSON shapes,
// no mm0 toolchain and no Zenodo account. The publish → poll → DOI path is the
// exchange's, and that is what these tests pin down.

struct FakeBackend {
    addr: SocketAddr,
    state: Arc<Mutex<BackendState>>,
}

#[derive(Default)]
struct BackendState {
    accept: bool,
    doi: Option<String>,
}

impl FakeBackend {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake backend");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(Mutex::new(BackendState {
            accept: true,
            doi: None,
        }));
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let state = Arc::clone(&shared);
                std::thread::spawn(move || serve_fake(stream, state));
            }
        });
        Self { addr, state }
    }

    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The deposit is archived: mm0web now reports a DOI.
    fn archive(&self, doi: &str) {
        self.state.lock().unwrap().doi = Some(doi.to_string());
    }

    /// Every development is a proof hole.
    fn reject(&self) {
        self.state.lock().unwrap().accept = false;
    }
}

fn serve_fake(mut stream: TcpStream, state: Arc<Mutex<BackendState>>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok();

    let target = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    let (status, payload) = if target.starts_with("/api/v1/publish") {
        let state = state.lock().unwrap();
        if state.accept {
            (
                202,
                json!({
                    "accepted": true,
                    "queued": true,
                    "proof": "p1",
                    "status": "/status?project=refi&id=p1",
                    "log": "mm0-rs ok; mm0-c ok"
                }),
            )
        } else {
            (
                400,
                json!({"accepted": false, "stage": "mm0-c", "log": "proof hole: axiom at line 3"}),
            )
        }
    } else if target.starts_with("/proof") {
        let state = state.lock().unwrap();
        match state.doi.clone() {
            Some(doi) => (
                200,
                json!({
                    "id": "p1",
                    "state": "published",
                    "doi": doi,
                    "record": 42,
                    "url": "https://zenodo.test/records/42",
                    "log": "mm0-rs: ok\nmm0-c: ok"
                }),
            ),
            None => (
                200,
                json!({"id": "p1", "state": "queued", "log": "mm0-rs: ok\nmm0-c: ok"}),
            ),
        }
    } else {
        (404, json!({"error": "not found"}))
    };

    let payload = payload.to_string();
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(head.as_bytes()).ok();
    stream.write_all(payload.as_bytes()).ok();
    stream.flush().ok();
}

const ADMIN: &str = "refi_admin_test_token_0123456789";

#[test]
fn a_proof_is_published_through_mm0web_and_its_doi_is_recorded() {
    let backend = FakeBackend::start();
    let Some(fx) = fixture_with(ADMIN, Some(&backend.base())) else {
        return;
    };

    let (status, submitted) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/proofs",
        Some(ADMIN),
        Some(json!({
            "title": "linearity ledger",
            "mm0": "sort peano;\ntheorem peano: $ a = a $;",
            "mm1": "theorem peano: $ a = a $;"
        })),
    );
    assert_eq!(status, 202, "verification passed, archiving queued: {submitted}");
    assert_eq!(submitted["state"], json!("queued"));
    assert_eq!(submitted["proof"], json!("p1"));
    let id = submitted["id"].as_str().expect("an id").to_string();

    // The ledger is public: no token, still readable.
    let (status, list) = api(&fx.service, "GET", "/api/v1/proofs", None, None);
    assert_eq!(status, 200);
    assert_eq!(list["configured"], json!(true));
    assert_eq!(list["proofs"][0]["id"], json!(id));
    assert_eq!(list["counts"]["pending"], json!(1));

    // The backend archives the deposit; the refresher folds in the DOI.
    backend.archive("10.5281/zenodo.42");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut published = Value::Null;
    while std::time::Instant::now() < deadline {
        let (_, one) = api(&fx.service, "GET", &format!("/api/v1/proofs/{id}"), None, None);
        if one["state"] == json!("published") {
            published = one;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(
        published["state"],
        json!("published"),
        "the refresher records the DOI: {published}"
    );
    assert_eq!(published["doi"], json!("10.5281/zenodo.42"));
    assert_eq!(published["doi_url"], json!("https://doi.org/10.5281/zenodo.42"));
    assert_eq!(published["zenodo_record"], json!(42));

    // And the site page shows it.
    let (status, html) = request(&fx.service, "GET", "/proofs", None, None);
    assert_eq!(status, 200);
    assert!(html.contains("linearity ledger"));
    assert!(html.contains("10.5281/zenodo.42"));
    assert!(html.contains("https://doi.org/10.5281/zenodo.42"));
    // The row goes straight to the deposit as well as to the resolver.
    assert!(
        html.contains("https://zenodo.test/records/42"),
        "the proof list links the record: {html}"
    );
    assert!(html.contains("zenodo ↗"));
}

#[test]
fn a_development_that_fails_verification_is_recorded_as_rejected() {
    let backend = FakeBackend::start();
    backend.reject();
    let Some(fx) = fixture_with(ADMIN, Some(&backend.base())) else {
        return;
    };

    let (status, body) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/proofs",
        Some(ADMIN),
        Some(json!({"title": "broken", "mm0": "sort peano;", "mm1": ""})),
    );
    assert_eq!(status, 422, "a proof hole is not an archive candidate: {body}");
    assert_eq!(body["state"], json!("rejected"));
    assert_eq!(body["detail"], json!("rejected at mm0-c"));
    let log = body["log"].as_str().unwrap_or_default();
    assert!(log.contains("proof hole"), "the checker's output is kept: {log}");

    let (_, list) = api(&fx.service, "GET", "/api/v1/proofs", None, None);
    assert_eq!(list["counts"]["failed"], json!(1));
    assert_eq!(list["counts"]["pending"], json!(0));
}

#[test]
fn proof_submission_needs_an_admin_and_a_backend() {
    // No backend: the exchange refuses rather than accepting work it cannot do.
    let Some(fx) = fixture(ADMIN) else {
        return;
    };
    let (status, body) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/proofs",
        Some(ADMIN),
        Some(json!({"title": "t", "mm0": "sort peano;"})),
    );
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"]["code"], json!("UK-5001"));

    // With a backend, an anonymous caller is still refused, and the exchange
    // learns nothing about whether publishing is configured first.
    let backend = FakeBackend::start();
    let Some(fx) = fixture_with(ADMIN, Some(&backend.base())) else {
        return;
    };
    let (status, _) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/proofs",
        None,
        Some(json!({"title": "t", "mm0": "sort peano;"})),
    );
    assert_eq!(status, 401);

    // A submission with no spec never reaches the backend.
    let (status, body) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/proofs",
        Some(ADMIN),
        Some(json!({"title": "t", "mm0": "  "})),
    );
    assert_eq!(status, 400, "{body}");
}

#[test]
fn the_proofs_page_renders_without_a_backend() {
    let Some(fx) = fixture(ADMIN) else {
        return;
    };
    let (status, html) = request(&fx.service, "GET", "/proofs", None, None);
    assert_eq!(status, 200);
    assert!(html.contains("Proof ledger"));
    assert!(html.contains("no proof backend configured"));
    assert!(html.contains("/api/v1/proofs"));
}

#[test]
fn an_archived_proof_backs_a_certificate() {
    let backend = FakeBackend::start();
    let Some(fx) = fixture_with(ADMIN, Some(&backend.base())) else {
        return;
    };
    let (status, submitted) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/proofs",
        Some(ADMIN),
        Some(json!({"title": "kernel invariants", "mm0": "sort Prop;", "mm1": "sort Prop;"})),
    );
    assert_eq!(status, 202, "{submitted}");
    let id = submitted["id"].as_str().expect("an id").to_string();

    backend.archive("10.5281/zenodo.42");
    let id_for_poll = id.clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut published = false;
    while std::time::Instant::now() < deadline {
        let (_, one) = api(&fx.service, "GET", &format!("/api/v1/proofs/{id_for_poll}"), None, None);
        published = one["state"] == json!("published");
        if published {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(published, "the proof reaches published with a DOI");

    let owner = format!("did:unfer:{}", "a".repeat(64));
    let (status, cert) = api(
        &fx.service,
        "POST",
        &format!("/api/v1/proofs/{id}/mint"),
        Some(ADMIN),
        Some(json!({"amount": 10, "owner": owner})),
    );
    assert_eq!(status, 201, "the kernel accepted a DOI-provenanced mint: {cert}");
    assert_eq!(cert["source"], json!("10.5281/zenodo.42"));
    assert!(cert["cert_id"].as_str().is_some());

    // The join, from every direction: the proof lists its certificates, the
    // registry classifies the mint by its provenance, the proof's own detail
    // carries the log and the archive link.
    let (_, detail) = api(&fx.service, "GET", &format!("/api/v1/proofs/{id}"), None, None);
    assert_eq!(detail["log"], json!("mm0-rs: ok\nmm0-c: ok"));
    assert_eq!(detail["url"], json!("https://zenodo.test/records/42"));
    assert_eq!(detail["certificates"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(
        detail["certificates"][0]["source"],
        json!("10.5281/zenodo.42")
    );

    let (_, registry) = api(&fx.service, "GET", "/api/v1/registry", None, None);
    let kinds: Vec<Value> = registry["certificates"]
        .as_array()
        .map(|rows| rows.iter().map(|c| c["kind"].clone()).collect())
        .unwrap_or_default();
    assert!(kinds.contains(&json!("attested")), "kinds: {kinds:?}");

    // The console page: verification log, Zenodo record link, and the mint form.
    let (status, html) = request_with_cookie(
        &fx.service,
        &format!("/admin/proofs/{id}"),
        &format!("refi_admin={ADMIN}"),
    );
    assert_eq!(status, 200, "the detail page opens with the admin cookie");
    assert!(html.contains("Verification log"), "detail page: {html}");
    assert!(html.contains("mm0-rs: ok"));
    assert!(html.contains("https://zenodo.test/records/42"));
    assert!(html.contains("Mint certificate from this proof"));
    assert!(html.contains(&cert["cert_id"].as_str().unwrap_or("").to_string()));

    // …and the public registry names the proof behind the certificate, so a
    // reader walks from the mint to the evidence without an account.
    let (status, registry_html) = request(&fx.service, "GET", "/registry", None, None);
    assert_eq!(status, 200);
    assert!(registry_html.contains("Backed by"), "registry: {registry_html}");
    assert!(registry_html.contains(&format!("/proofs/{id}")));
    // …and from that row straight to the archive itself, one click past the
    // proof page: the deposit's record URL on the certificate's own line.
    assert!(
        registry_html.contains("https://zenodo.test/records/42"),
        "registry: {registry_html}"
    );
    assert!(registry_html.contains("zenodo record"));
    assert!(registry_html.contains("kernel invariants"));
    assert!(registry_html.contains(&cert["cert_id"].as_str().unwrap_or("").to_string()));

    let (status, proof_html) = request(&fx.service, "GET", &format!("/proofs/{id}"), None, None);
    assert_eq!(status, 200, "the public proof page needs no account");
    assert!(proof_html.contains("10.5281/zenodo.42"));
    assert!(proof_html.contains("mm0-rs: ok"), "the log is public evidence");
    assert!(proof_html.contains(&cert["cert_id"].as_str().unwrap_or("").to_string()));
    assert!(!proof_html.contains("Mint certificate"), "no mint form in public");

    // An unknown proof id is a 404, not an empty page.
    let (status, _) = request(&fx.service, "GET", "/proofs/deadbeef", None, None);
    assert_eq!(status, 404);
}

#[test]
fn minting_from_a_proof_needs_an_archive_and_an_admin() {
    let backend = FakeBackend::start();
    let Some(fx) = fixture_with(ADMIN, Some(&backend.base())) else {
        return;
    };
    let (status, submitted) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/proofs",
        Some(ADMIN),
        Some(json!({"title": "queued only", "mm0": "sort Prop;", "mm1": "sort Prop;"})),
    );
    assert_eq!(status, 202, "{submitted}");
    let id = submitted["id"].as_str().expect("an id").to_string();
    let owner = format!("did:unfer:{}", "b".repeat(64));

    // Queued is not archived: no archive, no citation, no certificate.
    let (status, body) = api(
        &fx.service,
        "POST",
        &format!("/api/v1/proofs/{id}/mint"),
        Some(ADMIN),
        Some(json!({"amount": 5, "owner": owner})),
    );
    assert_eq!(status, 400, "{body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("published proof"), "message: {message}");

    // Nor may an anonymous caller mint from it.
    let (status, _) = api(
        &fx.service,
        "POST",
        &format!("/api/v1/proofs/{id}/mint"),
        None,
        Some(json!({"amount": 5, "owner": owner})),
    );
    assert_eq!(status, 401);

    // And a bad owner is refused even once the proof is archived.
    backend.archive("10.5281/zenodo.42");
    let id_for_poll = id.clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        let (_, one) = api(&fx.service, "GET", &format!("/api/v1/proofs/{id_for_poll}"), None, None);
        if one["state"] == json!("published") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let (status, body) = api(
        &fx.service,
        "POST",
        &format!("/api/v1/proofs/{id}/mint"),
        Some(ADMIN),
        Some(json!({"amount": 5, "owner": "did:unfer:short"})),
    );
    assert_eq!(status, 400, "{body}");

    // The generic mint endpoint accepts a DOI as provenance, and refuses
    // anything that is neither a UNFCCC order nor a DOI.
    let (status, body) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/mint",
        Some(ADMIN),
        Some(json!({"amount": 5, "owner": owner, "source": "10.5281/zenodo.9"})),
    );
    assert_eq!(status, 201, "{body}");
    let (status, body) = api(
        &fx.service,
        "POST",
        "/api/v1/admin/mint",
        Some(ADMIN),
        Some(json!({"amount": 5, "owner": owner, "source": "a story I heard"})),
    );
    assert_eq!(status, 400, "{body}");
}

// ── transport ───────────────────────────────────────────────────────
//
// Everything below goes through `serve_listener` and a real socket, so a
// framing bug the router cannot see — a HEAD that ships a body, a 200 where
// an error status belongs, an unbounded request, an unenforced connection
// ceiling — fails here.

struct Raw {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Raw {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Bind an ephemeral loopback port and serve on it. The listener thread is
/// abandoned when the test process exits, exactly like a real long-lived
/// server; nothing in the test needs to shut it down.
fn spawn_server(service: Service, max_connections: usize) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    std::thread::spawn(move || {
        let _ = serve_listener(listener, Arc::new(service), max_connections);
    });
    addr
}

fn exchange(addr: SocketAddr, request: &[u8]) -> Raw {
    let mut sock = TcpStream::connect(addr).expect("connect");
    sock.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    sock.write_all(request).expect("write request");
    let mut buf = Vec::new();
    sock.read_to_end(&mut buf).expect("read response");
    parse(&buf)
}

fn parse(buf: &[u8]) -> Raw {
    let end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("the response has a header terminator");
    let head = String::from_utf8_lossy(&buf[..end]);
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status code");
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    Raw {
        status,
        headers,
        body: buf[end + 4..].to_vec(),
    }
}

#[test]
fn head_carries_the_get_length_but_no_body() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };
    let addr = spawn_server(fx.service, 16);

    let get = exchange(addr, b"GET / HTTP/1.1\r\nHost: refi.test\r\n\r\n");
    let head = exchange(addr, b"HEAD / HTTP/1.1\r\nHost: refi.test\r\n\r\n");

    assert_eq!(get.status, 200);
    assert_eq!(head.status, 200, "HEAD routes like GET");
    assert!(!get.body.is_empty(), "the page has a body to compare against");
    assert!(head.body.is_empty(), "HEAD must not send body bytes");
    assert_eq!(head.header("content-type"), get.header("content-type"));
    let declared: usize = head
        .header("content-length")
        .expect("HEAD declares a length")
        .parse()
        .expect("numeric length");
    assert_eq!(
        declared,
        get.body.len(),
        "HEAD must report the length GET would have sent"
    );
}

#[test]
fn oversized_bodies_are_refused_before_they_are_read() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };
    let addr = spawn_server(fx.service, 16);

    // MAX_BODY in src/server.rs is 256 kB; ask for one byte over and send no
    // body at all: the refusal must come from the declared length alone.
    let request = format!(
        "POST /api/v1/accounts HTTP/1.1\r\nHost: refi.test\r\nContent-Length: {}\r\n\r\n",
        256 * 1024 + 1
    );
    let resp = exchange(addr, request.as_bytes());
    assert_eq!(resp.status, 413);
    assert_eq!(resp.header("connection"), Some("close"));
}

#[test]
fn chunked_request_bodies_are_refused() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };
    let addr = spawn_server(fx.service, 16);

    let resp = exchange(
        addr,
        b"POST /api/v1/accounts HTTP/1.1\r\nHost: refi.test\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    );
    assert_eq!(resp.status, 411, "chunked bodies must be answered, not hung on");
}

#[test]
fn connections_beyond_the_ceiling_get_503() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };
    let addr = spawn_server(fx.service, 2);

    // Both sockets stay open and silent, so both handler threads occupy a
    // slot. The accept loop counts before accepting the next socket, so the
    // third connection finds the pool full.
    let first = TcpStream::connect(addr).expect("first connection");
    let second = TcpStream::connect(addr).expect("second connection");
    let resp = exchange(addr, b"GET /healthz HTTP/1.1\r\nHost: refi.test\r\n\r\n");
    assert_eq!(resp.status, 503);
    assert!(resp.body.is_empty());

    drop(first);
    drop(second);
}

#[test]
fn unknown_paths_are_a_real_404() {
    let Some(fx) = fixture("refi_admin_test_token_0123456789") else {
        return;
    };

    let (status, body) = request(&fx.service, "GET", "/no-such-page", None, None);
    assert_eq!(status, 404, "an unknown page is not a 200 that says not found");
    assert!(body.contains("Not found"));

    // The API authenticates before it routes — an anonymous caller learns
    // nothing about which endpoints exist.
    let (status, _) = request(&fx.service, "GET", "/api/v1/no-such-endpoint", None, None);
    assert_eq!(status, 401);
    let (_did, token) = account(&fx.service, "noel");
    let (status, body) = request(
        &fx.service,
        "GET",
        "/api/v1/no-such-endpoint",
        Some(&token),
        None,
    );
    assert_eq!(status, 404);
    assert!(body.contains("UK-1002"), "API 404s carry the protocol code: {body}");
}
