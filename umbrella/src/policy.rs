//! Policy preflight (PLAN_umbrella_production U02).
//!
//! U02's rule is that three permissions are *separate*, and that the output
//! must not blur them:
//!
//! * **build** — may native source be compiled (a crate's build script runs)?
//! * **load** — may a cdylib be mapped into the process (its library
//!   initializer runs)?
//! * **call** — may a module invoke a granted extern symbol?
//!
//! A missing call grant therefore does not by itself forbid a build that the
//! project explicitly authorized — but a refusal must say that the build and
//! load already happened, never that they were contained.
//!
//! Everything here is decided *before* the first stage runs: `preflight` is
//! pure, and `run_stages` executes only the stages the preflight allowed.

use std::ffi::{c_void, CStr, CString};
use std::path::Path;
use std::ptr::NonNull;

use crate::project::{Profile, Project};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allowed(String),
    Denied(String),
}

impl Decision {
    pub fn allowed(&self) -> bool {
        matches!(self, Decision::Allowed(_))
    }

    pub fn reason(&self) -> &str {
        match self {
            Decision::Allowed(r) | Decision::Denied(r) => r,
        }
    }

    pub fn state(&self) -> &'static str {
        if self.allowed() {
            "allowed"
        } else {
            "denied"
        }
    }
}

#[derive(Debug, Clone)]
pub struct Preflight {
    pub profile: Profile,
    pub build: Decision,
    pub load: Decision,
    pub call: Decision,
}

impl Preflight {
    /// One line per permission plus the scope statement, so a reader can see
    /// which authorization produced which stage.
    pub fn report(&self) -> String {
        format!(
            "policy: profile={} build={} ({}) load={} ({}) call={} ({})\n\
             policy: scope: {}",
            self.profile.as_str(),
            self.build.state(),
            self.build.reason(),
            self.load.state(),
            self.load.reason(),
            self.call.state(),
            self.call.reason(),
            scope_statement(&self.build, &self.load, &self.call)
        )
    }
}

/// The sentence U02 requires: call grants never authorize (or excuse) the
/// earlier stages.
fn scope_statement(build: &Decision, load: &Decision, call: &Decision) -> &'static str {
    let earlier_ran = build.allowed() || load.allowed();
    match (earlier_ran, call.allowed()) {
        (true, false) => {
            "build/load authorization was exercised above; the call grant is \
             separate, so a denied call does not mean the build or library \
             initializer was contained"
        }
        (false, false) => "no stage ran; nothing was authorized",
        (_, true) => {
            "grants authorize extern calls only; they do not authorize \
                      building native source or loading a provider"
        }
    }
}

/// Validate the project against the running build and decide every stage
/// before anything executes. A profile whose rules do not exist yet is
/// refused here, not honoured.
pub fn preflight(p: &Project) -> Result<Preflight, String> {
    if p.profile == Profile::UntrustedNative {
        return Err(
            "profile `untrusted-native` is not implemented (U26 supplies its \
             build/native isolation); refusing before any stage"
                .to_string(),
        );
    }
    if !p.build.allowed && !p.load.allowed {
        return Err(format!(
            "profile `{}` authorizes neither build nor load; refusing before \
             cargo or dlopen",
            p.profile.as_str()
        ));
    }

    let build = if p.build.allowed {
        let krate = p
            .build
            .krate
            .as_ref()
            .map(|k| k.display().to_string())
            .unwrap_or_else(|| "<missing>".to_string());
        Decision::Allowed(format!("project authorizes building {krate}"))
    } else {
        Decision::Denied("project does not authorize building native source".to_string())
    };

    let load = if p.load.allowed {
        Decision::Allowed(format!(
            "project authorizes loading {} provider(s)",
            p.load.providers.len()
        ))
    } else {
        Decision::Denied("project does not authorize loading a provider".to_string())
    };

    let call = if p.grants.is_empty() {
        Decision::Denied("no extern call grants are declared".to_string())
    } else {
        Decision::Allowed(format!(
            "{} extern call grant(s) declared: {}",
            p.grants.len(),
            p.grants
                .iter()
                .map(|g| g.symbol.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    };

    Ok(Preflight {
        profile: p.profile,
        build,
        load,
        call,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Build,
    Load,
    Call,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Build => "build",
            Stage::Load => "load",
            Stage::Call => "call",
        }
    }
}

#[derive(Debug)]
pub struct StageError {
    pub stage: Stage,
    pub message: String,
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} stage failed: {}", self.stage.as_str(), self.message)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stages {
    pub built: bool,
    pub loaded: bool,
    pub called: bool,
}

/// Run exactly the authorized stages, in order, and nothing else. The
/// closures are how a test proves which stages were reached: a denied stage
/// is never invoked, so its marker is never written.
pub fn run_stages<B, L, C>(pf: &Preflight, build: B, load: L, call: C) -> Result<Stages, StageError>
where
    B: FnOnce() -> Result<(), String>,
    L: FnOnce() -> Result<(), String>,
    C: FnOnce() -> Result<(), String>,
{
    let mut stages = Stages::default();
    if pf.build.allowed() {
        build().map_err(|message| StageError {
            stage: Stage::Build,
            message,
        })?;
        stages.built = true;
    }
    if pf.load.allowed() {
        load().map_err(|message| StageError {
            stage: Stage::Load,
            message,
        })?;
        stages.loaded = true;
    }
    // A call needs both earlier stages: an unmapped provider cannot answer.
    if pf.call.allowed() && stages.loaded {
        call().map_err(|message| StageError {
            stage: Stage::Call,
            message,
        })?;
        stages.called = true;
    }
    Ok(stages)
}

/// Map a cdylib into the process: `RTLD_NOW` fails loudly on any unresolved
/// symbol, `RTLD_GLOBAL` puts its symbols where the bridge's
/// `dlsym(RTLD_DEFAULT)` resolver can see them. The handle is deliberately
/// never closed — JIT'd code may call into it for the rest of the process.
pub fn dlopen(path: &Path) -> Result<NonNull<c_void>, String> {
    let cpath = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|e| format!("path is not a valid CString: {e}"))?;
    let handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    NonNull::new(handle).ok_or_else(|| unsafe {
        let e = libc::dlerror();
        if e.is_null() {
            "dlopen failed".to_string()
        } else {
            CStr::from_ptr(e).to_string_lossy().into_owned()
        }
    })
}
