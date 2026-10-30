//! `refi_service` — the ReFi exchange web service.
//!
//! Boots in this order:
//!
//! 1. the state directory and its two secrets (treasury signing seed, admin
//!    token), created 0600 on first run;
//! 2. the exchange engine, which loads the persisted snapshot and JIT-compiles
//!    the Austral policy kernel (`modules/RefiPolicy.aum`) behind the manifest
//!    grant list;
//! 3. the HTTP front (`src/server.rs`), serving the JSON API and the site.
//!
//! `--check` does 1 and 2, prints the ledger status and exits — the deploy
//! step uses it to fail fast before the listener comes up.

use std::path::PathBuf;
use std::sync::Arc;

use refi_service::server;
use refi_service::engine::{Engine, EngineConfig, Read};
use refi_service::service::{Config, Service};
use refi_service::util::{parse_hex32, random_token};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

struct Cli {
    listen: String,
    state_dir: PathBuf,
    base_url: String,
    max_wire: u64,
    daily_cap: u64,
    denom_horizon: u64,
    open_registration: bool,
    mm0web: Option<String>,
    austral_bin: PathBuf,
    module_path: PathBuf,
    manifest_path: PathBuf,
    cps_path: Option<PathBuf>,
    check: bool,
}

fn usage() -> ! {
    eprintln!(
        "refi_service — ReFi exchange web service (sandbox settlement)\n\n\
         usage: refi_service [options]\n\n\
         --listen <addr>            HTTP listen address (default 127.0.0.1:8080)\n\
         --state-dir <dir>          state + secrets (default ./state)\n\
         --base-url <url>           public base URL shown on pages\n\
         --max-wire <n>             largest single amount accepted (default 100000)\n\
         --daily-cap <n>            per-account daily peg-in cap (default 500000)\n\
         --denom-horizon <n>        new denomination lifetime in seq steps (default 100000)\n\
         --open-registration        allow self-service customer accounts\n\
         --mm0web <url>             mm0web base URL for proof publishing (default $REFI_MM0WEB)\n\
         --austral-bin <path>       austral front-end binary (default $AUSTRAL_BIN or ../austral)\n\
         --module <path>            policy module source (default modules/RefiPolicy.aum)\n\
         --manifest <path>          policy grant manifest (default module.toml)\n\
         --cps <path>               prebuilt CPS artifact (default <state-dir>/RefiPolicy.cps)\n\
         --check                    validate states + policy, print status, exit\n"
    );
    std::process::exit(2)
}

fn parse_cli() -> Cli {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cli = Cli {
        listen: std::env::var("REFI_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
        state_dir: PathBuf::from(
            std::env::var("REFI_STATE_DIR").unwrap_or_else(|_| "state".to_string()),
        ),
        base_url: std::env::var("REFI_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string()),
        max_wire: 100_000,
        daily_cap: 500_000,
        denom_horizon: 100_000,
        open_registration: false,
        mm0web: std::env::var("REFI_MM0WEB")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        austral_bin: std::env::var("AUSTRAL_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(MANIFEST_DIR).join("../austral")),
        module_path: PathBuf::from(MANIFEST_DIR).join("modules/RefiPolicy.aum"),
        manifest_path: PathBuf::from(MANIFEST_DIR).join("module.toml"),
        cps_path: None,
        check: false,
    };
    let mut i = 0;
    let next = |args: &Vec<String>, i: &mut usize, flag: &str| -> String {
        *i += 1;
        args.get(*i)
            .cloned()
            .unwrap_or_else(|| {
                eprintln!("{flag} needs a value");
                usage()
            })
    };
    while i < args.len() {
        // systemd's `${VAR}` expansion can leave an empty argument when the
        // variable is defined but empty (e.g. REFI_OPEN_REGISTRATION=); ignore
        // it rather than treating it as an unknown flag.
        if args[i].is_empty() {
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--help" | "-h" => usage(),
            "--listen" => cli.listen = next(&args, &mut i, "--listen"),
            "--state-dir" => cli.state_dir = PathBuf::from(next(&args, &mut i, "--state-dir")),
            "--base-url" => cli.base_url = next(&args, &mut i, "--base-url"),
            "--max-wire" => cli.max_wire = parse_num(&next(&args, &mut i, "--max-wire")),
            "--daily-cap" => cli.daily_cap = parse_num(&next(&args, &mut i, "--daily-cap")),
            "--denom-horizon" => {
                cli.denom_horizon = parse_num(&next(&args, &mut i, "--denom-horizon"))
            }
            "--open-registration" => cli.open_registration = true,
            "--mm0web" => {
                // An empty value (systemd expanding an unset ${REFI_MM0WEB})
                // means "no proof backend", not a base URL of "".
                let value = next(&args, &mut i, "--mm0web");
                cli.mm0web = if value.trim().is_empty() {
                    None
                } else {
                    Some(value)
                };
            }
            "--austral-bin" => cli.austral_bin = PathBuf::from(next(&args, &mut i, "--austral-bin")),
            "--module" => cli.module_path = PathBuf::from(next(&args, &mut i, "--module")),
            "--manifest" => cli.manifest_path = PathBuf::from(next(&args, &mut i, "--manifest")),
            "--cps" => cli.cps_path = Some(PathBuf::from(next(&args, &mut i, "--cps"))),
            "--check" => cli.check = true,
            other => {
                eprintln!("unknown flag: {other}");
                usage();
            }
        }
        i += 1;
    }
    cli
}

fn parse_num(s: &str) -> u64 {
    s.parse().unwrap_or_else(|_| {
        eprintln!("expected a number, got {s}");
        usage()
    })
}

/// Read `<state>/<name>`, or create it with 32 random bytes (hex, 0600).
fn ensure_secret(path: &PathBuf) -> Result<[u8; 32], String> {
    if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        return parse_hex32(text.trim())
            .ok_or_else(|| format!("{} must hold 64 hex characters", path.display()));
    }
    let seed: [u8; 32] = rand::random();
    refi_service::util::write_atomic(path, hex::encode(seed).as_bytes(), true)?;
    Ok(seed)
}

fn ensure_admin_token(path: &PathBuf) -> Result<String, String> {
    if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let token = text.trim().to_string();
        if token.len() < 32 {
            return Err(format!("{} is too short to be an admin token", path.display()));
        }
        return Ok(token);
    }
    let token = random_token("refi_admin_");
    refi_service::util::write_atomic(path, token.as_bytes(), true)?;
    Ok(token)
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // The Cranelift JIT logs every emitted function at info; keep
                // that out of the service log unless asked for explicitly.
                tracing_subscriber::EnvFilter::new("refi_service=info")
            }),
        )
        .init();

    let cli = parse_cli();
    if let Err(e) = std::fs::create_dir_all(&cli.state_dir) {
        eprintln!("cannot create state dir {}: {e}", cli.state_dir.display());
        std::process::exit(1);
    }

    let treasury_path = cli.state_dir.join("treasury.key");
    let admin_path = cli.state_dir.join("admin.token");
    let treasury_seed = match ensure_secret(&treasury_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let admin_token = match ensure_admin_token(&admin_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let cps_path = cli
        .cps_path
        .clone()
        .unwrap_or_else(|| cli.state_dir.join("RefiPolicy.cps"));

    let engine_cfg = EngineConfig {
        state_path: cli.state_dir.join("exchange.json"),
        treasury_seed,
        manifest_path: cli.manifest_path.clone(),
        cps_path,
        austral_bin: cli.austral_bin.clone(),
        module_path: cli.module_path.clone(),
        max_wire: cli.max_wire,
        daily_cap: cli.daily_cap,
        denom_horizon: cli.denom_horizon,
        default_denoms: vec![1, 5, 10, 50, 100, 500, 1000],
    };

    let engine = match Engine::start(engine_cfg) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("startup failed: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!(
        state = %cli.state_dir.display(),
        manifest = %cli.manifest_path.display(),
        module = %cli.module_path.display(),
        "exchange engine up (Austral policy kernel compiled)"
    );

    if cli.check {
        match engine.read(Read::Status) {
            Ok(status) => {
                println!("{}", serde_json::to_string_pretty(&status).unwrap_or_default());
                println!("admin token file: {}", admin_path.display());
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("status read failed: {}", e.message);
                std::process::exit(1);
            }
        }
    }

    // TLS is terminated by the tunnel or reverse proxy in front of this
    // listener (see deploy/README.md); the service itself speaks loopback
    // HTTP. `secure_cookies` is therefore true whenever the public origin is
    // HTTPS, which is how the deployment is configured.
    let secure = cli.base_url.starts_with("https://");
    let service = Service::new(
        Config {
            state_dir: cli.state_dir.clone(),
            listen: cli.listen.clone(),
            base_url: cli.base_url.clone(),
            open_registration: cli.open_registration,
            secure_cookies: secure,
            mm0web: cli.mm0web.clone(),
            sandbox: true,
        },
        engine,
        &admin_token,
    )
    .unwrap_or_else(|e| {
        eprintln!("cannot open the service stores: {e}");
        std::process::exit(1);
    });

    let service = Arc::new(service);
    tracing::info!(
        base_url = %cli.base_url,
        registration = if cli.open_registration { "open" } else { "invite-only" },
        "refi_service ready"
    );
    if let Err(e) = server::serve(service, &cli.listen) {
        eprintln!("cannot listen on {}: {e}", cli.listen);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_created_once_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("treasury.key");
        let first = ensure_secret(&path).unwrap();
        let second = ensure_secret(&path).unwrap();
        assert_eq!(first, second);
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&path).unwrap().permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn admin_token_is_generated_and_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admin.token");
        let a = ensure_admin_token(&path).unwrap();
        let b = ensure_admin_token(&path).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("refi_admin_"));
    }
}
