//! Capability-based file access for modhost and Rust modules (B10).
//!
//! Tier 1 (in-process): cap-std as declared-intent + defense-in-depth.
//! Tier 2 (VM): the cloud-hypervisor guest is the hard boundary.
//!
//! HONESTY NOTE: Tier 1 cap-std is NOT a sandbox for untrusted Rust code.
//! A module with `unsafe` or `std::fs` can bypass cap-std entirely. cap-std
//! defines the *coupling* (which capabilities cross the boundary); the VM
//! provides the *isolation* for genuinely untrusted code.

#[cfg(feature = "capstd")]
use cap_std::fs::Dir;
#[cfg(feature = "capstd")]
use cap_std::ambient_authority;
#[cfg(feature = "capstd")]
use std::path::Path;

#[cfg(feature = "capstd")]
#[derive(Debug)]
pub struct CapFs {
    root: Dir,
}

#[cfg(feature = "capstd")]
impl CapFs {
    pub fn open(root_path: &Path) -> Result<Self, String> {
        let root = Dir::open_ambient_dir(root_path, ambient_authority())
            .map_err(|e| format!("cannot open root dir {}: {e}", root_path.display()))?;
        Ok(Self { root })
    }

    pub fn read_file(&self, relative: &str) -> Result<Vec<u8>, String> {
        self.root
            .read(relative)
            .map_err(|e| format!("read '{relative}': {e}"))
    }

    pub fn write_file(&self, relative: &str, data: &[u8]) -> Result<(), String> {
        self.root
            .write(relative, data)
            .map_err(|e| format!("write '{relative}': {e}"))
    }

    pub fn list_dir(&self, relative: &str) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        for entry in self
            .root
            .read_dir(relative)
            .map_err(|e| format!("list '{relative}': {e}"))?
        {
            let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        Ok(names)
    }

    pub fn exists(&self, relative: &str) -> bool {
        self.root.exists(relative)
    }
}

#[cfg(not(feature = "capstd"))]
#[derive(Debug)]
pub struct CapFs;

#[cfg(not(feature = "capstd"))]
impl CapFs {
    pub fn open(_root_path: &std::path::Path) -> Result<Self, String> {
        Err("capstd feature not enabled — rebuild with --features capstd".to_string())
    }
}
