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
    /// The symbols this crate exports for the JIT to call. With a local mapping
    /// they are registered explicitly, so the JIT never falls back to global
    /// scope to find an adapter.
    pub exports: Vec<String>,
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
    provider: Option<&Path>,
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
    let mut command = Command::new("rustc");
    command
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=umbrella_adapters")
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("austral_cranelift_bridge={}", rlib.display()));
    if let Some(provider) = provider {
        // U10: link the admitted provider by absolute path. The dynamic loader
        // records that absolute path as `DT_NEEDED`, so it reuses the image
        // already mapped by `ProviderSet` instead of searching global scope or
        // loading a second copy. The adapter can then be mapped `RTLD_LOCAL`.
        let provider = std::fs::canonicalize(provider).map_err(|e| {
            format!(
                "cannot resolve provider {} for adapter linking: {e}",
                provider.display()
            )
        })?;
        command
            .arg("-C")
            .arg(format!("link-arg={}", provider.display()));
    }
    let output = command
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
        exports: adapters.iter().map(|a| a.symbol().to_string()).collect(),
    })
}

/// Prefix of every scratch directory this process mints. The pid is in the
/// name, so a cleanup only ever removes directories its own process created.
fn scratch_prefix() -> String {
    format!("umbrella-adapters-{}-", std::process::id())
}

/// Remove this process's scratch directories when it exits.
///
/// Each generation is a whole cdylib build (~585 MB), and a binary that builds
/// adapters and then exits — the CLI, most visibly when a test spawns it as a
/// subprocess — would otherwise leave one behind per generation. Registering
/// from the library, rather than in each binary, is what keeps the naming and
/// the cleanup in one place. `atexit` runs on the normal exit *and* on
/// `std::process::exit`; a SIGKILLed process can still leak, which is why the
/// pid scoping matters.
fn register_scratch_cleanup() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        libc::atexit(remove_scratch_dirs);
    });
}

extern "C" fn remove_scratch_dirs() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let prefix = scratch_prefix();
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Build the next provider generation into a fresh directory. The generation is
/// process-wide, so the crate a reload builds refuses the handles the previous
/// one minted.
pub fn build_next(adapters: &[interface::GeneratedAdapter<'_>]) -> Result<AdapterCrate, String> {
    build_next_against(adapters, None)
}

/// Build the next generation linked against the admitted provider. The legacy
/// `build_next` leaves the reference unresolved for tests that map the provider
/// globally themselves; the driver uses this form.
pub fn build_next_against(
    adapters: &[interface::GeneratedAdapter<'_>],
    provider: Option<&Path>,
) -> Result<AdapterCrate, String> {
    register_scratch_cleanup();
    let generation = interface::next_provider_generation();
    let dir = std::env::temp_dir().join(format!(
        "umbrella-adapters-{}-{generation}",
        std::process::id()
    ));
    build_at(adapters, &dir, generation, provider)
}

/// Map an [`AdapterCrate`] into the process with the prototype's global scope.
/// Kept for the reload tests, which preload their provider globally and call the
/// wrappers directly. The driver uses [`load_for_jit`].
pub fn load(built: &AdapterCrate) -> Result<(), String> {
    crate::policy::dlopen(&built.path).map(|_| ())
}

/// Map an adapter crate locally and register its exports by exact address, the
/// shape the JIT needs when the provider is `RTLD_LOCAL`: the loader can find
/// each adapter symbol through the crate's own handle, and a preloaded same-named
/// export cannot answer instead.
pub fn load_for_jit(built: &AdapterCrate) -> Result<(), String> {
    let handle = crate::policy::dlopen_local(&built.path)
        .map_err(|e| format!("map adapter crate {}: {e}", built.path.display()))?;
    let existing = austral_cranelift_bridge::registered_static_symbols();
    let mut resolved = Vec::with_capacity(built.exports.len());
    for symbol in &built.exports {
        let name = std::ffi::CString::new(symbol.as_str()).map_err(|e| format!("{symbol}: {e}"))?;
        let address = unsafe { libc::dlsym(handle.as_ptr(), name.as_ptr()) };
        if address.is_null() {
            return Err(format!(
                "adapter crate {} does not export `{symbol}`; the JIT import cannot \
                 be bound to this build",
                built.path.display()
            ));
        }
        if let Some((_, old)) = existing.iter().find(|(n, _)| n == symbol) {
            if *old != address as usize {
                return Err(format!(
                    "adapter export `{symbol}` collides with an existing static \
                     registration at 0x{old:x} (selected 0x{:x})",
                    address as usize
                ));
            }
        }
        resolved.push((symbol, address));
    }
    for (symbol, address) in resolved {
        austral_cranelift_bridge::register_static_symbol(symbol, address as *const u8)
            .map_err(|e| format!("adapter export `{symbol}`: {e}"))?;
    }
    Ok(())
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
