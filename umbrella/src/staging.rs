//! Private per-build staging (PLAN_umbrella_production U06 bullet 1 / U07).
//!
//! Every build product the umbrella consumes is written into a directory this
//! process created, not next to a user's sources. The directory is created
//! with `create_dir` (an existing path is never reused, so a stale artifact
//! cannot be mistaken for this run's output) and removed on drop. Callers copy
//! what they need out before the guard falls out of scope.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// A fresh, writable directory owned by this process.
#[derive(Debug)]
pub struct Staging {
    path: PathBuf,
    keep: bool,
}

impl Staging {
    /// Create a new staging directory. Fails rather than reusing an existing
    /// path: a pre-existing output is not evidence about this run.
    pub fn create(label: &str) -> io::Result<Self> {
        let base = std::env::temp_dir();
        for _ in 0..64 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = base.join(format!(
                "umbrella-{label}-{}-{id}-{stamp}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Staging { path, keep: false });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a fresh staging directory",
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A path inside the staging directory that must not exist yet.
    pub fn fresh_path(&self, name: &str) -> Result<PathBuf, String> {
        let path = self.path.join(name);
        if path.exists() {
            return Err(format!(
                "staging output {} already exists; refusing to reuse it",
                path.display()
            ));
        }
        Ok(path)
    }

    /// Retain the directory for debugging instead of removing it on drop.
    pub fn retain(&mut self) {
        self.keep = true;
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staging_directory_is_private_and_removed_on_drop() {
        let path = {
            let s = Staging::create("test").unwrap();
            let inner = s.fresh_path("out.cps").unwrap();
            assert!(!inner.exists());
            fs::write(&inner, b"x").unwrap();
            s.path().to_path_buf()
        };
        assert!(!path.exists(), "the staging directory must be cleaned up");
    }

    #[test]
    fn a_pre_existing_output_is_refused() {
        let s = Staging::create("test").unwrap();
        let p = s.fresh_path("out.cps").unwrap();
        fs::write(&p, b"stale").unwrap();
        let err = s.fresh_path("out.cps").unwrap_err();
        assert!(err.contains("refusing to reuse"), "{err}");
    }
}
