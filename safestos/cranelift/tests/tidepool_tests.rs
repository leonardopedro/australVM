//! Tests for the tidepool module integration (B9).
//!
//! Without GHC/tidepool-extract, we test the Rust-side infrastructure:
//! manifest parsing with archetype/effects, effect gating, and the
//! stub compile_and_run_haskell path.

use austral_cranelift_bridge::module::ModuleManifest;

#[test]
fn manifest_parses_haskell_effect_archetype() {
    let toml = r#"
[module]
name = "hello_kernel"
version = "0.1.0"
archetype = "haskell_effect"
entry = "main"

[grants]
kernel = ["uk_version", "uk_evolve"]
effects = ["Kernel", "Console"]

[limits]
max_ms = 5000
"#;
    let m = ModuleManifest::from_toml_str(toml).unwrap();
    assert_eq!(m.name, "hello_kernel");
    assert_eq!(m.archetype, "haskell_effect");
    assert_eq!(m.entry, "main");
    assert_eq!(m.grants, vec!["uk_version", "uk_evolve"]);
    assert_eq!(m.effects, vec!["Kernel", "Console"]);
    assert_eq!(m.max_ms, Some(5000));
}

#[test]
fn manifest_defaults_archetype_to_austral_cps() {
    let toml = "[module]\nname = \"plain\"\n";
    let m = ModuleManifest::from_toml_str(toml).unwrap();
    assert_eq!(m.archetype, "austral_cps");
    assert!(m.effects.is_empty());
    assert_eq!(m.max_ms, None);
}

#[test]
fn manifest_parses_limits() {
    let toml = "[module]\nname = \"limited\"\n\n[limits]\nmax_ms = 3000\n";
    let m = ModuleManifest::from_toml_str(toml).unwrap();
    assert_eq!(m.max_ms, Some(3000));
}

#[test]
fn compile_and_run_haskell_stub_returns_error() {
    let result = austral_cranelift_bridge::tidepool_mod::compile_and_run_haskell(
        "main = pure 42",
        "main",
        "test_mod",
        &["Kernel".to_string()],
        Some(1000),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("tidepool feature not enabled"));
}

#[test]
fn effect_gating_rejects_ungranted_effect() {
    use austral_cranelift_bridge::auth;

    let engine = auth::ManifestAuthEngine::from_toml_str(
        "[module]\nname = \"gated\"\n\n[grants]\nkernel = [\"uk_version\"]\n",
    )
    .unwrap();
    auth::set_auth_engine(Box::new(engine));

    let result = auth::check("gated", "Call", "effect:Kernel");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("UK-4001"));
}

#[test]
fn effect_gating_allows_granted_effect() {
    use austral_cranelift_bridge::auth;

    let mut engine = auth::ManifestAuthEngine::new();
    engine.grant("gated_ok", "effect:Kernel");
    auth::set_auth_engine(Box::new(engine));

    let result = auth::check("gated_ok", "Call", "effect:Kernel");
    assert!(result.is_ok());
}
