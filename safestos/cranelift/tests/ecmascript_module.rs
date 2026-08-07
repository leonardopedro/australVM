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
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let grants_toml: Vec<String> = grants.iter().map(|g| format!("    \"{g}\",")).collect();
    let toml = format!(
        "[module]\nname = \"{name}\"\nversion = \"0.1.0\"\narchetypes = [\"ecmascript\"]\narchetype = \"ecmascript\"\nentry = \"src/main.js\"\n\n[grants]\nkernel = [\n{}\n]\n",
        grants_toml.join("\n")
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
    assert!(v["result"].is_object(), "expected result object, got {body}");
    assert_eq!(v["result"]["version"], 1);
    let prob = v["result"]["probability"].as_f64().unwrap();
    assert!(prob > 0.0 && prob <= 1.0, "probability out of range: {prob}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// UK-4001 negative path: the module calls a uk_* symbol that is NOT granted. Two independent
/// layers must both deny:
/// 1. The capability object (`kernel`) only exposes granted symbols — the harness stubs an
///    un-granted call to throw UK-4001 (`CallDenied`).
/// 2. The host-side kernel loopback re-validates `auth::check` (defense in depth).
#[test]
fn ecmascript_uk4001_ungranted_symbol() {
    if !workerd_available() {
        eprintln!("SKIP: no workerd runtime found (UNFER_WORKERD, $PATH, or fnm); install via `npm install -g workerd`");
        return;
    }

    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("ecma_test_uk4001");
    let _ = std::fs::remove_dir_all(&dir);

    // `uk_model_free` is intentionally NOT in the grants.
    let js = r#"
export async function run(kernel, args) {
  // uk_model_free is not granted: the kernel proxy must throw a UK-4001 CallDenied.
  try {
    await kernel.uk_model_free(123);
    return { denied: false };
  } catch (e) {
    return { denied: true, code: e.ukCode, message: String(e.message) };
  }
}
"#;

    make_ecma_module_dir(
        &dir,
        "ecma_uk4001",
        &["uk_version", "uk_model_create"],
        js,
    );

    let mut host = ModuleHost::new();
    host.load(&dir).unwrap();

    let body = host.call_json("ecma_uk4001", "run", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let result = &v["result"];
    assert_eq!(result["denied"], true, "expected denial, got {body}");
    assert_eq!(result["code"], 4001, "expected UK-4001, got {body}");
    assert!(
        result["message"].as_str().unwrap_or("").contains("UK-4001"),
        "expected UK-4001 message, got {body}"
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
    let sock_path = {
        let handle = host.get("ecma_loopback_deny").unwrap();
        handle.ecma().unwrap().loopback_sock().to_path_buf()
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
    assert!(response.contains("\"error\""), "expected error, got {response}");
    assert!(response.contains("4001"), "expected UK-4001, got {response}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// S3 escape-attempt tests: the workerd sidecar must run inside the OS sandbox. We assert on
/// the child's kernel-visible confinement (rather than re-implementing the sandbox's own
/// probe): a distinct user namespace, `no_new_privs` set, seccomp active, and a valid
/// uid/gid mapping. Only meaningful when the `sandbox` feature is on — workerd then runs under
/// `cranelift/src/sandbox.rs`.
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
        status.lines().any(|l| l.starts_with("NoNewPrivs:") && l.contains('1')),
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
