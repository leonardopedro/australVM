//! The exchange engine: one dedicated thread that owns the GNU Taler-style
//! exchange state, the Austral policy kernel (JIT-compiled by australVM's
//! Cranelift bridge) and the `rs_*` primitives the kernel calls.
//!
//! Why a thread: the bridge's JIT is thread-local — its symbol table and arena
//! allocator belong to the thread that compiled the module — so the module is
//! compiled *and* executed on this one thread. That also gives the service a
//! single writer for the exchange, which is the right shape for an
//! append-only-ordered financial state machine anyway.
//!
//! Command flow for a mutation:
//!
//! ```text
//! HTTP handler ──Cmd──▶ engine thread
//!                         │ set thread-local Ctx
//!                         │ execute_function_2(apply, code, amount)
//!                         │   └─ Austral policy calls rs_* primitives
//!                         │        (authorization, caps, mechanism, audit)
//!                         ▼
//!                       persist snapshot (atomic, fsync)
//!                         │
//! HTTP handler ◀──JSON────┘
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Sender};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unfer_consensus::signing::Keypair;
use sha2::Digest;
use unfer_consensus::certs::commit_coin;
use unfer_protocol::{CertId, CertificateOpKind, Code, CoinRef, ConsensusTransaction, Diagnostic};
use unfer_taler::{ExchangeSnapshot, SimulatedWireGateway, TalerExchange, WireRef};

use crate::util::{ApiError, parse_hex32, sha256_hex, today, write_atomic};

/// Engine tuning, supplied by the CLI and frozen for the process lifetime.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Where the exchange snapshot + usage counters are persisted.
    pub state_path: PathBuf,
    /// Treasury (mint authority) signing seed. Kept in its own 0600 file.
    pub treasury_seed: [u8; 32],
    pub manifest_path: PathBuf,
    /// Prebuilt CPS artifact. When absent it is compiled once via `austral`.
    pub cps_path: PathBuf,
    pub austral_bin: PathBuf,
    pub module_path: PathBuf,
    /// Largest single peg-in / peg-out / withdraw accepted.
    pub max_wire: u64,
    /// Per-account daily peg-in cap.
    pub daily_cap: u64,
    /// How far in the future a newly issued denomination expires (in consensus
    /// sequence steps).
    pub denom_horizon: u64,
    /// Denominations issued at genesis.
    pub default_denoms: Vec<u64>,
}

/// A mutating command. `admin` is the caller's already-verified role flag; the
/// service never trusts a role from the request body.
#[derive(Debug, Clone)]
pub enum Cmd {
    OpenReserve {
        actor: String,
        reserve: [u8; 32],
    },
    PegIn {
        actor: String,
        reserve: [u8; 32],
        amount: u64,
    },
    Withdraw {
        actor: String,
        reserve: [u8; 32],
        amount: u64,
    },
    Deposit {
        actor: String,
        secret: [u8; 32],
        coins: Vec<[u8; 32]>,
        merchant: String,
    },
    PegOut {
        actor: String,
        amount: u64,
        bank: String,
        admin: bool,
    },
    ConfirmPegOut {
        actor: String,
        wire: String,
        admin: bool,
    },
    CancelPegOut {
        actor: String,
        wire: String,
        admin: bool,
    },
    IssueDenom {
        actor: String,
        value: u64,
        expires_seq: Option<u64>,
        admin: bool,
    },
    Mint {
        actor: String,
        amount: u64,
        owner: String,
        source: String,
        admin: bool,
    },
    Transfer {
        actor: String,
        secret: [u8; 32],
        inputs: Vec<[u8; 32]>,
        outputs: Vec<(u64, String)>,
    },
}

impl Cmd {
    /// The Austral command code (see `modules/RefiPolicy.aum`).
    pub fn code(&self) -> i64 {
        match self {
            Cmd::OpenReserve { .. } => 1,
            Cmd::PegIn { .. } => 2,
            Cmd::Withdraw { .. } => 3,
            Cmd::Deposit { .. } => 4,
            Cmd::PegOut { .. } => 5,
            Cmd::ConfirmPegOut { .. } => 6,
            Cmd::CancelPegOut { .. } => 7,
            Cmd::IssueDenom { .. } => 8,
            Cmd::Mint { .. } => 9,
            Cmd::Transfer { .. } => 10,
        }
    }

    /// The numeric argument the policy kernel sees.
    pub fn amount(&self) -> u64 {
        match self {
            Cmd::OpenReserve { .. }
            | Cmd::ConfirmPegOut { .. }
            | Cmd::CancelPegOut { .. } => 0,
            Cmd::PegIn { amount, .. }
            | Cmd::Withdraw { amount, .. }
            | Cmd::PegOut { amount, .. }
            | Cmd::Mint { amount, .. } => *amount,
            Cmd::Deposit { coins, .. } => coins.len() as u64,
            Cmd::IssueDenom { value, .. } => *value,
            Cmd::Transfer { inputs, .. } => inputs.len() as u64,
        }
    }

    pub fn actor(&self) -> &str {
        match self {
            Cmd::OpenReserve { actor, .. }
            | Cmd::PegIn { actor, .. }
            | Cmd::Withdraw { actor, .. }
            | Cmd::Deposit { actor, .. }
            | Cmd::PegOut { actor, .. }
            | Cmd::ConfirmPegOut { actor, .. }
            | Cmd::CancelPegOut { actor, .. }
            | Cmd::IssueDenom { actor, .. }
            | Cmd::Mint { actor, .. }
            | Cmd::Transfer { actor, .. } => actor,
        }
    }

    pub fn is_admin(&self) -> bool {
        match self {
            Cmd::PegOut { admin, .. }
            | Cmd::ConfirmPegOut { admin, .. }
            | Cmd::CancelPegOut { admin, .. }
            | Cmd::IssueDenom { admin, .. }
            | Cmd::Mint { admin, .. } => *admin,
            _ => false,
        }
    }
}

/// A read-only query. Reads do not go through the JIT: the kernel's job is to
/// decide whether a *mutation* may happen, and routing queries through it
/// would only add latency.
#[derive(Debug, Clone)]
pub enum Read {
    Status,
    Audit,
    Registry { limit: usize, kind: Option<String> },
    Denominations,
    Ops { limit: usize },
    Reserve { id: [u8; 32] },
    Reserves { owner: String },
    Coins { owner: String },
    Merchant { did: String },
    PegOuts,
}

/// Handle to the engine thread.
pub struct Engine {
    tx: Sender<Job>,
}

enum Job {
    Call(Cmd, Sender<Result<Value, ApiError>>),
    Read(Read, Sender<Result<Value, ApiError>>),
    Stop,
}

impl Engine {
    pub fn start(cfg: EngineConfig) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("refi-policy".to_string())
            .spawn(move || engine_main(cfg, rx, ready_tx))
            .map_err(|e| format!("spawn policy thread: {e}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { tx }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("policy thread died during startup".to_string()),
        }
    }

    pub fn call(&self, cmd: Cmd) -> Result<Value, ApiError> {
        self.round_trip(|reply| Job::Call(cmd, reply))
    }

    pub fn read(&self, read: Read) -> Result<Value, ApiError> {
        self.round_trip(|reply| Job::Read(read, reply))
    }

    fn round_trip(&self, make: impl FnOnce(Sender<Result<Value, ApiError>>) -> Job) -> Result<Value, ApiError> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(make(tx))
            .map_err(|_| ApiError::unavailable("the exchange engine is not running"))?;
        rx.recv()
            .map_err(|_| ApiError::unavailable("the exchange engine dropped the request"))?
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Job::Stop);
    }
}

// ── thread-local state the rs_* primitives operate on ──────────────────

struct Ctx {
    cmd: Cmd,
    /// The specific UK diagnostic when a primitive failed, so the API can
    /// return the precise code instead of the policy kernel's coarse one.
    err: Option<Diagnostic>,
    result: Option<Value>,
}

impl Ctx {
    fn new(cmd: Cmd) -> Self {
        Self {
            cmd,
            err: None,
            result: None,
        }
    }

    fn fail(&mut self, diag: Diagnostic) -> i64 {
        let rc = match diag.code {
            Code::TALER_INSUFFICIENT_BALANCE => 3,
            Code::TALER_UNKNOWN_RESERVE => 4,
            Code::TALER_DENOM_UNSUPPORTED => 5,
            Code::TALER_COIN_ALREADY_DEPOSITED => 6,
            Code::TALER_UNKNOWN_E_COIN => 2,
            Code::TALER_UNCONFIRMED_WIRE => 4,
            _ => 7,
        };
        self.err = Some(diag);
        rc
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Usage {
    day: u64,
    per_account: HashMap<String, u64>,
}

impl Usage {
    fn allows(&self, did: &str, amount: u64, cap: u64) -> bool {
        let used = if self.day == today() {
            self.per_account.get(did).copied().unwrap_or(0)
        } else {
            0
        };
        used.saturating_add(amount) <= cap
    }

    fn add(&mut self, did: &str, amount: u64) {
        let day = today();
        if self.day != day {
            self.day = day;
            self.per_account.clear();
        }
        *self.per_account.entry(did.to_string()).or_insert(0) += amount;
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Persisted {
    version: u32,
    exchange: ExchangeSnapshot,
    usage: Usage,
}

struct Runtime {
    cfg: EngineConfig,
    exchange: TalerExchange,
    usage: Usage,
}

impl Runtime {
    fn open(cfg: EngineConfig) -> Result<Self, String> {
        let treasury = Keypair::from_bytes(&cfg.treasury_seed);
        match std::fs::read(&cfg.state_path) {
            Ok(bytes) => {
                let persisted: Persisted = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("state file {} is not valid: {e}", cfg.state_path.display()))?;
                let exchange = TalerExchange::from_snapshot(
                    treasury,
                    Box::new(SimulatedWireGateway::new()),
                    &persisted.exchange,
                )?;
                Ok(Self {
                    cfg,
                    exchange,
                    usage: persisted.usage,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut exchange =
                    TalerExchange::new(treasury, Box::new(SimulatedWireGateway::new()));
                for value in &cfg.default_denoms {
                    exchange.issue_denomination(*value, u64::MAX);
                }
                let rt = Self {
                    cfg,
                    exchange,
                    usage: Usage::default(),
                };
                rt.save()?;
                Ok(rt)
            }
            Err(e) => Err(format!("read {}: {e}", cfg.state_path.display())),
        }
    }

    fn save(&self) -> Result<(), String> {
        let persisted = Persisted {
            version: 1,
            exchange: self.exchange.snapshot(),
            usage: Usage {
                day: self.usage.day,
                per_account: self.usage.per_account.clone(),
            },
        };
        let bytes =
            serde_json::to_vec_pretty(&persisted).map_err(|e| format!("encode state: {e}"))?;
        write_atomic(&self.cfg.state_path, &bytes, true)
    }
}

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
    static POLICY_FN: std::cell::Cell<*const c_void> = const { std::cell::Cell::new(std::ptr::null()) };
}

fn with_rt(f: impl FnOnce(&mut Runtime) -> i64) -> i64 {
    RUNTIME.with(|cell| {
        let mut guard = match cell.try_borrow_mut() {
            Ok(g) => g,
            Err(_) => return 7,
        };
        match guard.as_mut() {
            Some(rt) => f(rt),
            None => 7,
        }
    })
}

fn with_ctx(f: impl FnOnce(&mut Ctx) -> i64) -> i64 {
    CTX.with(|cell| {
        let mut guard = match cell.try_borrow_mut() {
            Ok(g) => g,
            Err(_) => return 7,
        };
        match guard.as_mut() {
            Some(ctx) => f(ctx),
            None => 7,
        }
    })
}

fn with_both(f: impl FnOnce(&mut Runtime, &mut Ctx) -> i64) -> i64 {
    RUNTIME.with(|rt_cell| {
        CTX.with(|ctx_cell| {
            let mut rt_guard = match rt_cell.try_borrow_mut() {
                Ok(g) => g,
                Err(_) => return 7,
            };
            let mut ctx_guard = match ctx_cell.try_borrow_mut() {
                Ok(g) => g,
                Err(_) => return 7,
            };
            match (rt_guard.as_mut(), ctx_guard.as_mut()) {
                (Some(rt), Some(ctx)) => f(rt, ctx),
                _ => 7,
            }
        })
    })
}

/// Which value form a mint represents, from its provenance reference.
fn kind_of(source: Option<&str>) -> &'static str {
    match source {
        Some(s) if s.starts_with("unfccc:vc:") => "carbon",
        Some(s) if s.starts_with("taler:reserve:") => "e-coin",
        // A certificate minted against a published proof's DOI: the
        // provenance is an archive, not a registry order.
        Some(s) if crate::util::is_doi(s) => "attested",
        _ => "certificate",
    }
}

fn wire(amount: u64, to: &str) -> WireRef {
    WireRef {
        wire_id: String::new(),
        to: to.to_string(),
        amount,
        status: unfer_taler::WireStatus::Preparing,
    }
}

// ── policy queries ─────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn rs_role(required: i64) -> i64 {
    with_ctx(|ctx| match required {
        0 => 0,
        1 if ctx.cmd.is_admin() => 0,
        _ => 1,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_max_wire() -> i64 {
    with_rt(|rt| rt.cfg.max_wire as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_amount_ok(amount: i64) -> i64 {
    if amount > 0 && amount <= 1_000_000_000 {
        0
    } else {
        2
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_owns_reserve() -> i64 {
    with_both(|rt, ctx| {
        let reserve: &[u8; 32] = match &ctx.cmd {
            Cmd::PegIn { reserve, .. } | Cmd::Withdraw { reserve, .. } => reserve,
            Cmd::OpenReserve { reserve, .. } => reserve,
            _ => return 2,
        };
        match rt.exchange.reserve(reserve) {
            Some((owner, _)) if owner == ctx.cmd.actor() => 0,
            Some(_) => 4,
            None => 4,
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_owns_merchant() -> i64 {
    // A peg-out always redeems the caller's own merchant balance, so the
    // check is that the caller is an authenticated account (already true) and
    // that an admin override is explicit rather than inferred from a DID.
    with_ctx(|ctx| match &ctx.cmd {
        Cmd::PegOut { .. } => 0,
        _ => 2,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_daily_cap_ok(amount: i64) -> i64 {
    if amount <= 0 {
        return 2;
    }
    with_both(|rt, ctx| {
        if rt
            .usage
            .allows(ctx.cmd.actor(), amount as u64, rt.cfg.daily_cap)
        {
            0
        } else {
            8
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_coin_count() -> i64 {
    with_ctx(|ctx| match &ctx.cmd {
        Cmd::Deposit { coins, .. } => coins.len() as i64,
        _ => 0,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_coin_ok(index: i64) -> i64 {
    if index < 0 {
        return 2;
    }
    with_both(|rt, ctx| {
        let coins = match &ctx.cmd {
            Cmd::Deposit { coins, .. } => coins,
            _ => return 2,
        };
        let Some(id) = coins.get(index as usize) else {
            return 2;
        };
        if rt.exchange.is_deposited(&CertId(*id)) {
            return ctx.fail(Diagnostic::new(
                Code::TALER_COIN_ALREADY_DEPOSITED,
                format!("coin {} was already deposited", hex::encode(id)),
                unfer_protocol::Severity::Error,
            ));
        }
        match rt.exchange.ledger().utxo(&CertId(*id)) {
            Some(coin) if coin.owner == ctx.cmd.actor() => 0,
            Some(_) => 2,
            None => ctx.fail(Diagnostic::new(
                Code::CERT_NONEXISTENT_INPUT,
                format!("coin {} is not an unspent certificate", hex::encode(id)),
                unfer_protocol::Severity::Error,
            )),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_denom_ok(amount: i64) -> i64 {
    if amount <= 0 {
        return 5;
    }
    with_rt(|rt| {
        if rt.exchange.denomination(amount as u64).is_some() {
            0
        } else {
            5
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_audit_ok() -> i64 {
    with_rt(|rt| match rt.exchange.audit() {
        Ok(()) => 0,
        Err(d) => {
            tracing::error!(code = d.code.0, message = %d.message, "exchange audit failed");
            7
        }
    })
}

// ── mechanism ──────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn rs_open_reserve() -> i64 {
    with_both(|rt, ctx| {
        let (reserve, actor) = match &ctx.cmd {
            Cmd::OpenReserve { reserve, actor } => (*reserve, actor.clone()),
            _ => return 2,
        };
        rt.exchange.open_reserve(reserve, &actor);
        let balance = rt
            .exchange
            .reserve(&reserve)
            .map(|(_, b)| b)
            .unwrap_or(0);
        ctx.result = Some(json!({
            "reserve_id": hex::encode(reserve),
            "balance": balance,
            "owner": actor,
        }));
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_peg_in() -> i64 {
    with_both(|rt, ctx| {
        let (reserve, amount, actor) = match &ctx.cmd {
            Cmd::PegIn {
                reserve,
                amount,
                actor,
            } => (*reserve, *amount, actor.clone()),
            _ => return 2,
        };
        let bank = "sandbox:incoming-wire".to_string();
        let prepared = match rt.exchange.gateway_mut().prepare_transfer(&bank, amount) {
            Ok(w) => w,
            Err(e) => return ctx.fail(Diagnostic::new(Code::TALER_UNCONFIRMED_WIRE, e, unfer_protocol::Severity::Error)),
        };
        if let Err(e) = rt.exchange.gateway_mut().confirm(&prepared.wire_id) {
            return ctx.fail(Diagnostic::new(Code::TALER_UNCONFIRMED_WIRE, e, unfer_protocol::Severity::Error));
        }
        let Some(confirmed) = rt.exchange.gateway_mut().get(&prepared.wire_id).cloned() else {
            return ctx.fail(Diagnostic::new(Code::TALER_UNCONFIRMED_WIRE, "wire vanished", unfer_protocol::Severity::Error));
        };
        if let Err(d) = rt.exchange.peg_in(reserve, &confirmed) {
            return ctx.fail(d);
        }
        rt.usage.add(&actor, amount);
        let balance = rt.exchange.reserve(&reserve).map(|(_, b)| b).unwrap_or(0);
        ctx.result = Some(json!({
            "wire_id": confirmed.wire_id,
            "amount": amount,
            "reserve_id": hex::encode(reserve),
            "reserve_balance": balance,
            "sandbox": true,
        }));
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_withdraw() -> i64 {
    with_both(|rt, ctx| {
        let (reserve, amount) = match &ctx.cmd {
            Cmd::Withdraw { reserve, amount, .. } => (*reserve, *amount),
            _ => return 2,
        };
        match rt.exchange.withdraw(reserve, amount) {
            Ok(coin) => {
                let balance = rt.exchange.reserve(&reserve).map(|(_, b)| b).unwrap_or(0);
                ctx.result = Some(json!({
                    "coin_id": hex::encode(coin.0),
                    "amount": amount,
                    "reserve_id": hex::encode(reserve),
                    "reserve_balance": balance,
                }));
                0
            }
            Err(d) => ctx.fail(d),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_deposit() -> i64 {
    with_both(|rt, ctx| {
        let (actor, secret, ids, merchant) = match &ctx.cmd {
            Cmd::Deposit {
                actor,
                secret,
                coins,
                merchant,
            } => (actor.clone(), *secret, coins.clone(), merchant.clone()),
            _ => return 2,
        };
        let signer = Keypair::from_bytes(&secret);
        let mut refs: Vec<CoinRef> = Vec::with_capacity(ids.len());
        for id in &ids {
            let Some(coin) = rt.exchange.ledger().utxo(&CertId(*id)) else {
                return ctx.fail(Diagnostic::new(
                    Code::CERT_NONEXISTENT_INPUT,
                    format!("coin {} is not an unspent certificate", hex::encode(id)),
                    unfer_protocol::Severity::Error,
                ));
            };
            refs.push(CoinRef {
                coin_id: coin.coin_id,
                amount: coin.amount,
                owner: coin.owner.clone(),
            });
        }
        match rt.exchange.deposit(&signer, &refs, &merchant) {
            Ok(credited) => {
                let balance = rt.exchange.merchant_balance(&merchant);
                ctx.result = Some(json!({
                    "credited": credited,
                    "merchant": merchant,
                    "merchant_balance": balance,
                    "from": actor,
                }));
                0
            }
            Err(d) => ctx.fail(d),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_peg_out() -> i64 {
    with_both(|rt, ctx| {
        let (actor, amount, bank) = match &ctx.cmd {
            Cmd::PegOut {
                actor,
                amount,
                bank,
                ..
            } => (actor.clone(), *amount, bank.clone()),
            _ => return 2,
        };
        match rt.exchange.peg_out(&actor, amount, &bank) {
            Ok(peg) => {
                let balance = rt.exchange.merchant_balance(&actor);
                ctx.result = Some(json!({
                    "wire_id": peg.wire.wire_id,
                    "amount": peg.wire.amount,
                    "to": peg.wire.to,
                    "confirmed": false,
                    "merchant": actor,
                    "merchant_balance": balance,
                    "sandbox": true,
                }));
                0
            }
            Err(d) => ctx.fail(d),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_confirm_peg_out() -> i64 {
    with_both(|rt, ctx| {
        let wire = match &ctx.cmd {
            Cmd::ConfirmPegOut { wire, .. } => wire.clone(),
            _ => return 2,
        };
        match rt.exchange.confirm_peg_out(&wire) {
            Ok(()) => {
                let confirmed = rt
                    .exchange
                    .peg_out_wire(&wire)
                    .map(|p| p.confirmed)
                    .unwrap_or(false);
                ctx.result = Some(json!({"wire_id": wire, "confirmed": confirmed}));
                0
            }
            Err(d) => ctx.fail(d),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_cancel_peg_out() -> i64 {
    with_both(|rt, ctx| {
        let (wire, actor, admin) = match &ctx.cmd {
            Cmd::CancelPegOut { wire, actor, admin } => (wire.clone(), actor.clone(), *admin),
            _ => return 2,
        };
        let Some(merchant) = rt
            .exchange
            .peg_out_wire(&wire)
            .map(|p| (p.merchant_did.clone(), p.confirmed))
        else {
            return ctx.fail(Diagnostic::new(
                Code::TALER_UNCONFIRMED_WIRE,
                "unknown peg-out",
                unfer_protocol::Severity::Error,
            ));
        };
        if merchant.0 != actor && !admin {
            return 1;
        }
        match rt.exchange.cancel_peg_out(&wire) {
            Ok(()) => {
                let balance = rt.exchange.merchant_balance(&merchant.0);
                ctx.result = Some(json!({
                    "wire_id": wire,
                    "cancelled": true,
                    "merchant": merchant.0,
                    "merchant_balance": balance,
                }));
                0
            }
            Err(d) => ctx.fail(d),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_issue_denom() -> i64 {
    with_both(|rt, ctx| {
        let (value, requested) = match &ctx.cmd {
            Cmd::IssueDenom { value, expires_seq, .. } => (*value, *expires_seq),
            _ => return 2,
        };
        let expires = requested.unwrap_or_else(|| rt.exchange.current_seq() + rt.cfg.denom_horizon);
        rt.exchange.issue_denomination(value, expires);
        ctx.result = Some(json!({"value": value, "expires_seq": expires}));
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_mint() -> i64 {
    with_both(|rt, ctx| {
        let (amount, owner, source) = match &ctx.cmd {
            Cmd::Mint {
                amount,
                owner,
                source,
                ..
            } => (*amount, owner.clone(), source.clone()),
            _ => return 2,
        };
        let seq = rt.exchange.current_seq() + 1;
        let blinding = {
            let mut h = sha2::Sha256::new();
            h.update(b"refi:mint");
            h.update(seq.to_le_bytes());
            h.update(amount.to_le_bytes());
            h.update(owner.as_bytes());
            h.update(source.as_bytes());
            let out: [u8; 32] = h.finalize().into();
            out
        };
        match rt
            .exchange
            .mint_certificate(amount, &owner, &blinding, Some(source.clone()))
        {
            Ok(id) => {
                ctx.result = Some(json!({
                    "cert_id": hex::encode(id.0),
                    "amount": amount,
                    "owner": owner,
                    "source": source,
                    "status": "unspent",
                    "sandbox": true,
                }));
                0
            }
            Err(d) => ctx.fail(d),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rs_transfer() -> i64 {
    with_both(|rt, ctx| {
        let (actor, secret, inputs, outputs) = match &ctx.cmd {
            Cmd::Transfer {
                actor,
                secret,
                inputs,
                outputs,
            } => (actor.clone(), *secret, inputs.clone(), outputs.clone()),
            _ => return 2,
        };
        let signer = Keypair::from_bytes(&secret);
        let mut refs: Vec<CoinRef> = Vec::with_capacity(inputs.len());
        for id in &inputs {
            let Some(coin) = rt.exchange.ledger().utxo(&CertId(*id)) else {
                return ctx.fail(Diagnostic::new(
                    Code::CERT_NONEXISTENT_INPUT,
                    format!("coin {} is not an unspent certificate", hex::encode(id)),
                    unfer_protocol::Severity::Error,
                ));
            };
            refs.push(CoinRef {
                coin_id: coin.coin_id,
                amount: coin.amount,
                owner: coin.owner.clone(),
            });
        }
        match rt.exchange.transfer_certificate(&signer, &refs, &outputs) {
            Ok(ids) => {
                ctx.result = Some(json!({
                    "from": actor,
                    "inputs": inputs.iter().map(hex::encode).collect::<Vec<_>>(),
                    "outputs": ids.iter().map(|id| hex::encode(id.0)).collect::<Vec<_>>(),
                }));
                0
            }
            Err(d) => ctx.fail(d),
        }
    })
}

// ── the engine loop ────────────────────────────────────────────────────

fn engine_main(cfg: EngineConfig, rx: mpsc::Receiver<Job>, ready: Sender<Result<(), String>>) {
    let boot = (|| -> Result<*const c_void, String> {
        register_primitives()?;
        load_manifest(&cfg.manifest_path)?;
        let runtime = Runtime::open(cfg.clone())?;
        RUNTIME.with(|cell| *cell.borrow_mut() = Some(runtime));
        let cps = ensure_cps(&cfg)?;
        let entry = b"apply";
        let f = austral_cranelift_bridge::compile_to_function_named(
            cps.as_ptr(),
            cps.len(),
            entry.as_ptr(),
            entry.len(),
        );
        if f.is_null() {
            let err = austral_cranelift_bridge::cranelift_last_error();
            let msg = if err.is_null() {
                "policy JIT returned no function".to_string()
            } else {
                unsafe { std::ffi::CStr::from_ptr(err) }
                    .to_string_lossy()
                    .into_owned()
            };
            return Err(format!("compiling the Austral policy kernel failed: {msg}"));
        }
        POLICY_FN.with(|cell| cell.set(f));
        Ok(f)
    })();

    let f = match boot {
        Ok(f) => {
            let _ = ready.send(Ok(()));
            f
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    while let Ok(job) = rx.recv() {
        match job {
            Job::Stop => break,
            Job::Call(cmd, reply) => {
                let _ = reply.send(run_cmd(f, cmd));
            }
            Job::Read(read, reply) => {
                let _ = reply.send(read_state(read));
            }
        }
    }
}

fn run_cmd(f: *const c_void, cmd: Cmd) -> Result<Value, ApiError> {
    let code = cmd.code();
    let amount = cmd.amount() as i64;
    CTX.with(|cell| *cell.borrow_mut() = Some(Ctx::new(cmd)));
    let rc = austral_cranelift_bridge::execute_function_2(f, code, amount);
    let ctx = CTX
        .with(|cell| cell.borrow_mut().take())
        .unwrap_or_else(|| Ctx::new(Cmd::OpenReserve {
            actor: String::new(),
            reserve: [0u8; 32],
        }));

    if rc != 0 {
        return Err(policy_error(rc, ctx.err));
    }

    // Durable before the caller sees success: a crash after this point can
    // only lose the audit line, never the money movement.
    let saved = with_rt(|rt| match rt.save() {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!(error = %e, "failed to persist exchange state");
            1
        }
    });
    if saved != 0 {
        return Err(ApiError::internal(
            "the operation was applied but could not be persisted; the service is degraded",
        ));
    }
    Ok(ctx.result.unwrap_or(Value::Null))
}

fn policy_error(rc: i64, diag: Option<Diagnostic>) -> ApiError {
    if rc == i64::MIN {
        return ApiError::internal("the policy kernel panicked while applying the command");
    }
    if let Some(d) = diag {
        let status = match d.code {
            Code::TALER_UNKNOWN_RESERVE
            | Code::TALER_UNKNOWN_E_COIN
            | Code::CERT_NONEXISTENT_INPUT => 404,
            Code::TALER_INSUFFICIENT_BALANCE
            | Code::TALER_COIN_ALREADY_DEPOSITED
            | Code::CERT_DOUBLE_SPEND => 409,
            Code::CERT_MINT_NOT_AUTHORIZED | Code::CERT_OWNER_MISMATCH => 403,
            Code::TALER_DENOM_UNSUPPORTED | Code::CERT_AMOUNT_MISMATCH => 422,
            _ => 422,
        };
        return ApiError {
            status,
            code: format!("UK-{}", d.code.0),
            name: d.name,
            message: d.message,
        };
    }
    match rc {
        1 => ApiError::new(403, "UK-4001", "NotAuthorized", "the account may not perform this command"),
        2 => ApiError::new(422, "UK-7002", "BadAmount", "the amount or request shape is invalid"),
        3 => ApiError::new(409, "UK-7102", "InsufficientBalance", "insufficient balance"),
        4 => ApiError::new(404, "UK-7101", "UnknownReserve", "the reserve or wire is unknown"),
        5 => ApiError::new(422, "UK-7104", "DenominationUnsupported", "no live denomination for that value"),
        6 => ApiError::new(409, "UK-7105", "CoinAlreadyDeposited", "the coin was already deposited"),
        7 => ApiError::internal("the post-operation conservation audit failed"),
        8 => ApiError::limit("the request exceeds a policy limit (per-request or daily cap)"),
        _ => ApiError::internal(format!("the policy kernel returned unknown code {rc}")),
    }
}

fn read_state(read: Read) -> Result<Value, ApiError> {
    RUNTIME.with(|cell| {
        let mut guard = cell
            .try_borrow_mut()
            .map_err(|_| ApiError::internal("exchange state is busy"))?;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::unavailable("the exchange is not open"))?;
        read_exchange(rt, read)
    })
}

fn read_exchange(rt: &mut Runtime, read: Read) -> Result<Value, ApiError> {
    let ex = &mut rt.exchange;
    match read {
        Read::Status => {
            let reserves: u64 = ex.reserves().iter().map(|(_, _, b)| *b).sum();
            let merchants: u64 = ex.merchants().iter().map(|(_, b)| *b).sum();
            Ok(json!({
                "treasury_did": ex.treasury_did(),
                "seq": ex.current_seq(),
                "ledger": {
                    "root": hex::encode(ex.ledger().root()),
                    "total_supply": ex.ledger().total_supply(),
                    "unspent": ex.ledger().unspent_count(),
                },
                "fiat": {"in": ex.fiat_in(), "out": ex.fiat_out()},
                "reserves_total": reserves,
                "merchants_total": merchants,
                "funded_outstanding": ex.funded_outstanding(),
                "denominations": ex.denominations().iter().map(|d| d.value).collect::<Vec<_>>(),
                "peg_outs_open": ex.peg_outs().iter().filter(|p| !p.confirmed).count(),
                "sandbox": true,
            }))
        }
        Read::Audit => {
            let reserves: u64 = ex.reserves().iter().map(|(_, _, b)| *b).sum();
            let merchants: u64 = ex.merchants().iter().map(|(_, b)| *b).sum();
            let liability = reserves + merchants + ex.funded_outstanding();
            let asset = ex.fiat_in() - ex.fiat_out();
            let audit = ex.audit();
            Ok(json!({
                "ok": audit.is_ok(),
                "detail": audit.err().map(|d| d.message),
                "fiat_in": ex.fiat_in(),
                "fiat_out": ex.fiat_out(),
                "assets": asset,
                "liabilities": liability,
                "reserves_total": reserves,
                "merchants_total": merchants,
                "funded_outstanding": ex.funded_outstanding(),
                "ledger_root": hex::encode(ex.ledger().root()),
                "total_supply": ex.ledger().total_supply(),
                "unspent": ex.ledger().unspent_count(),
            }))
        }
        Read::Denominations => {
            let seq = ex.current_seq();
            let mut out: Vec<Value> = ex
                .denominations()
                .iter()
                .map(|d| json!({
                    "value": d.value,
                    "expires_seq": d.expires_seq,
                    "live": d.expires_seq >= seq,
                }))
                .collect();
            out.sort_by_key(|v| v["value"].as_u64().unwrap_or(0));
            Ok(json!({"seq": seq, "denominations": out}))
        }
        Read::Reserves { owner } => {
            let rows: Vec<Value> = ex
                .reserves()
                .into_iter()
                .filter(|(_, did, _)| did == &owner)
                .map(|(id, _, balance)| json!({"reserve_id": hex::encode(id), "balance": balance}))
                .collect();
            Ok(json!({"owner": owner, "reserves": rows}))
        }
        Read::Reserve { id } => match ex.reserve(&id) {
            Some((owner, balance)) => Ok(json!({
                "reserve_id": hex::encode(id),
                "owner": owner,
                "balance": balance,
            })),
            None => Err(ApiError::not_found("no such reserve")),
        },
        Read::Coins { owner } => {
            let mut coins: Vec<Value> = ex
                .ledger()
                .coins_of(&owner)
                .into_iter()
                .map(|c| json!({
                    "coin_id": hex::encode(c.coin_id.0),
                    "amount": c.amount,
                    "owner": c.owner,
                    "minted_seq": c.minted_seq,
                }))
                .collect();
            coins.sort_by_key(|v| v["minted_seq"].as_u64().unwrap_or(0));
            let total: u64 = ex.ledger().coins_of(&owner).iter().map(|c| c.amount).sum();
            Ok(json!({"owner": owner, "total": total, "coins": coins}))
        }
        Read::Merchant { did } => Ok(json!({
            "merchant": did,
            "balance": ex.merchant_balance(&did),
        })),
        Read::PegOuts => Ok(json!({
            "peg_outs": ex.peg_outs().iter().map(|p| json!({
                "wire_id": p.wire.wire_id,
                "to": p.wire.to,
                "amount": p.wire.amount,
                "confirmed": p.confirmed,
                "merchant": p.merchant_did,
            })).collect::<Vec<_>>(),
        })),
        Read::Ops { limit } => {
            let ops = ex.ops();
            let start = ops.len().saturating_sub(limit.clamp(1, 1000));
            let mut out = Vec::new();
            for tx in &ops[start..] {
                if let ConsensusTransaction::CertificateOp(op) = tx {
                    let (kind, detail) = match &op.kind {
                        CertificateOpKind::Mint {
                            amount,
                            owner,
                            source,
                            ..
                        } => (
                            "mint",
                            json!({"amount": amount, "owner": owner, "source": source}),
                        ),
                        CertificateOpKind::Transfer { inputs, outputs } => ("transfer", json!({
                            "inputs": inputs.len(),
                            "outputs": outputs.iter().map(|o| o.amount).collect::<Vec<_>>(),
                        })),
                        CertificateOpKind::Burn { inputs } => {
                            ("burn", json!({"inputs": inputs.len()}))
                        }
                    };
                    out.push(json!({"seq": op.seq, "did": op.did, "kind": kind, "detail": detail}));
                }
            }
            Ok(json!({"total": ops.len(), "ops": out}))
        }
        Read::Registry { limit, kind } => {
            // Three kinds share the certificate ledger: UNFCCC-backed carbon,
            // fiat-backed Taler e-coins (minted on withdraw), and anything
            // else. The kind of a coin is fixed at mint and inherited by
            // transfer outputs, so it is recovered by replaying the op log once
            // (a transfer row itself does not carry a source string).
            let mut kinds: HashMap<[u8; 32], &'static str> = HashMap::new();
            let mut mint_ids: HashMap<[u8; 32], &'static str> = HashMap::new();
            for tx in ex.ops() {
                let ConsensusTransaction::CertificateOp(op) = tx else {
                    continue;
                };
                match &op.kind {
                    CertificateOpKind::Mint {
                        amount,
                        owner,
                        blinding,
                        source,
                    } => {
                        let kind_name = kind_of(source.as_deref());
                        let id = commit_coin(*amount, owner, blinding);
                        kinds.insert(id.0, kind_name);
                        mint_ids.insert(id.0, kind_name);
                    }
                    CertificateOpKind::Transfer { inputs, outputs } => {
                        let inherited = inputs
                            .first()
                            .and_then(|i| kinds.get(&i.coin_id.0).copied())
                            .unwrap_or("certificate");
                        for out in outputs {
                            let id = commit_coin(out.amount, &out.owner, &[0u8; 32]);
                            kinds.insert(id.0, inherited);
                        }
                    }
                    CertificateOpKind::Burn { .. } => {}
                }
            }

            let mut rows: Vec<(u64, Value)> = Vec::new();
            for tx in ex.ops() {
                let ConsensusTransaction::CertificateOp(op) = tx else {
                    continue;
                };
                let CertificateOpKind::Mint {
                    amount,
                    owner,
                    blinding,
                    source,
                } = &op.kind
                else {
                    continue;
                };
                let id = commit_coin(*amount, owner, blinding);
                let kind_name = kinds.get(&id.0).copied().unwrap_or("certificate");
                if let Some(want) = &kind {
                    if want != kind_name {
                        continue;
                    }
                }
                let status = if ex.ledger().utxo(&id).is_some() {
                    "unspent"
                } else {
                    "spent"
                };
                rows.push((
                    op.seq,
                    json!({
                        "cert_id": hex::encode(id.0),
                        "kind": kind_name,
                        "amount": amount,
                        "owner": owner,
                        "source": source,
                        "status": status,
                        "seq": op.seq,
                    }),
                ));
            }

            // Holdings created by a transfer have no mint row of their own;
            // list the live ones so a transferred certificate does not vanish
            // from the registry.
            for tx in ex.ops() {
                let ConsensusTransaction::CertificateOp(op) = tx else {
                    continue;
                };
                let CertificateOpKind::Transfer { outputs, .. } = &op.kind else {
                    continue;
                };
                for out in outputs {
                    let id = commit_coin(out.amount, &out.owner, &[0u8; 32]);
                    if mint_ids.contains_key(&id.0) {
                        continue;
                    }
                    if ex.ledger().utxo(&id).is_none() {
                        continue;
                    }
                    let kind_name = kinds.get(&id.0).copied().unwrap_or("certificate");
                    if let Some(want) = &kind {
                        if want != kind_name {
                            continue;
                        }
                    }
                    rows.push((
                        op.seq,
                        json!({
                            "cert_id": hex::encode(id.0),
                            "kind": kind_name,
                            "amount": out.amount,
                            "owner": out.owner,
                            "source": Value::Null,
                            "status": "unspent",
                            "seq": op.seq,
                        }),
                    ));
                }
            }

            rows.sort_by(|a, b| b.0.cmp(&a.0));
            rows.truncate(limit.clamp(1, 1000));
            let rows: Vec<Value> = rows.into_iter().map(|(_, v)| v).collect();
            Ok(json!({"count": rows.len(), "certificates": rows}))
        }
    }
}

// ── boot helpers ───────────────────────────────────────────────────────

fn register_primitives() -> Result<(), String> {
    let prims: [(&str, *const u8); 20] = [
        ("rs_role", rs_role as *const () as *const u8),
        ("rs_max_wire", rs_max_wire as *const () as *const u8),
        ("rs_amount_ok", rs_amount_ok as *const () as *const u8),
        ("rs_owns_reserve", rs_owns_reserve as *const () as *const u8),
        ("rs_owns_merchant", rs_owns_merchant as *const () as *const u8),
        ("rs_daily_cap_ok", rs_daily_cap_ok as *const () as *const u8),
        ("rs_coin_count", rs_coin_count as *const () as *const u8),
        ("rs_coin_ok", rs_coin_ok as *const () as *const u8),
        ("rs_denom_ok", rs_denom_ok as *const () as *const u8),
        ("rs_audit_ok", rs_audit_ok as *const () as *const u8),
        ("rs_open_reserve", rs_open_reserve as *const () as *const u8),
        ("rs_peg_in", rs_peg_in as *const () as *const u8),
        ("rs_withdraw", rs_withdraw as *const () as *const u8),
        ("rs_deposit", rs_deposit as *const () as *const u8),
        ("rs_peg_out", rs_peg_out as *const () as *const u8),
        ("rs_confirm_peg_out", rs_confirm_peg_out as *const () as *const u8),
        ("rs_cancel_peg_out", rs_cancel_peg_out as *const () as *const u8),
        ("rs_issue_denom", rs_issue_denom as *const () as *const u8),
        ("rs_mint", rs_mint as *const () as *const u8),
        ("rs_transfer", rs_transfer as *const () as *const u8),
    ];
    for (name, addr) in prims {
        austral_cranelift_bridge::register_static_symbol(name, addr)
            .map_err(|e| format!("register {name}: {e}"))?;
    }
    Ok(())
}

fn load_manifest(path: &std::path::Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let rc = unsafe { austral_cranelift_bridge::auth::safestos_load_auth_manifest(bytes.as_ptr(), bytes.len()) };
    if rc == 1 {
        Ok(())
    } else {
        Err(format!(
            "the auth manifest {} was rejected (malformed TOML or unknown schema)",
            path.display()
        ))
    }
}

fn ensure_cps(cfg: &EngineConfig) -> Result<Vec<u8>, String> {
    if cfg.cps_path.exists() {
        return std::fs::read(&cfg.cps_path)
            .map_err(|e| format!("read CPS {}: {e}", cfg.cps_path.display()));
    }
    let out_dir = cfg
        .cps_path
        .parent()
        .ok_or_else(|| format!("{} has no parent", cfg.cps_path.display()))?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let mut cmd = Command::new(&cfg.austral_bin);
    cmd.arg("compile")
        .arg(&cfg.module_path)
        .arg("--use-cps-jit")
        .arg("--allow-all")
        .arg("--target-type=tc")
        .arg(format!("--emit-cps={}", cfg.cps_path.display()));
    if let Some(dir) = cfg.austral_bin.parent() {
        // Prepend the front-end's directory to the existing search path. The
        // old value is itself a list, so it is appended as text rather than
        // re-joined as one path.
        let mut lp = dir.display().to_string();
        if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
            lp.push(':');
            lp.push_str(&old.to_string_lossy());
        }
        cmd.env("LD_LIBRARY_PATH", lp);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("spawn {}: {e}", cfg.austral_bin.display()))?;
    if !cfg.cps_path.exists() {
        return Err(format!(
            "the Austral front-end produced no CPS (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    std::fs::read(&cfg.cps_path).map_err(|e| format!("read {}: {e}", cfg.cps_path.display()))
}

/// A stable reserve id for a DID, used when the caller does not pick one.
pub fn default_reserve_id(did: &str) -> [u8; 32] {
    let digest = sha2::Sha256::digest(format!("refi:reserve:{did}").as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Parse a reserve or certificate id from hex.
pub fn parse_id(s: &str) -> Option<[u8; 32]> {
    parse_hex32(s)
}

/// Unused but kept as the single place that names the sandbox bank account.
pub fn sandbox_bank() -> &'static str {
    "sandbox:incoming-wire"
}

/// A wire reference helper for tests and the sandbox console.
pub fn sandbox_wire(amount: u64) -> WireRef {
    wire(amount, sandbox_bank())
}

/// Unix-day helper re-exported for the metrics page.
pub fn day_number() -> u64 {
    today()
}

/// A hex digest used by the admin export endpoint to fingerprint state.
pub fn state_fingerprint(v: &Value) -> String {
    sha256_hex(v.to_string().as_bytes())
}

/// Convenience for tests: is this DID-shaped string hex after the prefix?
pub fn is_did(s: &str) -> bool {
    s.strip_prefix("did:unfer:")
        .map(|rest| rest.len() == 64 && rest.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_codes_match_the_austral_kernel() {
        let open = Cmd::OpenReserve { actor: "a".into(), reserve: [0; 32] };
        assert_eq!(open.code(), 1);
        assert_eq!(open.amount(), 0);
        let dep = Cmd::Deposit {
            actor: "a".into(),
            secret: [0; 32],
            coins: vec![[1; 32], [2; 32]],
            merchant: "m".into(),
        };
        assert_eq!(dep.code(), 4);
        assert_eq!(dep.amount(), 2, "deposit passes the coin count");
        let mint = Cmd::Mint {
            actor: "a".into(),
            amount: 50,
            owner: "o".into(),
            source: "unfccc:vc:1".into(),
            admin: true,
        };
        assert!(mint.is_admin());
        assert_eq!(mint.code(), 9);
    }

    #[test]
    fn default_reserve_id_is_deterministic_per_did() {
        assert_eq!(default_reserve_id("did:unfer:aa"), default_reserve_id("did:unfer:aa"));
        assert_ne!(default_reserve_id("did:unfer:aa"), default_reserve_id("did:unfer:bb"));
    }

    #[test]
    fn did_shape_check() {
        assert!(is_did(&format!("did:unfer:{}", "a".repeat(64))));
        assert!(!is_did("did:unfer:xyz"));
        assert!(!is_did("alice"));
    }

    #[test]
    fn usage_caps_reset_on_a_new_day() {
        let mut u = Usage::default();
        u.add("alice", 500);
        assert!(u.allows("alice", 500, 1000));
        assert!(!u.allows("alice", 501, 1000));
        assert!(u.allows("bob", 1000, 1000));
        u.day = 0; // pretend yesterday
        assert!(u.allows("alice", 1000, 1000), "yesterday's usage does not count");
    }
}
