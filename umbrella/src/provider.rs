//! Explicit provider binding (PLAN_umbrella_production U08).
//!
//! The prototype loads a cdylib with `RTLD_GLOBAL` and lets the bridge's
//! `dlsym(RTLD_DEFAULT)` fallback find its symbols. That makes the identity of a
//! granted name depend on global load order: a preloaded library exporting the
//! same name can win, and two providers exporting one name are silently
//! first-wins.
//!
//! U08 binds the declared exports instead. Each admitted library is mapped, the
//! declared names are resolved with `dlsym(*that handle*, name)`, and the
//! selected addresses are registered in the bridge's static-import registry
//! before the JIT initializes. Registration is still only resolution: the
//! name-based authorization gate in `cps::check_call_permission` is unchanged,
//! so an address in the symbol table never grants permission to call it.
//!
//! The mapping is `RTLD_LOCAL`: the provider is reachable only through its own
//! handle. Generated adapter crates link the admitted provider by absolute path
//! (`adapters::build_next_against`), so their ordinary `rs_*` references resolve
//! to that image rather than to global scope. This module's guarantee covers the
//! JIT imports it registers; the adapter linkage is the other half.
//!
//! Library handles are deliberately never `dlclose`d: the JIT bakes the
//! registered addresses into compiled code, so the mapping must outlive every
//! generated pointer/handle/callback. There is no `Drop`, and a failed
//! admission after mapping also leaves the image mapped rather than pulling it
//! out from under anything already resolved.

use std::collections::HashSet;
use std::ffi::{c_void, CString};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

/// A library that was admitted and whose declared exports were registered.
///
/// Holding this value is what keeps the mapping owned by the run. Dropping it
/// intentionally does **not** unmap the library; see the module docs.
#[derive(Debug)]
pub struct ProviderSet {
    handles: Vec<NonNull<c_void>>,
    paths: Vec<PathBuf>,
    exports: Vec<(String, usize)>,
}

impl ProviderSet {
    /// Map `paths` and bind exactly `declared`.
    ///
    /// Every declared name must resolve from exactly one admitted library. A
    /// name defined by two libraries is an ambiguity, not a load-order choice;
    /// a name defined by none is a refusal; a reserved native name is never a
    /// provider import. Nothing is registered until every name has been
    /// resolved and checked against the existing static registry, so a refusal
    /// leaves the registry untouched.
    pub fn load(paths: &[PathBuf], declared: &[String]) -> Result<Self, String> {
        if paths.is_empty() {
            return Err(
                "no provider library is declared; refusing an undeclared binding".to_string(),
            );
        }
        let paths = unique_paths(paths)?;
        for name in declared {
            validate_export_name(name)?;
        }
        let mut seen = HashSet::new();
        for name in declared {
            if !seen.insert(name.as_str()) {
                return Err(format!(
                    "provider export `{name}` is declared more than once; \
                     a duplicate declaration is an ambiguity"
                ));
            }
        }
        if declared.is_empty() {
            return Err(
                "no provider exports are declared; refusing to map an undeclared library"
                    .to_string(),
            );
        }

        let mut handles = Vec::with_capacity(paths.len());
        for path in &paths {
            handles.push(open(path)?);
        }

        let selected = select_from(handles.len(), declared, |provider, name| {
            lookup(handles[provider], name)
        })?;

        // The bridge's registry is process-global and the JIT consumes it at
        // initialization. A name already pointing somewhere else is a static
        // registry collision; silently replacing it would change which address
        // an earlier declaration named.
        check_registry(
            &selected,
            &austral_cranelift_bridge::registered_static_symbols(),
        )?;

        // Register only after the whole set validated. A failure here is
        // reported by name; the libraries stay mapped (see the module docs).
        for (name, address, _) in &selected {
            austral_cranelift_bridge::register_static_symbol(name, *address as *const u8)
                .map_err(|e| format!("provider export `{name}`: {e}"))?;
        }

        Ok(Self {
            handles,
            paths,
            exports: selected
                .into_iter()
                .map(|(name, address, _)| (name, address))
                .collect(),
        })
    }

    /// `(export, address)` pairs in declared order.
    pub fn exports(&self) -> &[(String, usize)] {
        &self.exports
    }

    /// The mapped libraries, in admission order.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Number of live mappings this value owns.
    pub fn mapped(&self) -> usize {
        self.handles.len()
    }
}

/// Resolve `declared` against `providers` exact handles; `lookup` is `dlsym`
/// scoped to one handle. Kept separate so the selection rule is testable
/// without building shared libraries.
fn select_from<F>(
    providers: usize,
    declared: &[String],
    mut lookup: F,
) -> Result<Vec<(String, usize, usize)>, String>
where
    F: FnMut(usize, &str) -> Option<usize>,
{
    let mut selected = Vec::with_capacity(declared.len());
    for name in declared {
        let mut found = Vec::new();
        for provider in 0..providers {
            if let Some(address) = lookup(provider, name) {
                found.push((provider, address));
            }
        }
        match found.as_slice() {
            [] => {
                return Err(format!(
                    "declared provider export `{name}` is not defined by any admitted \
                     library; the grant and the artifact do not describe the same ABI"
                ));
            }
            [(provider, address)] => selected.push((name.clone(), *address, *provider)),
            _ => {
                let labels: Vec<String> = found
                    .iter()
                    .map(|(p, a)| format!("provider #{p} (0x{a:x})"))
                    .collect();
                return Err(format!(
                    "declared provider export `{name}` is defined by more than one \
                     admitted library: {}; explicit binding refuses to choose by load \
                     order",
                    labels.join(", ")
                ));
            }
        }
    }
    Ok(selected)
}

fn open(path: &Path) -> Result<NonNull<c_void>, String> {
    // RTLD_NOW: every undefined symbol must resolve now. RTLD_LOCAL: the
    // provider is reachable only through this handle. Adapter crates link the
    // provider explicitly (U10), so nothing needs it in the global scope; the
    // identity guarantee for JIT imports is the explicit registration above.
    crate::policy::dlopen_local(path)
        .map_err(|detail| format!("cannot map provider {}: {detail}", path.display()))
}

/// `dlsym` scoped to one image, so a same-named export in another mapped
/// library (or in the global scope) is not considered.
fn lookup(handle: NonNull<c_void>, name: &str) -> Option<usize> {
    let cname = CString::new(name).ok()?;
    let address = unsafe { libc::dlsym(handle.as_ptr(), cname.as_ptr()) };
    if address.is_null() {
        None
    } else {
        Some(address as usize)
    }
}

/// Reject repeated paths, textually and after canonicalization (a symlink and
/// its target are one provider).
fn unique_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut textual = HashSet::new();
    for path in paths {
        if !textual.insert(path.clone()) {
            return Err(format!(
                "provider {} is declared more than once; a repeated library is not a \
                 second provider",
                path.display()
            ));
        }
    }
    let mut canonical = HashSet::new();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if !canonical.insert(resolved.clone()) {
            return Err(format!(
                "provider {} resolves to the same library as another declared \
                 provider ({}); refusing to bind one image twice",
                path.display(),
                resolved.display()
            ));
        }
        // Open the canonical path: the adapter crate records the same absolute
        // path as `DT_NEEDED`, so the loader reuses this mapping instead of
        // opening a second copy through a symlink/relative spelling.
        out.push(resolved);
    }
    Ok(out)
}

/// A provider import may not masquerade as a bridge/native symbol.
fn validate_export_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("provider export: empty name".to_string());
    }
    if name.contains('\0') {
        return Err(format!("provider export `{name}` contains a NUL byte"));
    }
    for (prefix, what) in [
        ("au_", "the Austral runtime"),
        ("__", "a CPS intrinsic"),
        ("uk_", "the unfer kernel"),
        ("uz_", "Zenodo storage"),
    ] {
        if name.starts_with(prefix) {
            return Err(format!(
                "provider export `{name}` uses the reserved `{prefix}` prefix ({what}); \
                 a provider cannot supply a bridge/native symbol"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Command;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("umbrella-provider-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn shared(dir: &Path, stem: &str, value: i64) -> PathBuf {
        let source = dir.join(format!("{stem}.c"));
        let mut f = std::fs::File::create(&source).expect("write fixture source");
        writeln!(
            f,
            "#include <stdint.h>\n\
             int64_t rs_umbrella_pick(void) {{ return {value}; }}"
        )
        .expect("fixture body");
        let library = dir.join(format!("lib{stem}.so"));
        let cc = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
        let out = Command::new(cc)
            .arg("-shared")
            .arg("-fPIC")
            .arg("-o")
            .arg(&library)
            .arg(&source)
            .output()
            .expect("spawn the C compiler");
        assert!(
            out.status.success(),
            "fixture build failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        library
    }

    fn preload_global(path: &Path) {
        let cpath = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        let handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
        assert!(!handle.is_null(), "preload failed");
    }

    #[test]
    fn the_admitted_image_wins_over_a_preload_and_duplicates_are_refused() {
        let dir = scratch("order");
        let first = shared(&dir, "first", 1);
        let second = shared(&dir, "second", 2);
        preload_global(&first);

        // Both images export `rs_umbrella_pick`; binding the admitted second
        // one must take its address, not the preloaded first one's.
        let declaration = vec!["rs_umbrella_pick".to_string()];
        let set = ProviderSet::load(&[second.clone()], &declaration)
            .expect("the exact handle defines the export");
        let address = set.exports()[0].1;
        let pick: extern "C" fn() -> i64 = unsafe { std::mem::transmute(address) };
        assert_eq!(pick(), 2, "the preloaded image must not win");
        assert_eq!(set.mapped(), 1);

        let err = ProviderSet::load(&[first, second], &declaration)
            .expect_err("two definitions are refused, not ordered");
        assert!(err.contains("more than one admitted library"), "{err}");
    }

    #[test]
    fn duplicate_provider_paths_are_refused_before_mapping() {
        let err = unique_paths(&[
            PathBuf::from("/tmp/libsame.so"),
            PathBuf::from("/tmp/libsame.so"),
        ])
        .expect_err("a repeated path is not a second provider");
        assert!(err.contains("declared more than once"), "{err}");
    }

    #[test]
    fn a_declared_export_defined_by_two_providers_is_refused() {
        let declared = vec!["rs_pick".to_string()];
        let err = select_from(2, &declared, |_, _| Some(0x1000))
            .expect_err("two definitions are ambiguous");
        assert!(err.contains("more than one admitted library"), "{err}");
        assert!(err.contains("rs_pick"), "{err}");
    }

    #[test]
    fn a_preloaded_global_export_does_not_win_over_the_exact_handle() {
        // Provider #0 models a preloaded library in the global scope; it does
        // not define the name. Provider #1 is the admitted library. Selection
        // must take #1's address and never fall back to global lookup.
        let declared = vec!["rs_pick".to_string()];
        let picked = select_from(2, &declared, |provider, name| {
            (provider == 1 && name == "rs_pick").then_some(0x2222)
        })
        .expect("exactly one exact-handle definition");
        assert_eq!(picked, vec![("rs_pick".to_string(), 0x2222, 1)]);
    }

    #[test]
    fn load_order_cannot_change_the_selected_address() {
        let declared = vec!["rs_pick".to_string()];
        let forward = select_from(2, &declared, |provider, name| {
            (name == "rs_pick").then_some(0x1000 + provider)
        })
        .expect_err("both providers define it");
        let reversed = select_from(2, &declared, |provider, name| {
            (name == "rs_pick").then_some(0x2000 - provider * 0x1000)
        })
        .expect_err("both providers define it");
        assert!(forward.contains("more than one"), "{forward}");
        assert!(reversed.contains("more than one"), "{reversed}");
    }

    #[test]
    fn a_missing_declared_export_is_refused() {
        let err = select_from(1, &["rs_absent".to_string()], |_, _| None)
            .expect_err("an absent export cannot be bound");
        assert!(err.contains("not defined by any admitted library"), "{err}");
    }

    #[test]
    fn reserved_native_names_are_not_provider_exports() {
        for name in ["au_alloc", "__slot_get", "uk_version", "uz_store"] {
            let err = validate_export_name(name).expect_err(name);
            assert!(err.contains("reserved"), "{name}: {err}");
        }
        assert!(validate_export_name("rs_ok").is_ok());
        assert!(validate_export_name("stageprobe_version").is_ok());
    }

    #[test]
    fn an_existing_registration_at_another_address_is_a_collision() {
        let selected = vec![("rs_provider_collision".to_string(), 0x2000usize, 0)];
        let existing = vec![("rs_provider_collision".to_string(), 0x1000usize)];
        let err = check_registry(&selected, &existing).expect_err("address changed");
        assert!(err.contains("collides"), "{err}");
        assert_eq!(
            check_registry(&selected, &[("rs_provider_collision".to_string(), 0x2000)]),
            Ok(())
        );
    }

    #[test]
    fn an_empty_declaration_is_refused_before_mapping() {
        let err = ProviderSet::load(&[PathBuf::from("/nonexistent/provider.so")], &[])
            .expect_err("undeclared provider");
        assert!(err.contains("no provider exports are declared"), "{err}");
    }
}

fn check_registry(
    selected: &[(String, usize, usize)],
    existing: &[(String, usize)],
) -> Result<(), String> {
    for (name, address, _) in selected {
        if let Some((_, old)) = existing.iter().find(|(n, _)| n == name) {
            if old != address {
                return Err(format!(
                    "provider export `{name}` collides with an existing static \
                     registration at 0x{old:x} (selected 0x{address:x})"
                ));
            }
        }
    }
    Ok(())
}
