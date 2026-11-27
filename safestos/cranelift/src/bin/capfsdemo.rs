//! `capfsdemo` — U22's bounded capability-workflow start.
//!
//! A granted filesystem read and a denied traversal through the same
//! `rs_cap_*` entry points a compiled module calls, with an independent oracle:
//! the bytes the capability returns are compared to a direct `std::fs::read`,
//! and a denied path must leave an outside sentinel byte-identical.
//!
//! Usage: `capfsdemo <example-dir>`
//!
//! The directory is the instance's state root. Index 0 carries no rights at
//! all; index 1 is the `data/` subtree granted `read`. The traversal attempt
//! names `../outside/sentinel.txt`, which lies outside the granted root even
//! though it is inside the state directory the process can read directly — a
//! refusal there is about the capability, not about filesystem access.
//!
//! Exit: 0 the read matched and the traversal was refused; 1 a check failed
//! (the reason is printed); 2 usage.

use austral_cranelift_bridge::auth::{self, ManifestAuthEngine};
use austral_cranelift_bridge::capstd_mod::caps::{self, FsRights, InstanceCaps, RootGrant};
use austral_cranelift_bridge::capstd_mod::{rs_cap_open_root, rs_cap_read, CAP_ERR_REFUSED};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: capfsdemo <example-dir>");
        return ExitCode::from(2);
    };
    match run(&dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            eprintln!("capfs: {reason}");
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &Path) -> Result<(), String> {
    let dir =
        std::fs::canonicalize(dir).map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    let granted = dir.join("data/records.txt");
    let sentinel = dir.join("outside/sentinel.txt");
    let expected = std::fs::read(&granted)
        .map_err(|e| format!("cannot read the granted file {}: {e}", granted.display()))?;
    let sentinel_before = std::fs::read(&sentinel)
        .map_err(|e| format!("cannot read the sentinel {}: {e}", sentinel.display()))?;

    // The instance root is the example directory; the only approved subtree is
    // `data/`, read-only. Nothing grants the state root itself.
    let caps = InstanceCaps::new_with_grants(
        "cap_fs_demo",
        &dir,
        Vec::new(),
        FsRights::NONE,
        vec![RootGrant {
            root: "data".to_string(),
            rights: FsRights::READ,
        }],
        None,
    )?;
    let context = auth::bind_instance_with_identity(
        "cap_fs_demo",
        false,
        None,
        None,
        ManifestAuthEngine::new(),
    );
    let _scope = auth::enter_caller(&context);
    caps::install(context.instance, caps);

    let handle = rs_cap_open_root(1);
    if handle < 0 {
        return Err(format!(
            "the granted root did not open (code {handle}); the read cannot be measured"
        ));
    }

    // 1. The granted read: `records.txt` under the approved root.
    let path = b"records.txt";
    let mut buffer = vec![0u8; 4096];
    let read = unsafe {
        rs_cap_read(
            handle,
            path.as_ptr(),
            path.len() as i64,
            buffer.as_mut_ptr(),
            buffer.len() as i64,
        )
    };
    if read < 0 {
        return Err(format!(
            "the granted read was refused (code {read}); expected {path:?} under the data root"
        ));
    }
    let read = read as usize;
    if read != expected.len() || buffer[..read] != expected[..] {
        return Err(format!(
            "the capability returned {read} byte(s) that do not match the direct read of {}",
            granted.display()
        ));
    }
    println!(
        "granted read: data/records.txt -> {read} byte(s), identical to a direct std::fs::read"
    );

    // 2. The denied traversal: outside the granted subtree, still inside the
    //    state directory. The code must be the named refusal, never a value.
    let denied = b"../outside/sentinel.txt";
    let mut sink = vec![0u8; 4096];
    let refused = unsafe {
        rs_cap_read(
            handle,
            denied.as_ptr(),
            denied.len() as i64,
            sink.as_mut_ptr(),
            sink.len() as i64,
        )
    };
    if refused != CAP_ERR_REFUSED {
        return Err(format!(
            "the traversal up from the granted root returned {refused}, expected the named \
             refusal {CAP_ERR_REFUSED}"
        ));
    }
    println!(
        "denied read: data/../outside/sentinel.txt -> refused (code {refused}), no bytes copied"
    );

    // 3. The outside sentinel is untouched, so the refusal is not merely a
    //    status: nothing outside the granted root changed.
    let sentinel_after = std::fs::read(&sentinel)
        .map_err(|e| format!("cannot re-read the sentinel {}: {e}", sentinel.display()))?;
    if sentinel_after != sentinel_before {
        return Err("the outside sentinel changed across the denied read".to_string());
    }
    println!(
        "outside sentinel: unchanged at {} ({} byte(s))",
        sentinel.display(),
        sentinel_after.len()
    );

    // 4. The grant is read-only: a write under the same handle is refused
    //    before any file is created.
    let write_path = b"written-by-demo.txt";
    let data = b"must not be written";
    let write = unsafe {
        austral_cranelift_bridge::capstd_mod::rs_cap_write(
            handle,
            write_path.as_ptr(),
            write_path.len() as i64,
            data.as_ptr(),
            data.len() as i64,
        )
    };
    if write != CAP_ERR_REFUSED || dir.join("data/written-by-demo.txt").exists() {
        return Err(format!(
            "a read-only grant allowed a write (code {write}); expected {CAP_ERR_REFUSED}"
        ));
    }
    println!("write under a read grant: refused (code {write}), no file created");

    println!("capfs workflow: granted read matched, traversal refused, sentinel intact");
    Ok(())
}
