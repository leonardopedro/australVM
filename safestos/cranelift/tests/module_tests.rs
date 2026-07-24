//! Integration tests for the persistent module hosting infrastructure (B8).
//!
//! Tests: load-once / call-many, manifest parsing, hot-swap with grant
//! escalation rejection, and entrypoint resolution.

use std::path::Path;

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

fn cps_v1_with_name(func_name: &str, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x43505331);
    write_u32(&mut buf, 1);
    write_str(&mut buf, func_name);
    write_u32(&mut buf, 0);
    buf.push(0);
    write_u32(&mut buf, body.len() as u32);
    buf.extend_from_slice(body);
    buf
}

fn body_return_const(val: i64) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07);
    body.push(0x01);
    write_i64(&mut body, val);
    body
}

fn make_module_dir(
    dir: &Path,
    name: &str,
    grants: &[&str],
    cps_data: &[u8],
) {
    std::fs::create_dir_all(dir).unwrap();
    let grants_toml: Vec<String> = grants.iter().map(|g| format!("    \"{g}\",")).collect();
    let toml = format!(
        "[module]\nname = \"{name}\"\nversion = \"0.1.0\"\narchetypes = [\"actor\"]\nentry = \"run\"\n\n[grants]\nkernel = [\n{}\n]\n",
        grants_toml.join("\n")
    );
    std::fs::write(dir.join("module.toml"), toml).unwrap();
    std::fs::write(dir.join("module.cps"), cps_data).unwrap();
}

#[test]
fn manifest_parses_correctly() {
    use austral_cranelift_bridge::module::ModuleManifest;
    let toml = r#"
[module]
name = "test_mod"
version = "1.2.3"
archetypes = ["actor", "data_source"]
entry = "run"

[grants]
kernel = ["uk_version", "uk_evolve"]
"#;
    let m = ModuleManifest::from_toml_str(toml).unwrap();
    assert_eq!(m.name, "test_mod");
    assert_eq!(m.version, "1.2.3");
    assert_eq!(m.archetypes, vec!["actor", "data_source"]);
    assert_eq!(m.entry, "run");
    assert_eq!(m.grants, vec!["uk_version", "uk_evolve"]);
}

#[test]
fn manifest_defaults() {
    use austral_cranelift_bridge::module::ModuleManifest;
    let toml = "[module]\nname = \"minimal\"\n";
    let m = ModuleManifest::from_toml_str(toml).unwrap();
    assert_eq!(m.name, "minimal");
    assert_eq!(m.version, "0.0.0");
    assert!(m.archetypes.is_empty());
    assert_eq!(m.entry, "run");
    assert!(m.grants.is_empty());
}

#[test]
fn load_and_call_multiple_times() {
    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("modhost_test_load_call");
    let _ = std::fs::remove_dir_all(&dir);
    let cps = cps_v1_with_name("run", &body_return_const(42));
    make_module_dir(&dir, "test_load", &["uk_version"], &cps);

    let mut host = ModuleHost::new();
    let handle = host.load(&dir).unwrap();
    assert_eq!(handle.manifest.name, "test_load");
    assert_eq!(handle.functions.len(), 1);
    assert!(handle.functions.contains_key("run"));

    for _ in 0..5 {
        let result = host.call("test_load", "run", &[]).unwrap();
        assert_eq!(result, 42);
    }

    let handle = host.get("test_load").unwrap();
    assert_eq!(handle.call_count, 5);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn call_nonexistent_entrypoint_fails() {
    use austral_cranelift_bridge::module::ModuleHost;

    let dir = std::env::temp_dir().join("modhost_test_bad_ep");
    let _ = std::fs::remove_dir_all(&dir);
    let cps = cps_v1_with_name("run", &body_return_const(1));
    make_module_dir(&dir, "test_ep", &[], &cps);

    let mut host = ModuleHost::new();
    host.load(&dir).unwrap();

    let err = host.call("test_ep", "nonexistent", &[]).unwrap_err();
    assert!(err.contains("not found"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn call_nonexistent_module_fails() {
    use austral_cranelift_bridge::module::ModuleHost;
    let mut host = ModuleHost::new();
    let err = host.call("ghost", "run", &[]).unwrap_err();
    assert!(err.contains("not loaded"));
}

#[test]
fn hotswap_preserves_name_rejects_escalation() {
    use austral_cranelift_bridge::module::ModuleHost;

    let dir_v1 = std::env::temp_dir().join("modhost_test_swap_v1");
    let dir_v2 = std::env::temp_dir().join("modhost_test_swap_v2");
    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);

    let cps_v1 = cps_v1_with_name("run", &body_return_const(10));
    let cps_v2 = cps_v1_with_name("run", &body_return_const(20));

    make_module_dir(&dir_v1, "swap_mod", &["uk_version"], &cps_v1);
    make_module_dir(&dir_v2, "swap_mod", &["uk_version", "uk_evolve"], &cps_v2);

    let mut host = ModuleHost::new();
    host.load(&dir_v1).unwrap();

    let result = host.call("swap_mod", "run", &[]).unwrap();
    assert_eq!(result, 10);

    let err = host.swap("swap_mod", &dir_v2).unwrap_err();
    assert!(err.contains("escalates grant"));

    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);
}

#[test]
fn hotswap_same_grants_succeeds() {
    use austral_cranelift_bridge::module::ModuleHost;

    let dir_v1 = std::env::temp_dir().join("modhost_test_swap_ok_v1");
    let dir_v2 = std::env::temp_dir().join("modhost_test_swap_ok_v2");
    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);

    let cps_v1 = cps_v1_with_name("run", &body_return_const(10));
    let cps_v2 = cps_v1_with_name("run", &body_return_const(99));

    make_module_dir(&dir_v1, "swap_ok", &["uk_version"], &cps_v1);
    make_module_dir(&dir_v2, "swap_ok", &["uk_version"], &cps_v2);

    let mut host = ModuleHost::new();
    host.load(&dir_v1).unwrap();
    assert_eq!(host.call("swap_ok", "run", &[]).unwrap(), 10);

    host.swap("swap_ok", &dir_v2).unwrap();
    assert_eq!(host.call("swap_ok", "run", &[]).unwrap(), 99);

    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);
}

#[test]
fn hotswap_name_mismatch_rejected() {
    use austral_cranelift_bridge::module::ModuleHost;

    let dir_v1 = std::env::temp_dir().join("modhost_test_swap_name_v1");
    let dir_v2 = std::env::temp_dir().join("modhost_test_swap_name_v2");
    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);

    let cps = cps_v1_with_name("run", &body_return_const(1));
    make_module_dir(&dir_v1, "mod_a", &[], &cps);
    make_module_dir(&dir_v2, "mod_b", &[], &cps);

    let mut host = ModuleHost::new();
    host.load(&dir_v1).unwrap();

    let err = host.swap("mod_a", &dir_v2).unwrap_err();
    assert!(err.contains("name mismatch"));

    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);
}

#[test]
fn loaded_modules_lists_all() {
    use austral_cranelift_bridge::module::ModuleHost;

    let dir_a = std::env::temp_dir().join("modhost_test_list_a");
    let dir_b = std::env::temp_dir().join("modhost_test_list_b");
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);

    let cps = cps_v1_with_name("run", &body_return_const(1));
    make_module_dir(&dir_a, "mod_alpha", &[], &cps);
    make_module_dir(&dir_b, "mod_beta", &[], &cps);

    let mut host = ModuleHost::new();
    host.load(&dir_a).unwrap();
    host.load(&dir_b).unwrap();

    let mut names = host.loaded_modules();
    names.sort();
    assert_eq!(names, vec!["mod_alpha", "mod_beta"]);

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
