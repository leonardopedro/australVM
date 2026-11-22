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

fn make_module_dir(dir: &Path, name: &str, grants: &[&str], cps_data: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    let grants_toml: Vec<String> = grants.iter().map(|g| format!("    \"{g}\",")).collect();
    // H8: the loader's archetype resolver only maps registered harness
    // profiles, so the helper uses the austral_cps JIT profile ("actor" was
    // never a harness profile; it made every load fail the resolver).
    let toml = format!(
        "[module]\nname = \"{name}\"\nversion = \"0.1.0\"\narchetypes = [\"austral_cps\"]\narchetype = \"austral_cps\"\nentry = \"run\"\n\n[grants]\nkernel = [\n{}\n]\n",
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
    assert_eq!(handle.function_count(), 1);
    assert!(handle.runtime.function_ptr("run").is_some());

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

// ── D08: scoped caller context and dynamic remote policy ─────────────

/// Body that tail-calls a 0-arg import (same encoding as the cps tests).
fn body_call_import(name: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // tail context
    body.push(0x04); // App
    write_str(&mut body, name);
    write_u32(&mut body, 0);
    body
}

/// One test for the whole D08 story on purpose: the authorization engine and the
/// instance registry are process-global, and `cargo test` runs test functions in
/// parallel. Splitting these would make them race each other's auth state through
/// no fault of the code under test.
#[test]
fn d08_instances_are_isolated_revocable_and_retired_by_swap() {
    use austral_cranelift_bridge::auth;
    use austral_cranelift_bridge::module::ModuleHost;

    let stamp = std::process::id();
    let dir_a = std::env::temp_dir().join(format!("d08_a_{stamp}"));
    let dir_b = std::env::temp_dir().join(format!("d08_b_{stamp}"));
    let dir_c = std::env::temp_dir().join(format!("d08_c_{stamp}"));
    let dir_a2 = std::env::temp_dir().join(format!("d08_a2_{stamp}"));
    for dir in [&dir_a, &dir_b, &dir_c, &dir_a2] {
        let _ = std::fs::remove_dir_all(dir);
    }

    // A and B both *call* uk_version; only A is granted it. B's body is a
    // constant, so B loads fine with a disjoint grant set.
    make_module_dir(&dir_a, "d08_alpha", &["uk_version"], &cps_v1_with_name("run", &body_call_import("uk_version")));
    make_module_dir(&dir_b, "d08_beta", &["uk_evolve"], &cps_v1_with_name("run", &body_return_const(7)));
    // C calls uk_evolve and is not granted it: its load must fail.
    make_module_dir(&dir_c, "d08_gamma", &[], &cps_v1_with_name("run", &body_call_import("uk_evolve")));
    make_module_dir(&dir_a2, "d08_alpha", &["uk_version"], &cps_v1_with_name("run", &body_return_const(11)));

    let mut host = ModuleHost::new();
    {
        let alpha = host.load(&dir_a).unwrap();
        assert_eq!(alpha.manifest.name, "d08_alpha");
        let alpha_context = alpha.context.clone();
        assert!(auth::is_live(&alpha_context));

        let beta = host.load(&dir_b).unwrap();
        let beta_context = beta.context.clone();
        // Two modules, two identities: loading B did not redefine A.
        assert_ne!(alpha_context.instance, beta_context.instance);
        assert!(auth::is_live(&alpha_context), "A stays live after B loads");
        assert!(
            auth::check_instance_call(&alpha_context, "uk_version").is_ok(),
            "A keeps its own grant after B is loaded"
        );
        assert!(
            auth::check_instance_call(&alpha_context, "uk_evolve").is_err(),
            "A never borrows B's grant"
        );
        assert!(
            auth::check_instance_call(&beta_context, "uk_evolve").is_ok(),
            "B keeps its own grant"
        );
        assert!(
            auth::check_instance_call(&beta_context, "uk_version").is_err(),
            "B never borrows A's grant"
        );

        // A failed load is not a policy change: C asks for a symbol it is not
        // granted, so its compile is refused, and the modules already loaded are
        // untouched (their grants and their identities both).
        let refusal = match host.load(&dir_c) {
            Ok(_) => panic!("an ungranted kernel call must fail the load"),
            Err(error) => error,
        };
        assert!(refusal.contains("UK-4001"), "got: {refusal}");
        assert_eq!(host.loaded_modules().len(), 2, "C was never installed");
        assert!(auth::is_live(&alpha_context));
        assert!(
            auth::check_instance_call(&alpha_context, "uk_version").is_ok(),
            "the failed load did not roll back A's grants"
        );
        assert_eq!(
            host.call("d08_alpha", "run", &[]).unwrap(),
            host.call("d08_alpha", "run", &[]).unwrap()
        );

        // Revocation is enforced at the host boundary, where the call is made.
        assert!(auth::revoke_instance(alpha_context.instance));
        let refused = host
            .call("d08_alpha", "run", &[])
            .expect_err("a revoked instance cannot be called");
        assert!(refused.contains("no longer live"), "got: {refused}");
        assert!(host.call("d08_beta", "run", &[]).is_ok(), "B is unaffected");

        // A hot-swap replaces the code and retires the identity behind the old
        // code: a caller still holding that identity is refused from then on.
        let before_swap = host.get("d08_beta").unwrap().context.clone();
        // beta cannot swap to alpha's dir (name mismatch); it swaps to a matching
        // one built next.
        let dir_beta2 = std::env::temp_dir().join(format!("d08_beta2_{stamp}"));
        let _ = std::fs::remove_dir_all(&dir_beta2);
        make_module_dir(&dir_beta2, "d08_beta", &["uk_evolve"], &cps_v1_with_name("run", &body_return_const(9)));
        host.swap("d08_beta", &dir_beta2).expect("same grants, same profile");
        let after_swap = host.get("d08_beta").unwrap().context.clone();
        assert_eq!(after_swap.instance, before_swap.instance);
        assert_ne!(after_swap.epoch, before_swap.epoch, "the epoch moved");
        assert!(!auth::is_live(&before_swap), "the pre-swap identity is retired");
        assert!(auth::is_live(&after_swap));
        assert_eq!(host.call("d08_beta", "run", &[]).unwrap(), 9);
        let _ = std::fs::remove_dir_all(&dir_beta2);

        // A swap that fails leaves the previous identity live and the previous
        // code callable.
        let identity = host.get("d08_beta").unwrap().context.clone();
        let bad = std::env::temp_dir().join(format!("d08_bad_{stamp}"));
        let _ = std::fs::remove_dir_all(&bad);
        // Escalation is refused before any compilation.
        make_module_dir(&bad, "d08_beta", &["uk_evolve", "uk_version"], &cps_v1_with_name("run", &body_return_const(1)));
        assert!(host.swap("d08_beta", &bad).is_err());
        assert!(auth::is_live(&identity), "a refused swap changes nothing");
        assert_eq!(host.call("d08_beta", "run", &[]).unwrap(), 9);
        let _ = std::fs::remove_dir_all(&bad);
    }

    for dir in [&dir_a, &dir_b, &dir_c, &dir_a2] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// D08 strict profile through the loader: a manifest asking for it is parsed,
/// an unknown profile is refused rather than silently downgraded, and a hot-swap
/// may not relax it.
#[test]
fn d08_strict_profile_is_explicit_and_cannot_be_swapped_away() {
    use austral_cranelift_bridge::module::{ModuleHost, ModuleManifest};

    let strict = ModuleManifest::from_toml_str(
        "[module]\nname = \"strict_mod\"\nprofile = \"strict\"\n\n[grants]\nkernel = [\"uk_version\"]\n",
    )
    .expect("strict profile parses");
    assert!(strict.strict);
    assert!(!ModuleManifest::from_toml_str("[module]\nname = \"plain\"\n").unwrap().strict);
    assert!(
        ModuleManifest::from_toml_str("  [module]\nname = \"odd\"\nprofile = \"paranoid\"\n").is_err(),
        "an unknown profile must not be silently downgraded"
    );

    let stamp = std::process::id();
    let dir_strict = std::env::temp_dir().join(format!("d08_strict_{stamp}"));
    let dir_lax = std::env::temp_dir().join(format!("d08_lax_{stamp}"));
    for dir in [&dir_strict, &dir_lax] {
        let _ = std::fs::remove_dir_all(dir);
    }
    std::fs::create_dir_all(&dir_strict).unwrap();
    std::fs::write(
        dir_strict.join("module.toml"),
        "[module]\nname = \"d08_strictmod\"\nversion = \"0.1.0\"\narchetypes = [\"austral_cps\"]\narchetype = \"austral_cps\"\nprofile = \"strict\"\nentry = \"run\"\n\n[grants]\nkernel = [\"uk_version\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir_strict.join("module.cps"),
        cps_v1_with_name("run", &body_return_const(3)),
    )
    .unwrap();
    // Same module name, same grants, no profile: a swap would relax the posture.
    std::fs::create_dir_all(&dir_lax).unwrap();
    std::fs::write(
        dir_lax.join("module.toml"),
        "[module]\nname = \"d08_strictmod\"\nversion = \"0.1.0\"\narchetypes = [\"austral_cps\"]\narchetype = \"austral_cps\"\nentry = \"run\"\n\n[grants]\nkernel = [\"uk_version\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir_lax.join("module.cps"),
        cps_v1_with_name("run", &body_return_const(4)),
    )
    .unwrap();

    let mut host = ModuleHost::new();
    let loaded = host.load(&dir_strict).unwrap();
    assert!(loaded.context.strict, "the loaded identity carries the profile");
    let refusal = host
        .swap("d08_strictmod", &dir_lax)
        .expect_err("a swap may not relax the authority profile");
    assert!(refusal.contains("authority profile"), "got: {refusal}");
    assert_eq!(host.call("d08_strictmod", "run", &[]).unwrap(), 3);
    let _ = std::fs::remove_dir_all(&dir_strict);
    let _ = std::fs::remove_dir_all(&dir_lax);
}
