//! U18 bounded start: a content-addressed cache for the front-end artifact.
//!
//! The front-end is the expensive, non-executing half of a run: the umbrella
//! spawns `austral` in compile-only mode and reads the CPS bundle and adapter
//! document it wrote. Nothing about that product depends on the provider, the
//! manifest or the JIT, so it can be reused when every input that shaped it is
//! unchanged.
//!
//! What the key covers: the schema of this cache and the literal compiler
//! arguments `frontend::compile` passes, the liquid mode, the compiler binary's
//! bytes, and every source file's bytes (which carry the module declarations
//! and import graph) in the derived leaf-first order. An edit to any of those
//! is a different key. What it deliberately does not cover is anything the
//! artifact does not depend on — provider features, cargo profile, grants; those
//! describe the *use* of the artifact, and they are re-evaluated on every run
//! because the cache sits before `trust::verify_*`, `install_auth` and the JIT
//! call gate. A cache entry is bytes plus the facts the compiler reported about
//! them, never an admission decision: a hit still has to pass the project's
//! signature/descriptor pins and the UK-4001 call gate.
//!
//! A corrupted or foreign entry is not silently used: it is reported and
//! rebuilt, so a half-written artifact cannot answer for this run.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cargo::BuildRequest;
use crate::frontend::{self, Artifact, Export, Liquid};

/// Bumped whenever the cache layout or the compiler invocation changes shape,
/// so an older entry cannot answer for a build that would now differ.
pub const SCHEMA: &str = "umbrella.frontend-cache/1";

/// The literal arguments `frontend::compile` passes besides the paths. They are
/// part of the key because a changed lowering flag changes the artifact even
/// when the sources do not.
const COMPILER_INPUTS: &str =
    "compile --use-cps-jit --emit-cps-only --emit-cps-bundle --target-type=tc";

/// What a cache lookup did. Reported to the caller and the event stream so a
/// warm run is observable instead of looking like a fast cold one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// No cache directory was requested.
    Disabled,
    /// The entry was found and validated; the compiler did not run.
    Hit(String),
    /// No entry for this key; the compiler ran and the result was stored.
    Miss(String),
    /// An entry exists but could not be trusted; the compiler ran and the
    /// fresh result replaced the reason for the refusal with a valid entry.
    Invalid(String, String),
}

impl Outcome {
    pub fn key(&self) -> Option<&str> {
        match self {
            Outcome::Disabled => None,
            Outcome::Hit(key) | Outcome::Miss(key) | Outcome::Invalid(key, _) => Some(key),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Disabled => "disabled",
            Outcome::Hit(_) => "hit",
            Outcome::Miss(_) => "miss",
            Outcome::Invalid(_, _) => "invalid",
        }
    }

    /// Human text for stderr: the key on a hit/miss, the refusal on an invalid
    /// entry.
    pub fn detail(&self) -> Option<String> {
        match self {
            Outcome::Disabled => None,
            Outcome::Hit(key) => Some(key.clone()),
            Outcome::Miss(key) => Some(key.clone()),
            Outcome::Invalid(_, reason) => Some(reason.clone()),
        }
    }
}

/// The record stored beside the CPS bytes. It repeats the fields
/// `Artifact` carries so a load can rebuild the artifact without the compiler,
/// and the digest and module list are re-checked on every load rather than
/// trusted.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    modules: Vec<PathBuf>,
    module_names: Vec<String>,
    /// `(name, params, returns)` per export, in the order the compiler reported
    /// them after the artifact's deduplication.
    exports: Vec<(String, Vec<String>, String)>,
    functions: usize,
    cps_sha256: String,
    adapters: String,
    liquid_required: bool,
    liquid_notes: Vec<String>,
}

impl Record {
    fn of(artifact: &Artifact) -> Self {
        Record {
            modules: artifact.modules.clone(),
            module_names: artifact.module_names.clone(),
            exports: artifact
                .exports
                .iter()
                .map(|e| (e.name.clone(), e.params.clone(), e.returns.clone()))
                .collect(),
            functions: artifact.functions,
            cps_sha256: artifact.cps_sha256.clone(),
            adapters: artifact.adapters.clone(),
            liquid_required: artifact.liquid == Liquid::Required,
            liquid_notes: artifact.liquid_notes.clone(),
        }
    }
}

/// Everything that shapes the provider build besides the sources: the request
/// and the build-affecting environment. Hashed into the run's key so a changed
/// profile, feature set or flag is a different build.
#[derive(Debug, Clone)]
pub struct ProviderInputs {
    pub request: BuildRequest,
    pub env: Vec<(String, String)>,
    /// Paths not to hash even though they sit under the crate directory — the
    /// cache itself, which a project-relative default puts inside the project.
    pub skip: Vec<PathBuf>,
}

/// The inputs one run's key covers. The provider half is absent when no build
/// is authorized, because then no provider build is cached and the front-end
/// artifact does not depend on one.
pub struct KeyInputs<'a> {
    pub austral: &'a Path,
    pub modules: &'a [PathBuf],
    pub liquid_required: bool,
    pub provider: Option<&'a ProviderInputs>,
}

fn hash_field(hasher: &mut Sha256, label: &str, bytes: &[u8]) {
    hasher.update((label.len() as u64).to_le_bytes());
    hasher.update(label.as_bytes());
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The key over everything that shapes the run's derived artifacts: the
/// compiler invocation and binary, every module source in the derived order,
/// and — when a provider build is authorized — the crate tree, the build
/// request, the recorded build environment and the toolchain. `modules` is the
/// derived leaf-first list, whose entries may be `interface,body` pairs exactly
/// as the compiler takes them, so each part is read.
///
/// The provider half is a *content* digest of what umbrella can see: the crate
/// directory's files, the explicit request and the build-affecting variables.
/// It deliberately does not digest the cargo registry, path dependencies
/// outside the crate directory or build-script inputs; a project that pins
/// `library_sha256` still refuses stale bytes before the provider is mapped,
/// and the cache says so in its event rather than claiming more than it can.
pub fn key(inputs: &KeyInputs<'_>) -> Result<String, String> {
    let KeyInputs {
        austral,
        modules,
        liquid_required,
        provider,
    } = inputs;
    let mut h = Sha256::new();
    hash_field(&mut h, "schema", SCHEMA.as_bytes());
    hash_field(&mut h, "compiler-inputs", COMPILER_INPUTS.as_bytes());
    hash_field(
        &mut h,
        "liquid",
        if *liquid_required {
            b"required"
        } else {
            b"optional"
        },
    );
    let compiler = std::fs::read(austral)
        .map_err(|e| format!("front-end cache: read compiler {}: {e}", austral.display()))?;
    hash_field(&mut h, "austral", &compiler);
    hash_field(&mut h, "module-count", &(modules.len() as u64).to_le_bytes());
    for module in modules.iter() {
        let label = module.to_string_lossy();
        hash_field(&mut h, "module", label.as_bytes());
        for part in label.split(',') {
            let path = PathBuf::from(part);
            // A source that does not exist is the compiler's error to report
            // (the ordering check already tolerates it), so the key records
            // absence rather than failing before the front-end runs.
            match std::fs::read(&path) {
                Ok(bytes) => hash_field(&mut h, "source", &bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    hash_field(&mut h, "source-absent", part.as_bytes())
                }
                Err(e) => {
                    return Err(format!(
                        "front-end cache: read module {}: {e}",
                        path.display()
                    ))
                }
            }
        }
    }
    if let Some(provider) = provider {
        hash_provider(&mut h, provider)?;
    }
    Ok(sha256_hex(h.finalize().as_slice()))
}

/// Hash the provider build's inputs: request, environment, toolchain and the
/// crate tree. Sorted by relative path so directory iteration order cannot
/// move the key.
fn hash_provider(h: &mut Sha256, provider: &ProviderInputs) -> Result<(), String> {
    let request = &provider.request;
    hash_field(h, "crate-root", request.crate_dir.to_string_lossy().as_bytes());
    hash_field(h, "crate-package", request.package.as_deref().unwrap_or("").as_bytes());
    hash_field(
        h,
        "crate-target-dir",
        request
            .target_dir
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
            .as_bytes(),
    );
    hash_field(h, "crate-profile", request.profile.as_str().as_bytes());
    hash_field(h, "crate-locked", &[request.locked as u8]);
    hash_field(h, "crate-offline", &[request.offline as u8]);
    hash_field(h, "crate-features", &(request.features.len() as u64).to_le_bytes());
    for feature in &request.features {
        hash_field(h, "crate-feature", feature.as_bytes());
    }
    hash_field(h, "crate-env", &(provider.env.len() as u64).to_le_bytes());
    for (name, value) in &provider.env {
        hash_field(h, "crate-env-name", name.as_bytes());
        hash_field(h, "crate-env-value", value.as_bytes());
    }
    let toolchain = toolchain_probe();
    hash_field(h, "toolchain", toolchain.as_bytes());

    let skip: Vec<PathBuf> = provider
        .skip
        .iter()
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .collect();
    let mut files = Vec::new();
    collect_crate_files(&request.crate_dir, &mut files)?;
    files.sort();
    for path in files {
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if skip.iter().any(|s| canonical.starts_with(s)) {
            continue;
        }
        let relative = path
            .strip_prefix(&request.crate_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        hash_field(h, "crate-path", relative.as_bytes());
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("front-end cache: stat crate file {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            // Do not follow: a symlinked directory can cycle, and the link's
            // target is part of what the build sees.
            let target = std::fs::read_link(&path)
                .map_err(|e| format!("front-end cache: read crate link {}: {e}", path.display()))?;
            hash_field(h, "crate-link", target.to_string_lossy().as_bytes());
        } else {
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("front-end cache: read crate file {}: {e}", path.display()))?;
            hash_field(h, "crate-bytes", &bytes);
        }
    }
    Ok(())
}

/// Every regular file and symlink under `root`, without descending into build
/// output, version control or the cache itself.
fn collect_crate_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(root)
        .map_err(|e| format!("front-end cache: read crate directory {}: {e}", root.display()))?;
    for entry in entries {
        let entry = entry
            .map_err(|e| format!("front-end cache: read crate directory {}: {e}", root.display()))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("front-end cache: stat crate entry {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            out.push(path);
            continue;
        }
        if meta.is_dir() {
            let name = entry.file_name();
            if name == "target" || name == ".git" || name == ".umbrella-cache" {
                continue;
            }
            collect_crate_files(&path, out)?;
        } else if meta.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// The toolchain's own version report. A cache hit skips cargo, so the compiler
/// that would have produced the artifact is an input; a missing tool processes
/// to an `unavailable` marker (the build fails later with its own message).
fn toolchain_probe() -> String {
    let mut out = String::new();
    for (program, args) in [("cargo", vec!["--version"]), ("rustc", vec!["-vV"])] {
        match std::process::Command::new(program).args(&args).output() {
            Ok(output) => {
                out.push_str(program);
                out.push('\n');
                out.push_str(&String::from_utf8_lossy(&output.stdout));
                out.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            Err(e) => out.push_str(&format!("{program}: unavailable: {e}\n")),
        }
    }
    out
}

/// Read and validate the entry for `key`. `Ok(None)` means there is no entry;
/// `Err` means one exists but must not answer for this run (a truncated CPS, a
/// digest mismatch, a different module list). The caller rebuilds in both
/// cases, and only an `Err` is reported as a corrupt entry.
pub fn load(root: &Path, key: &str, modules: &[PathBuf]) -> Result<Option<Artifact>, String> {
    let dir = root.join(key);
    if !dir.exists() {
        return Ok(None);
    }
    let record_path = dir.join("artifact.json");
    let text = std::fs::read_to_string(&record_path).map_err(|e| {
        format!(
            "front-end cache entry {} has no readable record ({e}); rebuilding",
            dir.display()
        )
    })?;
    let record: Record = serde_json::from_str(&text).map_err(|e| {
        format!(
            "front-end cache record {} does not parse ({e}); rebuilding",
            record_path.display()
        )
    })?;
    if record.modules != modules {
        return Err(format!(
            "front-end cache entry {} records a different module list; rebuilding",
            dir.display()
        ));
    }
    let cps_path = dir.join("module.cps");
    let cps = std::fs::read(&cps_path)
        .map_err(|e| format!("front-end cache entry has no {} ({e}); rebuilding", cps_path.display()))?;
    if cps.is_empty() {
        return Err(format!(
            "front-end cache entry {} holds an empty CPS artifact; rebuilding",
            cps_path.display()
        ));
    }
    let actual = sha256_hex(&cps);
    if actual != record.cps_sha256 {
        return Err(format!(
            "front-end cache entry {} records cps sha256 {} but holds {}; rebuilding",
            dir.display(),
            record.cps_sha256,
            actual
        ));
    }
    if record.functions == 0 || record.exports.is_empty() {
        return Err(format!(
            "front-end cache entry {} recorded no functions or no exports; rebuilding",
            dir.display()
        ));
    }
    let mut exports: Vec<Export> = record
        .exports
        .into_iter()
        .map(|(name, params, returns)| Export {
            name,
            params,
            returns,
        })
        .collect();
    exports.sort();
    Ok(Some(Artifact {
        modules: record.modules,
        module_names: record.module_names,
        exports,
        cps,
        cps_sha256: actual,
        functions: record.functions,
        adapters: record.adapters,
        liquid: if record.liquid_required {
            Liquid::Required
        } else {
            Liquid::Optional
        },
        liquid_notes: record.liquid_notes,
    }))
}

/// Remove an entry that failed validation. A refusal is a statement about the
/// bytes on disk, so it must not survive the rebuild that replaces them:
/// otherwise every later run would report the same corrupt entry and recompile.
pub fn invalidate(root: &Path, key: &str) -> Result<(), String> {
    let dir = root.join(key);
    // Only the front-end half: the same key can hold a valid provider build
    // and adapter crate, and a truncated CPS says nothing about those.
    for file in ["module.cps", "artifact.json"] {
        let path = dir.join(file);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(_) if !path.exists() => {}
            Err(e) => {
                return Err(format!(
                    "front-end cache: remove corrupt entry {}: {e}",
                    path.display()
                ))
            }
        }
    }
    Ok(())
}

/// Store `artifact` under `key`, atomically: the bytes are written into a
/// private directory and renamed into place, so a concurrent reader sees either
/// no entry or a complete one. An entry that already exists is left alone —
/// every writer of one key computed the same inputs, so the second result is
/// not more current than the first.
pub fn store(root: &Path, key: &str, artifact: &Artifact) -> Result<(), String> {
    let final_dir = root.join(key);
    if final_dir.join("artifact.json").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&final_dir)
        .map_err(|e| format!("front-end cache: create {}: {e}", final_dir.display()))?;
    // The key directory can already exist for the provider/adapter halves, so
    // each file is written aside and renamed into place rather than moving a
    // whole temporary directory onto a non-empty one.
    let tmp_cps = final_dir.join(format!(".tmp-cps-{}", std::process::id()));
    std::fs::write(&tmp_cps, &artifact.cps)
        .map_err(|e| format!("front-end cache: write CPS: {e}"))?;
    std::fs::rename(&tmp_cps, final_dir.join("module.cps"))
        .map_err(|e| format!("front-end cache: move CPS into place: {e}"))?;
    let record = serde_json::to_string(&Record::of(artifact))
        .map_err(|e| format!("front-end cache: render record: {e}"))?;
    let tmp_record = final_dir.join(format!(".tmp-record-{}", std::process::id()));
    std::fs::write(&tmp_record, record)
        .map_err(|e| format!("front-end cache: write record: {e}"))?;
    std::fs::rename(&tmp_record, final_dir.join("artifact.json"))
        .map_err(|e| format!("front-end cache: move record into place: {e}"))?;
    Ok(())
}

/// The provider build's cache record. The copy is what a hit maps, so the
/// digest is re-checked on load and the recorded source path is diagnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderRecord {
    source: PathBuf,
    file: String,
    sha256: String,
}

/// The cached provider library this key's cargo build produced, if the entry
/// is present and intact. The returned path is the cache's own copy: adapter
/// crates link it and the provider is mapped from it, so one path is shared by
/// every consumer in the run.
pub fn load_provider(root: &Path, key: &str) -> Result<Option<PathBuf>, String> {
    let dir = root.join(key).join("provider");
    let record_path = dir.join("provider.json");
    if !record_path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&record_path).map_err(|e| {
        format!(
            "provider cache entry {} has no readable record ({e}); rebuilding",
            dir.display()
        )
    })?;
    let record: ProviderRecord = serde_json::from_str(&text).map_err(|e| {
        format!(
            "provider cache record {} does not parse ({e}); rebuilding",
            record_path.display()
        )
    })?;
    let path = dir.join(&record.file);
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("provider cache entry has no {} ({e}); rebuilding", path.display()))?;
    if bytes.is_empty() {
        return Err(format!(
            "provider cache entry {} is empty; rebuilding",
            path.display()
        ));
    }
    let actual = sha256_hex(&bytes);
    if actual != record.sha256 {
        return Err(format!(
            "provider cache entry {} records sha256 {} but holds {}; rebuilding",
            path.display(),
            record.sha256,
            actual
        ));
    }
    Ok(Some(path))
}

/// Copy a built provider into the key's entry and return the cache path. An
/// existing copy is replaced: the key changed or the old entry was refused, so
/// the fresh bytes are the ones this run built.
pub fn store_provider(root: &Path, key: &str, source: &Path) -> Result<PathBuf, String> {
    let dir = root.join(key).join("provider");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("provider cache: create {}: {e}", dir.display()))?;
    let file = source
        .file_name()
        .ok_or_else(|| format!("provider cache: {} has no file name", source.display()))?
        .to_owned();
    let bytes = std::fs::read(source)
        .map_err(|e| format!("provider cache: read {}: {e}", source.display()))?;
    let final_path = dir.join(&file);
    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), file.to_string_lossy()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("provider cache: write copy: {e}"))?;
    std::fs::rename(&tmp, &final_path)
        .map_err(|e| format!("provider cache: move copy into place: {e}"))?;
    let record = ProviderRecord {
        source: source.to_path_buf(),
        file: file.to_string_lossy().into_owned(),
        sha256: sha256_hex(&bytes),
    };
    let text = serde_json::to_string(&record)
        .map_err(|e| format!("provider cache: render record: {e}"))?;
    let tmp_record = dir.join(format!(".tmp-record-{}", std::process::id()));
    std::fs::write(&tmp_record, text)
        .map_err(|e| format!("provider cache: write record: {e}"))?;
    std::fs::rename(&tmp_record, dir.join("provider.json"))
        .map_err(|e| format!("provider cache: move record into place: {e}"))?;
    Ok(final_path)
}

/// Remove a provider copy that failed validation, leaving the front-end half of
/// the entry in place.
pub fn invalidate_provider(root: &Path, key: &str) -> Result<(), String> {
    let dir = root.join(key).join("provider");
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(_) if !dir.exists() => Ok(()),
        Err(e) => Err(format!(
            "provider cache: remove corrupt entry {}: {e}",
            dir.display()
        )),
    }
}

/// The digest an adapter crate is stored under. It covers the rendered wrapper
/// source, the generation its tables name, the provider image it links and the
/// bridge rlib it compiles against — the provider path as well as its bytes,
/// because that path is what the artifact records as `DT_NEEDED`.
pub fn adapter_digest(
    source: &str,
    generation: u32,
    provider: &Path,
    bridge_rlib: &Path,
) -> Result<String, String> {
    let mut h = Sha256::new();
    hash_field(&mut h, "schema", SCHEMA.as_bytes());
    hash_field(&mut h, "adapter-source", source.as_bytes());
    hash_field(&mut h, "generation", &generation.to_le_bytes());
    hash_field(
        &mut h,
        "provider-path",
        provider.to_string_lossy().as_bytes(),
    );
    let provider_bytes = std::fs::read(provider)
        .map_err(|e| format!("adapter cache: read provider {}: {e}", provider.display()))?;
    hash_field(&mut h, "provider", &provider_bytes);
    let rlib = std::fs::read(bridge_rlib)
        .map_err(|e| format!("adapter cache: read bridge {}: {e}", bridge_rlib.display()))?;
    hash_field(&mut h, "bridge", &rlib);
    Ok(sha256_hex(h.finalize().as_slice()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AdapterRecord {
    file: String,
    sha256: String,
    generation: u32,
}

/// A validated cached adapter crate: the image to map and the generation its
/// handle tables were rendered at.
pub fn load_adapter(
    root: &Path,
    key: &str,
    digest: &str,
) -> Result<Option<(PathBuf, u32)>, String> {
    let dir = root.join(key).join("adapters").join(digest);
    let record_path = dir.join("adapters.json");
    if !record_path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&record_path).map_err(|e| {
        format!(
            "adapter cache entry {} has no readable record ({e}); rebuilding",
            dir.display()
        )
    })?;
    let record: AdapterRecord = serde_json::from_str(&text).map_err(|e| {
        format!(
            "adapter cache record {} does not parse ({e}); rebuilding",
            record_path.display()
        )
    })?;
    let path = dir.join(&record.file);
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("adapter cache entry has no {} ({e}); rebuilding", path.display()))?;
    if bytes.is_empty() {
        return Err(format!(
            "adapter cache entry {} is empty; rebuilding",
            path.display()
        ));
    }
    let actual = sha256_hex(&bytes);
    if actual != record.sha256 {
        return Err(format!(
            "adapter cache entry {} records sha256 {} but holds {}; rebuilding",
            path.display(),
            record.sha256,
            actual
        ));
    }
    Ok(Some((path, record.generation)))
}

/// Copy a built adapter crate into its digest's entry and return the cache
/// path. The copy — not the scratch build — is what the process maps, so a
/// warm run reuses exactly the bytes this run would have built.
pub fn store_adapter(
    root: &Path,
    key: &str,
    digest: &str,
    built: &Path,
    generation: u32,
) -> Result<PathBuf, String> {
    let dir = root.join(key).join("adapters").join(digest);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("adapter cache: create {}: {e}", dir.display()))?;
    let file = built
        .file_name()
        .ok_or_else(|| format!("adapter cache: {} has no file name", built.display()))?
        .to_owned();
    let bytes = std::fs::read(built)
        .map_err(|e| format!("adapter cache: read {}: {e}", built.display()))?;
    let final_path = dir.join(&file);
    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), file.to_string_lossy()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("adapter cache: write copy: {e}"))?;
    std::fs::rename(&tmp, &final_path)
        .map_err(|e| format!("adapter cache: move copy into place: {e}"))?;
    let record = AdapterRecord {
        file: file.to_string_lossy().into_owned(),
        sha256: sha256_hex(&bytes),
        generation,
    };
    let text = serde_json::to_string(&record)
        .map_err(|e| format!("adapter cache: render record: {e}"))?;
    let tmp_record = dir.join(format!(".tmp-record-{}", std::process::id()));
    std::fs::write(&tmp_record, text)
        .map_err(|e| format!("adapter cache: write record: {e}"))?;
    std::fs::rename(&tmp_record, dir.join("adapters.json"))
        .map_err(|e| format!("adapter cache: move record into place: {e}"))?;
    Ok(final_path)
}

/// Compile `modules` in `austral`, reusing a validated entry when `root` is
/// set and the key matches. The returned [`Outcome`] says which path ran; the
/// artifact is the same shape either way, so every admission check the caller
/// performs afterwards is unchanged and unskippable.
pub fn compile_cached(
    austral: &Path,
    modules: &[PathBuf],
    liquid_required: bool,
    root: Option<&Path>,
) -> Result<(Artifact, Outcome), String> {
    let Some(root) = root else {
        let artifact = frontend::compile(austral, modules, liquid_required)?;
        return Ok((artifact, Outcome::Disabled));
    };
    let key = key(&KeyInputs {
        austral,
        modules,
        liquid_required,
        provider: None,
    })?;
    compile_cached_keyed(austral, modules, liquid_required, root, &key)
}

/// The same lookup with the key already derived, so one run's front-end,
/// provider and adapter artifacts all live under one key.
pub fn compile_cached_keyed(
    austral: &Path,
    modules: &[PathBuf],
    liquid_required: bool,
    root: &Path,
    key: &str,
) -> Result<(Artifact, Outcome), String> {
    let key = key.to_string();
    match load(root, &key, modules) {
        Ok(Some(artifact)) => return Ok((artifact, Outcome::Hit(key))),
        Ok(None) => {}
        Err(reason) => {
            let artifact = frontend::compile(austral, modules, liquid_required)?;
            invalidate(root, &key)?;
            store(root, &key, &artifact)?;
            return Ok((artifact, Outcome::Invalid(key, reason)));
        }
    }
    let artifact = frontend::compile(austral, modules, liquid_required)?;
    store(root, &key, &artifact)?;
    Ok((artifact, Outcome::Miss(key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "umbrella-cache-unit-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn artifact(cps: &[u8], modules: &[PathBuf]) -> Artifact {
        Artifact {
            modules: modules.to_vec(),
            module_names: vec!["M".to_string()],
            exports: vec![Export {
                name: "compute".to_string(),
                params: Vec::new(),
                returns: "Int64".to_string(),
            }],
            cps: cps.to_vec(),
            cps_sha256: sha256_hex(cps),
            functions: 1,
            adapters: String::new(),
            liquid: Liquid::Optional,
            liquid_notes: Vec::new(),
        }
    }

    #[test]
    fn a_stored_artifact_round_trips_and_a_shortened_cps_is_refused() {
        let dir = scratch("roundtrip");
        let modules = vec![dir.join("M.aum")];
        let source = artifact(b"CPS1", &modules);
        let root = dir.join("cache");
        store(&root, "k", &source).unwrap();
        let loaded = load(&root, "k", &modules).unwrap().expect("entry");
        assert_eq!(loaded, source);
        // An entry whose bytes no longer match its recorded digest is not used.
        std::fs::write(root.join("k/module.cps"), b"CPS").unwrap();
        let err = load(&root, "k", &modules).unwrap_err();
        assert!(err.contains("rebuilding"), "{err}");
        assert!(err.contains("sha256"), "{err}");
    }

    #[test]
    fn a_different_module_list_is_not_answered_by_another_entry() {
        let dir = scratch("modules");
        let stored_modules = vec![dir.join("M.aum")];
        let root = dir.join("cache");
        store(&root, "k", &artifact(b"CPS1", &stored_modules)).unwrap();
        let asked = vec![dir.join("Other.aum")];
        let err = load(&root, "k", &asked).unwrap_err();
        assert!(err.contains("different module list"), "{err}");
    }

    #[test]
    fn a_missing_entry_is_a_miss_not_an_error() {
        let dir = scratch("missing");
        assert!(load(&dir.join("cache"), "nope", &[]).unwrap().is_none());
    }

    #[test]
    fn a_provider_copy_round_trips_and_a_corrupt_copy_is_refused() {
        let dir = scratch("provider");
        let source = dir.join("libprovider.so");
        std::fs::write(&source, b"ELF-ish").unwrap();
        let root = dir.join("cache");
        let cached = store_provider(&root, "k", &source).unwrap();
        assert_eq!(std::fs::read(&cached).unwrap(), b"ELF-ish");
        assert_eq!(load_provider(&root, "k").unwrap(), Some(cached.clone()));

        // A copy whose bytes no longer match its record is not mapped.
        std::fs::write(&cached, b"truncated").unwrap();
        let err = load_provider(&root, "k").unwrap_err();
        assert!(err.contains("rebuilding") && err.contains("sha256"), "{err}");
        invalidate_provider(&root, "k").unwrap();
        assert!(load_provider(&root, "k").unwrap().is_none());
    }

    fn key_for(
        compiler: &Path,
        module: &Path,
        provider: Option<&ProviderInputs>,
    ) -> String {
        key(&KeyInputs {
            austral: compiler,
            modules: std::slice::from_ref(&module.to_path_buf()),
            liquid_required: false,
            provider,
        })
        .expect("derive key")
    }

    #[test]
    fn the_key_covers_the_provider_crate_and_request() {
        let dir = scratch("key-provider");
        let compiler = dir.join("austral");
        std::fs::write(&compiler, b"compiler").unwrap();
        let module = dir.join("M.aum");
        std::fs::write(&module, b"module M;\n").unwrap();
        let crate_dir = dir.join("crate");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(crate_dir.join("Cargo.toml"), b"[package]\nname=\"p\"\n").unwrap();
        std::fs::write(crate_dir.join("src/lib.rs"), b"pub fn a() {}\n").unwrap();

        let request = BuildRequest::new(crate_dir.clone());
        let provider = ProviderInputs {
            request: request.clone(),
            env: Vec::new(),
            skip: Vec::new(),
        };
        let front_end_only = key_for(&compiler, &module, None);
        let with_provider = key_for(&compiler, &module, Some(&provider));
        assert_ne!(
            front_end_only, with_provider,
            "a cached provider build must not answer for a run that has none"
        );

        std::fs::write(crate_dir.join("src/lib.rs"), b"pub fn b() {}\n").unwrap();
        assert_ne!(
            with_provider,
            key_for(&compiler, &module, Some(&provider)),
            "an edited crate is a different key"
        );

        let mut featured = request.clone();
        featured.features.push("extra".to_string());
        let featured_provider = ProviderInputs {
            request: featured,
            env: Vec::new(),
            skip: Vec::new(),
        };
        assert_ne!(
            with_provider,
            key_for(&compiler, &module, Some(&featured_provider)),
            "the build request is part of the key"
        );
    }

    #[test]
    fn an_adapter_entry_is_keyed_by_source_generation_and_provider() {
        let dir = scratch("adapter");
        let provider = dir.join("libprovider.so");
        std::fs::write(&provider, b"provider").unwrap();
        let rlib = dir.join("libbridge.rlib");
        std::fs::write(&rlib, b"bridge").unwrap();

        let digest = adapter_digest("generated", 1, &provider, &rlib).unwrap();
        assert_eq!(
            digest,
            adapter_digest("generated", 1, &provider, &rlib).unwrap()
        );
        assert_ne!(
            digest,
            adapter_digest("generated", 2, &provider, &rlib).unwrap(),
            "a generation's tables are part of the artifact"
        );

        let built = dir.join("libadapters_1.so");
        std::fs::write(&built, b"adapter image").unwrap();
        let root = dir.join("cache");
        let cached = store_adapter(&root, "k", &digest, &built, 1).unwrap();
        assert_eq!(
            load_adapter(&root, "k", &digest).unwrap(),
            Some((cached, 1))
        );
        assert_eq!(load_adapter(&root, "k", "other").unwrap(), None);
    }
}
