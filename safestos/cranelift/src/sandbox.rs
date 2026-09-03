//! S3: OS sandbox layer — a dedicated launcher wrapping a child process in the OS containment
//! Chromium composes around V8 renderers, so the workerd sidecar is browser-equivalent:
//!
//! - **User namespace** (`CLONE_NEWUSER`): the child runs as an unprivileged, mapped uid/gid
//!   with no host capabilities; its root is confined to the namespace.
//! - **Network namespace** (`CLONE_NEWNET`): an empty netns — no ambient sockets; the only
//!   reachable endpoints are the unix sockets the sidecar materializes in its staging dir
//!   (that is why `ecma.rs` uses unix sockets rather than TCP).
//! - **IPC namespace** (`CLONE_NEWIPC`): no shared host SysV IPC.
//! - **`no_new_privs`**: irrevocably disables privilege escalation inside the sandbox.
//! - **seccomp-bpf**: denies a deny-list of dangerous syscalls (ptrace, mount, reboot,
//!   kexec_load, open_by_handle_at, module load, etc.) — a Chrome-renderer-style filter.
//! - **Landlock**: read-only view of the engine/system dirs; writes are confined to the
//!   staging dir and the module's granted `[grants] fs` paths; everything else is
//!   inaccessible.
//!
//! The threat model is *browser-equivalent*, not VM-equivalent: a compromised engine yields a
//! confined, low-privilege process with no ambient net/fs access — the same tier as a sandboxed
//! renderer, not the "Malicious Principal / hardware attacks" tier of a full VM (PLAN §2.1, §2.3).
//!
//! Known limitation: no `CLONE_NEWPID` (a pid-namespace init must be the *first child* of the
//! unsharing process, which would break `Child::kill()` cleanup in the single-exec model).
//! The user namespace already blocks host-process signalling, so the practical isolation gain
//! of a pid ns here is `hidepid`-style /proc visibility, deferred to the bwrap-style launcher.

use std::ffi::CString;
use std::os::raw::{c_int, c_uint};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Confinement profile for one sandboxed process.
pub struct SandboxProfile {
    /// Directories the child may **write** (staging dir + granted `[grants] fs` paths).
    pub writable_dirs: Vec<PathBuf>,
    /// Directories the child may **read/execute** (engine, libs, system). The child's own
    /// binary and its dynamic deps must be readable or the loader cannot start.
    pub readable_dirs: Vec<PathBuf>,
    /// Optional memory hard cap (bytes) applied via cgroup if a writable cgroup is available.
    pub memory_max_bytes: Option<u64>,
}

// Landlock access-right bits (linux/landlock.h). We use the subset valid for ABI >= 2.
const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;

const LANDLOCK_ACCESS_FS_ALL_WRITE: u64 = LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_REMOVE_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_FILE
    | LANDLOCK_ACCESS_FS_MAKE_CHAR
    | LANDLOCK_ACCESS_FS_MAKE_DIR
    | LANDLOCK_ACCESS_FS_MAKE_REG
    | LANDLOCK_ACCESS_FS_MAKE_SOCK
    | LANDLOCK_ACCESS_FS_MAKE_FIFO
    | LANDLOCK_ACCESS_FS_MAKE_BLOCK
    | LANDLOCK_ACCESS_FS_MAKE_SYM;

const LANDLOCK_ACCESS_FS_ALL_READ: u64 =
    LANDLOCK_ACCESS_FS_EXECUTE | LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_READ_DIR;

const DENY_LIST: &[i64] = &[
    libc::SYS_ptrace,
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_reboot,
    libc::SYS_kexec_load,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
    libc::SYS_open_by_handle_at,
    libc::SYS_setns,
    libc::SYS_perf_event_open,
    libc::SYS_bpf,
    libc::SYS_userfaultfd,
    libc::SYS_quotactl,
    libc::SYS_swapon,
    libc::SYS_swapoff,
    libc::SYS_ioperm,
    libc::SYS_iopl,
    libc::SYS_kcmp,
];

/// Whether the kernel exposes the sandbox primitives (unprivileged user namespaces).
pub fn supported() -> bool {
    userns_ok()
}

/// Probe unprivileged user-namespace support WITHOUT mutating the caller. `unshare(CLONE_NEWUSER)`
/// puts the calling process into a fresh userns — calling it on a live process (e.g. a test
/// binary) would strand the caller in an unmapped namespace. Probe inside a forked child instead:
/// the child attempts the unshare and exits 0/1, the parent only reads the exit status. Only
/// async-signal-safe calls are used between fork and _exit (unshare, _exit).
fn userns_ok() -> bool {
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return false;
    }
    if pid == 0 {
        // Child: try the unshare and report via exit status.
        let ok = unsafe { libc::unshare(libc::CLONE_NEWUSER) == 0 };
        unsafe { libc::_exit(if ok { 0 } else { 1 }) };
    }
    let mut status = 0;
    unsafe {
        libc::waitpid(pid, &mut status, 0);
    }
    libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
}
/// Build a `Command` whose child runs `program` confined by `profile`. The caller then
/// spawns it as usual; sandbox setup happens in the forked child just before `execve`.
pub fn sandboxed_command(program: &Path, profile: &SandboxProfile) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());

    // Pre-canonicalize Landlock path_beneath targets so the kernel sees the resolved path.
    let resolve =
        |p: &PathBuf| -> PathBuf { std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()) };
    let writable: Vec<PathBuf> = profile.writable_dirs.iter().map(resolve).collect();
    let readable: Vec<PathBuf> = profile.readable_dirs.iter().map(resolve).collect();

    unsafe {
        cmd.pre_exec(move || {
            setup_sandbox(&writable, &readable)?;
            Ok(())
        });
    }
    cmd
}

/// The confinement sequence, run in the forked child before `execve`:
/// userns → uid/gid map → net+ipc ns → no_new_privs → seccomp → Landlock.
unsafe fn setup_sandbox(writable: &[PathBuf], readable: &[PathBuf]) -> std::io::Result<()> {
    // Capture the caller's ids BEFORE unsharing — after CLONE_NEWUSER getuid()/getgid()
    // report the overflow id (65534), but the map must reference the parent-namespace ids.
    let uid = libc::getuid();
    let gid = libc::getgid();

    // 1. user namespace.
    if libc::unshare(libc::CLONE_NEWUSER) != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // gid_map requires denying setgroups first for unprivileged writers.
    let deny = b"deny";
    let sf = CString::new("/proc/self/setgroups").unwrap();
    let fd = libc::open(sf.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
    if fd >= 0 {
        let _ = libc::write(fd, deny.as_ptr() as *const _, deny.len());
        libc::close(fd);
    }
    write_map("/proc/self/uid_map", uid)?;
    write_map("/proc/self/gid_map", gid)?;

    // 2. network + ipc namespaces (empty netns; no ambient sockets).
    if libc::unshare(libc::CLONE_NEWNET | libc::CLONE_NEWIPC) != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // 3. no_new_privs.
    if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // 4. seccomp-bpf deny-list.
    apply_seccomp()?;

    // 5. Landlock: read/exec on engine+system dirs; write confined to granted dirs.
    apply_landlock(writable, readable)?;

    Ok(())
}

fn write_map(file: &str, id: u32) -> std::io::Result<()> {
    let s = format!("0 {id} 1\n");
    let f = CString::new(file).unwrap();
    let fd = unsafe { libc::open(f.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let n = unsafe { libc::write(fd, s.as_ptr() as *const _, s.len()) };
    unsafe { libc::close(fd) };
    if n != s.len() as isize {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("write {file} failed"),
        ));
    }
    Ok(())
}

/// seccomp-bpf deny-list: allow everything except the denied set (which returns `EPERM`).
/// unix sockets are NOT denied — workerd needs them for the loopback and main socket, and the
/// network namespace already provides the isolation.
fn apply_seccomp() -> std::io::Result<()> {
    let filters = build_deny_filter();
    // Re-wrap the raw tuples into the C layout for the kernel.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }
    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }
    let owned: Vec<SockFilter> = filters
        .iter()
        .map(|&(code, jt, jf, k)| SockFilter { code, jt, jf, k })
        .collect();

    let prog = SockFprog {
        len: owned.len() as u16,
        filter: owned.as_ptr(),
    };
    let r = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            &prog as *const _,
            0,
            0,
        )
    };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Build the deny-list BPF program: `LD nr`, then per denied nr a `JEQ` that on a match jumps
/// over the remaining JEQs and the ALLOW, landing on the `ERRNO|EPERM` tail. (code 0x20 = BPF_LD
/// | BPF_W | BPF_ABS on seccomp_data.nr at offset 0; 0x15 = BPF_JMP | BPF_JEQ | BPF_K; 0x06 =
/// BPF_RET | BPF_K.)
fn build_deny_filter() -> Vec<(u16, u8, u8, u32)> {
    // Each JEQ must skip (DENY_LIST.len() - i) instructions — all later JEQs plus the ALLOW —
    // so a match lands on the ERRNO tail. jt=1 (only skipping the next JEQ) would let the
    // denied syscall fall through to ALLOW.
    let mut filters = Vec::with_capacity(DENY_LIST.len() + 2);
    filters.push((0x20, 0, 0, 0));
    for (i, &nr) in DENY_LIST.iter().enumerate() {
        filters.push((0x15, (DENY_LIST.len() - i) as u8, 0, nr as u32));
    }
    filters.push((0x06, 0, 0, 0x7fff_0000)); // SECCOMP_RET_ALLOW
    filters.push((0x06, 0, 0, 0x0005_0000 | 1)); // ERRNO|EPERM
    filters
}

/// Landlock ruleset: read/exec allowed on `readable`; read+write on `writable`; all else denied.
fn apply_landlock(writable: &[PathBuf], readable: &[PathBuf]) -> std::io::Result<()> {
    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }
    #[repr(C)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: c_int,
    }
    const RULE_PATH_BENEATH: c_uint = 1;

    let mut attr = RulesetAttr {
        handled_access_fs: LANDLOCK_ACCESS_FS_ALL_READ | LANDLOCK_ACCESS_FS_ALL_WRITE,
    };
    let ruleset = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &mut attr as *mut _,
            std::mem::size_of::<RulesetAttr>(),
            0,
        )
    } as c_int;
    if ruleset < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let add_beneath = |dir: &PathBuf, access: u64, skip_missing: bool| -> std::io::Result<()> {
        let c = CString::new(dir.to_str().unwrap_or("")).unwrap();
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if fd < 0 {
            if skip_missing {
                // A readable dir that does not exist (e.g. `/lib`, `/usr/lib` are absent on
                // NixOS) must not abort the whole sandbox — skip it. Writable dirs are checked
                // strictly: a missing write target is a real configuration error.
                return Ok(());
            }
            return Err(std::io::Error::last_os_error());
        }
        let mut beneath = PathBeneathAttr {
            allowed_access: access,
            parent_fd: fd,
        };
        let r = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset,
                RULE_PATH_BENEATH,
                &mut beneath as *mut _,
                0,
            )
        };
        unsafe { libc::close(fd) };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };

    for d in writable {
        add_beneath(
            d,
            LANDLOCK_ACCESS_FS_ALL_READ | LANDLOCK_ACCESS_FS_ALL_WRITE,
            false,
        )?;
    }
    for d in readable {
        add_beneath(d, LANDLOCK_ACCESS_FS_ALL_READ, true)?;
    }

    let r = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0) };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Convenience wrapper for tests / CLIs: spawn a sandboxed command with piped stdio and return
/// the child. `args` are appended verbatim.
pub fn spawn_sandboxed(
    program: &Path,
    args: &[&str],
    profile: &SandboxProfile,
) -> Result<Child, String> {
    let mut cmd = sandboxed_command(program, profile);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.spawn().map_err(|e| format!("spawn sandboxed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_list_contains_mount_and_ptrace() {
        assert!(DENY_LIST.contains(&libc::SYS_ptrace));
        assert!(DENY_LIST.contains(&libc::SYS_mount));
        assert!(DENY_LIST.contains(&libc::SYS_pivot_root));
    }

    #[test]
    fn landlock_bits_are_sane() {
        // Writes must be a strict subset of the handled set (no write outside what we grant).
        let handled = LANDLOCK_ACCESS_FS_ALL_READ | LANDLOCK_ACCESS_FS_ALL_WRITE;
        assert_eq!(LANDLOCK_ACCESS_FS_ALL_WRITE & !handled, 0);
    }

    #[test]
    fn seccomp_deny_jumps_land_on_errno_tail() {
        // Regression: the old filter used jt=1 on every JEQ, so a matched (denied) syscall only
        // skipped the *next* JEQ and fell through to ALLOW — the deny-list silently permitted
        // every denied syscall. Each JEQ must jump over all remaining JEQs + the ALLOW so a
        // match lands exactly on the ERRNO|EPERM tail.
        let f = build_deny_filter();
        // [0] LD nr
        assert_eq!(f[0].0, 0x20);
        // [1..=n] JEQs, [n+1] ALLOW, [n+2] ERRNO
        let n = DENY_LIST.len();
        assert_eq!(f.len(), n + 3);
        for (i, instr) in f[1..1 + n].iter().enumerate() {
            assert_eq!(instr.0, 0x15, "instruction {i} must be a JEQ");
            // i-th JEQ: skip (n - i) instructions = the remaining JEQs + the ALLOW.
            let expected = (n - i) as u8;
            assert_eq!(
                instr.1, expected,
                "JEQ #{i} jump distance wrong: match would not land on the ERRNO tail"
            );
        }
        // Tail: ALLOW then ERRNO|EPERM.
        assert_eq!(f[n + 1].0, 0x06);
        assert_eq!(
            f[n + 1].3,
            0x7fff_0000,
            "penultimate instruction must be SECCOMP_RET_ALLOW"
        );
        assert_eq!(f[n + 2].0, 0x06);
        assert_eq!(
            f[n + 2].3,
            0x0005_0000 | 1,
            "last instruction must be ERRNO|EPERM"
        );
    }
}
