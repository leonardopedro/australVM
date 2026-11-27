//! Tests for cap-std Rust module support (B10).

use austral_cranelift_bridge::module::ModuleManifest;

#[test]
fn manifest_parses_rust_capstd_archetype() {
    let toml = r#"
[module]
name = "rust_kv"
version = "0.1.0"
archetype = "rust_capstd"
entry = "run"

[grants]
kernel = ["uk_version"]
fs = ["data/", "config/"]
net = ["api.example.com:443"]

[limits]
max_ms = 5000
"#;
    let m = ModuleManifest::from_toml_str(toml).unwrap();
    assert_eq!(m.name, "rust_kv");
    assert_eq!(m.archetype, "rust_capstd");
    assert_eq!(m.fs_grants, vec!["data/", "config/"]);
    assert_eq!(m.net_grants, vec!["api.example.com:443"]);
    assert_eq!(m.max_ms, Some(5000));
}

#[test]
fn manifest_defaults_fs_net_grants_empty() {
    let toml = "[module]\nname = \"plain\"\n";
    let m = ModuleManifest::from_toml_str(toml).unwrap();
    assert!(m.fs_grants.is_empty());
    assert!(m.net_grants.is_empty());
}

#[test]
fn hotswap_rejects_fs_grant_escalation() {
    use austral_cranelift_bridge::module::ModuleHost;

    let dir_v1 = std::env::temp_dir().join("capstd_swap_v1");
    let dir_v2 = std::env::temp_dir().join("capstd_swap_v2");
    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);

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
    let mut body = Vec::new();
    body.push(0x07);
    body.push(0x01);
    write_i64(&mut body, 1);
    let mut cps = Vec::new();
    write_u32(&mut cps, 0x43505331);
    write_u32(&mut cps, 1);
    write_str(&mut cps, "run");
    write_u32(&mut cps, 0);
    cps.push(0);
    write_u32(&mut cps, body.len() as u32);
    cps.extend_from_slice(&body);

    std::fs::create_dir_all(&dir_v1).unwrap();
    std::fs::create_dir_all(&dir_v2).unwrap();
    std::fs::write(dir_v1.join("module.cps"), &cps).unwrap();
    std::fs::write(dir_v2.join("module.cps"), &cps).unwrap();
    // H8: the archetype must be a registered harness profile the loader maps
    // (rust_capstd is a manifest-parsing label, not a harness profile — the
    // resolver would fall back to austral_cps and reject the mismatch). The
    // test's subject is fs-grant escalation on swap, so both sides use the
    // austral_cps JIT profile with an explicit allowlist.
    std::fs::write(
        dir_v1.join("module.toml"),
        "[module]\nname = \"cap_mod\"\narchetypes = [\"austral_cps\"]\narchetype = \"austral_cps\"\n\n[grants]\nfs = [\"data/\"]\n",
    ).unwrap();
    std::fs::write(
        dir_v2.join("module.toml"),
        "[module]\nname = \"cap_mod\"\narchetypes = [\"austral_cps\"]\narchetype = \"austral_cps\"\n\n[grants]\nfs = [\"data/\", \"/etc/\"]\n",
    ).unwrap();

    let mut host = ModuleHost::new();
    host.load(&dir_v1).unwrap();

    let err = host.swap("cap_mod", &dir_v2).unwrap_err();
    assert!(
        err.contains("escalates"),
        "expected escalation rejection, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);
}

#[cfg(feature = "capstd")]
#[test]
fn capstd_blocks_path_traversal() {
    use austral_cranelift_bridge::capstd_mod::CapFs;
    use std::path::Path;

    let dir = std::env::temp_dir().join("capstd_traversal_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("safe.txt"), b"ok").unwrap();

    let fs = CapFs::open(&dir).unwrap();
    assert!(fs.read_file("safe.txt").is_ok());
    assert!(fs.read_file("../../etc/passwd").is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(not(feature = "capstd"))]
#[test]
fn capstd_stub_returns_error() {
    use austral_cranelift_bridge::capstd_mod::CapFs;
    let result = CapFs::open(std::path::Path::new("/tmp"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("capstd feature not enabled"));
}

// ── D09: instance capabilities ──────────────────────────────────────────────
//
// The host builds one capability set per instance. These tests are the step-4
// negatives: a denied peer produces no sink call, a path cannot leave the state
// directory, a handle cannot be used by another module, and a cancelled request
// never reaches the sink. One test function on purpose: the auth/capability
// registries are process-global and `cargo test` runs functions in parallel.

mod d09 {
    use austral_cranelift_bridge::capstd_mod::caps::{InstanceCaps, Kind, MAX_FRAME};

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("d09_caps_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sink_lines(caps: &InstanceCaps) -> Vec<String> {
        std::fs::read_to_string(caps.root().join("sink.log"))
            .map(|text| text.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn handles_are_instance_scoped_bounded_and_never_reach_a_denied_target() {
        let dir = scratch("main");
        let caps = InstanceCaps::new("d09_mod", &dir, vec!["node-7".to_string()]).unwrap();

        // A directory capability is rooted in the state dir: a relative path
        // stays inside it, and the two ways out are refused before any I/O.
        let dh = caps.open(Kind::Dir).unwrap();
        assert_eq!(
            caps.write(dh, "d09_mod", "notes/a.txt", b"hello").unwrap(),
            5
        );
        assert_eq!(caps.read(dh, "d09_mod", "notes/a.txt").unwrap(), b"hello");
        let escape = caps
            .read(dh, "d09_mod", "../outside.txt")
            .expect_err("`..` must not resolve");
        assert!(escape.to_string().contains("escapes"), "got: {escape}");
        let absolute = caps
            .read(dh, "d09_mod", "/etc/hostname")
            .expect_err("an absolute path must not resolve");
        assert!(
            absolute.to_string().contains("absolute"),
            "got: {absolute}"
        );

        // The handle was issued to one module; a neighbour presenting the same
        // number is refused, so a handle is a capability and not a guessable id.
        let foreign = caps
            .write(dh, "other_mod", "notes/a.txt", b"x")
            .expect_err("another module may not use this handle");
        assert!(
            foreign.to_string().contains("belongs to 'd09_mod'"),
            "got: {foreign}"
        );

        // A frame over the bound is refused rather than allocated.
        let too_big = vec![0u8; MAX_FRAME + 1];
        let over = caps
            .write(dh, "d09_mod", "big.bin", &too_big)
            .expect_err("an oversized write must be refused");
        assert!(over.to_string().contains("exceeds"), "got: {over}");

        // The net capability addresses exactly the configured peer. A denied
        // peer is refused *before* delivery, so the sink sees nothing.
        let nh = caps.open(Kind::Net).unwrap();
        let denied = caps
            .send(nh, "d09_mod", "node-8", b"ping")
            .expect_err("an unconfigured peer must be refused");
        assert!(denied.contains("not configured"), "got: {denied}");
        assert!(
            sink_lines(&caps).is_empty(),
            "a denied send must not reach the sink"
        );

        // An accepted frame is pending until the transport settles it.
        let id = caps.send(nh, "d09_mod", "node-7", b"ping").unwrap();
        assert!(caps.query(nh, "d09_mod", id).unwrap().is_none());
        caps.settle(nh, "transport", id, b"pong").unwrap();
        assert_eq!(caps.query(nh, "d09_mod", id).unwrap().unwrap(), b"pong");
        assert_eq!(sink_lines(&caps).len(), 1, "one delivery, one sink line");

        // A cancelled request is dropped: it never reaches the sink and cannot be
        // cancelled again after it settles.
        let second = caps.send(nh, "d09_mod", "node-7", b"dropped").unwrap();
        caps.cancel(nh, "d09_mod", second).unwrap();
        let cancelled = caps
            .query(nh, "d09_mod", second)
            .expect_err("a cancelled request has no response");
        assert!(cancelled.contains("cancelled"), "got: {cancelled}");
        assert_eq!(
            sink_lines(&caps).len(),
            1,
            "a cancelled send must not reach the sink"
        );

        // Closing the handle ends the capability; the number is not reused by a
        // later open, so a stale id cannot be revived by a neighbour opening one.
        caps.close(nh, "d09_mod").unwrap();
        let by_id = caps.open(Kind::Net).unwrap();
        assert!(by_id > nh, "handle ids are not reused");
        assert!(
            caps.query(nh, "d09_mod", id).is_err(),
            "a closed handle is gone"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_rs_boundary_uses_the_current_instance_and_refuses_a_stale_one() {
        use austral_cranelift_bridge::auth;
        use austral_cranelift_bridge::capstd_mod::caps;

        let dir = scratch("ffi");
        let context = auth::bind_instance("d09_ffi", false);
        caps::install(
            context.instance,
            InstanceCaps::new("d09_ffi", &dir, vec!["node-7".to_string()]).unwrap(),
        );

        // No caller on this thread yet: nothing to attribute a call to.
        assert_eq!(austral_cranelift_bridge::capstd_mod::rs_cap_dir_open(), -1);

        let handle = {
            let _scope = auth::enter_caller(&context);
            let handle = austral_cranelift_bridge::capstd_mod::rs_cap_dir_open();
            assert!(
                handle > 0,
                "the host installed capabilities for this instance"
            );
            let path = "ffi.txt";
            let written = unsafe {
                austral_cranelift_bridge::capstd_mod::rs_cap_write(
                    handle,
                    path.as_ptr(),
                    path.len() as i64,
                    b"via ffi".as_ptr(),
                    7,
                )
            };
            assert_eq!(written, 7);

            // A buffer that is too small reports the required length negatively,
            // so a caller never mistakes a truncation for a full frame.
            let out = [0u8; 2];
            let short = unsafe {
                austral_cranelift_bridge::capstd_mod::rs_cap_read(
                    handle,
                    path.as_ptr(),
                    path.len() as i64,
                    out.as_ptr() as *mut u8,
                    out.len() as i64,
                )
            };
            assert_eq!(short, -7, "required length, negated");

            // An absolute path is refused at the boundary as a bad argument.
            let absolute = "/etc/hostname";
            let refused = unsafe {
                austral_cranelift_bridge::capstd_mod::rs_cap_read(
                    handle,
                    absolute.as_ptr(),
                    absolute.len() as i64,
                    std::ptr::null_mut(),
                    0,
                )
            };
            assert!(refused < 0);
            handle
        };

        // Revoking the instance makes the capability unreachable even though the
        // handle number is still in the table.
        assert!(auth::revoke_instance(context.instance));
        let stale = {
            let _scope = auth::enter_caller(&context);
            austral_cranelift_bridge::capstd_mod::rs_cap_close(handle)
        };
        assert_eq!(stale, -3, "a revoked instance is refused before any handle");
        caps::uninstall(context.instance);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
