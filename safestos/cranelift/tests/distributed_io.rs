//! D09 step 4: the Austral demo, end to end through the manifest gate.
//!
//! `umbrella/modules/distributed_io/module.cps` is the CPS the ordinary
//! front-end emits for `DistributedIo.aum` (`gen_cps.sh` regenerates it). It is
//! committed (with an exception in `umbrella/.gitignore`) so this test runs on a
//! fresh checkout, where the OCaml compiler that emits it has not been built. The
//! module calls `rs_cap_dir_open` and `rs_cap_write`: capability verbs that do
//! not exist in the module's own memory, only as host-issued handles.
//!
//! Two runs, one variable: the manifest grant. With it, the call reaches the
//! capability and a file appears in the instance state directory. Without it,
//! the JIT refuses the call with UK-4001 at load time and the file is never
//! written — "missing grant produces no sink call" at the module boundary, not
//! only inside the Rust unit tests.

use std::path::{Path, PathBuf};
use std::process::Command;

fn demo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../umbrella/modules/distributed_io")
}

/// Copy the demo into a scratch directory so the run's `.state` lands in the
/// test's own temp tree, and rewrite `module.toml` for the variant under test.
fn stage(tag: &str, grants: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("d09_distributed_io_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = demo_dir();
    for name in ["module.cps", "DistributedIo.aum"] {
        std::fs::copy(source.join(name), dir.join(name))
            .unwrap_or_else(|e| panic!("copy {name}: {e}"));
    }
    let grants: Vec<String> = grants.iter().map(|g| format!("    \"{g}\",")).collect();
    let manifest = format!(
        "[module]\nname = \"DistributedIo\"\nversion = \"0.1.0\"\n\
         archetypes = [\"austral_cps\"]\narchetype = \"austral_cps\"\n\
         entry = \"main\"\nnode = \"node-7\"\n\n[grants]\nkernel = [\n{}\n]\n",
        grants.join("\n")
    );
    std::fs::write(dir.join("module.toml"), manifest).unwrap();
    dir
}

fn run(dir: &Path) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_modhost"))
        .args(["host", dir.to_str().unwrap(), "--call", "compute"])
        .output()
        .expect("modhost runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_granted_module_writes_through_its_capability() {
    let dir = stage("granted", &["rs_cap_dir_open", "rs_cap_write"]);
    let (code, stdout, stderr) = run(&dir);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stdout.contains("compute: 25"),
        "the byte count of the frame: {stdout}"
    );
    let written = std::fs::read_to_string(dir.join(".state/note.txt"))
        .expect("the capability wrote the file inside the instance state dir");
    assert_eq!(written, "hello from distributed_io");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_the_grant_the_call_is_refused_and_nothing_is_written() {
    // `rs_cap_dir_open` is granted, `rs_cap_write` is not: the refusal must name
    // the missing grant, and the file must not exist afterwards.
    let dir = stage("ungranted", &["rs_cap_dir_open"]);
    let (code, stdout, stderr) = run(&dir);
    assert_eq!(code, 1, "a refused load exits 1 (stdout={stdout})");
    assert!(
        stderr.contains("UK-4001") && stderr.contains("rs_cap_write"),
        "the refusal names the ungranted verb: {stderr}"
    );
    assert!(
        !dir.join(".state/note.txt").exists(),
        "no capability call happened, so no file was written"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
