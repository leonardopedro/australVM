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
use cap_std::ambient_authority;
#[cfg(feature = "capstd")]
use cap_std::fs::Dir;
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

// ── D09: instance capabilities ──────────────────────────────────────────────
//
// The host constructs one set of capabilities per loaded instance and hands the
// adapter **handles**, never a path it could re-root and never the right to dial
// an arbitrary host. Everything below is the Tier 1 coupling described at the
// top of this file: a module that can call `std::fs` directly is not contained
// by this, and the VM tier is what contains it.

pub mod caps {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// Largest frame one `send` will accept, and the largest response one
    /// `query` will hand back. Stated once so the check and the message agree.
    pub const MAX_FRAME: usize = 1 << 20;
    /// A directory read is bounded by the same limit: a capability that returns
    /// unbounded bytes is a way to exhaust the caller's memory.
    pub const MAX_READ: usize = 1 << 20;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Kind {
        Dir,
        Net,
    }

    /// The filesystem rights a directory handle carries. A handle is opened
    /// with exactly the rights its grant names; nothing is implied by opening a
    /// handle, so a read-only capability cannot create, list, delete or rename.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct FsRights(u8);

    impl FsRights {
        pub const NONE: FsRights = FsRights(0);
        pub const READ: FsRights = FsRights(1 << 0);
        pub const WRITE: FsRights = FsRights(1 << 1);
        pub const CREATE: FsRights = FsRights(1 << 2);
        pub const LIST: FsRights = FsRights(1 << 3);
        pub const DELETE: FsRights = FsRights(1 << 4);
        pub const RENAME: FsRights = FsRights(1 << 5);
        /// The historical `rs_cap_dir_open` grant: every right on one root. It
        /// is named so a caller cannot mistake it for a default.
        pub const ALL: FsRights = FsRights(0b11_1111);

        pub const fn union(self, other: FsRights) -> FsRights {
            FsRights(self.0 | other.0)
        }

        pub const fn contains(self, right: FsRights) -> bool {
            self.0 & right.0 == right.0
        }

        /// Parse a comma-separated rights list (`read,write,create`). Unknown
        /// names are refused instead of ignored, so a typo cannot silently
        /// leave a capability read-only when the author meant writable.
        pub fn parse(text: &str) -> Result<FsRights, String> {
            let mut rights = FsRights::NONE;
            for name in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                rights = rights.union(match name {
                    "read" => FsRights::READ,
                    "write" => FsRights::WRITE,
                    "create" => FsRights::CREATE,
                    "list" => FsRights::LIST,
                    "delete" => FsRights::DELETE,
                    "rename" => FsRights::RENAME,
                    "all" => FsRights::ALL,
                    other => {
                        return Err(format!(
                            "unknown filesystem right `{other}` (expected read, write, \
                             create, list, delete, rename or all)"
                        ))
                    }
                });
            }
            Ok(rights)
        }
    }

    impl std::fmt::Display for FsRights {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let mut names = Vec::new();
            for (right, name) in [
                (FsRights::READ, "read"),
                (FsRights::WRITE, "write"),
                (FsRights::CREATE, "create"),
                (FsRights::LIST, "list"),
                (FsRights::DELETE, "delete"),
                (FsRights::RENAME, "rename"),
            ] {
                if self.contains(right) {
                    names.push(name);
                }
            }
            if names.is_empty() {
                f.write_str("none")
            } else {
                f.write_str(&names.join(","))
            }
        }
    }

    /// A filesystem refusal. The kind is part of the error, not something a
    /// caller recovers by matching message text: a missing file, a denied right
    /// and a path that escapes the capability are different answers.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum FsError {
        /// The handle does not carry the right the operation needs.
        Denied(String),
        /// The path leaves the handle's root, or traverses a symlink.
        Escape(String),
        /// The path does not exist.
        NotFound(String),
        /// The path exists but is not the kind the operation needs.
        NotFile(String),
        /// A create/rename target is already present.
        Exists(String),
        /// The operation is larger than the documented bound, or would take the
        /// instance past the quota its grant names.
        TooLarge(String),
        /// The capability was revoked (or its instance unloaded) after the handle
        /// was issued. Revocation is checked before any filesystem syscall, so a
        /// handle cannot be used for one more operation after it is revoked.
        Revoked(String),
        /// Any other host error, with the host's own text.
        Io(String),
    }

    impl std::fmt::Display for FsError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                FsError::Denied(m)
                | FsError::Escape(m)
                | FsError::NotFound(m)
                | FsError::NotFile(m)
                | FsError::Exists(m)
                | FsError::TooLarge(m)
                | FsError::Revoked(m)
                | FsError::Io(m) => f.write_str(m),
            }
        }
    }

    impl std::error::Error for FsError {}

    fn io_error(context: &str, error: std::io::Error) -> FsError {
        if error.kind() == std::io::ErrorKind::NotFound {
            FsError::NotFound(format!("{context}: not found"))
        } else if error.kind() == std::io::ErrorKind::PermissionDenied {
            FsError::Denied(format!("{context}: {error}"))
        } else {
            FsError::Io(format!("{context}: {error}"))
        }
    }

    /// Split a capability-relative path into its components. This is the only
    /// lexical rule: it refuses an empty path, an absolute path, a NUL byte and
    /// any `..` component before a syscall is made. Containment is then enforced
    /// by the kernel — one `openat` per component with `O_NOFOLLOW` — so a
    /// symlink swapped in between any two steps is refused, never followed.
    fn components(relative: &str) -> Result<Vec<&str>, FsError> {
        if relative.trim().is_empty() {
            return Err(FsError::Denied("empty path".to_string()));
        }
        if relative.starts_with('/') {
            return Err(FsError::Escape(format!(
                "absolute path '{relative}' is not under the capability root"
            )));
        }
        if relative.contains('\0') {
            return Err(FsError::Escape("path contains a NUL byte".to_string()));
        }
        if relative.split('/').any(|part| part == "..") {
            return Err(FsError::Escape(format!(
                "path '{relative}' escapes the capability root"
            )));
        }
        let parts: Vec<&str> = relative
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        if parts.is_empty() {
            return Err(FsError::Denied("empty path".to_string()));
        }
        Ok(parts)
    }

    /// One approved root, opened once by the host and shared by every handle
    /// that names it. The descriptor is the authority: relative paths are
    /// resolved from it with `openat`, never by handing a path back to the host.
    #[cfg(unix)]
    #[derive(Debug)]
    struct RootDir {
        fd: std::os::unix::io::OwnedFd,
        /// Diagnostics only; never used to address a file.
        label: PathBuf,
    }

    #[cfg(not(unix))]
    #[derive(Debug)]
    struct RootDir {
        label: PathBuf,
    }

    #[cfg(unix)]
    impl RootDir {
        fn open(label: PathBuf) -> Result<Self, FsError> {
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;
            use std::os::unix::io::FromRawFd;
            let name = CString::new(label.as_os_str().as_bytes())
                .map_err(|_| FsError::Io("capability root contains a NUL byte".to_string()))?;
            loop {
                let fd = unsafe {
                    libc::open(
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    )
                };
                if fd >= 0 {
                    return Ok(Self {
                        fd: unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fd) },
                        label,
                    });
                }
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                // Opening the root itself refuses to follow a symlink: the
                // host approved a directory, not a link to one.
                return Err(system_error(
                    "cannot open capability root",
                    &label.display().to_string(),
                    error,
                ));
            }
        }

        fn from_fd(fd: std::os::unix::io::OwnedFd, label: PathBuf) -> Self {
            Self { fd, label }
        }

        fn duplicate(&self) -> Result<std::os::unix::io::OwnedFd, FsError> {
            use std::os::unix::io::{AsRawFd, FromRawFd};
            let fd = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
            if fd < 0 {
                return Err(io_error(
                    "cannot duplicate capability root",
                    std::io::Error::last_os_error(),
                ));
            }
            Ok(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fd) })
        }

        /// Walk `parts` from this root, opening each component with
        /// `O_DIRECTORY | O_NOFOLLOW`. The returned descriptor is the directory
        /// that holds the final component.
        fn parent(
            &self,
            parts: &[&str],
            relative: &str,
            create_parents: bool,
        ) -> Result<(std::os::unix::io::OwnedFd, std::ffi::CString), FsError> {
            use std::os::unix::io::AsRawFd;
            let mut dir = self.duplicate()?;
            for part in &parts[..parts.len() - 1] {
                let name = c_name(part, relative)?;
                match open_at(
                    dir.as_raw_fd(),
                    &name,
                    libc::O_RDONLY | libc::O_DIRECTORY,
                    0,
                    "cannot open directory",
                    relative,
                ) {
                    Ok(next) => dir = next,
                    Err(FsError::NotFound(_)) if create_parents => {
                        mkdir_at(dir.as_raw_fd(), &name, relative)?;
                        dir = open_at(
                            dir.as_raw_fd(),
                            &name,
                            libc::O_RDONLY | libc::O_DIRECTORY,
                            0,
                            "cannot open directory",
                            relative,
                        )?;
                    }
                    Err(error) => return Err(error),
                }
            }
            let name = c_name(parts[parts.len() - 1], relative)?;
            Ok((dir, name))
        }

        fn open_final(
            &self,
            relative: &str,
            flags: i32,
            mode: u32,
            create_parents: bool,
        ) -> Result<std::os::unix::io::OwnedFd, FsError> {
            use std::os::unix::io::AsRawFd;
            let parts = components(relative)?;
            let (dir, name) = self.parent(&parts, relative, create_parents)?;
            open_at(dir.as_raw_fd(), &name, flags, mode, "cannot open", relative)
        }

        fn open_directory(&self, relative: &str) -> Result<std::os::unix::io::OwnedFd, FsError> {
            self.open_final(relative, libc::O_RDONLY | libc::O_DIRECTORY, 0, false)
        }

        /// Provision a granted root inside this capability: create each missing
        /// component with `mkdirat` and return the directory descriptor. This is
        /// the host creating the instance's own subtree, never a way for a module
        /// to name a path: the lexical rules above and `O_NOFOLLOW` still apply.
        fn create_directory(&self, relative: &str) -> Result<std::os::unix::io::OwnedFd, FsError> {
            use std::os::unix::io::AsRawFd;
            let parts = components(relative)?;
            let mut dir = self.duplicate()?;
            for part in &parts {
                let name = c_name(part, relative)?;
                match open_at(
                    dir.as_raw_fd(),
                    &name,
                    libc::O_RDONLY | libc::O_DIRECTORY,
                    0,
                    "cannot open directory",
                    relative,
                ) {
                    Ok(next) => dir = next,
                    Err(FsError::NotFound(_)) => {
                        mkdir_at(dir.as_raw_fd(), &name, relative)?;
                        dir = open_at(
                            dir.as_raw_fd(),
                            &name,
                            libc::O_RDONLY | libc::O_DIRECTORY,
                            0,
                            "cannot open directory",
                            relative,
                        )?;
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(dir)
        }

        fn read_file(&self, relative: &str, max: usize) -> Result<Vec<u8>, FsError> {
            use std::io::Read;
            let fd = self.open_final(relative, libc::O_RDONLY, 0, false)?;
            let file = std::fs::File::from(fd);
            let meta = file
                .metadata()
                .map_err(|e| io_error(&format!("cannot stat '{relative}'"), e))?;
            if !meta.is_file() {
                return Err(FsError::NotFile(format!(
                    "{} is not a regular file",
                    self.label.join(relative).display()
                )));
            }
            if meta.len() as usize > max {
                return Err(FsError::TooLarge(format!(
                    "{} is {} bytes, over the {max}-byte limit",
                    self.label.join(relative).display(),
                    meta.len()
                )));
            }
            let mut data = Vec::with_capacity(meta.len() as usize);
            (&file)
                .take(max as u64 + 1)
                .read_to_end(&mut data)
                .map_err(|e| io_error(&format!("cannot read '{relative}'"), e))?;
            if data.len() > max {
                return Err(FsError::TooLarge(format!(
                    "{} grew over the {max}-byte limit while being read",
                    self.label.join(relative).display()
                )));
            }
            Ok(data)
        }

        /// Open an existing regular file for replacement. `O_NOFOLLOW` on the
        /// final component means a symlink is refused here, even if it replaced
        /// the file after the caller looked at it.
        fn open_for_write(&self, relative: &str) -> Result<std::fs::File, FsError> {
            let fd = self.open_final(relative, libc::O_WRONLY | libc::O_TRUNC, 0, false)?;
            let file = std::fs::File::from(fd);
            let meta = file
                .metadata()
                .map_err(|e| io_error(&format!("cannot stat '{relative}'"), e))?;
            if !meta.is_file() {
                return Err(FsError::NotFile(format!(
                    "{} is not a regular file",
                    self.label.join(relative).display()
                )));
            }
            Ok(file)
        }

        /// Create a new file (`O_CREAT | O_EXCL`), creating missing parent
        /// directories inside the capability first. An existing target is
        /// `Exists`, so a create-only right cannot replace a file.
        fn create_file(&self, relative: &str) -> Result<std::fs::File, FsError> {
            let fd = self.open_final(
                relative,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o666,
                true,
            )?;
            Ok(std::fs::File::from(fd))
        }

        fn list_dir(&self, relative: &str) -> Result<Vec<String>, FsError> {
            use std::os::unix::io::IntoRawFd;
            let trimmed = relative.trim();
            let fd = if trimmed.is_empty() || trimmed == "." {
                // The handle's own root is listable; only a *file* operation
                // needs a component below it.
                self.duplicate()?
            } else {
                self.open_directory(relative)?
            };
            // `readdir` on the descriptor we resolved: no path is handed back
            // to the host, so the listing cannot be re-rooted by a swap.
            let dir = unsafe { libc::fdopendir(fd.into_raw_fd()) };
            if dir.is_null() {
                return Err(system_error(
                    "cannot list",
                    relative,
                    std::io::Error::last_os_error(),
                ));
            }
            let mut names = Vec::new();
            loop {
                let entry = unsafe { libc::readdir(dir) };
                if entry.is_null() {
                    break;
                }
                let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
                let name = name.to_string_lossy().into_owned();
                if name != "." && name != ".." {
                    names.push(name);
                }
            }
            unsafe { libc::closedir(dir) };
            names.sort();
            Ok(names)
        }

        fn delete_file(&self, relative: &str) -> Result<(), FsError> {
            use std::os::unix::io::AsRawFd;
            let parts = components(relative)?;
            let (dir, name) = self.parent(&parts, relative, false)?;
            let stat = stat_at(dir.as_raw_fd(), &name, relative)?;
            if stat.st_mode & libc::S_IFMT == libc::S_IFLNK {
                return Err(FsError::Escape(format!(
                    "path '{relative}' is a symlink; a capability does not follow one"
                )));
            }
            if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
                return Err(FsError::NotFile(format!(
                    "{} is not a regular file",
                    self.label.join(relative).display()
                )));
            }
            let rc = unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
            if rc == 0 {
                Ok(())
            } else {
                Err(system_error(
                    "cannot delete",
                    relative,
                    std::io::Error::last_os_error(),
                ))
            }
        }

        fn rename_file(&self, from: &str, to: &str) -> Result<(), FsError> {
            use std::os::unix::io::AsRawFd;
            let from_parts = components(from)?;
            let to_parts = components(to)?;
            let (from_dir, from_name) = self.parent(&from_parts, from, false)?;
            let (to_dir, to_name) = self.parent(&to_parts, to, false)?;
            let source = stat_at(from_dir.as_raw_fd(), &from_name, from)?;
            match source.st_mode & libc::S_IFMT {
                libc::S_IFLNK => {
                    return Err(FsError::Escape(format!(
                        "path '{from}' is a symlink; a capability does not follow one"
                    )))
                }
                libc::S_IFREG => {}
                _ => {
                    return Err(FsError::NotFile(format!(
                        "'{}' is not a regular file",
                        self.label.join(from).display()
                    )))
                }
            }
            match stat_at(to_dir.as_raw_fd(), &to_name, to) {
                Ok(_) => {
                    return Err(FsError::Exists(format!(
                        "rename target '{to}' already exists; refusing to overwrite"
                    )))
                }
                Err(FsError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
            // `RENAME_NOREPLACE` is the race-free form: the kernel refuses a
            // destination that appeared after the stat above.
            let rc = unsafe {
                libc::renameat2(
                    from_dir.as_raw_fd(),
                    from_name.as_ptr(),
                    to_dir.as_raw_fd(),
                    to_name.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            if rc == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ENOSYS) | Some(libc::EINVAL)) {
                // Old kernels: no `renameat2`. The `fstatat` above is the only
                // guard, so this path is documented as best-effort.
                let rc = unsafe {
                    libc::renameat(
                        from_dir.as_raw_fd(),
                        from_name.as_ptr(),
                        to_dir.as_raw_fd(),
                        to_name.as_ptr(),
                    )
                };
                if rc == 0 {
                    return Ok(());
                }
                return Err(system_error(
                    "cannot rename",
                    from,
                    std::io::Error::last_os_error(),
                ));
            }
            Err(system_error("cannot rename", from, error))
        }
    }

    /// The error for an OS call, keeping ENOENT/ELOOP/ENOTDIR/EEXIST distinct so
    /// "not there", "you may not" and "escapes" stay different answers.
    #[cfg(unix)]
    fn system_error(context: &str, relative: &str, error: std::io::Error) -> FsError {
        match error.raw_os_error() {
            Some(libc::ENOENT) => FsError::NotFound(format!("{context} '{relative}': not found")),
            Some(libc::ELOOP) => FsError::Escape(format!(
                "{context} '{relative}': traverses a symlink"
            )),
            Some(libc::ENOTDIR) | Some(libc::EISDIR) => FsError::NotFile(format!(
                "{context} '{relative}': not a regular file"
            )),
            Some(libc::EEXIST) => FsError::Exists(format!("{context} '{relative}': exists")),
            Some(libc::EACCES) | Some(libc::EPERM) => {
                FsError::Denied(format!("{context} '{relative}': {error}"))
            }
            _ => FsError::Io(format!("{context} '{relative}': {error}")),
        }
    }

    #[cfg(unix)]
    fn c_name(part: &str, relative: &str) -> Result<std::ffi::CString, FsError> {
        std::ffi::CString::new(part).map_err(|_| {
            FsError::Escape(format!("path '{relative}' contains a NUL byte"))
        })
    }

    #[cfg(unix)]
    fn open_at(
        dirfd: std::os::unix::io::RawFd,
        name: &std::ffi::CStr,
        flags: i32,
        mode: u32,
        context: &str,
        relative: &str,
    ) -> Result<std::os::unix::io::OwnedFd, FsError> {
        use std::os::unix::io::FromRawFd;
        loop {
            let fd = unsafe {
                libc::openat(
                    dirfd,
                    name.as_ptr(),
                    flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    mode,
                )
            };
            if fd >= 0 {
                return Ok(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fd) });
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            let mapped = system_error(context, relative, error);
            if matches!(mapped, FsError::NotFile(_)) {
                // `O_DIRECTORY | O_NOFOLLOW` reports a symlinked component as
                // ENOTDIR on Linux. Ask the kernel what it is so a symlink is
                // an escape, not a plain "not a file".
                let mut stat: libc::stat = unsafe { std::mem::zeroed() };
                let rc = unsafe {
                    libc::fstatat(dirfd, name.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW)
                };
                if rc == 0 && stat.st_mode & libc::S_IFMT == libc::S_IFLNK {
                    return Err(FsError::Escape(format!(
                        "{context} '{relative}': traverses a symlink"
                    )));
                }
            }
            return Err(mapped);
        }
    }

    #[cfg(unix)]
    fn mkdir_at(
        dirfd: std::os::unix::io::RawFd,
        name: &std::ffi::CStr,
        relative: &str,
    ) -> Result<(), FsError> {
        let rc = unsafe { libc::mkdirat(dirfd, name.as_ptr(), 0o777) };
        if rc == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return Ok(());
        }
        Err(system_error("cannot create directory", relative, error))
    }

    #[cfg(unix)]
    fn stat_at(
        dirfd: std::os::unix::io::RawFd,
        name: &std::ffi::CStr,
        relative: &str,
    ) -> Result<libc::stat, FsError> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::fstatat(
                dirfd,
                name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if rc == 0 {
            Ok(stat)
        } else {
            Err(system_error(
                "cannot stat",
                relative,
                std::io::Error::last_os_error(),
            ))
        }
    }

    #[cfg(not(unix))]
    impl RootDir {
        fn open(label: PathBuf) -> Result<Self, FsError> {
            let meta = std::fs::metadata(&label)
                .map_err(|e| io_error(&format!("cannot stat {}", label.display()), e))?;
            if !meta.is_dir() {
                return Err(FsError::NotFile(format!(
                    "{} is not a directory",
                    label.display()
                )));
            }
            Ok(Self { label })
        }

        fn read_file(&self, _: &str, _: usize) -> Result<Vec<u8>, FsError> {
            Err(unsupported_host())
        }
        fn open_for_write(&self, _: &str) -> Result<std::fs::File, FsError> {
            Err(unsupported_host())
        }
        fn create_file(&self, _: &str) -> Result<std::fs::File, FsError> {
            Err(unsupported_host())
        }
        fn list_dir(&self, _: &str) -> Result<Vec<String>, FsError> {
            Err(unsupported_host())
        }
        fn delete_file(&self, _: &str) -> Result<(), FsError> {
            Err(unsupported_host())
        }
        fn rename_file(&self, _: &str, _: &str) -> Result<(), FsError> {
            Err(unsupported_host())
        }
        fn open_directory(&self, _: &str) -> Result<std::fs::File, FsError> {
            Err(unsupported_host())
        }
        fn create_directory(&self, _: &str) -> Result<std::fs::File, FsError> {
            Err(unsupported_host())
        }
    }

    #[cfg(not(unix))]
    fn unsupported_host() -> FsError {
        FsError::Io(
            "descriptor-relative capability operations require a unix host".to_string(),
        )
    }

    /// One request that was accepted for delivery. It carries owned bytes — no
    /// borrowed pointer is retained across the boundary — and lives until it is
    /// settled, cancelled or dropped.
    #[derive(Debug, Clone)]
    pub struct Request {
        pub id: u64,
        pub peer: String,
        pub payload: Vec<u8>,
        pub response: Option<Vec<u8>>,
        pub cancelled: bool,
    }

    #[derive(Debug)]
    struct Handle {
        kind: Kind,
        owner: String,
        pending: HashMap<u64, Request>,
        /// For a directory handle: the open root descriptor every relative path
        /// is resolved from. The descriptor is owned by the handle (and shared
        /// with the capability set); closing the handle drops its reference and
        /// the root closes with the last one.
        root: Option<Arc<RootDir>>,
        rights: FsRights,
        /// U20: the one connection this `Net` handle owns, if it has dialed.
        /// The socket is dropped with the handle, so a module cannot keep a
        /// connection alive past revocation by holding a second number.
        conn: Option<crate::netstd_mod::Connection>,
        /// Set by `revoke`; a revoked handle is refused before any syscall.
        revoked: bool,
    }

    /// One approved root and the rights a handle opened on it carries.
    #[derive(Debug, Clone)]
    struct RootSlot {
        dir: Arc<RootDir>,
        rights: FsRights,
    }

    /// A root the host approves in addition to the instance state directory.
    /// `root` is relative to that directory, so a grant cannot name a path that
    /// re-roots the capability.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RootGrant {
        pub root: String,
        pub rights: FsRights,
    }

    #[derive(Debug, Default)]
    struct Table {
        next: u64,
        handles: HashMap<u64, Handle>,
    }

    /// The capabilities of one loaded instance.
    #[derive(Debug)]
    pub struct InstanceCaps {
        /// The module this set belongs to; a handle is only usable by it.
        pub module: String,
        /// The instance state directory every relative path resolves under.
        root: PathBuf,
        /// The approved roots, index 0 being the state directory. Each root is
        /// opened once; a handle names one of them and carries its rights.
        roots: Vec<RootSlot>,
        /// Exactly the peers this instance may address (`[module] node`).
        peers: Vec<String>,
        /// Written by `send`, so a test can observe whether a sink call happened.
        sink: PathBuf,
        /// Cumulative bytes a write may add through this capability set. `None`
        /// means the grant names no quota.
        quota: Option<u64>,
        written: std::sync::atomic::AtomicU64,
        /// Set by `revoke_all`: every handle, including future opens, refuses.
        revoked: std::sync::atomic::AtomicBool,
        /// U20: the endpoints, address ranges, trust anchors and deadline this
        /// instance may dial. `None` means it has no network authority at all;
        /// a `Net` handle is never issued without one.
        net: Option<std::sync::Arc<crate::netstd_mod::NetPolicy>>,
        table: Mutex<Table>,
    }

    impl InstanceCaps {
        /// Build the set for `module`, rooted at `state_dir`, with the historical
        /// grant: the whole state directory with every right and no quota. The
        /// host's granted form is [`InstanceCaps::new_with_grants`].
        pub fn new(
            module: impl Into<String>,
            state_dir: &Path,
            peers: Vec<String>,
        ) -> Result<Self, String> {
            Self::new_with_grants(module, state_dir, peers, FsRights::ALL, Vec::new(), None)
        }

        /// Build the set with the rights and roots a grant names. `state_rights`
        /// is what a handle on index 0 carries; `roots` adds approved subtrees,
        /// each with its own rights. Every extra root is opened by walking from
        /// the state descriptor with `O_NOFOLLOW`, so an approved name cannot
        /// resolve to a symlink or to a path outside the state directory.
        pub fn new_with_grants(
            module: impl Into<String>,
            state_dir: &Path,
            peers: Vec<String>,
            state_rights: FsRights,
            roots: Vec<RootGrant>,
            quota: Option<u64>,
        ) -> Result<Self, String> {
            std::fs::create_dir_all(state_dir)
                .map_err(|e| format!("cannot create state dir {}: {e}", state_dir.display()))?;
            let root = std::fs::canonicalize(state_dir)
                .map_err(|e| format!("cannot resolve state dir {}: {e}", state_dir.display()))?;
            let state = RootDir::open(root.clone())
                .map_err(|e| format!("cannot open state dir {}: {e}", root.display()))?;
            let state = Arc::new(state);
            let mut opened = vec![RootSlot {
                dir: state.clone(),
                rights: state_rights,
            }];
            for grant in roots {
                #[cfg(unix)]
                let dir = {
                    // A granted root inside the instance's own state directory
                    // is provisioned when absent; an existing one must be a real
                    // directory (a symlink or file is refused).
                    let fd = match state.open_directory(&grant.root) {
                        Ok(fd) => fd,
                        Err(FsError::NotFound(_)) => state
                            .create_directory(&grant.root)
                            .map_err(|e| format!("capability root '{}': {e}", grant.root))?,
                        Err(error) => {
                            return Err(format!(
                                "capability root '{}': {error} (an approved root must be a \
                                 real directory, not a symlink or file)",
                                grant.root
                            ))
                        }
                    };
                    Arc::new(RootDir::from_fd(fd, root.join(grant.root.trim())))
                };
                #[cfg(not(unix))]
                let dir = {
                    let _ = &grant;
                    return Err("capability roots require a unix host".to_string());
                };
                opened.push(RootSlot {
                    dir,
                    rights: grant.rights,
                });
            }
            let mut allowed = Vec::new();
            for peer in peers {
                let peer = peer.trim().to_string();
                if peer.is_empty() {
                    return Err("a configured peer id must not be empty".to_string());
                }
                if !allowed.contains(&peer) {
                    allowed.push(peer);
                }
            }
            Ok(Self {
                module: module.into(),
                sink: root.join("sink.log"),
                root,
                roots: opened,
                peers: allowed,
                quota,
                written: std::sync::atomic::AtomicU64::new(0),
                revoked: std::sync::atomic::AtomicBool::new(false),
                net: None,
                table: Mutex::new(Table::default()),
            })
        }

        /// Attach the host's network policy. Without one the instance has no
        /// network authority: `open(Kind::Net)` refuses, so a module cannot
        /// acquire a handle it could not use.
        pub fn with_net_policy(mut self, policy: crate::netstd_mod::NetPolicy) -> Self {
            self.net = Some(std::sync::Arc::new(policy));
            self
        }

        /// The policy this instance dials under, if it was granted one.
        pub fn net_policy(&self) -> Option<&std::sync::Arc<crate::netstd_mod::NetPolicy>> {
            self.net.as_ref()
        }

        /// The peers this instance may address. Empty means it may address none:
        /// no `[module] node` is not a permission to dial anyone.
        pub fn peers(&self) -> &[String] {
            &self.peers
        }

        pub fn root(&self) -> &Path {
            &self.root
        }

        /// Open a capability handle owned by this instance's module. The
        /// directory form is the historical grant: the whole state directory
        /// with every right, named `FsRights::ALL` so it is an explicit choice.
        /// A host that wants least privilege calls [`InstanceCaps::open_root`]
        /// with the rights its grant names.
        pub fn open(&self, kind: Kind) -> Result<u64, String> {
            match kind {
                Kind::Dir => self
                    .open_root(0)
                    .map_err(|e| format!("cannot open directory capability: {e}")),
                Kind::Net => {
                    if self.net.is_none() {
                        return Err(format!(
                            "'{}' has no network capability: the host granted no endpoint",
                            self.module
                        ));
                    }
                    self.insert_handle(Kind::Net, None, FsRights::NONE)
                        .map_err(|e| e.to_string())
                }
            }
        }

        fn insert_handle(
            &self,
            kind: Kind,
            root: Option<Arc<RootDir>>,
            rights: FsRights,
        ) -> Result<u64, FsError> {
            if self.is_revoked() {
                return Err(FsError::Revoked(format!(
                    "capabilities for '{}' are revoked; no new handle",
                    self.module
                )));
            }
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            table.next += 1;
            let id = table.next;
            table.handles.insert(
                id,
                Handle {
                    kind,
                    owner: self.module.clone(),
                    pending: HashMap::new(),
                    root,
                    rights,
                    conn: None,
                    revoked: false,
                },
            );
            Ok(id)
        }

        /// Open a host-approved root by index (0 is the state directory) with the
        /// rights that root was granted. This is the injection seam a manifest or
        /// project document names: the module receives a handle, never a path.
        pub fn open_root(&self, index: usize) -> Result<u64, FsError> {
            let slot = self.roots.get(index).ok_or_else(|| {
                FsError::Denied(format!(
                    "this instance has no capability root {index} (it has {})",
                    self.roots.len()
                ))
            })?;
            self.insert_handle(Kind::Dir, Some(slot.dir.clone()), slot.rights)
        }

        /// The number of approved roots, index 0 being the state directory.
        pub fn root_count(&self) -> usize {
            self.roots.len()
        }

        /// The rights a handle opened on `index` would carry.
        pub fn root_rights(&self, index: usize) -> Option<FsRights> {
            self.roots.get(index).map(|slot| slot.rights)
        }

        /// Open a directory handle rooted at `relative_root` inside the state
        /// directory with exactly `rights`. This is the host's injection seam:
        /// an approved root becomes a handle, and the module never sees a path
        /// it could re-root. An empty root (or `.`) names the state directory
        /// itself. The subtree is opened by walking with `O_NOFOLLOW`, so a
        /// symlink at any component is refused here, not followed.
        pub fn open_dir(&self, relative_root: &str, rights: FsRights) -> Result<u64, FsError> {
            if self.is_revoked() {
                return Err(FsError::Revoked(format!(
                    "capabilities for '{}' are revoked",
                    self.module
                )));
            }
            let state = self
                .roots
                .first()
                .ok_or_else(|| FsError::Denied("no state directory capability".to_string()))?;
            if relative_root.is_empty() || relative_root == "." {
                return self.insert_handle(Kind::Dir, Some(state.dir.clone()), rights);
            }
            #[cfg(unix)]
            {
                let fd = state.dir.open_directory(relative_root)?;
                let dir = Arc::new(RootDir::from_fd(
                    fd,
                    self.root.join(relative_root.trim()),
                ));
                self.insert_handle(Kind::Dir, Some(dir), rights)
            }
            #[cfg(not(unix))]
            {
                let _ = rights;
                Err(unsupported_host())
            }
        }

        /// Revoke one handle. It is refused before any syscall from then on; the
        /// descriptor closes when the last handle naming it is dropped.
        pub fn revoke(&self, handle: u64, caller: &str) -> Result<(), FsError> {
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let entry = table
                .handles
                .get_mut(&handle)
                .ok_or_else(|| FsError::Denied(format!("no capability handle {handle}")))?;
            if entry.owner != caller {
                return Err(FsError::Denied(format!(
                    "capability handle {handle} belongs to '{}', not to '{caller}'",
                    entry.owner
                )));
            }
            entry.revoked = true;
            entry.pending.clear();
            entry.pending.shrink_to_fit();
            // A revoked network handle drops its socket here rather than at
            // close, so revocation is the end of the connection and not just a
            // refusal on the next verb.
            entry.conn = None;
            Ok(())
        }

        /// Revoke the whole set (instance unload or hot-swap). Every outstanding
        /// handle is dropped, and a later `open`/`open_root` refuses; the root
        /// descriptors close when this set is dropped.
        pub fn revoke_all(&self) {
            self.revoked
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            table.handles.clear();
        }

        pub fn is_revoked(&self) -> bool {
            self.revoked.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// Bytes this capability set has accepted for writing. Quota is
        /// cumulative across the set, not a live disk-usage figure: deleting a
        /// file does not refund a write.
        pub fn written(&self) -> u64 {
            self.written.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// Reserve `bytes` against the granted quota. Refused writes are never
        /// charged, and a write that fails after being charged is refunded.
        fn charge(&self, bytes: usize) -> Result<(), FsError> {
            let Some(quota) = self.quota else {
                return Ok(());
            };
            let mut current = self.written.load(std::sync::atomic::Ordering::SeqCst);
            loop {
                let next = current.saturating_add(bytes as u64);
                if next > quota {
                    return Err(FsError::TooLarge(format!(
                        "write of {bytes} bytes would take '{}' past its {quota}-byte \
                         capability quota ({} already written)",
                        self.module, current
                    )));
                }
                match self.written.compare_exchange_weak(
                    current,
                    next,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                ) {
                    Ok(_) => return Ok(()),
                    Err(actual) => current = actual,
                }
            }
        }

        fn refund(&self, bytes: usize) {
            if self.quota.is_some() {
                self.written
                    .fetch_sub(bytes as u64, std::sync::atomic::Ordering::SeqCst);
            }
        }

        /// A handle used by another module is refused: the capability was issued
        /// to one instance, and a number is not a capability if any neighbour can
        /// present it.
        fn with_handle<T>(
            &self,
            handle: u64,
            caller: &str,
            want: Kind,
            f: impl FnOnce(&mut Handle) -> Result<T, String>,
        ) -> Result<T, String> {
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let entry = table
                .handles
                .get_mut(&handle)
                .ok_or_else(|| format!("no capability handle {handle}"))?;
            if entry.owner != caller {
                return Err(format!(
                    "capability handle {handle} belongs to '{}', not to '{caller}'",
                    entry.owner
                ));
            }
            if entry.kind != want {
                return Err(format!(
                    "capability handle {handle} is not a {want:?} handle"
                ));
            }
            if entry.revoked {
                return Err(format!("capability handle {handle} is revoked"));
            }
            if self.is_revoked() {
                return Err(format!(
                    "capabilities for '{}' are revoked",
                    self.module
                ));
            }
            f(entry)
        }

        /// Run `f` with the open root descriptor and rights of a directory
        /// handle, after checking ownership, kind, revocation and that `needs`
        /// is granted. Every filesystem operation goes through here, so a right
        /// cannot be checked in one verb and forgotten in another.
        fn with_fs_handle<T>(
            &self,
            handle: u64,
            caller: &str,
            needs: FsRights,
            f: impl FnOnce(&RootDir, FsRights) -> Result<T, FsError>,
        ) -> Result<T, FsError> {
            if self.is_revoked() {
                return Err(FsError::Revoked(format!(
                    "capabilities for '{}' are revoked",
                    self.module
                )));
            }
            let table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let entry = table
                .handles
                .get(&handle)
                .ok_or_else(|| FsError::Denied(format!("no capability handle {handle}")))?;
            if entry.owner != caller {
                return Err(FsError::Denied(format!(
                    "capability handle {handle} belongs to '{}', not to '{caller}'",
                    entry.owner
                )));
            }
            if entry.kind != Kind::Dir {
                return Err(FsError::Denied(format!(
                    "capability handle {handle} is not a directory handle"
                )));
            }
            if entry.revoked {
                return Err(FsError::Revoked(format!(
                    "capability handle {handle} is revoked"
                )));
            }
            if !entry.rights.contains(needs) {
                return Err(FsError::Denied(format!(
                    "capability handle {handle} grants [{}], so it cannot {needs}",
                    entry.rights
                )));
            }
            let root = entry.root.as_ref().ok_or_else(|| {
                FsError::Denied(format!(
                    "capability handle {handle} has no directory root"
                ))
            })?;
            f(root, entry.rights)
        }

        /// Write `data` at `relative` under a directory capability. A new path
        /// needs `create`; an existing path needs `write`. The two are separate
        /// so a writer cannot grow the tree it was only allowed to edit.
        pub fn write(
            &self,
            handle: u64,
            caller: &str,
            relative: &str,
            data: &[u8],
        ) -> Result<u64, FsError> {
            if data.len() > MAX_FRAME {
                return Err(FsError::TooLarge(format!(
                    "write of {} bytes exceeds the {MAX_FRAME}-byte limit",
                    data.len()
                )));
            }
            self.with_fs_handle(handle, caller, FsRights::NONE, |root, rights| {
                let can_write = rights.contains(FsRights::WRITE);
                let can_create = rights.contains(FsRights::CREATE);
                if !can_write && !can_create {
                    return Err(FsError::Denied(format!(
                        "handle grants [{}], so it cannot write '{relative}'",
                        rights
                    )));
                }
                // The quota is charged before the descriptor is opened, so a
                // refused write never truncates or creates the target.
                self.charge(data.len())?;
                let written = (|| -> Result<u64, FsError> {
                    let file = if can_write {
                        match root.open_for_write(relative) {
                            Ok(file) => file,
                            Err(FsError::NotFound(_)) if can_create => {
                                match root.create_file(relative) {
                                    Ok(file) => file,
                                    // It appeared between the two opens; the
                                    // `write` right covers replacing it.
                                    Err(FsError::Exists(_)) => root.open_for_write(relative)?,
                                    Err(error) => return Err(error),
                                }
                            }
                            Err(FsError::NotFound(_)) => {
                                return Err(FsError::Denied(format!(
                                    "handle grants [{}], so it cannot create '{relative}'",
                                    rights
                                )))
                            }
                            Err(error) => return Err(error),
                        }
                    } else {
                        match root.create_file(relative) {
                            Ok(file) => file,
                            Err(FsError::Exists(_)) => {
                                return Err(FsError::Denied(format!(
                                    "handle grants [{}], so it cannot replace existing \
                                     '{relative}'",
                                    rights
                                )))
                            }
                            Err(error) => return Err(error),
                        }
                    };
                    use std::io::Write;
                    let mut file = file;
                    file.write_all(data).map_err(|e| {
                        io_error(&format!("cannot write '{relative}'"), e)
                    })?;
                    Ok(data.len() as u64)
                })();
                if written.is_err() {
                    self.refund(data.len());
                }
                written
            })
        }

        /// Read a regular file under a directory capability. A symlink is never
        /// followed, and a missing file is reported as `NotFound`, not as a
        /// rights refusal.
        pub fn read(
            &self,
            handle: u64,
            caller: &str,
            relative: &str,
        ) -> Result<Vec<u8>, FsError> {
            self.with_fs_handle(handle, caller, FsRights::READ, |root, _| {
                root.read_file(relative, MAX_READ)
            })
        }

        /// List a directory under a directory capability. Names are returned as
        /// the host spells them, lossily decoded where a name is not UTF-8; the
        /// module ABI carries UTF-8, so a non-UTF-8 name is listed in its
        /// replacement form rather than silently dropped.
        pub fn list(
            &self,
            handle: u64,
            caller: &str,
            relative: &str,
        ) -> Result<Vec<String>, FsError> {
            self.with_fs_handle(handle, caller, FsRights::LIST, |root, _| {
                root.list_dir(relative)
            })
        }

        /// Delete one regular file. Directories are refused: a tree delete is a
        /// different right, and removing a directory can discard files the
        /// caller never saw.
        pub fn delete(&self, handle: u64, caller: &str, relative: &str) -> Result<(), FsError> {
            self.with_fs_handle(handle, caller, FsRights::DELETE, |root, _| {
                root.delete_file(relative)
            })
        }

        /// Rename one entry inside the same capability. Both paths are resolved
        /// under the handle's root and no destination overwrite happens, so a
        /// rename cannot replace a file the caller was not allowed to write.
        pub fn rename(
            &self,
            handle: u64,
            caller: &str,
            from: &str,
            to: &str,
        ) -> Result<(), FsError> {
            self.with_fs_handle(handle, caller, FsRights::RENAME, |root, _| {
                root.rename_file(from, to)
            })
        }

        /// Owner/kind/revocation check for a `Net` handle. Kept separate from
        /// `with_handle` so a refusal keeps its typed reason (policy, deadline,
        /// TLS) instead of collapsing into one string.
        fn with_net_handle<T>(
            &self,
            handle: u64,
            caller: &str,
            f: impl FnOnce(&mut Handle, &crate::netstd_mod::NetPolicy) -> Result<T, crate::netstd_mod::NetError>,
        ) -> Result<T, crate::netstd_mod::NetError> {
            use crate::netstd_mod::NetError;
            if self.is_revoked() {
                return Err(NetError::Denied(format!(
                    "capabilities for '{}' are revoked",
                    self.module
                )));
            }
            let policy = self.net.as_ref().ok_or_else(|| {
                NetError::Denied(format!("'{}' has no network capability", self.module))
            })?;
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let entry = table
                .handles
                .get_mut(&handle)
                .ok_or_else(|| NetError::Denied(format!("no capability handle {handle}")))?;
            if entry.owner != caller {
                return Err(NetError::Denied(format!(
                    "capability handle {handle} belongs to '{}', not to '{caller}'",
                    entry.owner
                )));
            }
            if entry.kind != Kind::Net {
                return Err(NetError::Denied(format!(
                    "capability handle {handle} is not a Net handle"
                )));
            }
            if entry.revoked {
                return Err(NetError::Denied(format!(
                    "capability handle {handle} is revoked"
                )));
            }
            f(entry, policy)
        }

        /// Dial an approved endpoint through a `Net` handle. The host re-checks
        /// the endpoint, every address the name resolved to and (with `tls`)
        /// the peer's certificate identity; the returned address is the one
        /// that actually accepted, not the name that was requested.
        pub fn dial(
            &self,
            handle: u64,
            caller: &str,
            host: &str,
            port: u16,
            tls: bool,
        ) -> Result<std::net::SocketAddr, crate::netstd_mod::NetError> {
            self.with_net_handle(handle, caller, |entry, policy| {
                if entry.conn.is_some() {
                    return Err(crate::netstd_mod::NetError::Denied(format!(
                        "handle {handle} already has an open connection; close it before dialing \
                         again"
                    )));
                }
                let connection = policy.dial(host, port, tls)?;
                let peer = connection.peer();
                entry.conn = Some(connection);
                Ok(peer)
            })
        }

        /// Write one bounded frame on a dialed connection.
        pub fn net_write(
            &self,
            handle: u64,
            caller: &str,
            data: &[u8],
        ) -> Result<usize, crate::netstd_mod::NetError> {
            use crate::netstd_mod::NetError;
            if data.len() > MAX_FRAME {
                return Err(NetError::TooLarge(format!(
                    "write of {} bytes exceeds the {MAX_FRAME}-byte network frame limit",
                    data.len()
                )));
            }
            self.with_net_handle(handle, caller, |entry, _policy| {
                entry
                    .conn
                    .as_mut()
                    .ok_or_else(|| NetError::Denied(format!("handle {handle} has not dialed")))?
                    .write(data)
            })
        }

        /// Read once, up to `max` bytes, on a dialed connection. An empty
        /// return is EOF.
        pub fn net_read(
            &self,
            handle: u64,
            caller: &str,
            max: usize,
        ) -> Result<Vec<u8>, crate::netstd_mod::NetError> {
            use crate::netstd_mod::NetError;
            let max = max.min(MAX_READ);
            if max == 0 {
                return Ok(Vec::new());
            }
            self.with_net_handle(handle, caller, |entry, _policy| {
                entry
                    .conn
                    .as_mut()
                    .ok_or_else(|| NetError::Denied(format!("handle {handle} has not dialed")))?
                    .read(max)
            })
        }

        /// Half-close the write side of a dialed connection. The read side stays
        /// open so the response can still arrive.
        pub fn net_shutdown(
            &self,
            handle: u64,
            caller: &str,
        ) -> Result<(), crate::netstd_mod::NetError> {
            self.with_net_handle(handle, caller, |entry, _policy| {
                entry
                    .conn
                    .as_mut()
                    .ok_or_else(|| {
                        crate::netstd_mod::NetError::Denied(format!(
                            "handle {handle} has not dialed"
                        ))
                    })?
                    .shutdown_write();
                Ok(())
            })
        }

        /// Accept one bounded frame for `peer`. A peer outside the configured set
        /// is refused *here*, before anything is delivered: the adapter has no
        /// permission to dial arbitrary hosts.
        pub fn send(
            &self,
            handle: u64,
            caller: &str,
            peer: &str,
            payload: &[u8],
        ) -> Result<u64, String> {
            if payload.len() > MAX_FRAME {
                return Err(format!(
                    "frame of {} bytes exceeds the {MAX_FRAME}-byte limit",
                    payload.len()
                ));
            }
            if !self.peers.iter().any(|p| p == peer) {
                return Err(format!(
                    "peer '{peer}' is not configured for '{}' (allowed: {:?})",
                    self.module, self.peers
                ));
            }
            self.with_handle(handle, caller, Kind::Net, |entry| {
                let id = entry.pending.keys().max().copied().unwrap_or(0) + 1;
                entry.pending.insert(
                    id,
                    Request {
                        id,
                        peer: peer.to_string(),
                        // Owned copy: nothing borrowed from the caller survives
                        // the call.
                        payload: payload.to_vec(),
                        response: None,
                        cancelled: false,
                    },
                );
                Ok(id)
            })
        }

        /// Settle a pending request. The transport (or, in tests, the demo sink)
        /// is the only caller: a module cannot answer its own request.
        /// Called by the transport, not by the module: a caller presenting its
        /// own request id would otherwise be able to answer itself. That is why
        /// there is no `caller` argument to check here.
        pub fn settle(
            &self,
            handle: u64,
            _caller: &str,
            id: u64,
            response: &[u8],
        ) -> Result<(), String> {
            let delivered = {
                let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
                let entry = table
                    .handles
                    .get_mut(&handle)
                    .ok_or_else(|| format!("no capability handle {handle}"))?;
                match entry.pending.get_mut(&id) {
                    None => return Err(format!("no pending request {id}")),
                    Some(request) if request.cancelled => {
                        return Err(format!("request {id} was cancelled"))
                    }
                    Some(request) => {
                        request.response = Some(response.to_vec());
                        (request.peer.clone(), request.payload.clone())
                    }
                }
            };
            self.deliver(&delivered.0, &delivered.1, response)
        }

        /// The demo sink: append the frame and the response to `sink.log` in the
        /// state directory. A refused or cancelled request never reaches it, which
        /// is what the tests observe.
        fn deliver(&self, peer: &str, payload: &[u8], response: &[u8]) -> Result<(), String> {
            use std::io::Write;
            let line = format!(
                "peer={peer} sent={} got={}\n",
                payload.len(),
                response.len()
            );
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.sink)
                .map_err(|e| format!("cannot open sink {}: {e}", self.sink.display()))?;
            file.write_all(line.as_bytes())
                .map_err(|e| format!("cannot append to sink: {e}"))
        }

        pub fn query(&self, handle: u64, caller: &str, id: u64) -> Result<Option<Vec<u8>>, String> {
            self.with_handle(handle, caller, Kind::Net, |entry| {
                match entry.pending.get(&id) {
                    None => Err(format!("no pending request {id}")),
                    Some(request) if request.cancelled => {
                        Err(format!("request {id} was cancelled"))
                    }
                    Some(request) => Ok(request.response.clone()),
                }
            })
        }

        /// Wait for a request to settle, up to `timeout_ms`. Bounded on purpose:
        /// a module that can block the host forever is a denial of service.
        pub fn wait(
            &self,
            handle: u64,
            caller: &str,
            id: u64,
            timeout_ms: u64,
        ) -> Result<Option<Vec<u8>>, String> {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
            loop {
                match self.query(handle, caller, id)? {
                    Some(response) => return Ok(Some(response)),
                    None if std::time::Instant::now() >= deadline => return Ok(None),
                    None => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        }

        /// Cancel a request that has not settled. Its frame is dropped: no `send`
        /// is retried behind the caller's back, so a dropped request cannot become
        /// an unobserved effect.
        pub fn cancel(&self, handle: u64, caller: &str, id: u64) -> Result<(), String> {
            self.with_handle(handle, caller, Kind::Net, |entry| {
                match entry.pending.get_mut(&id) {
                    None => Err(format!("no pending request {id}")),
                    Some(request) if request.response.is_some() => Err(format!(
                        "request {id} already settled and cannot be cancelled"
                    )),
                    Some(request) => {
                        request.cancelled = true;
                        request.payload.clear();
                        Ok(())
                    }
                }
            })
        }

        /// Close a handle, dropping any request still pending under it.
        pub fn close(&self, handle: u64, caller: &str) -> Result<(), String> {
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            match table.handles.get(&handle) {
                None => Err(format!("no capability handle {handle}")),
                Some(entry) if entry.owner != caller => Err(format!(
                    "capability handle {handle} belongs to '{}', not to '{caller}'",
                    entry.owner
                )),
                Some(_) => {
                    table.handles.remove(&handle);
                    Ok(())
                }
            }
        }
    }

    // ── The registry the JIT resolves handles against ──────────────────────

    use std::sync::{Arc, OnceLock};

    static INSTALLED: OnceLock<Mutex<HashMap<u64, Arc<InstanceCaps>>>> = OnceLock::new();

    fn installed() -> &'static Mutex<HashMap<u64, Arc<InstanceCaps>>> {
        INSTALLED.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Give one instance its capabilities. The host calls this after the identity
    /// is bound; a module never constructs its own. A replacement revokes the
    /// previous set, so an outstanding handle from the old generation cannot be
    /// used after the swap.
    pub fn install(instance: u64, caps: InstanceCaps) {
        let mut map = installed().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = map.get(&instance) {
            previous.revoke_all();
        }
        map.insert(instance, Arc::new(caps));
    }

    /// Drop an instance's capabilities (failed load, revocation, unload). The
    /// set is revoked before it is released, so a caller that already cloned the
    /// `Arc` sees a dead capability rather than a live one for one more call.
    pub fn uninstall(instance: u64) {
        let mut map = installed().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(caps) = map.remove(&instance) {
            caps.revoke_all();
        }
    }

    pub fn caps_for(instance: u64) -> Option<Arc<InstanceCaps>> {
        installed()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&instance)
            .cloned()
    }
}

// ── D09 step 3: the `rs_cap_*` boundary ─────────────────────────────────────
//
// Every one of these is a C-ABI function an Austral module reaches through the
// manifest gate. The rules they all keep:
//
//   * the caller is the *current* instance context (D08), never an argument, so
//     a module cannot name another module's identity;
//   * a revoked or replaced instance is refused before any handle is looked up;
//   * lengths are checked against fixed bounds before allocation;
//   * buffers are copied in and out — no borrowed pointer is retained;
//   * a panic is contained and reported as a negative code, because unwinding
//     into JIT'd code is undefined.

/// Negative return codes, so a caller can tell the refusals apart.
pub const CAP_ERR_NO_INSTANCE: i64 = -1;
pub const CAP_ERR_NO_CAPS: i64 = -2;
pub const CAP_ERR_STALE: i64 = -3;
pub const CAP_ERR_BAD_ARG: i64 = -4;
pub const CAP_ERR_REFUSED: i64 = -5;
pub const CAP_ERR_TOO_LONG: i64 = -6;
/// The path does not exist. Distinct from a rights refusal: a caller can tell
/// "you may not" from "it is not there" and report the right reason. Like every
/// negative code here it shares the space `copy_out` uses for "required length,
/// negated", so a caller that gets a negative value must know which call it
/// made and how large its buffer was.
pub const CAP_ERR_NOT_FOUND: i64 = -7;
/// The handle (or the whole capability set) was revoked. Distinct from a rights
/// refusal: the grant did not change, the authority was withdrawn.
pub const CAP_ERR_REVOKED: i64 = -8;
/// The dial or an individual IO operation exceeded its deadline.
pub const CAP_ERR_TIMEOUT: i64 = -9;
/// TLS identity verification failed (wrong name, untrusted chain, peer close
/// during the handshake). There is no insecure fallback: this is the result.
pub const CAP_ERR_TLS: i64 = -10;
/// The requested network operation (UDP, listen, raw socket) is not provided.
pub const CAP_ERR_UNSUPPORTED: i64 = -11;
pub const CAP_ERR_IO: i64 = -12;
pub const CAP_ERR_PANIC: i64 = -101;

/// The ABI code for a network refusal. Each variant is distinct on purpose: a
/// module reporting "timed out" must not be able to mistake it for "denied".
fn net_code(error: &crate::netstd_mod::NetError) -> i64 {
    use crate::netstd_mod::NetError;
    match error {
        NetError::NotFound(_) => CAP_ERR_NOT_FOUND,
        NetError::Timeout(_) => CAP_ERR_TIMEOUT,
        NetError::Tls(_) => CAP_ERR_TLS,
        NetError::Unsupported(_) => CAP_ERR_UNSUPPORTED,
        NetError::TooLarge(_) => CAP_ERR_TOO_LONG,
        NetError::Io(_) => CAP_ERR_IO,
        NetError::Denied(_) => CAP_ERR_REFUSED,
    }
}

/// The ABI code for a filesystem refusal.
fn fs_code(error: &caps::FsError) -> i64 {
    match error {
        caps::FsError::NotFound(_) => CAP_ERR_NOT_FOUND,
        caps::FsError::TooLarge(_) => CAP_ERR_TOO_LONG,
        caps::FsError::Revoked(_) => CAP_ERR_REVOKED,
        _ => CAP_ERR_REFUSED,
    }
}

/// Run `f`, turning a panic into [`CAP_ERR_PANIC`].
fn contained(f: impl FnOnce() -> Result<i64, i64>) -> i64 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(v)) => v,
        Ok(Err(code)) => code,
        Err(_) => CAP_ERR_PANIC,
    }
}

/// The current instance and its capabilities, or the code explaining why not.
fn current() -> Result<(std::sync::Arc<caps::InstanceCaps>, String), i64> {
    let context = crate::auth::current_caller().ok_or(CAP_ERR_NO_INSTANCE)?;
    if crate::auth::context_refusal(&context).is_some() {
        return Err(CAP_ERR_STALE);
    }
    let caps = caps::caps_for(context.instance).ok_or(CAP_ERR_NO_CAPS)?;
    Ok((caps, context.module.clone()))
}

unsafe fn bytes<'a>(ptr: *const u8, len: i64) -> Result<&'a [u8], i64> {
    if len < 0 || len as usize > caps::MAX_FRAME {
        return Err(CAP_ERR_TOO_LONG);
    }
    if ptr.is_null() {
        if len == 0 {
            return Ok(&[]);
        }
        return Err(CAP_ERR_BAD_ARG);
    }
    Ok(std::slice::from_raw_parts(ptr, len as usize))
}

unsafe fn text(ptr: *const u8, len: i64) -> Result<String, i64> {
    let raw = bytes(ptr, len)?;
    std::str::from_utf8(raw)
        .map(|s| s.to_string())
        .map_err(|_| CAP_ERR_BAD_ARG)
}

/// Copy `data` into the caller's buffer. Returns the length written, or the
/// required length as a negative number when the buffer is too small, so a
/// caller never receives a truncated frame it might mistake for a full one.
unsafe fn copy_out(data: &[u8], out: *mut u8, cap: i64) -> i64 {
    if cap < 0 {
        return CAP_ERR_BAD_ARG;
    }
    if data.len() > cap as usize {
        return -(data.len() as i64);
    }
    if data.is_empty() {
        return 0;
    }
    if out.is_null() {
        return CAP_ERR_BAD_ARG;
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), out, data.len());
    data.len() as i64
}

#[no_mangle]
pub extern "C" fn rs_cap_dir_open() -> i64 {
    contained(|| {
        let (caps, _caller) = current()?;
        caps.open(caps::Kind::Dir)
            .map(|h| h as i64)
            .map_err(|_| CAP_ERR_REFUSED)
    })
}

#[no_mangle]
pub extern "C" fn rs_cap_net_open() -> i64 {
    contained(|| {
        let (caps, _caller) = current()?;
        caps.open(caps::Kind::Net)
            .map(|h| h as i64)
            .map_err(|_| CAP_ERR_REFUSED)
    })
}

#[no_mangle]
pub extern "C" fn rs_cap_close(handle: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        caps.close(handle as u64, &caller)
            .map(|()| 0)
            .map_err(|_| CAP_ERR_REFUSED)
    })
}

/// Write `data` under a directory capability at `path`.
///
/// # Safety
///
/// `path_ptr` must be null or point to `path_len` readable bytes, and
/// `data_ptr` to `data_len` readable bytes; both regions must stay alive and
/// unmodified across the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_write(
    handle: i64,
    path_ptr: *const u8,
    path_len: i64,
    data_ptr: *const u8,
    data_len: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let path = text(path_ptr, path_len)?;
        let data = bytes(data_ptr, data_len)?;
        caps.write(handle as u64, &caller, &path, data)
            .map(|n| n as i64)
            .map_err(|e| fs_code(&e))
    })
}

/// Read a file under a directory capability into the caller's buffer.
///
/// # Safety
///
/// `path_ptr` must be null or point to `path_len` readable bytes. `out_ptr`
/// must be null or point to `out_cap` writable bytes, and the caller must keep
/// that buffer alive and exclusive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_read(
    handle: i64,
    path_ptr: *const u8,
    path_len: i64,
    out_ptr: *mut u8,
    out_cap: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let path = text(path_ptr, path_len)?;
        let data = caps
            .read(handle as u64, &caller, &path)
            .map_err(|e| fs_code(&e))?;
        Ok(copy_out(&data, out_ptr, out_cap))
    })
}

/// Send `data` to a peer capability.
///
/// # Safety
///
/// `peer_ptr` must be null or point to `peer_len` readable bytes, and
/// `data_ptr` to `data_len` readable bytes; both regions must stay alive and
/// unmodified across the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_send(
    handle: i64,
    peer_ptr: *const u8,
    peer_len: i64,
    data_ptr: *const u8,
    data_len: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let peer = text(peer_ptr, peer_len)?;
        let data = bytes(data_ptr, data_len)?;
        caps.send(handle as u64, &caller, &peer, data)
            .map(|id| id as i64)
            .map_err(|e| {
                if e.contains("exceeds") {
                    CAP_ERR_TOO_LONG
                } else if e.contains("not configured") {
                    CAP_ERR_BAD_ARG
                } else {
                    CAP_ERR_REFUSED
                }
            })
    })
}

/// Collect a settled response into the caller's buffer, 0 while it is pending.
///
/// # Safety
///
/// `out_ptr` must be null or point to `out_cap` writable bytes, and the caller
/// must keep that buffer alive and exclusive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_query(handle: i64, id: i64, out_ptr: *mut u8, out_cap: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        match caps
            .query(handle as u64, &caller, id as u64)
            .map_err(|_| CAP_ERR_REFUSED)?
        {
            // Not settled yet is 0, distinct from an empty response (-0 is still 0),
            // so a caller polls rather than mis-reading a pending request.
            None => Ok(0),
            Some(response) => Ok(copy_out(&response, out_ptr, out_cap)),
        }
    })
}

#[no_mangle]
pub extern "C" fn rs_cap_cancel(handle: i64, id: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        caps.cancel(handle as u64, &caller, id as u64)
            .map(|()| 0)
            .map_err(|_| CAP_ERR_REFUSED)
    })
}

/// Open one of the roots the grant approved, by index. Index 0 is the instance
/// state directory; the extra roots are the ones the manifest or project named,
/// each with the rights its grant stated. There is no way to ask for a root the
/// host did not approve, and no path crosses this boundary.
#[no_mangle]
pub extern "C" fn rs_cap_open_root(index: i64) -> i64 {
    contained(|| {
        let (caps, _caller) = current()?;
        if index < 0 {
            return Err(CAP_ERR_BAD_ARG);
        }
        caps.open_root(index as usize)
            .map(|handle| handle as i64)
            .map_err(|error| fs_code(&error))
    })
}

/// Revoke one handle. Every later verb on it is `CAP_ERR_REVOKED`, and the
/// directory descriptor closes when the last handle naming it is dropped.
#[no_mangle]
pub extern "C" fn rs_cap_revoke(handle: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        caps.revoke(handle as u64, &caller)
            .map(|()| 0)
            .map_err(|error| fs_code(&error))
    })
}

/// Dial an approved endpoint through a `Net` handle.
///
/// # Safety
///
/// `host_ptr` must be null or point to `host_len` readable bytes, and the
/// region must stay alive and unmodified across the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_net_dial(
    handle: i64,
    host_ptr: *const u8,
    host_len: i64,
    port: i64,
    tls: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let host = text(host_ptr, host_len)?;
        if !(0..=65535).contains(&port) {
            return Err(CAP_ERR_BAD_ARG);
        }
        if tls != 0 && tls != 1 {
            return Err(CAP_ERR_BAD_ARG);
        }
        caps.dial(handle as u64, &caller, &host, port as u16, tls == 1)
            .map(|_peer| 0)
            .map_err(|error| net_code(&error))
    })
}

/// Read once from a dialed connection into the caller's buffer. A return of 0
/// is EOF; the required-length convention is not used here because the read is
/// already bounded by the caller's capacity.
///
/// # Safety
///
/// `out_ptr` must be null or point to `out_cap` writable bytes, and the caller
/// must keep that buffer alive and exclusive for the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_net_read(handle: i64, out_ptr: *mut u8, out_cap: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        if out_cap < 0 {
            return Err(CAP_ERR_BAD_ARG);
        }
        let data = caps
            .net_read(handle as u64, &caller, out_cap as usize)
            .map_err(|error| net_code(&error))?;
        Ok(copy_out(&data, out_ptr, out_cap))
    })
}

/// Write one bounded frame to a dialed connection.
///
/// # Safety
///
/// `data_ptr` must be null or point to `data_len` readable bytes, and the
/// region must stay alive and unmodified across the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_net_write(
    handle: i64,
    data_ptr: *const u8,
    data_len: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let data = bytes(data_ptr, data_len)?;
        caps.net_write(handle as u64, &caller, data)
            .map(|n| n as i64)
            .map_err(|error| net_code(&error))
    })
}

#[no_mangle]
pub extern "C" fn rs_cap_net_shutdown(handle: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        caps.net_shutdown(handle as u64, &caller)
            .map(|()| 0)
            .map_err(|error| net_code(&error))
    })
}

/// Refuse a network operation the capability does not provide. UDP, listening
/// and raw sockets have no rights model here, so asking for one is an explicit
/// `CAP_ERR_UNSUPPORTED` instead of a library default or an ambient socket.
///
/// # Safety
///
/// `op_ptr` must be null or point to `op_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_net_unsupported(op_ptr: *const u8, op_len: i64) -> i64 {
    contained(|| {
        let (caps, _caller) = current()?;
        let operation = text(op_ptr, op_len)?;
        let policy = caps.net_policy().ok_or(CAP_ERR_REFUSED)?;
        Err(net_code(&policy.unsupported(&operation)))
    })
}

/// List `path` under a directory capability as newline-separated names.
///
/// # Safety
///
/// `path_ptr` must be null or point to `path_len` readable bytes. `out_ptr`
/// must be null or point to `out_cap` writable bytes, and the caller must keep
/// that buffer alive and exclusive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_list(
    handle: i64,
    path_ptr: *const u8,
    path_len: i64,
    out_ptr: *mut u8,
    out_cap: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let path = text(path_ptr, path_len)?;
        let names = caps
            .list(handle as u64, &caller, &path)
            .map_err(|error| fs_code(&error))?;
        let joined = names.join("\n");
        Ok(copy_out(joined.as_bytes(), out_ptr, out_cap))
    })
}

/// Delete one regular file under a directory capability.
///
/// # Safety
///
/// `path_ptr` must be null or point to `path_len` readable bytes, and the region
/// must stay alive and unmodified across the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_delete(
    handle: i64,
    path_ptr: *const u8,
    path_len: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let path = text(path_ptr, path_len)?;
        caps.delete(handle as u64, &caller, &path)
            .map(|()| 0)
            .map_err(|error| fs_code(&error))
    })
}

/// Rename one entry inside the same directory capability. An existing
/// destination is refused, never replaced.
///
/// # Safety
///
/// Each pointer must be null or point to its stated number of readable bytes,
/// and both regions must stay alive and unmodified across the call.
#[no_mangle]
pub unsafe extern "C" fn rs_cap_rename(
    handle: i64,
    from_ptr: *const u8,
    from_len: i64,
    to_ptr: *const u8,
    to_len: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let from = text(from_ptr, from_len)?;
        let to = text(to_ptr, to_len)?;
        caps.rename(handle as u64, &caller, &from, &to)
            .map(|()| 0)
            .map_err(|error| fs_code(&error))
    })
}

// ── U19 tests: right-scoped, rooted directory capabilities ─────────────────
//
// Each test builds its own state directory under a fresh scratch root, with an
// outside sentinel beside it. `InstanceCaps` keeps no process-global state, so
// these can run in parallel; the directory is unique per test.
#[cfg(test)]
mod fs_tests {
    use super::caps::{FsError, FsRights, InstanceCaps};
    use std::path::{Path, PathBuf};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "u19_fs_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state(base: &Path) -> (InstanceCaps, PathBuf) {
        let state = base.join("state");
        std::fs::create_dir_all(&state).unwrap();
        (
            InstanceCaps::new("fs_mod", &state, Vec::new()).unwrap(),
            state,
        )
    }

    fn seed(caps: &InstanceCaps, relative: &str, data: &[u8]) {
        let handle = caps.open_dir("", FsRights::ALL).unwrap();
        caps.write(handle, "fs_mod", relative, data).unwrap();
    }

    fn read(caps: &InstanceCaps, handle: u64, relative: &str) -> Result<Vec<u8>, FsError> {
        caps.read(handle, "fs_mod", relative)
    }

    #[test]
    fn each_right_admits_only_its_own_operations() {
        let base = scratch("rights");
        let (caps, state_dir) = state(&base);
        seed(&caps, "data/existing.txt", b"old");
        seed(&caps, "data/other.txt", b"kept");

        let read_only = caps.open_dir("", FsRights::READ).unwrap();
        assert_eq!(read(&caps, read_only, "data/existing.txt").unwrap(), b"old");
        assert!(matches!(
            caps.write(read_only, "fs_mod", "data/existing.txt", b"new"),
            Err(FsError::Denied(_))
        ));
        assert!(matches!(
            caps.write(read_only, "fs_mod", "new.txt", b"new"),
            Err(FsError::Denied(_))
        ));
        assert!(matches!(
            caps.list(read_only, "fs_mod", "data"),
            Err(FsError::Denied(_))
        ));
        assert!(matches!(
            caps.delete(read_only, "fs_mod", "data/other.txt"),
            Err(FsError::Denied(_))
        ));
        assert!(matches!(
            caps.rename(read_only, "fs_mod", "data/other.txt", "data/moved.txt"),
            Err(FsError::Denied(_))
        ));

        // `create` may add a file but not replace one; `write` is the other
        // way round. Neither implies `read`.
        let create_only = caps.open_dir("", FsRights::CREATE).unwrap();
        assert_eq!(
            caps.write(create_only, "fs_mod", "new.txt", b"made").unwrap(),
            4
        );
        assert!(matches!(
            caps.write(create_only, "fs_mod", "new.txt", b"again"),
            Err(FsError::Denied(_))
        ));
        assert!(matches!(
            read(&caps, create_only, "new.txt"),
            Err(FsError::Denied(_))
        ));

        let write_only = caps.open_dir("", FsRights::WRITE).unwrap();
        assert_eq!(
            caps.write(write_only, "fs_mod", "data/existing.txt", b"new")
                .unwrap(),
            3
        );
        assert!(matches!(
            caps.write(write_only, "fs_mod", "second.txt", b"no"),
            Err(FsError::Denied(_))
        ));

        let list_only = caps.open_dir("", FsRights::LIST).unwrap();
        assert_eq!(
            caps.list(list_only, "fs_mod", "data").unwrap(),
            vec!["existing.txt".to_string(), "other.txt".to_string()]
        );
        assert!(matches!(
            read(&caps, list_only, "data/existing.txt"),
            Err(FsError::Denied(_))
        ));

        let delete_only = caps.open_dir("", FsRights::DELETE).unwrap();
        caps.delete(delete_only, "fs_mod", "data/other.txt")
            .unwrap();
        assert!(!state_dir.join("data/other.txt").exists());
        assert!(matches!(
            read(&caps, delete_only, "data/existing.txt"),
            Err(FsError::Denied(_))
        ));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn outside_sentinels_and_symlinks_are_never_followed() {
        let base = scratch("escape");
        let (caps, state_dir) = state(&base);
        let outside = base.join("outside.txt");
        std::fs::write(&outside, b"outside-bytes").unwrap();
        let outside_dir = base.join("outside-dir");
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("sentinel.txt"), b"sentinel").unwrap();

        let full = caps.open_dir("", FsRights::ALL).unwrap();
        std::os::unix::fs::symlink(&outside, state_dir.join("file-link")).unwrap();
        std::os::unix::fs::symlink(&outside_dir, state_dir.join("dir-link")).unwrap();

        // A symlinked file, a symlinked directory and a lexical escape are all
        // refused; the outside bytes never change.
        assert!(matches!(
            read(&caps, full, "file-link"),
            Err(FsError::Escape(_))
        ));
        assert!(matches!(
            read(&caps, full, "dir-link/sentinel.txt"),
            Err(FsError::Escape(_))
        ));
        assert!(matches!(
            caps.write(full, "fs_mod", "file-link", b"overwritten"),
            Err(FsError::Escape(_))
        ));
        assert!(matches!(
            read(&caps, full, "../outside.txt"),
            Err(FsError::Escape(_))
        ));
        assert!(matches!(
            read(&caps, full, "/etc/hostname"),
            Err(FsError::Escape(_))
        ));
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside-bytes");
        assert_eq!(
            std::fs::read(outside_dir.join("sentinel.txt")).unwrap(),
            b"sentinel"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_and_denied_are_distinct_answers() {
        let base = scratch("distinct");
        let (caps, _state_dir) = state(&base);
        seed(&caps, "present.txt", b"here");
        seed(&caps, "data/dir-member.txt", b"m");

        let read_only = caps.open_dir("", FsRights::READ).unwrap();
        assert!(matches!(
            read(&caps, read_only, "absent.txt"),
            Err(FsError::NotFound(_))
        ));
        assert!(matches!(
            read(&caps, read_only, "data"),
            Err(FsError::NotFile(_))
        ));

        let create_only = caps.open_dir("", FsRights::CREATE).unwrap();
        assert!(matches!(
            read(&caps, create_only, "present.txt"),
            Err(FsError::Denied(_))
        ));

        let delete_only = caps.open_dir("", FsRights::DELETE).unwrap();
        assert!(matches!(
            caps.delete(delete_only, "fs_mod", "absent.txt"),
            Err(FsError::NotFound(_))
        ));
        assert!(matches!(
            caps.delete(delete_only, "fs_mod", "data"),
            Err(FsError::NotFile(_))
        ));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_rooted_handle_cannot_leave_its_subtree() {
        let base = scratch("rooted");
        let (caps, state_dir) = state(&base);
        seed(&caps, "data/a.txt", b"a");
        seed(&caps, "other.txt", b"other");

        let data = caps.open_dir("data", FsRights::READ).unwrap();
        assert_eq!(read(&caps, data, "a.txt").unwrap(), b"a");
        assert!(matches!(
            read(&caps, data, "../other.txt"),
            Err(FsError::Escape(_))
        ));
        assert!(matches!(
            read(&caps, data, "other.txt"),
            Err(FsError::NotFound(_))
        ));
        assert!(
            state_dir.join("other.txt").exists(),
            "the rooted handle never wrote or moved the sibling"
        );

        assert!(matches!(
            caps.open_dir("missing", FsRights::READ),
            Err(FsError::NotFound(_))
        ));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rename_moves_within_the_capability_and_refuses_to_overwrite() {
        let base = scratch("rename");
        let (caps, state_dir) = state(&base);
        seed(&caps, "a.txt", b"a");
        seed(&caps, "b.txt", b"b");

        let handle = caps.open_dir("", FsRights::RENAME).unwrap();
        caps.rename(handle, "fs_mod", "a.txt", "c.txt").unwrap();
        assert!(!state_dir.join("a.txt").exists());
        assert_eq!(std::fs::read(state_dir.join("c.txt")).unwrap(), b"a");

        assert!(matches!(
            caps.rename(handle, "fs_mod", "c.txt", "b.txt"),
            Err(FsError::Exists(_))
        ));
        assert!(matches!(
            caps.rename(handle, "fs_mod", "gone.txt", "d.txt"),
            Err(FsError::NotFound(_))
        ));
        // A directory move would take files the caller never listed with it.
        std::fs::create_dir_all(state_dir.join("subdir")).unwrap();
        assert!(matches!(
            caps.rename(handle, "fs_mod", "subdir", "moved-dir"),
            Err(FsError::NotFile(_))
        ));
        assert_eq!(std::fs::read(state_dir.join("b.txt")).unwrap(), b"b");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rights_syntax_refuses_unknown_names() {
        assert_eq!(
            FsRights::parse("read, write").unwrap(),
            FsRights::READ.union(FsRights::WRITE)
        );
        assert_eq!(FsRights::parse("all").unwrap(), FsRights::ALL);
        assert!(FsRights::parse("read,wite").is_err());
        assert!(!FsRights::READ.contains(FsRights::WRITE));
        assert_eq!(FsRights::READ.to_string(), "read");
    }

    /// U19: a grant is per root, and a handle opens the root the host approved —
    /// index 0 is the state directory, the rest carry exactly their own rights.
    #[test]
    fn each_approved_root_carries_its_own_rights() {
        use super::caps::RootGrant;
        let base = scratch("roots");
        let (caps, state_dir) = state(&base);
        seed(&caps, "tree/one.txt", b"one");

        let caps = InstanceCaps::new_with_grants(
            "fs_mod",
            &state_dir,
            Vec::new(),
            FsRights::READ,
            vec![RootGrant {
                root: "tree".to_string(),
                rights: FsRights::READ.union(FsRights::LIST),
            }],
            None,
        )
        .unwrap();
        assert_eq!(caps.root_count(), 2);
        assert_eq!(caps.root_rights(0), Some(FsRights::READ));
        assert_eq!(
            caps.root_rights(1),
            Some(FsRights::READ.union(FsRights::LIST))
        );

        // The state-directory root carries only `read`; the extra root carries
        // `read,list` and is confined to its subtree.
        let state_handle = caps.open_root(0).unwrap();
        assert_eq!(read(&caps, state_handle, "tree/one.txt").unwrap(), b"one");
        assert!(matches!(
            caps.write(state_handle, "fs_mod", "new.txt", b"x"),
            Err(FsError::Denied(_))
        ));

        let tree = caps.open_root(1).unwrap();
        assert_eq!(
            caps.list(tree, "fs_mod", "").unwrap(),
            vec!["one.txt".to_string()]
        );
        assert_eq!(read(&caps, tree, "one.txt").unwrap(), b"one");
        assert!(matches!(
            caps.write(tree, "fs_mod", "two.txt", b"x"),
            Err(FsError::Denied(_))
        ));
        assert!(matches!(
            read(&caps, tree, "../outside"),
            Err(FsError::Escape(_))
        ));
        assert!(matches!(caps.open_root(2), Err(FsError::Denied(_))));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_granted_root_cannot_escape_the_state_directory() {
        use super::caps::RootGrant;
        let base = scratch("grant_escape");
        let (_caps, state_dir) = state(&base);
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        let err = InstanceCaps::new_with_grants(
            "fs_mod",
            &state_dir,
            Vec::new(),
            FsRights::ALL,
            vec![RootGrant {
                root: "../outside".to_string(),
                rights: FsRights::READ,
            }],
            None,
        )
        .err()
        .expect("a root above the state directory is not a capability");
        assert!(err.contains("escapes"), "{err}");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, state_dir.join("link")).unwrap();
            let err = InstanceCaps::new_with_grants(
                "fs_mod",
                &state_dir,
                Vec::new(),
                FsRights::ALL,
                vec![RootGrant {
                    root: "link".to_string(),
                    rights: FsRights::READ,
                }],
                None,
            )
            .err()
            .expect("a symlinked root is not a directory the host approved");
            assert!(err.contains("symlink"), "{err}");
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_quota_is_cumulative_and_refuses_before_writing() {
        let base = scratch("quota");
        let (_caps, state_dir) = state(&base);
        let caps = InstanceCaps::new_with_grants(
            "fs_mod",
            &state_dir,
            Vec::new(),
            FsRights::ALL,
            Vec::new(),
            Some(5),
        )
        .unwrap();
        let handle = caps.open_root(0).unwrap();

        assert_eq!(
            caps.write(handle, "fs_mod", "a.txt", b"1234").unwrap(),
            4
        );
        // Over quota: refused, and the target keeps its old bytes.
        assert!(matches!(
            caps.write(handle, "fs_mod", "a.txt", b"56"),
            Err(FsError::TooLarge(_))
        ));
        assert_eq!(read(&caps, handle, "a.txt").unwrap(), b"1234");
        assert_eq!(caps.written(), 4);

        caps.write(handle, "fs_mod", "b.txt", b"5").unwrap();
        assert!(matches!(
            caps.write(handle, "fs_mod", "b.txt", b"6"),
            Err(FsError::TooLarge(_))
        ));
        assert_eq!(read(&caps, handle, "b.txt").unwrap(), b"5");
        assert_eq!(caps.written(), 5);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn revocation_refuses_every_verb_and_every_later_open() {
        let base = scratch("revoke");
        let (caps, state_dir) = state(&base);
        seed(&caps, "a.txt", b"a");
        seed(&caps, "b.txt", b"b");

        let handle = caps.open_root(0).unwrap();
        caps.revoke(handle, "fs_mod").unwrap();
        assert!(matches!(
            read(&caps, handle, "a.txt"),
            Err(FsError::Revoked(_))
        ));
        assert!(matches!(
            caps.write(handle, "fs_mod", "a.txt", b"x"),
            Err(FsError::Revoked(_))
        ));
        assert!(matches!(
            caps.list(handle, "fs_mod", ""),
            Err(FsError::Revoked(_))
        ));
        assert!(matches!(
            caps.delete(handle, "fs_mod", "a.txt"),
            Err(FsError::Revoked(_))
        ));
        assert!(matches!(
            caps.rename(handle, "fs_mod", "a.txt", "c.txt"),
            Err(FsError::Revoked(_))
        ));
        // The files are untouched by every refused verb.
        assert_eq!(std::fs::read(state_dir.join("a.txt")).unwrap(), b"a");

        // A closed handle cannot be presented again.
        caps.close(handle, "fs_mod").unwrap();
        assert!(matches!(
            read(&caps, handle, "a.txt"),
            Err(FsError::Denied(_))
        ));

        // Revoking the set refuses the handles that outlive it and any new open.
        let other = caps.open_root(0).unwrap();
        caps.revoke_all();
        assert!(caps.is_revoked());
        assert!(matches!(
            read(&caps, other, "b.txt"),
            Err(FsError::Revoked(_))
        ));
        assert!(matches!(caps.open_root(0), Err(FsError::Revoked(_))));
        assert!(matches!(
            caps.open_dir("", FsRights::READ),
            Err(FsError::Revoked(_))
        ));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// U19 descriptor lifetime: installing a replacement revokes the previous
    /// set, and uninstalling revokes what it releases.
    #[test]
    fn replacing_or_uninstalling_a_set_revokes_it() {
        let base = scratch("generations");
        let (first, state_dir) = state(&base);
        let id = 0xffff_0000_0000_0000u64
            | (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
                & 0x0000_ffff_ffff_ffff);

        super::caps::install(id, first);
        let old = super::caps::caps_for(id).expect("installed");
        assert!(!old.is_revoked());

        super::caps::install(
            id,
            InstanceCaps::new("fs_mod", &state_dir, Vec::new()).unwrap(),
        );
        assert!(old.is_revoked(), "the replaced set is dead");

        let current = super::caps::caps_for(id).expect("replacement installed");
        super::caps::uninstall(id);
        assert!(current.is_revoked(), "the uninstalled set is dead");
        assert!(super::caps::caps_for(id).is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    /// FS03: a component swapped for a symlink between two operations is refused
    /// by the kernel (`O_NOFOLLOW` per component), not resolved by a path check
    /// and then followed by the syscall.
    #[cfg(unix)]
    #[test]
    fn a_symlink_swapped_under_the_root_is_refused_and_never_followed() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let base = scratch("race");
        let (caps, state_dir) = state(&base);
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"outside-bytes").unwrap();

        let slot = state_dir.join("race/slot");
        std::fs::create_dir_all(&slot).unwrap();
        std::fs::write(slot.join("secret.txt"), b"inside").unwrap();
        let handle = caps.open_dir("race", FsRights::READ).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let swapper = {
            let stop = stop.clone();
            let slot = slot.clone();
            let outside = outside.clone();
            std::thread::spawn(move || {
                let mut flip = false;
                while !stop.load(Ordering::SeqCst) {
                    let _ = std::fs::remove_file(&slot);
                    let _ = std::fs::remove_dir_all(&slot);
                    if flip {
                        let _ = std::os::unix::fs::symlink(&outside, &slot);
                    } else {
                        let _ = std::fs::create_dir(&slot);
                        let _ = std::fs::write(slot.join("secret.txt"), b"inside");
                    }
                    flip = !flip;
                }
            })
        };

        for _ in 0..400 {
            match read(&caps, handle, "slot/secret.txt") {
                // The swapper may be between `create` and `write`, so an empty
                // inside file is legitimate; outside bytes never are.
                Ok(bytes) => assert!(
                    bytes == b"inside" || bytes.is_empty(),
                    "a capability read must never return bytes from outside its root: {bytes:?}"
                ),
                Err(FsError::Escape(_))
                | Err(FsError::NotFound(_))
                | Err(FsError::NotFile(_))
                | Err(FsError::Io(_)) => {}
                Err(other) => panic!("unexpected refusal during the race: {other:?}"),
            }
        }
        stop.store(true, Ordering::SeqCst);
        swapper.join().unwrap();

        assert_eq!(
            std::fs::read(outside.join("secret.txt")).unwrap(),
            b"outside-bytes"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// U19: the ABI verbs address a handle, never a path, and each one needs its
    /// own right; `open_root` names an approved root by index.
    #[test]
    fn the_abi_verbs_open_list_delete_rename_and_revoke() {
        let base = scratch("abi");
        let (seed_caps, state_dir) = state(&base);
        seed(&seed_caps, "data/a.txt", b"a");
        let caps = InstanceCaps::new("fs_abi", &state_dir, Vec::new()).unwrap();

        let context = crate::auth::bind_instance_with_identity(
            "fs_abi",
            false,
            None,
            None,
            crate::auth::ManifestAuthEngine::new(),
        );
        let _scope = crate::auth::enter_caller(&context);
        super::caps::install(context.instance, caps);

        let handle = super::rs_cap_open_root(0);
        assert!(handle >= 0, "state root opens, got {handle}");
        assert_eq!(super::rs_cap_open_root(9), super::CAP_ERR_REFUSED);

        let path = b"data/b.txt";
        let data = b"bee";
        assert_eq!(
            unsafe {
                super::rs_cap_write(
                    handle,
                    path.as_ptr(),
                    path.len() as i64,
                    data.as_ptr(),
                    data.len() as i64,
                )
            },
            3
        );

        let data_path = b"data";
        let mut out = [0u8; 128];
        let listed = unsafe {
            super::rs_cap_list(
                handle,
                data_path.as_ptr(),
                data_path.len() as i64,
                out.as_mut_ptr(),
                out.len() as i64,
            )
        };
        assert!(listed > 0, "list returns the byte count, got {listed}");
        let listed = String::from_utf8(out[..listed as usize].to_vec()).unwrap();
        assert_eq!(listed, "a.txt\nb.txt");

        let from = b"data/b.txt";
        let to = b"data/c.txt";
        assert_eq!(
            unsafe {
                super::rs_cap_rename(
                    handle,
                    from.as_ptr(),
                    from.len() as i64,
                    to.as_ptr(),
                    to.len() as i64,
                )
            },
            0
        );
        assert!(!state_dir.join("data/b.txt").exists());
        assert_eq!(std::fs::read(state_dir.join("data/c.txt")).unwrap(), b"bee");

        let gone = b"data/a.txt";
        assert_eq!(
            unsafe { super::rs_cap_delete(handle, gone.as_ptr(), gone.len() as i64) },
            0
        );
        assert!(!state_dir.join("data/a.txt").exists());

        // A missing file is `NOT_FOUND`, a revoked handle is `REVOKED`.
        let missing = b"data/absent.txt";
        let mut small = [0u8; 8];
        assert_eq!(
            unsafe {
                super::rs_cap_read(
                    handle,
                    missing.as_ptr(),
                    missing.len() as i64,
                    small.as_mut_ptr(),
                    small.len() as i64,
                )
            },
            super::CAP_ERR_NOT_FOUND
        );
        assert_eq!(super::rs_cap_revoke(handle), 0);
        assert_eq!(
            unsafe {
                super::rs_cap_delete(handle, gone.as_ptr(), gone.len() as i64)
            },
            super::CAP_ERR_REVOKED
        );

        super::caps::uninstall(context.instance);
        crate::auth::revoke_instance(context.instance);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// U19: manifest entries carry their rights, and a malformed one is refused
    /// by name instead of becoming a wider grant.
    #[test]
    fn manifest_fs_grants_carry_rights() {
        let manifest = crate::module::ModuleManifest::from_toml_str(
            "[module]\nname = \"m\"\n\n[grants]\nfs = [\"data/\", \
             { root = \"cache/\", rights = \"read, list\" }]\n\n[limits]\nmax_write_bytes = 1024\n",
        )
        .unwrap();
        assert_eq!(manifest.fs_grants, vec!["data/", "cache/"]);
        assert_eq!(
            manifest.fs_root_rights[0],
            ("data/".to_string(), FsRights::ALL)
        );
        assert_eq!(
            manifest.fs_root_rights[1],
            (
                "cache/".to_string(),
                FsRights::READ.union(FsRights::LIST)
            )
        );
        assert_eq!(manifest.max_write_bytes, Some(1024));

        for (document, needle) in [
            (
                "[module]\nname = \"m\"\n\n[grants]\nfs = [{ root = \"data/\" }]",
                "needs `rights`",
            ),
            (
                "[module]\nname = \"m\"\n\n[grants]\nfs = [{ root = \"data/\", \
                 rights = \"read\", extra = 1 }]",
                "unknown key",
            ),
            (
                "[module]\nname = \"m\"\n\n[grants]\nfs = [{ root = \"data/\", \
                 rights = \"reed\" }]",
                "unknown filesystem right",
            ),
            (
                "[module]\nname = \"m\"\n\n[grants]\nfs = [42]",
                "path string",
            ),
        ] {
            let error = crate::module::ModuleManifest::from_toml_str(document)
                .expect_err("malformed fs grant must be refused");
            assert!(error.contains(needle), "{error}");
        }
    }
}

// ── U20: bounded network capability tests ────────────────────────────────────
//
// Every fixture is loopback-only and ephemeral: a listener on 127.0.0.1:0, its
// real port read back, and a short window in which the denied case must observe
// **zero** accepted connections. No public network is touched, and a denied
// dial is measured by the server's own accept counter rather than by the
// client's error alone.
#[cfg(test)]
mod net_tests {
    use super::caps::{InstanceCaps, Kind};
    use super::CAP_ERR_UNSUPPORTED;
    use crate::netstd_mod::{NetError, NetPolicy};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "u20_net_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn net_caps(state: &PathBuf, policy: NetPolicy, module: &str) -> InstanceCaps {
        InstanceCaps::new(module, &state.join("state"), Vec::new())
            .unwrap()
            .with_net_policy(policy)
    }

    fn listener() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    /// Count accepted connections for `window`, then stop. The count is what
    /// decides a denied case: a client error alone does not prove the server
    /// was never reached.
    fn count_accepts(listener: TcpListener, window: Duration) -> (Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + window;
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((_stream, _)) => {
                        seen.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        (count, handle)
    }

    fn policy(port: u16, ranges: &[&str]) -> NetPolicy {
        NetPolicy::new(
            &[format!("127.0.0.1:{port}")],
            &ranges.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
            Vec::new(),
            Duration::from_millis(2000),
        )
        .unwrap()
    }

    #[test]
    fn a_handle_without_a_policy_is_never_issued() {
        let base = scratch("no_policy");
        let caps = InstanceCaps::new("no_net", &base.join("state"), Vec::new()).unwrap();
        assert!(caps.open(Kind::Net).is_err());
        assert!(caps.open(Kind::Dir).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// NET01: an endpoint outside the grant is refused before any connection,
    /// and the denied server's accept count stays zero.
    #[test]
    fn a_denied_endpoint_never_opens_a_connection() {
        let base = scratch("denied");
        let (listener, port) = listener();
        let (count, accepts) = count_accepts(listener, Duration::from_millis(200));
        let caps = net_caps(&base, policy(port.wrapping_add(1).max(1), &["127.0.0.0/8"]), "net_denied");
        let handle = caps.open(Kind::Net).unwrap();
        let refused = caps.dial(handle, "net_denied", "127.0.0.1", port, false);
        assert!(matches!(refused, Err(NetError::Denied(_))), "{refused:?}");
        accepts.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0, "a denied endpoint must accept nothing");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// NET01 positive path: the granted endpoint carries a bounded exchange.
    #[test]
    fn an_allowed_loopback_dial_carries_a_bounded_exchange() {
        let base = scratch("exchange");
        let (listener, port) = listener();
        listener.set_nonblocking(false).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").unwrap();
        });
        let caps = net_caps(&base, policy(port, &["127.0.0.0/8"]), "net_exchange");
        let handle = caps.open(Kind::Net).unwrap();
        let peer = caps.dial(handle, "net_exchange", "127.0.0.1", port, false).unwrap();
        assert_eq!(peer.port(), port);
        assert_eq!(caps.net_write(handle, "net_exchange", b"ping").unwrap(), 4);
        let mut response = Vec::new();
        while response.len() < 4 {
            response.extend(caps.net_read(handle, "net_exchange", 16).unwrap());
        }
        assert_eq!(response, b"pong");
        caps.net_shutdown(handle, "net_exchange").unwrap();
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Read an HTTP response through the capability: one request, then reads
    /// until the header terminator and (when `Content-Length` is present) the
    /// body. Returns the status line's code and the `Location` header.
    fn http_get(
        caps: &InstanceCaps,
        module: &str,
        handle: u64,
        host: &str,
        port: u16,
        path: &str,
    ) -> Result<(u16, Option<String>, Vec<u8>), NetError> {
        caps.dial(handle, module, host, port, false)?;
        let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        caps.net_write(handle, module, request.as_bytes())?;
        // The fixture answers with `Connection: close`, so EOF is the end of
        // the response; the read is still bounded by the capability's frame
        // limit and deadline.
        let mut response = Vec::new();
        loop {
            let chunk = caps.net_read(handle, module, 1024)?;
            if chunk.is_empty() {
                break;
            }
            response.extend(chunk);
            if response.len() > 1 << 20 {
                break;
            }
        }
        let text = String::from_utf8_lossy(&response).to_string();
        let (head, body) = text
            .split_once("\r\n\r\n")
            .map(|(h, b)| (h.to_string(), b.as_bytes().to_vec()))
            .unwrap_or((text.clone(), Vec::new()));
        let code = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let location = head.lines().find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("location"))
                .map(|(_, value)| value.trim().to_string())
        });
        Ok((code, location, body))
    }

    /// Echo the request path back as an HTTP response, then close.
    fn http_responder(listener: TcpListener, status: u16, body: &'static str, location: Option<String>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let mut response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                if let Some(location) = location {
                    response.push_str(&format!("Location: {location}\r\n"));
                }
                response.push_str("\r\n");
                response.push_str(body);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        })
    }

    /// NET03: a redirect is a *new* dial through the same policy. Following one
    /// to an approved endpoint works; following one to a destination the grant
    /// does not name is refused before the second server is contacted.
    #[test]
    fn a_redirect_is_a_new_checked_dial() {
        let parse_location = |location: String| {
            let rest = location.strip_prefix("http://").expect("plain http");
            let (authority, path) = rest.split_once('/').map(|(a, p)| (a, format!("/{p}"))).unwrap_or((rest, "/".to_string()));
            let (host, port) = authority.split_once(':').unwrap();
            (host.to_string(), port.parse::<u16>().unwrap(), path)
        };

        // Allowed: both endpoints are granted, so the follow completes.
        let base = scratch("redirect_ok");
        let (first, first_port) = listener();
        let (second, second_port) = listener();
        first.set_nonblocking(false).unwrap();
        second.set_nonblocking(false).unwrap();
        let a = http_responder(
            first,
            302,
            "",
            Some(format!("http://127.0.0.1:{second_port}/final")),
        );
        let b = http_responder(second, 200, "ok", None);
        let policy = NetPolicy::new(
            &[
                format!("127.0.0.1:{first_port}"),
                format!("127.0.0.1:{second_port}"),
            ],
            &["127.0.0.0/8".to_string()],
            Vec::new(),
            Duration::from_millis(3000),
        )
        .unwrap();
        let caps = net_caps(&base, policy, "net_redirect_ok");
        let handle = caps.open(Kind::Net).unwrap();
        let (code, location, _) = http_get(&caps, "net_redirect_ok", handle, "127.0.0.1", first_port, "/start").unwrap();
        assert_eq!(code, 302);
        let (host, port, path) = parse_location(location.expect("Location header"));
        caps.close(handle, "net_redirect_ok").unwrap();
        let handle = caps.open(Kind::Net).unwrap();
        let (code, _, body) = http_get(&caps, "net_redirect_ok", handle, &host, port, &path).unwrap();
        assert_eq!(code, 200);
        assert_eq!(body, b"ok");
        a.join().unwrap();
        b.join().unwrap();
        let _ = std::fs::remove_dir_all(&base);

        // Denied: only the redirecting endpoint is granted. The second server's
        // accept count stays zero and the refusal names the endpoint.
        let base = scratch("redirect_denied");
        let (first, first_port) = listener();
        let (second, second_port) = listener();
        first.set_nonblocking(false).unwrap();
        let a = http_responder(
            first,
            302,
            "",
            Some(format!("http://127.0.0.1:{second_port}/final")),
        );
        let (count, accepts) = count_accepts(second, Duration::from_millis(300));
        let policy = NetPolicy::new(
            &[format!("127.0.0.1:{first_port}")],
            &["127.0.0.0/8".to_string()],
            Vec::new(),
            Duration::from_millis(3000),
        )
        .unwrap();
        let caps = net_caps(&base, policy, "net_redirect_denied");
        let handle = caps.open(Kind::Net).unwrap();
        let (code, location, _) = http_get(&caps, "net_redirect_denied", handle, "127.0.0.1", first_port, "/start").unwrap();
        assert_eq!(code, 302);
        let (host, port, _) = parse_location(location.expect("Location header"));
        caps.close(handle, "net_redirect_denied").unwrap();
        let handle = caps.open(Kind::Net).unwrap();
        let refused = caps.dial(handle, "net_redirect_denied", &host, port, false);
        assert!(matches!(refused, Err(NetError::Denied(_))), "{refused:?}");
        a.join().unwrap();
        accepts.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0, "the redirect target must not be contacted");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// NET02: a name inside the endpoint list but resolving outside the
    /// approved ranges is refused at resolution time, before connecting.
    #[test]
    fn a_name_resolving_outside_the_approved_ranges_is_refused_before_connecting() {
        let base = scratch("rebinding");
        let (listener, port) = listener();
        let (count, accepts) = count_accepts(listener, Duration::from_millis(200));
        let policy = NetPolicy::new(
            &[format!("localhost:{port}")],
            &["10.0.0.0/8".to_string()],
            Vec::new(),
            Duration::from_millis(500),
        )
        .unwrap();
        let caps = net_caps(&base, policy, "net_rebinding");
        let handle = caps.open(Kind::Net).unwrap();
        let refused = caps.dial(handle, "net_rebinding", "localhost", port, false);
        assert!(matches!(refused, Err(NetError::Denied(_))), "{refused:?}");
        accepts.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// NET04: TLS verifies the peer's identity against the granted anchor. The
    /// same CA signs both a certificate for 127.0.0.1 (accepted) and one for
    /// `good.test` only (refused at the address's name check) — there is no
    /// insecure fallback in either direction.
    #[test]
    fn tls_accepts_the_granted_identity_and_refuses_a_name_mismatch() {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};

        let fixture = |name: &str| {
            std::fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/tls")
                    .join(name),
            )
            .unwrap()
        };
        let ca_pem = fixture("ca.pem");
        let anchors = crate::netstd_mod::certs_from_pem(&ca_pem).unwrap();
        let key_der = crate::netstd_mod::pem_blocks(&fixture("server.key"), "PRIVATE KEY")
            .unwrap()
            .remove(0);

        let serve = |cert: &str| {
            let (listener, port) = listener();
            listener.set_nonblocking(false).unwrap();
            let certs = crate::netstd_mod::certs_from_pem(&fixture(cert)).unwrap();
            let key = PrivateKeyDer::Pkcs8(key_der.clone().into());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    certs.into_iter().map(CertificateDer::from).collect(),
                    key,
                )
                .unwrap();
            let server = std::thread::spawn(move || {
                if let Ok((stream, _)) = listener.accept() {
                    let conn = rustls::ServerConnection::new(Arc::new(config)).unwrap();
                    let mut stream = rustls::StreamOwned::new(conn, stream);
                    while stream.conn.is_handshaking() {
                        if stream.conn.complete_io(&mut stream.sock).is_err() {
                            return;
                        }
                    }
                    let _ = stream.write_all(b"pong");
                    let _ = stream.conn.send_close_notify();
                    let _ = stream.conn.complete_io(&mut stream.sock);
                }
            });
            (port, server)
        };

        // Accepted: the granted anchor trusts this certificate for 127.0.0.1.
        let base = scratch("tls_ok");
        let (port, server) = serve("good.pem");
        let policy = NetPolicy::new(
            &[format!("127.0.0.1:{port}")],
            &["127.0.0.0/8".to_string()],
            anchors.clone(),
            Duration::from_millis(3000),
        )
        .unwrap();
        let caps = net_caps(&base, policy, "net_tls_ok");
        let handle = caps.open(Kind::Net).unwrap();
        let dialed = caps.dial(handle, "net_tls_ok", "127.0.0.1", port, true);
        assert!(dialed.is_ok(), "TLS dial to the granted identity failed: {dialed:?}");
        let mut response = Vec::new();
        while response.len() < 4 {
            response.extend(caps.net_read(handle, "net_tls_ok", 16).unwrap());
        }
        assert_eq!(response, b"pong");
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&base);

        // Refused: the CA is trusted, but the certificate names `good.test`,
        // not the address that was dialed.
        let base = scratch("tls_mismatch");
        let (port, server) = serve("wrong.pem");
        let policy = NetPolicy::new(
            &[format!("127.0.0.1:{port}")],
            &["127.0.0.0/8".to_string()],
            anchors,
            Duration::from_millis(3000),
        )
        .unwrap();
        let caps = net_caps(&base, policy, "net_tls_mismatch");
        let handle = caps.open(Kind::Net).unwrap();
        let refused = caps.dial(handle, "net_tls_mismatch", "127.0.0.1", port, true);
        assert!(matches!(refused, Err(NetError::Tls(_))), "{refused:?}");
        // The handle did not keep a connection that failed its handshake.
        assert!(caps.net_write(handle, "net_tls_mismatch", b"x").is_err());
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// NET05/NET06: revocation ends the connection and later dials, and the
    /// unsupported operations are refused by name at the ABI.
    #[test]
    fn revocation_ends_dials_and_unsupported_operations_fail_by_name() {
        let base = scratch("revoke_unsupported");
        let (listener, port) = listener();
        let (count, accepts) = count_accepts(listener, Duration::from_millis(300));
        let caps = net_caps(&base, policy(port, &["127.0.0.0/8"]), "net_revoke");

        let context = crate::auth::bind_instance_with_identity(
            "net_revoke",
            false,
            None,
            None,
            crate::auth::ManifestAuthEngine::new(),
        );
        let _scope = crate::auth::enter_caller(&context);
        super::caps::install(context.instance, caps);

        let handle = super::rs_cap_net_open();
        assert!(handle >= 0, "a granted Net handle opens, got {handle}");
        let host = b"127.0.0.1";
        assert_eq!(
            unsafe {
                super::rs_cap_net_dial(handle, host.as_ptr(), host.len() as i64, port as i64, 0)
            },
            0
        );
        assert_eq!(super::rs_cap_revoke(handle), 0);
        assert_eq!(
            unsafe { super::rs_cap_net_write(handle, b"x".as_ptr(), 1) },
            super::CAP_ERR_REFUSED
        );

        let udp = b"udp";
        assert_eq!(
            unsafe { super::rs_cap_net_unsupported(udp.as_ptr(), udp.len() as i64) },
            CAP_ERR_UNSUPPORTED
        );
        let raw = b"raw-socket";
        assert_eq!(
            unsafe { super::rs_cap_net_unsupported(raw.as_ptr(), raw.len() as i64) },
            CAP_ERR_UNSUPPORTED
        );
        let listen = b"listen";
        assert_eq!(
            unsafe { super::rs_cap_net_unsupported(listen.as_ptr(), listen.len() as i64) },
            CAP_ERR_UNSUPPORTED
        );

        accepts.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1, "the granted dial reached the server once");

        super::caps::uninstall(context.instance);
        crate::auth::revoke_instance(context.instance);
        let _ = std::fs::remove_dir_all(&base);
    }
}
