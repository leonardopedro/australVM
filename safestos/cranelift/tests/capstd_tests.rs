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
    use std::path::Path;

    let dir_v1 = std::env::temp_dir().join("capstd_swap_v1");
    let dir_v2 = std::env::temp_dir().join("capstd_swap_v2");
    let _ = std::fs::remove_dir_all(&dir_v1);
    let _ = std::fs::remove_dir_all(&dir_v2);

    fn write_u32(buf: &mut Vec<u8>, v: u32) { buf.extend_from_slice(&v.to_le_bytes()); }
    fn write_i64(buf: &mut Vec<u8>, v: i64) { buf.extend_from_slice(&v.to_le_bytes()); }
    fn write_str(buf: &mut Vec<u8>, s: &str) { write_u32(buf, s.len() as u32); buf.extend_from_slice(s.as_bytes()); }
    let mut body = Vec::new();
    body.push(0x07); body.push(0x01); write_i64(&mut body, 1);
    let mut cps = Vec::new();
    write_u32(&mut cps, 0x43505331); write_u32(&mut cps, 1);
    write_str(&mut cps, "run"); write_u32(&mut cps, 0); cps.push(0);
    write_u32(&mut cps, body.len() as u32); cps.extend_from_slice(&body);

    std::fs::create_dir_all(&dir_v1).unwrap();
    std::fs::create_dir_all(&dir_v2).unwrap();
    std::fs::write(dir_v1.join("module.cps"), &cps).unwrap();
    std::fs::write(dir_v2.join("module.cps"), &cps).unwrap();
    std::fs::write(
        dir_v1.join("module.toml"),
        "[module]\nname = \"cap_mod\"\narchetype = \"rust_capstd\"\n\n[grants]\nfs = [\"data/\"]\n",
    ).unwrap();
    std::fs::write(
        dir_v2.join("module.toml"),
        "[module]\nname = \"cap_mod\"\narchetype = \"rust_capstd\"\n\n[grants]\nfs = [\"data/\", \"/etc/\"]\n",
    ).unwrap();

    let mut host = ModuleHost::new();
    host.load(&dir_v1).unwrap();

    let err = host.swap("cap_mod", &dir_v2).unwrap_err();
    assert!(err.contains("escalates"), "expected escalation rejection, got: {err}");

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
