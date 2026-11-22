//! G14: the manifest-gated `rs_gpu_*` service contract, exercised through the
//! exported ABI. The bundle is built in a scratch directory, opened by path,
//! run with deterministic inputs, and closed. With `device` enabled the same
//! path requires a physical adapter; without it the run refuses rather than
//! silently returning the reference result.

#![cfg(feature = "service")]

use avm_gpu::schema::*;
use std::path::{Path, PathBuf};

const ADD_WGSL: &str = r#"
@group(0) @binding(0) var<storage, read> input0: array<u32>;
@group(0) @binding(1) var<storage, read> input1: array<u32>;
@group(0) @binding(2) var<storage, read_write> output0: array<u32>;

fn scalar_entry(p0: u32, p1: u32) -> u32 {
    return p0 + p1;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&input0)) { return; }
    output0[i] = scalar_entry(input0[i], input1[i]);
}
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "avm-gpu-service-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_add_bundle(dir: &Path) {
    let bindings: Vec<serde_json::Value> = ["input", "input", "output"]
        .iter()
        .enumerate()
        .map(|(i, role)| {
            serde_json::json!({
                "group": 0,
                "binding": i,
                "role": role,
                "element": "u32",
                "stride_bytes": 4,
            })
        })
        .collect();
    let mut bundle = KernelBundle {
        schema: SCHEMA.to_string(),
        backend: BACKEND.to_string(),
        numeric_profile: NUMERIC_PROFILE.to_string(),
        entry: "Add:k".to_string(),
        input_arity: 2,
        compiler: "g14-service-test".to_string(),
        sources: vec![Source {
            path: "Add.aum".to_string(),
            sha256: sha256_hex(b"module body Add\n"),
        }],
        kernel_ir: KernelIr {
            entry: "Add:k".to_string(),
            input_arity: 2,
            helpers: Vec::new(),
            body: Stmt::Return {
                value: Expr::Intrinsic {
                    op: "add".to_string(),
                    args: vec![Expr::Param { index: 0 }, Expr::Param { index: 1 }],
                },
            },
            memory: MemoryPlan {
                map: MapKind::Map2,
                output_index: "identity".to_string(),
                guard_before_load: true,
                workgroup_size: WORKGROUP_SIZE,
            },
            numeric_profile: NUMERIC_PROFILE.to_string(),
        },
        ir_sha256: String::new(),
        wgsl_sha256: String::new(),
        requirements: serde_json::json!({
            "bindings": bindings,
            "numeric": {
                "profile": NUMERIC_PROFILE,
                "element": "u32",
                "operations": ["add", "subtract", "multiply", "bitwiseAnd", "bitwiseOr", "bitwiseXor", "bitwiseNot"],
                "rounding": "modulo-2^32",
                "contraction": "not-applicable",
                "domain": "all-2^32",
                "subnormals": "not-applicable",
                "overflow": "defined-wraparound",
                "tolerance_ulp_per_operation": 0
            }
        }),
        evidence: Evidence {
            source: "test".to_string(),
            source_profile: None,
            shader_validation: "naga".to_string(),
            device_kind: "none".to_string(),
            execution: "reference".to_string(),
            proof: "none".to_string(),
        },
        proof_manifest: None,
    };
    bundle.ir_sha256 = sha256_hex(&bundle.kernel_ir.canonical_bytes().unwrap());
    bundle.wgsl_sha256 = sha256_hex(ADD_WGSL.as_bytes());
    std::fs::write(
        dir.join("kernel.json"),
        serde_json::to_vec_pretty(&bundle).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("kernel.wgsl"), ADD_WGSL).unwrap();
}

fn open(path: &Path, require_hardware: i64) -> i64 {
    let bytes = path.to_str().unwrap().as_bytes();
    avm_gpu::service::rs_gpu_open_at(
        bytes.as_ptr(),
        bytes.len() as i64,
        require_hardware,
        -1,
    )
}

#[test]
fn the_service_opens_runs_and_closes_a_real_bundle() {
    let scratch = Scratch::new("reference");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    assert!(h > 0, "open returned {h}");
    // outputs are 2*i for seed 0, so the checksum for n=8 is 56.
    assert_eq!(avm_gpu::service::rs_gpu_run(h, 8, 0), 56);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_run(h, 8, 0),
        avm_gpu::service::UNKNOWN_HANDLE
    );
}

#[test]
fn the_service_refuses_bad_lengths_and_handles() {
    let scratch = Scratch::new("refusals");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    assert!(h > 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_run(h, MAX_ELEMENTS as i64 + 1, 0),
        avm_gpu::service::LIMIT_REFUSED
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_run(h, 5, 0),
        20,
        "outputs are 2*i"
    );
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_close(h),
        avm_gpu::service::UNKNOWN_HANDLE
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_run(h, 1, 0),
        avm_gpu::service::UNKNOWN_HANDLE
    );
}

#[cfg(feature = "device")]
#[test]
fn a_physical_required_run_is_a_device_launch() {
    let scratch = Scratch::new("device");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 1);
    assert!(h > 0, "open returned {h}");
    let result = avm_gpu::service::rs_gpu_run(h, 8, 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
    if result < 0 {
        assert!(
            result == avm_gpu::service::DEVICE_REFUSED
                || result == avm_gpu::service::DEVICE_UNAVAILABLE,
            "unexpected device status {result}"
        );
        if std::env::var_os("AVM_GPU_ALLOW_NO_DEVICE").is_some() {
            eprintln!("[gap] G14 service device launch not run: no physical adapter");
        } else {
            panic!("G14 service needs a physical adapter; result {result}");
        }
    } else {
        assert_eq!(result, 56);
        println!("[pass] G14 service launched the add bundle on a physical adapter");
    }
}

/// One open/submit/poll/read/close cycle on two live contexts.
///
/// Kept as a function so the same teardown path can run more than once: the
/// device is dropped by the worker that `rs_gpu_close` joins, and a regression
/// there shows up as a crash in the process, not as a failed assertion.
#[cfg(feature = "device")]
fn one_physical_cycle(scratch: &Path, adapter_index: i64) -> Option<(i64, i64)> {
    let path = scratch.to_str().unwrap().as_bytes();
    let open_on = |require_hardware: i64| {
        avm_gpu::service::rs_gpu_open_at(
            path.as_ptr(),
            path.len() as i64,
            require_hardware,
            adapter_index,
        )
    };
    let first = open_on(1);
    let second = open_on(1);
    assert!(first > 0 && second > 0, "open returned {first}/{second}");

    let first_buffer = avm_gpu::service::rs_gpu_buffer_new(first, 16);
    let second_buffer = avm_gpu::service::rs_gpu_buffer_new(second, 8);
    assert!(first_buffer > 0 && second_buffer > 0);
    // Two live contexts: the second submit runs while the first job is
    // outstanding. Each result must belong to its own buffer.
    let first_job = avm_gpu::service::rs_gpu_submit(first, first_buffer, 16, 0);
    let second_job = avm_gpu::service::rs_gpu_submit(second, second_buffer, 8, 100);
    assert!(first_job > 0 && second_job > 0);
    let first_state = poll_until_terminal(first, first_job);
    let second_state = poll_until_terminal(second, second_job);

    let no_device = |code: i64| {
        code == avm_gpu::service::DEVICE_REFUSED
            || code == avm_gpu::service::DEVICE_UNAVAILABLE
    };
    let mut out = Vec::new();
    if no_device(first_state) || no_device(second_state) {
        assert!(
            no_device(first_state) && no_device(second_state),
            "one context completed ({first_state}/{second_state}) while the other had no device"
        );
        assert_eq!(avm_gpu::service::rs_gpu_buffer_free(first, first_buffer), 0);
        assert_eq!(avm_gpu::service::rs_gpu_buffer_free(second, second_buffer), 0);
        assert_eq!(avm_gpu::service::rs_gpu_close(first), 0);
        assert_eq!(avm_gpu::service::rs_gpu_close(second), 0);
        return Some((first_state, second_state));
    }
    assert_eq!(first_state, avm_gpu::service::COMPLETE);
    assert_eq!(second_state, avm_gpu::service::COMPLETE);

    // outputs are input0 + input1 = seed + 2*i, owned by each job's buffer.
    out.resize(16 * 4, 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_result(first, first_job, out.as_mut_ptr(), 64),
        64
    );
    let first_words: Vec<u32> = out
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    assert_eq!(first_words, (0..16u32).map(|i| 2 * i).collect::<Vec<_>>());

    out.clear();
    out.resize(8 * 4, 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_result(second, second_job, out.as_mut_ptr(), 32),
        32
    );
    let second_words: Vec<u32> = out
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    assert_eq!(second_words, (0..8u32).map(|i| 100 + 2 * i).collect::<Vec<_>>());

    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(first, first_buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(second, second_buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(first), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(second), 0);
    None
}

/// The one stress iteration, run in a child process.
///
/// The teardown race this guards is invisible to a loop in one process: the
/// crash is the device being destroyed under a worker *while the process is
/// exiting*, after `rs_gpu_close` already returned. So the stress mode re-execs
/// the test binary once per run and each child exits immediately after the
/// cycle.
#[cfg(feature = "device")]
#[test]
#[ignore = "spawned by AVM_GPU_STRESS_RUNS"]
fn a_physical_submit_child_cycle() {
    let scratch = Scratch::new("device-jobs-child");
    write_add_bundle(&scratch.0);
    let adapter_index = adapter_index();
    if let Some((first_state, second_state)) = one_physical_cycle(&scratch.0, adapter_index) {
        assert!(
            first_state == avm_gpu::service::DEVICE_REFUSED
                || first_state == avm_gpu::service::DEVICE_UNAVAILABLE
        );
        assert_eq!(first_state, second_state);
        if std::env::var_os("AVM_GPU_ALLOW_NO_DEVICE").is_some() {
            eprintln!("[gap] G15 device lifecycle not run: no physical adapter");
            return;
        }
        panic!("G15 service jobs need a physical adapter; result {first_state}/{second_state}");
    }
}

// Default is wgpu's first suitable adapter; AVM_GPU_ADAPTER_INDEX selects one
// explicitly on a multi-GPU host (e.g. 1 for the discrete GPU here).
#[cfg(feature = "device")]
fn adapter_index() -> i64 {
    std::env::var("AVM_GPU_ADAPTER_INDEX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1)
}

/// Re-exec this test binary `runs` times, once per cycle.
///
/// Each child does one open/submit/close and then exits normally, which is the
/// only way to observe a crash in the process exit path: a parent that keeps
/// running has already survived the teardown.
#[cfg(feature = "device")]
fn stress_children(runs: usize) {
    let exe = std::env::current_exe().expect("current test binary");
    let mut failures = 0;
    for run in 0..runs {
        let status = std::process::Command::new(&exe)
            .args([
                "--exact",
                "a_physical_submit_child_cycle",
                "--ignored",
                "--nocapture",
            ])
            .env("AVM_GPU_STRESS_CHILD", "1")
            .status()
            .expect("spawn stress child");
        if !status.success() {
            failures += 1;
            eprintln!("[FAIL] G15 stress run {run} exited with {status}");
        }
    }
    assert_eq!(
        failures, 0,
        "{failures} of {runs} device teardowns did not exit cleanly"
    );
    println!("[pass] G15 {runs} physical cycles in separate processes, each exiting cleanly");
}

#[cfg(feature = "device")]
#[test]
fn a_physical_submit_completes_each_context_with_its_own_result() {
    // `AVM_GPU_STRESS_RUNS=N` repeats the whole cycle, one process per run, so
    // a teardown that races process exit shows up as a failed child. The
    // default is one in-process cycle, which keeps CI at a single launch.
    let runs: usize = std::env::var("AVM_GPU_STRESS_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(1);
    if runs > 1 {
        stress_children(runs);
        return;
    }
    let scratch = Scratch::new("device-jobs");
    write_add_bundle(&scratch.0);
    let adapter_index = adapter_index();
    if let Some((first_state, second_state)) = one_physical_cycle(&scratch.0, adapter_index) {
        assert!(
            first_state == avm_gpu::service::DEVICE_REFUSED
                || first_state == avm_gpu::service::DEVICE_UNAVAILABLE
        );
        assert_eq!(first_state, second_state);
        if std::env::var_os("AVM_GPU_ALLOW_NO_DEVICE").is_some() {
            eprintln!("[gap] G15 device lifecycle not run: no physical adapter");
            return;
        }
        panic!("G15 service jobs need a physical adapter; result {first_state}/{second_state}");
    }
    println!("[pass] G15 two physical contexts submitted and completed their own results");
}

// ---------------------------------------------------------------------------
// G15: buffers, asynchronous jobs and the mock lifecycle.
//
// `service_test_submit` creates a pending job with no worker; the test
// publishes the terminal event explicitly. That is how the lifecycle is
// exercised deterministically; the real worker path is covered by
// `the_service_opens_runs_and_closes_a_real_bundle` and the device test.
// ---------------------------------------------------------------------------

fn write_words(h: i64, buffer: i64, words: &[u32]) -> i64 {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for w in words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    avm_gpu::service::rs_gpu_buffer_write(
        h,
        buffer,
        0,
        bytes.as_ptr(),
        bytes.len() as i64,
    )
}

fn read_words(h: i64, buffer: i64, count: usize) -> Vec<u32> {
    let mut bytes = vec![0u8; count * 4];
    let read = avm_gpu::service::rs_gpu_buffer_read(
        h,
        buffer,
        0,
        bytes.as_mut_ptr(),
        bytes.len() as i64,
    );
    assert_eq!(read, bytes.len() as i64, "buffer read failed: {read}");
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn poll_until_terminal(h: i64, job: i64) -> i64 {
    for _ in 0..2000 {
        let state = avm_gpu::service::rs_gpu_poll(h, job);
        if state != avm_gpu::service::PENDING {
            return state;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("job {job} never reached a terminal state");
}

#[test]
fn buffers_roundtrip_and_a_completed_job_writes_its_result() {
    let scratch = Scratch::new("buffer-roundtrip");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    assert!(h > 0, "open returned {h}");
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 8);
    assert!(buffer > 0, "buffer_new returned {buffer}");
    assert_eq!(write_words(h, buffer, &[7; 8]), 0);
    assert_eq!(read_words(h, buffer, 8), vec![7; 8]);

    let job = avm_gpu::service::rs_gpu_submit(h, buffer, 8, 0);
    assert!(job > 0, "submit returned {job}");
    // The job owns the buffer until it completes.
    assert_eq!(
        avm_gpu::service::rs_gpu_buffer_read(h, buffer, 0, std::ptr::null_mut(), 0),
        avm_gpu::service::BUSY
    );
    assert_eq!(poll_until_terminal(h, job), avm_gpu::service::COMPLETE);
    assert_eq!(read_words(h, buffer, 8), (0..8u32).map(|i| 2 * i).collect::<Vec<_>>());

    // The receipt is single-use.
    let mut out = vec![0u8; 32];
    assert_eq!(
        avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 32),
        32
    );
    assert_eq!(out, read_words(h, buffer, 8).iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<_>>());
    assert_eq!(
        avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 32),
        avm_gpu::service::STALE
    );
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(h, buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

#[test]
fn a_second_live_job_is_refused_with_busy() {
    let scratch = Scratch::new("busy");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 4);
    let job = avm_gpu::service::service_test_submit(h, buffer, 4, 0);
    assert!(job > 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_submit(h, buffer, 4, 0),
        avm_gpu::service::BUSY
    );
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::COMPLETE, &[1, 2, 3, 4]),
        0
    );
    let second = avm_gpu::service::rs_gpu_submit(h, buffer, 4, 0);
    assert!(second > 0 && second != job);
    assert_eq!(poll_until_terminal(h, second), avm_gpu::service::COMPLETE);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

#[test]
fn a_second_context_advances_while_the_first_job_is_pending() {
    let scratch = Scratch::new("two-contexts");
    write_add_bundle(&scratch.0);
    let first = open(&scratch.0, 0);
    let second = open(&scratch.0, 0);
    assert!(first > 0 && second > 0 && first != second);

    let first_buffer = avm_gpu::service::rs_gpu_buffer_new(first, 2);
    let first_job = avm_gpu::service::service_test_submit(first, first_buffer, 2, 0);
    assert!(first_job > 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_poll(first, first_job),
        avm_gpu::service::PENDING
    );

    // A second context is independent: it allocates, submits and completes
    // while the first job has not terminated.
    let second_buffer = avm_gpu::service::rs_gpu_buffer_new(second, 3);
    let second_job = avm_gpu::service::service_test_submit(second, second_buffer, 3, 0);
    assert!(second_job > 0 && second_job != first_job);
    assert_eq!(
        avm_gpu::service::service_test_publish(
            second_job,
            avm_gpu::service::COMPLETE,
            &[4, 5, 6]
        ),
        0
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_poll(second, second_job),
        avm_gpu::service::COMPLETE
    );
    assert_eq!(read_words(second, second_buffer, 3), vec![4, 5, 6]);
    // The first job is untouched by the second context's completion.
    assert_eq!(
        avm_gpu::service::rs_gpu_poll(first, first_job),
        avm_gpu::service::PENDING
    );

    // Identities do not cross contexts: neither the other job nor the other
    // buffer is visible through the first context.
    assert_eq!(
        avm_gpu::service::rs_gpu_poll(first, second_job),
        avm_gpu::service::UNKNOWN_HANDLE
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_buffer_read(
            first,
            second_buffer,
            0,
            std::ptr::null_mut(),
            0
        ),
        avm_gpu::service::UNKNOWN_HANDLE
    );
    // A pending job still quarantines its context.
    assert_eq!(
        avm_gpu::service::rs_gpu_close(first),
        avm_gpu::service::BUSY
    );

    assert_eq!(
        avm_gpu::service::service_test_publish(first_job, avm_gpu::service::COMPLETE, &[7, 8]),
        0
    );
    assert_eq!(read_words(first, first_buffer, 2), vec![7, 8]);
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(first, first_buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(second, second_buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(first), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(second), 0);
}

#[test]
fn repeated_and_reordered_completion_events_keep_one_owner() {
    let scratch = Scratch::new("repeat");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 3);
    let job = avm_gpu::service::service_test_submit(h, buffer, 3, 0);
    assert_eq!(avm_gpu::service::rs_gpu_poll(h, job), avm_gpu::service::PENDING);
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::CANCELLED, &[]),
        0
    );
    // A later/reordered COMPLETE must not overwrite the first terminal event.
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::COMPLETE, &[9, 9, 9]),
        avm_gpu::service::STALE
    );
    assert_eq!(avm_gpu::service::rs_gpu_poll(h, job), avm_gpu::service::CANCELLED);
    let mut out = vec![0u8; 12];
    assert_eq!(
        avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 12),
        avm_gpu::service::STALE
    );
    assert_eq!(read_words(h, buffer, 3), vec![0; 3]);
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(h, buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

#[test]
fn stale_generation_and_forged_identities_are_refused() {
    let scratch = Scratch::new("stale");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 2);
    let job = avm_gpu::service::service_test_submit(h, buffer, 2, 0);
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::COMPLETE, &[1, 2]),
        0
    );
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
    // The job belongs to a closed generation even though its number is live.
    assert_eq!(avm_gpu::service::rs_gpu_poll(h, job), avm_gpu::service::STALE);
    let mut out = vec![0u8; 8];
    assert_eq!(
        avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 8),
        avm_gpu::service::STALE
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_cancel(h, job, avm_gpu::service::CANCEL_WAIT),
        avm_gpu::service::STALE
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_buffer_read(h, buffer, 0, out.as_mut_ptr(), 8),
        avm_gpu::service::STALE
    );
    assert_eq!(avm_gpu::service::rs_gpu_close(h), avm_gpu::service::UNKNOWN_HANDLE);
    assert_eq!(
        avm_gpu::service::rs_gpu_poll(h, 987_654_321),
        avm_gpu::service::UNKNOWN_HANDLE
    );
}

#[test]
fn close_during_a_pending_job_is_refused() {
    let scratch = Scratch::new("close-pending");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 2);
    let job = avm_gpu::service::service_test_submit(h, buffer, 2, 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), avm_gpu::service::BUSY);
    assert_eq!(avm_gpu::service::rs_gpu_poll(h, job), avm_gpu::service::PENDING);
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(h, buffer), avm_gpu::service::BUSY);
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::COMPLETE, &[5, 6]),
        0
    );
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

#[test]
fn cancellation_keeps_the_buffer_quarantined_until_the_event() {
    let scratch = Scratch::new("cancel");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 4);
    let job = avm_gpu::service::service_test_submit(h, buffer, 4, 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_cancel(h, job, avm_gpu::service::CANCEL_DEVICE_WORK),
        0
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_cancel(h, job, 7),
        avm_gpu::service::INVALID_ARGUMENT
    );
    // The request is not a release: the job still owns the buffer.
    assert_eq!(avm_gpu::service::rs_gpu_poll(h, job), avm_gpu::service::PENDING);
    let mut out = vec![0u8; 16];
    assert_eq!(
        avm_gpu::service::rs_gpu_buffer_read(h, buffer, 0, out.as_mut_ptr(), 16),
        avm_gpu::service::BUSY
    );
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::CANCELLED, &[]),
        0
    );
    assert_eq!(avm_gpu::service::rs_gpu_poll(h, job), avm_gpu::service::CANCELLED);
    assert_eq!(
        avm_gpu::service::rs_gpu_cancel(h, job, avm_gpu::service::CANCEL_WAIT),
        avm_gpu::service::STALE
    );
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(h, buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

#[test]
fn a_timeout_is_terminal_and_returns_the_allocation() {
    let scratch = Scratch::new("timeout");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 3);
    let job = avm_gpu::service::service_test_submit(h, buffer, 3, 0);
    assert_eq!(
        avm_gpu::service::rs_gpu_cancel(h, job, avm_gpu::service::CANCEL_DEVICE_WORK),
        0
    );
    // A cancellation request is not a timeout: the job is still pending and
    // the buffer is still quarantined.
    assert_eq!(avm_gpu::service::rs_gpu_poll(h, job), avm_gpu::service::PENDING);
    assert_eq!(
        avm_gpu::service::rs_gpu_buffer_free(h, buffer),
        avm_gpu::service::BUSY
    );
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::TIMEOUT, &[]),
        0
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_poll(h, job),
        avm_gpu::service::TIMEOUT
    );
    let mut out = vec![0u8; 12];
    assert_eq!(
        avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 12),
        avm_gpu::service::TIMEOUT
    );
    // A late completion never overwrites the timeout.
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::COMPLETE, &[1, 2, 3]),
        avm_gpu::service::STALE
    );
    assert_ne!(
        avm_gpu::service::rs_gpu_poll(h, job),
        avm_gpu::service::COMPLETE
    );
    // The terminal failure releases ownership: the caller can free and close.
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(h, buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

// ---------------------------------------------------------------------------
// G15a: the host-state hazards, held at the exact window each race needs.
//
// `service_test_set_hook` installs a closure at a named point inside a
// critical section. The window below is a two-way handoff: the service thread
// announces it is inside the window, then waits until the other side reports
// it finished. The other side's report is the observation the test needs:
// with the window reopened it arrives immediately (the racer was not blocked),
// with the window held it cannot arrive until the held lock is released, so
// the wait ends by timeout instead. That makes each test fail on an assertion
// against the unfixed ordering and pass on observation, not on a sleep.
// ---------------------------------------------------------------------------

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use avm_gpu::service::{service_test_set_hook, TestPoint};

#[derive(Default)]
struct WindowState {
    entered: Mutex<bool>,
    done: Mutex<bool>,
    entered_cv: Condvar,
    done_cv: Condvar,
}

struct Window(Arc<WindowState>);

impl Window {
    fn new() -> Arc<Window> {
        Arc::new(Window(Arc::new(WindowState::default())))
    }

    fn hook(self: &Arc<Window>, timeout: Duration) -> Arc<dyn Fn() + Send + Sync> {
        let window = self.clone();
        Arc::new(move || {
            {
                let mut entered = window.0.entered.lock().unwrap();
                *entered = true;
                window.0.entered_cv.notify_all();
            }
            window.wait_done(timeout);
        })
    }

    /// Called by the thread on the other side of the window once it has an
    /// outcome to report.
    fn mark_done(&self) {
        *self.0.done.lock().unwrap() = true;
        self.0.done_cv.notify_all();
    }

    fn wait_entered(&self) {
        let mut entered = self.0.entered.lock().unwrap();
        while !*entered {
            let (guard, timeout) = self
                .0
                .entered_cv
                .wait_timeout(entered, Duration::from_secs(20))
                .unwrap();
            entered = guard;
            assert!(!timeout.timed_out(), "the service never reached the hook");
        }
    }

    fn wait_done(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut done: MutexGuard<'_, bool> = self.0.done.lock().unwrap();
        while !*done {
            let now = std::time::Instant::now();
            if now >= deadline {
                return *done;
            }
            let (guard, _) = self.0.done_cv.wait_timeout(done, deadline - now).unwrap();
            done = guard;
        }
        true
    }
}

fn clear_hook(point: TestPoint, identity: i64) {
    service_test_set_hook(point, identity, None);
}

fn words(bytes: Vec<u8>) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[test]
fn a_result_never_observes_bytes_the_completion_has_not_copied() {
    let scratch = Scratch::new("publish-window");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 4);
    // Pre-fill the output (before the job owns it) so a reader that wins the
    // window returns visible stale bytes instead of zeros that a broken
    // implementation might also produce by accident.
    assert_eq!(write_words(h, buffer, &[9, 9, 9, 9]), 0);
    let job = avm_gpu::service::service_test_submit(h, buffer, 4, 0);
    assert!(job > 0);

    let window = Window::new();
    service_test_set_hook(
        TestPoint::PublishBeforeCopy,
        job,
        Some(window.hook(Duration::from_secs(2))),
    );
    let publisher = std::thread::spawn(move || {
        avm_gpu::service::service_test_publish(
            job,
            avm_gpu::service::COMPLETE,
            &[11, 22, 33, 44],
        )
    });
    window.wait_entered();
    let reader = std::thread::spawn({
        let window = window.clone();
        move || {
            let mut out = vec![0u8; 16];
            let read = avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 16);
            window.mark_done();
            (read, out)
        }
    });
    let published = publisher.join().unwrap();
    let (read, bytes) = reader.join().unwrap();
    clear_hook(TestPoint::PublishBeforeCopy, job);

    assert_eq!(published, 0);
    assert_eq!(read, 16, "the receipt was not readable once published");
    assert_eq!(
        words(bytes),
        vec![11, 22, 33, 44],
        "the reader copied bytes from before the completion"
    );
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(h, buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

#[test]
fn one_receipt_is_claimed_by_exactly_one_reader() {
    let scratch = Scratch::new("receipt-claim");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 4);
    let job = avm_gpu::service::service_test_submit(h, buffer, 4, 0);
    assert_eq!(
        avm_gpu::service::service_test_publish(
            job,
            avm_gpu::service::COMPLETE,
            &[5, 6, 7, 8]
        ),
        0
    );

    let window = Window::new();
    service_test_set_hook(
        TestPoint::ResultBeforeCopy,
        job,
        Some(window.hook(Duration::from_secs(2))),
    );
    let first = std::thread::spawn({
        let window = window.clone();
        move || {
            let mut out = vec![0u8; 16];
            let read = avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 16);
            window.mark_done();
            (read, out)
        }
    });
    window.wait_entered();
    let second = std::thread::spawn({
        let window = window.clone();
        move || {
            let mut out = vec![0u8; 16];
            let read = avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 16);
            window.mark_done();
            (read, out)
        }
    });
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    clear_hook(TestPoint::ResultBeforeCopy, job);

    let mut reads = vec![first.0, second.0];
    reads.sort_by_key(|r| -(*r));
    assert_eq!(
        reads,
        vec![16, avm_gpu::service::STALE],
        "exactly one reader claims the receipt"
    );
    let claimed = if first.0 == 16 { first.1 } else { second.1 };
    assert_eq!(words(claimed), vec![5, 6, 7, 8]);
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(h, buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

#[test]
fn a_destination_too_small_leaves_the_receipt_unclaimed() {
    let scratch = Scratch::new("small-target");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 2);
    let job = avm_gpu::service::service_test_submit(h, buffer, 2, 0);
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::COMPLETE, &[1, 2]),
        0
    );
    let mut small = vec![0u8; 4];
    assert_eq!(
        avm_gpu::service::rs_gpu_result(h, job, small.as_mut_ptr(), 4),
        avm_gpu::service::LIMIT_REFUSED
    );
    // The refusal did not consume the receipt, so the same job is readable with
    // a large enough destination...
    let mut full = vec![0u8; 8];
    assert_eq!(avm_gpu::service::rs_gpu_result(h, job, full.as_mut_ptr(), 8), 8);
    assert_eq!(words(full), vec![1, 2]);
    // ...exactly once.
    assert_eq!(
        avm_gpu::service::rs_gpu_result(h, job, small.as_mut_ptr(), 8),
        avm_gpu::service::STALE
    );
    assert_eq!(avm_gpu::service::rs_gpu_buffer_free(h, buffer), 0);
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}

#[test]
fn a_submission_that_raced_a_close_refuses_and_starts_no_worker() {
    let scratch = Scratch::new("submit-close");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 4);

    let window = Window::new();
    service_test_set_hook(
        TestPoint::SubmitBeforeAdmission,
        h,
        Some(window.hook(Duration::from_secs(20))),
    );
    let submitter =
        std::thread::spawn(move || avm_gpu::service::rs_gpu_submit(h, buffer, 4, 0));
    window.wait_entered();
    // The submission has resolved the context and its buffer but has not been
    // admitted, so nobody holds the context: the close succeeds here.
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
    window.mark_done();
    let job = submitter.join().unwrap();
    clear_hook(TestPoint::SubmitBeforeAdmission, h);

    assert_eq!(
        job,
        avm_gpu::service::UNKNOWN_HANDLE,
        "a submission whose context closed under it must refuse"
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_poll(h, job),
        avm_gpu::service::UNKNOWN_HANDLE
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_close(h),
        avm_gpu::service::UNKNOWN_HANDLE
    );
}

#[test]
fn a_mock_loss_is_never_a_completion() {
    let scratch = Scratch::new("loss");
    write_add_bundle(&scratch.0);
    let h = open(&scratch.0, 0);
    let buffer = avm_gpu::service::rs_gpu_buffer_new(h, 2);
    let job = avm_gpu::service::service_test_submit(h, buffer, 2, 0);
    assert_eq!(
        avm_gpu::service::service_test_publish(job, avm_gpu::service::DEVICE_LOST, &[]),
        0
    );
    assert_eq!(
        avm_gpu::service::rs_gpu_poll(h, job),
        avm_gpu::service::DEVICE_LOST
    );
    let mut out = vec![0u8; 8];
    assert_eq!(
        avm_gpu::service::rs_gpu_result(h, job, out.as_mut_ptr(), 8),
        avm_gpu::service::DEVICE_LOST
    );
    assert_ne!(
        avm_gpu::service::rs_gpu_poll(h, job),
        avm_gpu::service::COMPLETE
    );
    assert_eq!(avm_gpu::service::rs_gpu_close(h), 0);
}
