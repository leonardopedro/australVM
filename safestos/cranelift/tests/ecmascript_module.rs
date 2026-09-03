//! Integration tests for the ECMAScript module backend (S1): workerd sidecar + kernel
//! capability loopback.
//!
//! These tests need the workerd binary. It is auto-discovered (mirroring `ecma.rs`): first
//! `UNFER_WORKERD`, then `workerd` on `$PATH`, then fnm-managed Node installs. If no runtime
//! is found they are skipped with a message (the runtime is optional, mirroring the repo's
//! "CUDA optional" convention).

use std::path::{Path, PathBuf};

/// The harmonic_chain model spec used by the unfer_ffi tests (valid, minimal).
const HARMONIC_SPEC: &str = r#"{
  "hamiltonian":{"kind":"builtin","name":"harmonic_chain","params":{"n_modes":2,"omega":1.0}},
  "prior":{"kind":"vacuum"},
  "solver":{"krylov_dim":4,"prune_eps":1e-12,"max_components":null,"restarts":1,"device":{"kind":"cpu"},"adaptive":false}
}"#;

fn workerd_available() -> bool {
    if let Some(b) = std::env::var("UNFER_WORKERD").ok().map(PathBuf::from) {
        return b.exists();
    }
    if std::env::var("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("workerd").is_file()))
        .unwrap_or(false)
    {
        return true;
    }
    find_in_fnm().is_some()
}

fn find_in_fnm() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let root = Path::new(&home).join(".local/share/fnm/node-versions");
    let mut versions: Vec<_> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    versions.sort();
    versions.into_iter().rev().find_map(|v| {
        let c = v.join("installation/lib/node_modules/workerd/bin/workerd");
        c.is_file().then_some(c)
    })
}

fn make_ecma_module_dir(dir: &Path, name: &str, grants: &[&str], js: &str) {
    make_ecma_module_dir_full(dir, name, grants, &[], &[], js);
}

fn make_ecma_module_dir_full(
    dir: &Path,
    name: &str,
    grants: &[&str],
    effects: &[&str],
    observers: &[&str],
    js: &str,
) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let grants_toml: Vec<String> = grants.iter().map(|g| format!("    \"{g}\",")).collect();
    let effects_toml: Vec<String> = effects.iter().map(|e| format!("    \"{e}\",")).collect();
    let observers_toml: Vec<String> = observers.iter().map(|o| format!("    \"{o}\",")).collect();
    let toml = format!(
        "[module]\nname = \"{name}\"\nversion = \"0.1.0\"\narchetypes = [\"ecmascript\"]\narchetype = \"ecmascript\"\nentry = \"src/main.js\"\n\n[grants]\nkernel = [\n{}\n]\neffects = [\n{}\n]\nobservers = [\n{}\n]\n",
        grants_toml.join("\n"),
        effects_toml.join("\n"),
        observers_toml.join("\n")
    );
    std::fs::write(dir.join("module.toml"), toml).unwrap();
    std::fs::write(dir.join("src/main.js"), js).unwrap();
}

/// Positive path: a JS module that creates a model, sets prior, evolves, computes a probability,
/// reads the result, and returns a JSON summary. All called uk_* symbols are granted.
#[test]
fn ecmascript_positive_model_lifecycle() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("ecma_test_positive");
    let _ = std::fs::remove_dir_all(&dir);

    let js = r#"
export async function run(kernel, args) {
  const version = await kernel.uk_version();
  const model = await kernel.uk_model_create(args.spec);
  await kernel.uk_set_prior(model, { kind: "vacuum" });
  await kernel.uk_evolve(model, { t: 0.01 });
  await kernel.uk_event_probability(model, { kind: "vacuum" });
  const result = await kernel.uk_get_result(model);
  await kernel.uk_model_free(model);
  return { version, probability: result.probability };
}
"#;

    make_ecma_module_dir(
        &dir,
        "ecma_positive",
        &[
            "uk_version",
            "uk_model_create",
            "uk_set_prior",
            "uk_evolve",
            "uk_event_probability",
            "uk_get_result",
            "uk_model_free",
        ],
        js,
    );

    let mut host = ModuleHost::new();
    host.load(&dir).unwrap();

    let args = format!(r#"{{"spec": {HARMONIC_SPEC}}}"#);
    let body = host.call_json("ecma_positive", "run", &args).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        v["result"].is_object(),
        "expected result object, got {body}"
    );
    assert_eq!(v["result"]["version"], 1);
    let prob = v["result"]["probability"].as_f64().unwrap();
    assert!(
        prob > 0.0 && prob <= 1.0,
        "probability out of range: {prob}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// F5 negative path: the capability object a module receives must be EXACTLY its
/// `[grants] kernel` set — un-granted `uk_*` symbols are absent (not stubbed, not
/// enumerable), so a module cannot probe the kernel's full symbol table. The
/// host-side loopback is the only layer that emits UK-4001 (tested separately in
/// `ecmascript_loopback_denies_ungranted`).
#[test]
fn ecmascript_capability_exposes_only_granted_symbols() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("ecma_test_f5");
    let _ = std::fs::remove_dir_all(&dir);

    // `uk_model_free` is intentionally NOT in the grants.
    let js = r#"
export async function run(kernel, args) {
  // F5: the capability object is the granted set — un-granted names are absent.
  const granted = Object.keys(kernel).filter(n => n.startsWith("uk_") || n.startsWith("uz_"));
  return {
    versionPresent: typeof kernel.uk_version === "function",
    createPresent: typeof kernel.uk_model_create === "function",
    freeAbsent: typeof kernel.uk_model_free === "undefined",
    freeNotOwn: !("uk_model_free" in kernel),
    granted,
  };
}
"#;

    make_ecma_module_dir(&dir, "ecma_f5", &["uk_version", "uk_model_create"], js);

    let mut host = ModuleHost::new();
    host.load(&dir).unwrap();

    let body = host.call_json("ecma_f5", "run", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let result = &v["result"];
    assert_eq!(
        result["versionPresent"], true,
        "granted symbol must be present, got {body}"
    );
    assert_eq!(
        result["createPresent"], true,
        "granted symbol must be present, got {body}"
    );
    assert_eq!(
        result["freeAbsent"], true,
        "un-granted symbol must be absent, got {body}"
    );
    assert_eq!(
        result["freeNotOwn"], true,
        "un-granted symbol must not be discoverable, got {body}"
    );
    let granted = result["granted"].as_array().expect("granted names array");
    assert_eq!(
        granted.len(),
        2,
        "capability must be exactly the granted set, got {body}"
    );
    let mut names: Vec<&str> = granted.iter().filter_map(|g| g.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        ["uk_model_create", "uk_version"],
        "capability = grants, got {body}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Host-side layer-2 defense in depth: hit the kernel loopback directly with an un-granted symbol.
/// The loopback must reject it with UK-4001 even though the capability object would never expose it.
#[test]
fn ecmascript_loopback_denies_ungranted() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("ecma_test_loopback_deny");
    let _ = std::fs::remove_dir_all(&dir);

    let js = "export async function run(kernel, args) { return { ok: true }; }";
    make_ecma_module_dir(&dir, "ecma_loopback_deny", &["uk_version"], js);

    let mut host = ModuleHost::new();
    host.load(&dir).unwrap();

    // Directly POST to the module's kernel loopback for a symbol it was not granted.
    // S11 peer lockdown: the loopback is armed to the workerd child's pid by
    // default, which would refuse this host-side connection at the transport.
    // This is the host-side loopback test, so re-arm the peer check to this
    // process's own pid — the dispatch layer (UK-4001 for un-granted symbols)
    // is what's under test, not the peer lockdown (covered by ecma.rs unit
    // tests).
    let sock_path = {
        let handle = host.get("ecma_loopback_deny").unwrap();
        let ecma = handle.ecma().unwrap();
        ecma.arm_loopback_peer(std::process::id());
        ecma.loopback_sock().to_path_buf()
    };
    use std::io::{BufReader, Read, Write};
    use std::os::unix::net::UnixStream;
    let mut res = BufReader::new(UnixStream::connect(&sock_path).unwrap());
    let req = "POST /kernel/uk_model_free HTTP/1.1\r\nHost: kernel\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]";
    {
        let stream = res.get_mut();
        stream.write_all(req.as_bytes()).unwrap();
    }
    let mut response = String::new();
    res.read_to_string(&mut response).unwrap();
    assert!(
        response.contains("\"error\""),
        "expected error, got {response}"
    );
    assert!(
        response.contains("4001"),
        "expected UK-4001, got {response}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[cfg(feature = "sandbox")]
fn ecmascript_sidecar_os_sandbox_confines_child() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }
    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("ecma_test_sandbox");
    let _ = std::fs::remove_dir_all(&dir);

    let js = "export async function run(kernel, args) { return { ok: true }; }";
    make_ecma_module_dir(&dir, "ecma_sandbox", &["uk_version"], js);

    let mut host = ModuleHost::new();
    host.load(&dir).unwrap();
    let handle = host.get("ecma_sandbox").unwrap();
    let sidecar = handle.ecma().unwrap();
    let pid = sidecar.child_pid();

    // 1. Distinct user namespace from the test process.
    let child_ns = std::fs::read_link(format!("/proc/{pid}/ns/user")).unwrap();
    let self_ns = std::fs::read_link("/proc/self/ns/user").unwrap();
    assert!(
        child_ns != self_ns,
        "sidecar must run in its own user namespace (sandbox): got {child_ns:?}"
    );

    // 2. no_new_privs must be set (read back via /proc/<pid>/status).
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    assert!(
        status
            .lines()
            .any(|l| l.starts_with("NoNewPrivs:") && l.contains('1')),
        "no_new_privs must be set in the sandboxed sidecar"
    );

    // 3. seccomp must be active.
    let seccomp = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("Seccomp:"))
        .unwrap_or("")
        .trim()
        .to_string();
    assert!(
        matches!(seccomp.as_str(), "1" | "2"),
        "seccomp mode must be 1/2 in the sandboxed sidecar, got {seccomp:?}"
    );

    // 4. uid/gid map must be a single mapped line (namespace root 0 -> our uid).
    let uid_map = std::fs::read_to_string(format!("/proc/{pid}/uid_map")).unwrap();
    assert!(
        uid_map.lines().count() == 1 && uid_map.trim().starts_with("0 "),
        "uid_map must map namespace root to our uid, got {uid_map:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── S4: deferred approval + local simulation (gatekeeper/client pair) ────
//
// Two ECMAScript modules run side by side in one `ModuleHost` (the same process, sharing
// the kernel action store). The *client* holds the `effects = ["send_notification"]`
// grant: its `uk_action_submit` returns a provisional (simulated) result immediately and
// queues a Pending `ActionRecord`. The *gatekeeper* holds the kernel grants to list and
// resolve actions; it approves the pending record. The client then re-reads and sees the
// merged `approved` result.

/// Positive flow: submit → provisional result → gatekeeper approves → client sees the
/// applied result. The effects grant (not the kernel grants) gates submission; the
/// gatekeeper's `uk_action_apply` resolves the pending record.
#[test]
fn ecmascript_effects_deferred_approval_flow() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let client_dir = std::env::temp_dir().join("ecma_effects_client");
    let gatekeeper_dir = std::env::temp_dir().join("ecma_effects_gatekeeper");
    let _ = std::fs::remove_dir_all(&client_dir);
    let _ = std::fs::remove_dir_all(&gatekeeper_dir);

    let client_js = r#"
export async function submit(kernel, args) {
  const handle = await kernel.uk_action_submit({
    effect: "send_notification",
    params: { to: "alice" },
  });
  const record = await kernel.uk_action_get(handle);
  return { handle, state: record.state, simulated: record.result.simulated };
}
export async function read(kernel, args) {
  const record = await kernel.uk_action_get(args.handle);
  return { state: record.state, applied: record.result.applied ?? null };
}
"#;
    // Client: kernel grant exposes the submit/get symbols (harness binding); the effects
    // grant authorizes THIS effect name at the loopback (two-layer, F5).
    make_ecma_module_dir_full(
        &client_dir,
        "ecma_client",
        &["uk_action_submit", "uk_action_get"],
        &["send_notification"],
        &[],
        client_js,
    );

    let gatekeeper_js = r#"
export async function approve(kernel, args) {
  const actions = await kernel.uk_action_list();
  const pending = actions.find(a => a.state === "pending" && a.effect === args.effect);
  if (!pending) return { applied: false, reason: "no pending action" };
  await kernel.uk_action_apply(pending.handle);
  return { applied: true };
}
"#;
    // Gatekeeper: list + resolve are kernel grants; no effects grant needed. F8: it
    // declares `observers = ["ecma_client"]` so its `uk_action_list` view includes the
    // client's records (a module can only read principals it may observe).
    make_ecma_module_dir_full(
        &gatekeeper_dir,
        "ecma_gatekeeper",
        &["uk_action_list", "uk_action_apply"],
        &[],
        &["ecma_client"],
        gatekeeper_js,
    );

    let mut host = ModuleHost::new();
    host.load(&client_dir).unwrap();
    host.load(&gatekeeper_dir).unwrap();

    // 1. Client submits → provisional result, record pending.
    let body = host.call_json("ecma_client", "submit", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let result = &v["result"];
    assert_eq!(
        result["state"], "pending",
        "client must see pending record, got {body}"
    );
    assert_eq!(
        result["simulated"], true,
        "provisional result must be simulated, got {body}"
    );
    let handle = result["handle"].as_i64().expect("action handle");

    // 2. Gatekeeper lists the pending action and approves it.
    let body = host
        .call_json(
            "ecma_gatekeeper",
            "approve",
            r#"{"effect":"send_notification"}"#,
        )
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["result"]["applied"], true,
        "gatekeeper must approve, got {body}"
    );

    // 3. Client re-reads → merged applied result.
    let body = host
        .call_json("ecma_client", "read", &format!(r#"{{"handle":{handle}}}"#))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let result = &v["result"];
    assert_eq!(
        result["state"], "approved",
        "client must see approved record, got {body}"
    );
    assert_eq!(
        result["applied"], true,
        "merged result must be the applied one, got {body}"
    );

    let _ = std::fs::remove_dir_all(&client_dir);
    let _ = std::fs::remove_dir_all(&gatekeeper_dir);
}

// ── F8: observer re-check closes the cross-module read leak ──────────────
//
// A module holding `uk_action_list`/`uk_action_get` may only read records for its
// own principal and any principal listed in `[grants] observers`. A snooping module
// with no observer grant sees nothing of another module's actions, and `uk_action_get`
// on a foreign handle is indistinguishable from a missing record.

/// A module with `uk_action_list` + `uk_action_get` but NO observer grant cannot see
/// another module's pending action; a third module that declares the actor as an
/// observer can.
#[test]
fn ecmascript_observers_filter_action_reads() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let client_dir = std::env::temp_dir().join("ecma_f8_client");
    let snoop_dir = std::env::temp_dir().join("ecma_f8_snoop");
    let observer_dir = std::env::temp_dir().join("ecma_f8_observer");
    for d in [&client_dir, &snoop_dir, &observer_dir] {
        let _ = std::fs::remove_dir_all(d);
    }

    let client_js = r#"
export async function submit(kernel, args) {
  const handle = await kernel.uk_action_submit({
    effect: "send_notification",
    params: { to: "mallory" },
  });
  const record = await kernel.uk_action_get(handle);
  return { handle, state: record.state };
}
"#;
    make_ecma_module_dir_full(
        &client_dir,
        "f8_client",
        &["uk_action_submit", "uk_action_get"],
        &["send_notification"],
        &[],
        client_js,
    );

    let snoop_js = r#"
export async function list(kernel, args) {
  const actions = await kernel.uk_action_list();
  return { seen: actions.map(a => a.principal) };
}
export async function read(kernel, args) {
  try {
    const record = await kernel.uk_action_get(args.handle);
    return { ok: true, principal: record.principal };
  } catch (e) {
    return { ok: false, code: e.ukCode };
  }
}
"#;
    make_ecma_module_dir(
        &snoop_dir,
        "f8_snoop",
        &["uk_action_list", "uk_action_get"],
        snoop_js,
    );

    let observer_js = r#"
export async function list(kernel, args) {
  const actions = await kernel.uk_action_list();
  return { seen: actions.map(a => a.principal) };
}
"#;
    make_ecma_module_dir_full(
        &observer_dir,
        "f8_observer",
        &["uk_action_list"],
        &[],
        &["f8_client"],
        observer_js,
    );

    let mut host = ModuleHost::new();
    host.load(&client_dir).unwrap();
    host.load(&snoop_dir).unwrap();
    host.load(&observer_dir).unwrap();

    // 1. Client submits → pending record, own read works.
    let body = host.call_json("f8_client", "submit", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["result"]["state"], "pending",
        "client must see pending, got {body}"
    );
    let handle = v["result"]["handle"].as_i64().expect("action handle");

    // 2. Snoop (no observer grant) sees nothing of the client's records.
    let body = host.call_json("f8_snoop", "list", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let seen = v["result"]["seen"].as_array().expect("seen array");
    assert!(
        !seen.iter().any(|p| p == "f8_client"),
        "snoop must not observe f8_client's record, got {body}"
    );

    // 3. Snoop's uk_action_get on the foreign handle is indistinguishable from missing.
    let body = host
        .call_json("f8_snoop", "read", &format!(r#"{{"handle":{handle}}}"#))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["result"]["ok"], false, "snoop must be denied, got {body}");
    assert_eq!(v["result"]["code"], 4004, "expected UK-4004, got {body}");

    // 4. Observer (declared observer of f8_client) can see the record.
    let body = host.call_json("f8_observer", "list", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let seen = v["result"]["seen"].as_array().expect("seen array");
    assert!(
        seen.iter().any(|p| p == "f8_client"),
        "observer of f8_client must see its record, got {body}"
    );

    for d in [&client_dir, &snoop_dir, &observer_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// Negative path: a module grants `kernel = ["uk_action_submit"]` but has NO `effects`
/// grant. The loopback's effects gate (not the harness stub) must deny with UK-4001 —
/// the effect name the module does not hold is the resource being checked.
#[test]
fn ecmascript_effects_deny_when_not_granted() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("ecma_effects_deny");
    let _ = std::fs::remove_dir_all(&dir);

    let js = r#"
export async function run(kernel, args) {
  try {
    await kernel.uk_action_submit({ effect: "send_notification", params: {} });
    return { denied: false };
  } catch (e) {
    return { denied: true, code: e.ukCode, message: String(e.message) };
  }
}
"#;
    make_ecma_module_dir_full(
        &dir,
        "ecma_effects_deny",
        &["uk_action_submit"],
        &[], // no effects grant
        &[],
        js,
    );

    let mut host = ModuleHost::new();
    host.load(&dir).unwrap();

    let body = host.call_json("ecma_effects_deny", "run", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let result = &v["result"];
    assert_eq!(result["denied"], true, "expected denial, got {body}");
    assert_eq!(result["code"], 4001, "expected UK-4001, got {body}");
    assert!(
        result["message"]
            .as_str()
            .unwrap_or("")
            .contains("send_notification"),
        "denial must name the un-granted effect, got {body}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── S5: instance isolation + blueprints ────────────────────────────────
//
// `ModuleHost::instantiate` gives every instance of a module its own workerd sidecar (private
// staging dir + unix sockets + PID), so instances of the same module cannot observe each other's
// sockets or step on each other's sockets. `instantiate_from_blueprint` materializes the
// archived module files, spawns a fresh sidecar, and restores the packaged session snapshot —
// the gate is a snapshot/restore round-trip whose restored state reproduces the original's.

/// Two instances of the SAME module dir: distinct staging dirs, distinct PIDs, both callable.
#[test]
fn modulehost_instantiate_isolates_instances() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("ecma_inst_isolation");
    let _ = std::fs::remove_dir_all(&dir);

    let js = "export async function run(kernel, args) { return { pid_hint: kernel, ok: true }; }";
    make_ecma_module_dir(&dir, "ecma_inst", &["uk_version"], js);

    let mut host = ModuleHost::new();
    let k0 = host.instantiate(&dir, "i0").unwrap();
    let k1 = host.instantiate(&dir, "i1").unwrap();
    assert_ne!(k0, k1);

    // Distinct staging dirs → distinct main/loopback sockets.
    let s0 = host
        .instance(&k0)
        .unwrap()
        .sidecar
        .staging_dir()
        .to_path_buf();
    let s1 = host
        .instance(&k1)
        .unwrap()
        .sidecar
        .staging_dir()
        .to_path_buf();
    assert_ne!(s0, s1, "instances must not share a staging dir");

    // Distinct OS processes.
    let p0 = host.instance(&k0).unwrap().sidecar.child_pid();
    let p1 = host.instance(&k1).unwrap().sidecar.child_pid();
    assert_ne!(p0, p1, "instances must be separate sidecar processes");

    // Both instances are independently callable.
    for key in [&k0, &k1] {
        let body = host.call_json_instance(key, "run", "{}").unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["ok"], true, "instance {key} failed: {body}");
    }

    host.drop_instance(&k0).unwrap();
    host.drop_instance(&k1).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// E2E blueprint round-trip: instantiate → run (creates + evolves a model) → snapshot the
/// session → package a `.cell` (module files + session) → `instantiate_from_blueprint` →
/// read the vacuum probability off the RESTORED session → must equal the original's.
#[test]
fn modulehost_blueprint_roundtrip_restores_session() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("ecma_blueprint");
    let parent = std::env::temp_dir().join("ecma_blueprint_materialized");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&parent);

    let grants = [
        "uk_model_create",
        "uk_set_prior",
        "uk_evolve",
        "uk_event_probability",
        "uk_get_result",
    ];
    let js = r#"
export async function run(kernel, args) {
  const model = await kernel.uk_model_create(args.spec);
  await kernel.uk_set_prior(model, { kind: "vacuum" });
  await kernel.uk_evolve(model, { t: 0.05 });
  await kernel.uk_event_probability(model, { kind: "vacuum" });
  const result = await kernel.uk_get_result(model);
  return { handle: model, probability: result.probability };
}
export async function read(kernel, args) {
  await kernel.uk_event_probability(args.handle, { kind: "vacuum" });
  const result = await kernel.uk_get_result(args.handle);
  return { probability: result.probability };
}
"#;
    make_ecma_module_dir(&dir, "ecma_bp", &grants, js);

    let mut host = ModuleHost::new();
    let k0 = host.instantiate(&dir, "i0").unwrap();

    // 1. Run on the original instance → model handle + evolved vacuum probability.
    let args = format!(r#"{{"spec": {HARMONIC_SPEC}}}"#);
    let body = host.call_json_instance(&k0, "run", &args).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let result = &v["result"];
    let model = result["handle"].as_i64().expect("model handle");
    let p0 = result["probability"].as_f64().unwrap();
    assert!(p0 > 0.0 && p0 <= 1.0, "probability out of range: {p0}");

    // 2. Snapshot the session the worker created (host-visible via the shared kernel store).
    let session_json = host.snapshot_session(model).unwrap();

    // 3. Package a .cell with the module files + session snapshot.
    let module_toml = std::fs::read(dir.join("module.toml")).unwrap();
    let main_js = std::fs::read(dir.join("src/main.js")).unwrap();
    let mut builder = unfer_protocol::CellBuilder::new("ecma_bp");
    builder.set_archetype("ecmascript");
    builder.set_entry("src/main.js");
    builder.add_file("module.toml", &module_toml).unwrap();
    builder.add_file("src/main.js", &main_js).unwrap();
    builder.set_session(session_json.as_bytes());
    let cell = builder.build().unwrap();

    // 4. Instantiate a fresh instance from the blueprint: files materialized + session restored.
    let (k1, restored) = host
        .instantiate_from_blueprint(&cell, &parent, "i1")
        .unwrap();
    assert_ne!(k0, k1);
    let restored = restored.expect("blueprint must restore a session");

    // 5. The restored session reproduces the original's evolved probability.
    let body = host
        .call_json_instance(&k1, "read", &format!(r#"{{"handle":{restored}}}"#))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let p1 = v["result"]["probability"].as_f64().unwrap();
    assert!(
        (p1 - p0).abs() < 1e-9,
        "restored session must reproduce the original state: p0={p0} p1={p1}"
    );

    // The snapshot→restore→snapshot identity also holds at the JSON level.
    let resnap = host.snapshot_session(restored).unwrap();
    assert_eq!(
        resnap, session_json,
        "session must round-trip byte-identically"
    );

    host.drop_instance(&k0).unwrap();
    host.drop_instance(&k1).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&parent);
}

/// Blueprint with a `../` traversal entry must be rejected before any sidecar spawns (UK-4001).
#[test]
fn modulehost_blueprint_rejects_path_traversal() {
    use austral_cranelift_bridge::module::ModuleHost;

    let parent = std::env::temp_dir().join("ecma_bp_traversal");
    let _ = std::fs::remove_dir_all(&parent);

    let mut builder = unfer_protocol::CellBuilder::new("evil");
    builder
        .add_file("module.toml", b"[module]\nname = \"evil\"\n")
        .unwrap();
    builder.add_file("../escape.txt", b"boom").unwrap();
    let cell = builder.build().unwrap();

    let mut host = ModuleHost::new();
    let err = host
        .instantiate_from_blueprint(&cell, &parent, "i0")
        .unwrap_err();
    assert!(
        err.contains("traversal"),
        "expected path-traversal rejection, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&parent);
}

/// A blueprint archive without module.toml cannot be instantiated (UK-4100).
#[test]
fn modulehost_blueprint_requires_module_toml() {
    use austral_cranelift_bridge::module::ModuleHost;

    let parent = std::env::temp_dir().join("ecma_bp_notoml");
    let _ = std::fs::remove_dir_all(&parent);

    let mut builder = unfer_protocol::CellBuilder::new("naked");
    builder
        .add_file("src/main.js", b"export async function run(){return {};}")
        .unwrap();
    let cell = builder.build().unwrap();

    let mut host = ModuleHost::new();
    let err = host
        .instantiate_from_blueprint(&cell, &parent, "i0")
        .unwrap_err();
    assert!(
        err.contains("no module.toml"),
        "expected missing-module.toml rejection, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&parent);
}
