//! The host side of the generated adapter crate: render the wrappers the
//! front-end asked for, compile them against the bridge, and load them where
//! the JIT can resolve them.
//!
//! This is a *build*, not a cache: every call renders at the next provider
//! generation and writes a new artifact path, so a provider reload can call it
//! again instead of reusing the previous crate. `dlopen` keys on the file, so
//! reloading the same path would hand back the old image; a new path is what
//! makes the second build's registries — and its generation — real.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::interface;

/// One built adapter crate: the artifact to load and the generation its handle
/// tables were rendered at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterCrate {
    pub path: PathBuf,
    pub generation: u32,
}

/// The bridge rlib the generated wrappers compile against. The wrappers call
/// the same `encode`/`decode` the JIT's scalar ABI implements, so it is linked
/// rather than reimplemented; umbrella already links that crate, so its
/// artifact is next to this executable.
fn bridge_rlib() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let profile = exe
        .parent()
        .ok_or_else(|| "the umbrella executable has no directory".to_string())?;
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in [profile.join("deps"), profile.to_path_buf()] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            let is_rlib = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("libaustral_cranelift_bridge") && n.ends_with(".rlib"))
                .unwrap_or(false);
            if is_rlib {
                found.push(path);
            }
        }
    }
    found.sort();
    found.pop().ok_or_else(|| {
        format!(
            "no bridge rlib under {}; generate the wrappers from a cargo build of umbrella",
            profile.display()
        )
    })
}

/// Render `adapters` at `generation` and compile them into a new cdylib under
/// `dir`. Nothing is loaded here: the caller decides when the artifact becomes
/// visible.
pub fn build_at(
    adapters: &[interface::GeneratedAdapter<'_>],
    dir: &Path,
    generation: u32,
) -> Result<AdapterCrate, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let mut source = String::new();
    for (index, adapter) in adapters.iter().enumerate() {
        let wrapper = adapter
            .render_at(generation)
            .map_err(|e| format!("render {}: {e}", adapter.symbol()))?;
        source.push_str(&format!("mod w{index} {{\n{wrapper}\n}}\n"));
    }
    let file = dir.join(format!("adapters_{generation}.rs"));
    std::fs::write(&file, source).map_err(|e| format!("write {}: {e}", file.display()))?;
    let rlib = bridge_rlib()?;
    let deps = rlib.parent().unwrap_or(dir);
    let out = dir.join(format!("libumbrella_adapters_{generation}.so"));
    let output = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=umbrella_adapters")
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("austral_cranelift_bridge={}", rlib.display()))
        .arg("-o")
        .arg(&out)
        .arg(&file)
        .output()
        .map_err(|e| format!("spawn rustc: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "rustc refused the generated wrappers:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(AdapterCrate {
        path: out,
        generation,
    })
}

/// Build the next provider generation into a fresh directory. The generation is
/// process-wide, so the crate a reload builds refuses the handles the previous
/// one minted.
pub fn build_next(adapters: &[interface::GeneratedAdapter<'_>]) -> Result<AdapterCrate, String> {
    let generation = interface::next_provider_generation();
    let dir = std::env::temp_dir().join(format!(
        "umbrella-adapters-{}-{generation}",
        std::process::id()
    ));
    build_at(adapters, &dir, generation)
}

/// Map an [`AdapterCrate`] into the process. `RTLD_NOW` binds the provider
/// symbols the crate's wrapper code calls; `RTLD_GLOBAL` is what the bridge's
/// `dlsym` fallback needs.
pub fn load(built: &AdapterCrate) -> Result<(), String> {
    crate::policy::dlopen(&built.path).map(|_| ())
}

/// Resolve one symbol from one mapped image. `dlsym` here is scoped to the
/// image, not `RTLD_DEFAULT`: two generations export the same forwarder names,
/// and the reload needs the implementation from a *named* build.
fn resolve(image: *mut libc::c_void, symbol: &str) -> Result<*mut libc::c_void, String> {
    let name = std::ffi::CString::new(symbol).map_err(|e| format!("{symbol}: {e}"))?;
    let ptr = unsafe { libc::dlsym(image, name.as_ptr()) };
    if ptr.is_null() {
        return Err(format!("`{symbol}` is not exported by the mapped build"));
    }
    Ok(ptr)
}

/// Point a loaded build's forwarders at another build's implementations: the
/// live reload's last step. `forwarding` is the image the module's compiled
/// calls target (the first build loaded); `implementations` is the build that
/// just loaded. Both stay mapped — the forwarders still frame the call, and
/// the previous generation's values keep the destructor their own table names.
///
/// This is what lets a reload reach a module that is *already running*: the
/// JIT baked the forwarder addresses when it compiled, so nothing here
/// recompiles or renames; it swaps the body behind those addresses.
pub fn bind(
    forwarding: &Path,
    implementations: &AdapterCrate,
    adapter: &interface::HandleAdapter,
) -> Result<(), String> {
    let forwarding_image = crate::policy::dlopen(forwarding)?;
    let implementations_image = crate::policy::dlopen(&implementations.path)?;
    let bind_symbol = resolve(forwarding_image.as_ptr(), &adapter.bind_symbol())?;
    // SAFETY: the generated signature is `(name: *const u8, len: usize,
    // ptr: *mut c_void) -> i64`, and the image's own export produced it.
    let bind: unsafe extern "C" fn(*const u8, usize, *mut libc::c_void) -> i64 =
        unsafe { std::mem::transmute(bind_symbol) };
    for forwarder in adapter.forwarded_symbols() {
        let implementation_symbol =
            interface::HandleAdapter::implementation_symbol(&forwarder, implementations.generation);
        let implementation = resolve(implementations_image.as_ptr(), &implementation_symbol)?;
        // SAFETY: the name bytes live for the call; the implementation
        // pointer is this build's export of the same forwarder's signature.
        let status = unsafe {
            bind(
                forwarder.as_ptr(),
                forwarder.len(),
                implementation.cast::<libc::c_void>(),
            )
        };
        if status != 0 {
            return Err(format!(
                "`{}` refused to bind `{forwarder}` to `{implementation_symbol}` (status {status})",
                adapter.bind_symbol()
            ));
        }
    }
    Ok(())
}
