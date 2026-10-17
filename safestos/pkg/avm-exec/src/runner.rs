//! The capability-checked sandbox (spec §3.1, task 3.1): *integrate Cedar
//! capability queries before spawning builder processes; isolate mount,
//! PID, IPC, and network namespaces; forbid host access, mounting only
//! declared inputs.*
//!
//! Two gates, in order:
//!
//! 1. **Cedar** — the cell's capability table (the `.avm` header's JSON)
//!    is parsed as Cedar entities; a request the policy set does not allow
//!    is refused with a genuine `EPERM` (`io::ErrorKind::PermissionDenied`)
//!    *before any process exists*. No capability ⇒ no phase — this is the
//!    runner half of §5's capability row; the type half is Austral's
//!    linear capability records (`stdlib/pkg/Derivation.au`, compiled by
//!    the totality/type passes).
//! 2. **Namespaces** — the spawn goes through `unshare` with whichever of
//!    `--map-root-user --net --mount --ipc --pid --fork --mount-proc` the
//!    host actually grants. Availability is *probed, not assumed*, and the
//!    report says exactly what was entered — a degraded host degrades to
//!    capability-gating only, recorded in `Isolation`, never silently.
//!
//! Honest limits (PLAN_avm_pkg.md §4): inside the mount namespace the
//! host tree is still *visible* (ro-unmounting `/usr`,`/bin`,`/tmp` is not
//! implemented — that needs in-namespace mount surgery), so "forbid
//! /bin,/usr,/tmp" is enforced today by a scrubbed environment + staging
//! cwd, not by unmounting; and an empty netns fails outbound sockets with
//! `ENETUNREACH`, while the *Cedar* refusal is the path that surfaces
//! `EPERM`. seccomp/landlock hardening would require `unsafe` and lives
//! behind the spec's §6.3 restriction — recorded, not faked.
//!
//! Zero `unsafe`: isolation is delegated to the `unshare(1)` binary.

use cedar_policy::{Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request};
use std::ffi::{OsStr, OsString};
use std::process::{Command, Output};

/// Cedar configuration: the capability entities a cell carries plus the
/// policy set that judges requests against them.
pub struct Sandbox {
    policies: PolicySet,
    entities: Entities,
}

impl Sandbox {
    /// `capability_table` is the `.avm` header's Cedar capability table —
    /// cedar's entities JSON: an array (or single object) of
    /// `{ "uid": {"type": …, "id": …}, "attrs": …, "parents": … }`.
    /// `policy_src` is Cedar policy text.
    pub fn new(capability_table: serde_json::Value, policy_src: &str) -> Result<Self, String> {
        let value = match &capability_table {
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => capability_table,
            other => {
                return Err(format!(
                    "capability table must be a JSON array/object, got {}",
                    other
                ))
            }
        };
        let entities = Entities::from_json_value(value, None)
            .map_err(|e| format!("capability table rejected by cedar: {e}"))?;
        let policies: PolicySet = policy_src
            .parse()
            .map_err(|e| format!("cedar policy set: {e}"))?;
        Ok(Sandbox { policies, entities })
    }

    /// Is `action` (e.g. `Execute`, `Connect`, `Bind`) permitted for the
    /// builder cell? Deny is the default: an empty policy set allows
    /// nothing.
    pub fn check(&self, action: &str) -> bool {
        let (Ok(principal), Ok(action_uid), Ok(resource)) = (
            "Cell::\"builder\"".parse::<EntityUid>(),
            format!("Action::\"{action}\"").parse::<EntityUid>(),
            "Sandbox::\"host\"".parse::<EntityUid>(),
        ) else {
            return false;
        };
        let Ok(req) = Request::new(principal, action_uid, resource, Context::empty(), None) else {
            return false;
        };
        Authorizer::new()
            .is_authorized(&req, &self.policies, &self.entities)
            .decision()
            == Decision::Allow
    }
}

/// What the host actually granted, from probing — never assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Isolation {
    pub unshare_available: bool,
    pub user_net_mount_ipc: bool,
    pub user_net_mount_ipc_pid: bool,
    pub mount_proc: bool,
}

impl Isolation {
    /// Probe by *trying*, one throwaway child per tier. A child that
    /// fails to start (no `unshare` on PATH) marks everything false.
    pub fn probe() -> Self {
        let base = ["--map-root-user"];
        let tier1 = ["--net", "--mount", "--ipc"];
        let tier2 = ["--pid", "--fork"];
        let has = |flags: &[&str]| -> bool {
            Command::new("unshare")
                .args(base)
                .args(flags)
                .arg("--")
                .arg("true")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        let user_net_mount_ipc = has(&tier1);
        if !user_net_mount_ipc {
            // Distinguish "no unshare at all" from "host blocks userns".
            let present = Command::new("unshare").arg("--help").output().is_ok();
            return Isolation {
                unshare_available: present,
                user_net_mount_ipc: false,
                user_net_mount_ipc_pid: false,
                mount_proc: false,
            };
        }
        let full = has(&[&tier1[..], &tier2[..]].concat());
        let mount_proc = full && has(&[&tier1[..], &tier2[..], &["--mount-proc"]].concat());
        Isolation {
            unshare_available: true,
            user_net_mount_ipc: true,
            user_net_mount_ipc_pid: full,
            mount_proc,
        }
    }

    pub fn any_ns(&self) -> bool {
        self.user_net_mount_ipc
    }
}

/// Runner failures. `Denied` is the capability refusal — and it *is* an
/// EPERM: [`RunnerError::as_io_error`] surfaces
/// `io::ErrorKind::PermissionDenied`, the same errno class the OS raises
/// for a forbidden operation.
#[derive(Debug)]
pub enum RunnerError {
    /// Cedar refused the requested capability.
    Denied(String),
    Cedar(String),
    Io(std::io::Error),
}

impl RunnerError {
    pub fn as_io_error(&self) -> std::io::Error {
        match self {
            RunnerError::Denied(_) => std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            RunnerError::Cedar(_) => {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, self.to_string())
            }
            RunnerError::Io(e) => std::io::Error::new(e.kind(), e.to_string()),
        }
    }
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerError::Denied(a) => write!(f, "EPERM: capability denied for action {a:?}"),
            RunnerError::Cedar(m) => write!(f, "cedar: {m}"),
            RunnerError::Io(e) => write!(f, "runner i/o: {e}"),
        }
    }
}

impl std::error::Error for RunnerError {}

impl From<std::io::Error> for RunnerError {
    fn from(e: std::io::Error) -> Self {
        RunnerError::Io(e)
    }
}

pub struct Runner {
    sandbox: Sandbox,
    isolation: Isolation,
}

impl Runner {
    pub fn new(sandbox: Sandbox) -> Self {
        Runner {
            sandbox,
            isolation: Isolation::probe(),
        }
    }

    pub fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    pub fn isolation(&self) -> Isolation {
        self.isolation
    }

    /// Cedar gate by itself (also what tests assert EPERM through).
    pub fn authorize(&self, action: &str) -> Result<(), RunnerError> {
        if self.sandbox.check(action) {
            Ok(())
        } else {
            Err(RunnerError::Denied(action.to_string()))
        }
    }

    /// Run `program` for a builder that requested `action`:
    /// capability check first (EPERM before any process), then spawn
    /// inside every namespace the host grants, with `staged` inputs
    /// written to a fresh staging directory that becomes the cwd, and a
    /// scrubbed environment (no inherited PATH/HOME/host variables).
    pub fn run<P: AsRef<OsStr>>(
        &self,
        action: &str,
        program: P,
        args: &[OsString],
        staged: &[(&str, &[u8])],
    ) -> Result<Output, RunnerError> {
        self.run_with_env(action, program, args, staged, &[])
    }

    /// [`Self::run`] plus explicit extra environment entries — the only
    /// way a caller passes controlled values into the scrubbed child
    /// (the host environment itself is never forwarded wholesale).
    pub fn run_with_env<P: AsRef<OsStr>>(
        &self,
        action: &str,
        program: P,
        args: &[OsString],
        staged: &[(&str, &[u8])],
        extra_env: &[(String, String)],
    ) -> Result<Output, RunnerError> {
        self.authorize(action)?;

        let stage = tempfile::tempdir()?;
        for (name, bytes) in staged {
            let p = stage.path().join(name);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(p, bytes)?;
        }

        let mut cmd = if self.isolation.user_net_mount_ipc {
            let mut c = Command::new("unshare");
            c.args(["--map-root-user"]);
            if self.isolation.user_net_mount_ipc {
                c.args(["--net", "--mount", "--ipc"]);
            }
            if self.isolation.user_net_mount_ipc_pid {
                c.args(["--pid", "--fork"]);
                if self.isolation.mount_proc {
                    c.arg("--mount-proc");
                }
            }
            c.arg("--");
            c
        } else {
            Command::new(program.as_ref())
        };
        if self.isolation.user_net_mount_ipc {
            cmd.arg(program.as_ref());
        }
        cmd.args(args);
        cmd.current_dir(stage.path());
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin:/run/current-system/sw/bin");
        cmd.env("HOME", stage.path());
        cmd.env("TMPDIR", stage.path());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        Ok(cmd.output()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The `Connect` action from §5's capability row: with no permitting
    /// policy, a builder that *has* a NetworkCap entity still cannot act
    /// — and the refusal is a genuine EPERM, before any process exists.
    #[test]
    fn missing_network_cap_connect_is_ephem() {
        // Policy grants only Execute. NetworkCap entity present — but the
        // policy does not permit Connect, so connect is denied (a
        // capability nobody granted is not a capability).
        let caps = serde_json::json!([
            { "uid": { "type": "ExecCap", "id": "exec" }, "attrs": {}, "parents": [] },
            { "uid": { "type": "NetworkCap", "id": "net" }, "attrs": {}, "parents": [] }
        ]);
        let sandbox = Sandbox::new(
            caps,
            r#"permit(principal, action, resource) when { action == Action::"Execute" };"#,
        )
        .unwrap();
        let runner = Runner::new(sandbox);

        let err = runner.authorize("Connect").unwrap_err();
        assert!(matches!(err, RunnerError::Denied(_)), "denied: {err}");
        let io = err.as_io_error();
        assert_eq!(
            io.kind(),
            std::io::ErrorKind::PermissionDenied,
            "denial must surface as EPERM"
        );
        // Bind likewise.
        assert!(matches!(
            runner.authorize("Bind").unwrap_err(),
            RunnerError::Denied(_)
        ));
        // The granted action passes.
        runner.authorize("Execute").unwrap();

        // And `run` refuses before spawning: program never executes.
        let out = runner.run(
            "Connect",
            "sh",
            &["-c".into(), "echo SHOULD_NOT_RUN".into()],
            &[],
        );
        assert!(out.is_err(), "run must refuse at the capability gate");
        let io = out.unwrap_err().as_io_error();
        assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
    }

    /// A permissive policy + staged inputs: the builder sees the staged
    /// file as its cwd content, with a scrubbed environment.
    #[test]
    fn permitted_builder_sees_staged_inputs() {
        let sandbox = Sandbox::new(
            serde_json::json!([]),
            "permit(principal, action, resource);",
        )
        .unwrap();
        let runner = Runner::new(sandbox);
        let out = runner
            .run(
                "Execute",
                "sh",
                &["-c".into(), "cat input.txt && printf ' CWD_OK'".into()],
                &[("input.txt", b"staged-bytes".as_slice())],
            )
            .expect("permitted builder must run");
        assert!(out.status.success(), "builder failed: {out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("staged-bytes CWD_OK"), "stdout: {stdout:?}");
    }

    /// The OS half of §5's row: a builder running in the isolated tier
    /// finds no route off-host. The child is *this test binary* running
    /// the probe test below, spawned through the runner.
    #[test]
    fn sandboxed_child_cannot_reach_outside() {
        if std::env::var_os("AVM_CHILD_NET_PROBE").is_some() {
            // ── child side: attempt an outbound connect ──
            let addr: std::net::SocketAddr = "192.0.2.1:80".parse().unwrap();
            let res = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400));
            match res {
                Ok(_) => println!("CHILD_NET_RESULT=connected"),
                Err(e) => println!("CHILD_NET_RESULT={e}"),
            }
            std::process::exit(0);
        }

        let sandbox = Sandbox::new(
            serde_json::json!([]),
            "permit(principal, action, resource);",
        )
        .unwrap();
        let runner = Runner::new(sandbox);
        if !runner.isolation().user_net_mount_ipc {
            // Honest degradation: record it, do not fake the assertion.
            eprintln!(
                "host grants no unshare namespaces ({:?}); OS-level network \
                 isolation not demonstrated here",
                runner.isolation()
            );
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let out = runner
            .run_with_env(
                "Execute",
                &exe,
                &[
                    "runner::tests::sandboxed_child_cannot_reach_outside".into(),
                    "--exact".into(),
                    "--nocapture".into(),
                ],
                &[],
                &[("AVM_CHILD_NET_PROBE".to_string(), "1".to_string())],
            )
            .expect("child spawn");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "child failed: {out:?} stderr={:?}",
            out.stderr
        );
        let result = stdout
            .lines()
            .find_map(|l| l.strip_prefix("CHILD_NET_RESULT="))
            .unwrap_or_else(|| panic!("no CHILD_NET_RESULT in child stdout:\n{stdout}"));
        assert!(
            result.contains("unreachable") || result.contains("refused"),
            "in an empty netns an outbound connect must find no route, got: {result}"
        );
    }
}
