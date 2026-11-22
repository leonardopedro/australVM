//! G14/G15: a narrow, versioned GPU service contract for manifest-gated modules.
//!
//! The exports use only opaque `Int64` identities and explicit lengths. No
//! wgpu type, Rust layout or device pointer crosses the ABI. The adapter is a
//! normal `rs_*` foreign namespace, so the bridge's `check_call_permission`
//! gates every call from the module manifest; there is no privileged `au_*`
//! shortcut here.
//!
//! Status convention: non-negative returns are results, negative returns are
//! typed refusals. A handle is always positive; it is opaque and becomes stale
//! after `rs_gpu_close`.
//!
//! ```text
//!   -1  invalid argument (null/oversized path, bad boolean, bad selector)
//!   -2  bundle refused (load/validation/digest failure)
//!   -3  unknown or closed handle
//!   -4  request outside the v1 element/limit contract
//!   -5  device present but the request was refused
//!   -6  no physical adapter is available for a required-hardware request
//!       (a CPU-only build also uses this code for a required-hardware run)
//!   -7  another job is still live on this context
//!   -8  the identity belongs to an older context generation or was
//!       already consumed (a receipt is single-use)
//!   -9  the device was lost; never reported as a completion
//!  -10  a cancellation/receipt deadline expired while still quarantined
//! ```
//!
//! Lock order (G15a) is part of the contract because two state locks are
//! sometimes needed at once: a **job** (never while holding a buffer), then the
//! **buffers** map, then a **buffer** state. A context (**entry**) lock is taken
//! alone; `job_pending` is queried after a buffer state is released, and
//! `rs_gpu_close` releases the entry lock before removing the handle. Publishing
//! a terminal event and copying its words are one critical section under the
//! job lock, so a receipt reader (which takes that same lock) never observes
//! `COMPLETE` with the previous bytes still in the buffer, and two readers
//! cannot both claim the single receipt.
//!
//! v2 (`SERVICE_VERSION = 2`) adds buffers and asynchronous jobs. v1
//! (`rs_gpu_run`) keeps its v1 meaning. Completion is published by the worker
//! that ran the request — on the device path from the queue's
//! submitted-work-done callback, in tests from an explicit mock event; a poll
//! never manufactures a completion. The worker owns the context's `Arc`, so
//! closing while a job is live is refused rather than freeing memory the
//! worker may still touch, and a close joins finished workers so the device is
//! dropped before the context reports closed.
//!
//! The service does not claim an OS sandbox. It is trusted native code that a
//! deployment loads only when the project authorizes loading the provider.

use crate::schema::MAX_ELEMENTS;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const SERVICE_VERSION: i64 = 2;

pub const INVALID_ARGUMENT: i64 = -1;
pub const BUNDLE_REFUSED: i64 = -2;
pub const UNKNOWN_HANDLE: i64 = -3;
pub const LIMIT_REFUSED: i64 = -4;
pub const DEVICE_REFUSED: i64 = -5;
pub const DEVICE_UNAVAILABLE: i64 = -6;
pub const BUSY: i64 = -7;
pub const STALE: i64 = -8;
pub const DEVICE_LOST: i64 = -9;
pub const TIMEOUT: i64 = -10;

/// `rs_gpu_poll` observations. These are not checksums: a job's completion is
/// the worker's published event.
pub const PENDING: i64 = 0;
pub const COMPLETE: i64 = 1;
pub const CANCELLED: i64 = 2;

/// Cancellation modes: wait for the terminal event, or ask the adapter to drop
/// the work. Neither mode frees the buffer; only a terminal event releases the
/// job's ownership of it.
pub const CANCEL_WAIT: i64 = 0;
pub const CANCEL_DEVICE_WORK: i64 = 1;

/// The largest bundle path this contract accepts. The host supplies the
/// bytes; the service copies them immediately and does not retain the pointer.
pub const MAX_PATH_BYTES: usize = 4096;

const ELEMENT_BYTES: i64 = 4;

struct Entry {
    loaded: crate::LoadedBundle,
    require_hardware: bool,
    adapter_index: Option<usize>,
    /// Changes when a context is created; a job from another generation is
    /// `STALE` even if its numeric handle is alive.
    generation: i64,
    next_sequence: i64,
    live_job: Option<i64>,
    /// Set under the entry lock by `rs_gpu_close` before the handle leaves the
    /// registry. A submission that already holds the entry therefore refuses
    /// instead of starting a worker for a context that is closing.
    closed: bool,
    /// Worker threads that ran this context's jobs. A completion is published
    /// from the device's work-done callback, so the job can be terminal while
    /// its worker still owns the device; `rs_gpu_close` joins these before it
    /// returns so teardown never overlaps a live device thread.
    workers: Vec<std::thread::JoinHandle<()>>,
}

type Contexts = Mutex<HashMap<i64, Arc<Mutex<Entry>>>>;
type Buffers = Mutex<HashMap<i64, Arc<Mutex<BufferState>>>>;
type Jobs = Mutex<HashMap<i64, Arc<Mutex<Job>>>>;

fn contexts() -> &'static Contexts {
    static REGISTRY: OnceLock<Contexts> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn buffers() -> &'static Buffers {
    static REGISTRY: OnceLock<Buffers> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn jobs() -> &'static Jobs {
    static REGISTRY: OnceLock<Jobs> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn next_generation() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Test-only handoff points.
//
// Each sits inside the critical section whose ordering the G15a tests pin, so a
// test can hold a publisher, a receipt reader or a submitter at exactly the
// window a race would need and assert what the other side observes. The hooks
// are keyed by job (publish/result) or context (submit) identity, so a test's
// hook cannot fire inside another test's job; nothing here is exported as a
// symbol.
// ---------------------------------------------------------------------------

/// A test-only handoff point inside a service critical section.
#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TestPoint {
    /// `publish` holds the job lock and has not copied the words or set the
    /// terminal state yet.
    PublishBeforeCopy,
    /// `rs_gpu_result` holds the job lock, has checked the receipt, and has not
    /// written the caller's destination yet.
    ResultBeforeCopy,
    /// `begin_job` has a live context and a validated output buffer, and has not
    /// admitted the job against the context yet.
    SubmitBeforeAdmission,
}

type TestHook = Arc<dyn Fn() + Send + Sync>;

fn test_hooks() -> &'static Mutex<Vec<(TestPoint, i64, TestHook)>> {
    static HOOKS: OnceLock<Mutex<Vec<(TestPoint, i64, TestHook)>>> = OnceLock::new();
    HOOKS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Install (`Some`) or remove (`None`) the hook called at `point` for
/// `identity` — the job handle for the publish/result points, the context
/// handle for the submit point. Test-only and not an exported symbol.
#[doc(hidden)]
pub fn service_test_set_hook(point: TestPoint, identity: i64, hook: Option<TestHook>) {
    if let Ok(mut hooks) = test_hooks().lock() {
        hooks.retain(|(p, id, _)| !(*p == point && *id == identity));
        if let Some(hook) = hook {
            hooks.push((point, identity, hook));
        }
    }
}

fn run_test_hook(point: TestPoint, identity: i64) {
    // Clone out from under the registry lock: a hook blocks by design, and it
    // must not hold the registry while another thread installs a hook.
    let hook = test_hooks().lock().ok().and_then(|hooks| {
        hooks
            .iter()
            .find(|(p, id, _)| *p == point && *id == identity)
            .map(|(_, _, hook)| hook.clone())
    });
    if let Some(hook) = hook {
        hook();
    }
}

fn lookup(handle: i64) -> Result<Arc<Mutex<Entry>>, i64> {
    contexts()
        .lock()
        .map_err(|_| UNKNOWN_HANDLE)?
        .get(&handle)
        .cloned()
        .ok_or(UNKNOWN_HANDLE)
}

fn bundle_path() -> Result<String, i64> {
    let path = std::env::var_os("AVM_GPU_BUNDLE").ok_or(BUNDLE_REFUSED)?;
    let text = path.to_str().ok_or(INVALID_ARGUMENT)?;
    if text.is_empty() || text.len() > MAX_PATH_BYTES || text.as_bytes().contains(&0) {
        return Err(INVALID_ARGUMENT);
    }
    Ok(text.to_string())
}

fn copy_path(ptr: *const u8, len: i64) -> Result<String, i64> {
    if ptr.is_null() || len <= 0 || len as usize > MAX_PATH_BYTES {
        return Err(INVALID_ARGUMENT);
    }
    // SAFETY: the caller promises `len` readable bytes at `ptr` for this call.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    let text = std::str::from_utf8(bytes).map_err(|_| INVALID_ARGUMENT)?;
    if text.as_bytes().contains(&0) {
        return Err(INVALID_ARGUMENT);
    }
    Ok(text.to_string())
}

fn open_path(path: &str, require_hardware: i64, adapter_index: i64) -> i64 {
    if require_hardware != 0 && require_hardware != 1 {
        return INVALID_ARGUMENT;
    }
    if adapter_index < -1 {
        return INVALID_ARGUMENT;
    }
    let loaded = match crate::load_bundle(std::path::Path::new(path)) {
        Ok(b) => b,
        Err(_) => return BUNDLE_REFUSED,
    };
    if crate::validate::validate_bundle(&loaded).is_err() {
        return BUNDLE_REFUSED;
    }
    // The service synthesises its own inputs as ``seed + i``, which is a u32
    // contract. An f32 bundle still validates, but those words would be
    // bit patterns rather than the values a caller expects, so refuse it by
    // name instead of running a silently wrong job.
    if loaded.bundle.kernel_ir.numeric_profile != crate::schema::NUMERIC_PROFILE {
        return BUNDLE_REFUSED;
    }
    let handle = next_handle();
    let entry = Arc::new(Mutex::new(Entry {
        loaded,
        require_hardware: require_hardware != 0,
        adapter_index: if adapter_index < 0 {
            None
        } else {
            Some(adapter_index as usize)
        },
        generation: next_generation(),
        next_sequence: 0,
        live_job: None,
        closed: false,
        workers: Vec::new(),
    }));
    match contexts().lock() {
        Ok(mut map) => {
            map.insert(handle, entry);
            handle
        }
        Err(_) => UNKNOWN_HANDLE,
    }
}

/// Open the deployment-selected bundle. The path is **not** a module argument:
/// it comes from `AVM_GPU_BUNDLE`, so a module cannot ask the service to open
/// an arbitrary file. The environment value is length-checked before
/// `load_bundle` sees it.
#[no_mangle]
pub extern "C" fn rs_gpu_open(require_hardware: i64, adapter_index: i64) -> i64 {
    if require_hardware != 0 && require_hardware != 1 {
        return INVALID_ARGUMENT;
    }
    if adapter_index < -1 {
        return INVALID_ARGUMENT;
    }
    match bundle_path() {
        Ok(path) => open_path(&path, require_hardware, adapter_index),
        Err(code) => code,
    }
}

/// Explicit-length path variant for trusted deployment code and tests. The
/// module-facing entry point is `rs_gpu_open` so a module cannot choose a path.
#[no_mangle]
pub extern "C" fn rs_gpu_open_at(
    path: *const u8,
    path_len: i64,
    require_hardware: i64,
    adapter_index: i64,
) -> i64 {
    match copy_path(path, path_len) {
        Ok(path) => open_path(&path, require_hardware, adapter_index),
        Err(code) => code,
    }
}

/// Run one deterministic request and return a checksum of the output.
///
/// The request is `(n, seed)`: the service synthesises the bundle's input
/// arrays (`input0[i] = seed + i`, `input1[i] = i`) so no byte buffer or
/// device pointer crosses the ABI. The returned value is the wrapping sum of
/// the output words, masked to 63 bits; negative values are refusals.
#[no_mangle]
pub extern "C" fn rs_gpu_run(handle: i64, n: i64, seed: i64) -> i64 {
    if n < 0 || n as u64 > MAX_ELEMENTS as u64 {
        return LIMIT_REFUSED;
    }
    let entry = match lookup(handle) {
        Ok(e) => e,
        Err(code) => return code,
    };
    let (loaded, require_hardware, adapter_index) = {
        let e = match entry.lock() {
            Ok(e) => e,
            Err(_) => return UNKNOWN_HANDLE,
        };
        (
            e.loaded.clone_for_service(),
            e.require_hardware,
            e.adapter_index,
        )
    };
    let n = n as usize;
    let seed = seed as i64 as u32;
    let inputs = synth_inputs(loaded.bundle.input_arity as usize, n, seed);
    let output = execute(&loaded, &inputs, require_hardware, adapter_index);
    match output {
        Ok(out) => checksum(&out),
        Err(code) => code,
    }
}

fn synth_inputs(arity: usize, n: usize, seed: u32) -> Vec<Vec<u32>> {
    (0..arity)
        .map(|i| {
            (0..n)
                .map(|j| {
                    if i == 0 {
                        seed.wrapping_add(j as u32)
                    } else {
                        j as u32
                    }
                })
                .collect()
        })
        .collect()
}

fn checksum(out: &[u32]) -> i64 {
    if out.is_empty() {
        0
    } else {
        let sum = out.iter().fold(0u64, |a, b| a.wrapping_add(*b as u64));
        (sum & 0x7fff_ffff_ffff_ffff) as i64
    }
}

fn execute(
    loaded: &crate::LoadedBundle,
    inputs: &[Vec<u32>],
    require_hardware: bool,
    adapter_index: Option<usize>,
) -> Result<Vec<u32>, i64> {
    #[cfg(feature = "device")]
    {
        let selection = crate::device::Selection {
            name: None,
            index: adapter_index,
        };
        match crate::run::run(loaded, inputs, require_hardware, &selection) {
            Ok(out) => return Ok(out.output),
            Err(crate::run::RunFailure::Unavailable(_)) if !require_hardware => {
                // A CPU-only deployment may still use the reference path; a
                // required-hardware request never falls back.
            }
            Err(crate::run::RunFailure::Unavailable(_)) => return Err(DEVICE_UNAVAILABLE),
            Err(crate::run::RunFailure::Refused(_)) => return Err(DEVICE_REFUSED),
        }
    }
    #[cfg(not(feature = "device"))]
    let _ = adapter_index;
    if require_hardware {
        return Err(DEVICE_UNAVAILABLE);
    }
    let mut evaluator = crate::reference::Evaluator::new(
        &loaded.bundle.kernel_ir,
        crate::reference::Limits::default(),
    )
    .map_err(|_| BUNDLE_REFUSED)?;
    evaluator.run(inputs).map_err(|_| LIMIT_REFUSED)
}

struct BufferState {
    context: i64,
    generation: i64,
    bytes: Vec<u8>,
    /// The submitted job that currently owns this buffer, if any. While a job
    /// owns it, reads/writes/frees are refused; the linear Austral token is
    /// what makes the ownership move a source-level rule, and this is the
    /// runtime backstop for forged calls.
    owner: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobState {
    Pending,
    Complete,
    Cancelled,
    Failed(i64),
}

struct Job {
    context: i64,
    generation: i64,
    /// Monotonic per-context identity. The handle is the lookup key; the
    /// sequence is recorded so a registry can tell a reused number from a
    /// new job without trusting the caller's token.
    #[allow(dead_code)]
    sequence: i64,
    output: i64,
    bytes: usize,
    state: JobState,
    result_taken: bool,
    cancel_requested: Option<i64>,
}

/// Create an output buffer of `elements` u32 words.
#[no_mangle]
pub extern "C" fn rs_gpu_buffer_new(context: i64, elements: i64) -> i64 {
    if elements < 0 || elements as u64 > MAX_ELEMENTS as u64 {
        return LIMIT_REFUSED;
    }
    let entry = match lookup(context) {
        Ok(e) => e,
        Err(code) => return code,
    };
    let generation = match entry.lock() {
        Ok(e) => e.generation,
        Err(_) => return UNKNOWN_HANDLE,
    };
    let handle = next_handle();
    let bytes = vec![0u8; elements as usize * ELEMENT_BYTES as usize];
    let state = Arc::new(Mutex::new(BufferState {
        context,
        generation,
        bytes,
        owner: None,
    }));
    match buffers().lock() {
        Ok(mut map) => {
            map.insert(handle, state);
            handle
        }
        Err(_) => UNKNOWN_HANDLE,
    }
}

fn buffer_for(context: i64, buffer: i64) -> Result<Arc<Mutex<BufferState>>, i64> {
    let state = buffers()
        .lock()
        .map_err(|_| UNKNOWN_HANDLE)?
        .get(&buffer)
        .cloned()
        .ok_or(UNKNOWN_HANDLE)?;
    let (owner_context, generation, owner) = {
        let b = state.lock().map_err(|_| UNKNOWN_HANDLE)?;
        (b.context, b.generation, b.owner)
    };
    if owner_context != context {
        return Err(UNKNOWN_HANDLE);
    }
    match lookup(context) {
        Ok(entry) => {
            let current = entry.lock().map_err(|_| UNKNOWN_HANDLE)?.generation;
            if current != generation {
                return Err(STALE);
            }
        }
        Err(_) => return Err(STALE),
    }
    if let Some(job) = owner {
        if job_pending(job) {
            return Err(BUSY);
        }
    }
    Ok(state)
}

fn job_pending(job: i64) -> bool {
    jobs()
        .lock()
        .ok()
        .and_then(|map| map.get(&job).cloned())
        .and_then(|j| j.lock().ok().map(|j| j.state == JobState::Pending))
        .unwrap_or(false)
}

/// Copy `len` bytes at `offset_bytes` into the buffer. The pointer is read for
/// this call only and never retained.
#[no_mangle]
pub extern "C" fn rs_gpu_buffer_write(
    context: i64,
    buffer: i64,
    offset_bytes: i64,
    ptr: *const u8,
    len: i64,
) -> i64 {
    if len < 0 || (len > 0 && ptr.is_null()) {
        return INVALID_ARGUMENT;
    }
    let state = match buffer_for(context, buffer) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let mut b = match state.lock() {
        Ok(b) => b,
        Err(_) => return UNKNOWN_HANDLE,
    };
    if offset_bytes < 0 || offset_bytes + len > b.bytes.len() as i64 {
        return LIMIT_REFUSED;
    }
    if len == 0 {
        return 0;
    }
    let src = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    b.bytes[offset_bytes as usize..(offset_bytes + len) as usize].copy_from_slice(src);
    0
}

/// Copy `cap` bytes at `offset_bytes` out of the buffer. `cap` may be zero
/// only when no pointer is required.
#[no_mangle]
pub extern "C" fn rs_gpu_buffer_read(
    context: i64,
    buffer: i64,
    offset_bytes: i64,
    ptr: *mut u8,
    cap: i64,
) -> i64 {
    if cap < 0 || (cap > 0 && ptr.is_null()) {
        return INVALID_ARGUMENT;
    }
    let state = match buffer_for(context, buffer) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let b = match state.lock() {
        Ok(b) => b,
        Err(_) => return UNKNOWN_HANDLE,
    };
    if offset_bytes < 0 || offset_bytes + cap > b.bytes.len() as i64 {
        return LIMIT_REFUSED;
    }
    if cap == 0 {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(
            b.bytes[offset_bytes as usize..].as_ptr(),
            ptr,
            cap as usize,
        );
    }
    cap
}

/// Release a buffer. A buffer a live job owns is `BUSY` until that job reaches
/// a terminal state, so a close/free cannot race the worker.
#[no_mangle]
pub extern "C" fn rs_gpu_buffer_free(context: i64, buffer: i64) -> i64 {
    let state = match buffer_for(context, buffer) {
        Ok(s) => s,
        Err(code) => return code,
    };
    match buffers().lock() {
        Ok(mut map) => {
            if map.remove(&buffer).is_some() {
                0
            } else {
                UNKNOWN_HANDLE
            }
        }
        Err(_) => {
            drop(state);
            UNKNOWN_HANDLE
        }
    }
}

/// Insert a pending job and (unless `hold` is set) start the worker.
fn begin_job(context: i64, output: i64, n: i64, seed: i64, hold: bool) -> i64 {
    if n < 0 || n as u64 > MAX_ELEMENTS as u64 {
        return LIMIT_REFUSED;
    }
    let entry = match lookup(context) {
        Ok(e) => e,
        Err(code) => return code,
    };
    let state = match buffer_for(context, output) {
        Ok(s) => s,
        Err(code) => return code,
    };
    run_test_hook(TestPoint::SubmitBeforeAdmission, context);
    let mut e = match entry.lock() {
        Ok(e) => e,
        Err(_) => return UNKNOWN_HANDLE,
    };
    // `rs_gpu_close` sets this under the same lock, before the handle leaves
    // the registry, so a submission that already resolved the entry cannot
    // start a worker for a context that is closing.
    if e.closed {
        return UNKNOWN_HANDLE;
    }
    if let Some(live) = e.live_job {
        if job_pending(live) {
            return BUSY;
        }
    }
    let bytes = n as usize * ELEMENT_BYTES as usize;
    {
        let b = match state.lock() {
            Ok(b) => b,
            Err(_) => return UNKNOWN_HANDLE,
        };
        if b.bytes.len() != bytes {
            return INVALID_ARGUMENT;
        }
    }
    let job_id = next_handle();
    let sequence = e.next_sequence;
    e.next_sequence += 1;
    e.live_job = Some(job_id);
    let job = Job {
        context,
        generation: e.generation,
        sequence,
        output,
        bytes,
        state: JobState::Pending,
        result_taken: false,
        cancel_requested: None,
    };
    if jobs()
        .lock()
        .map(|mut map| map.insert(job_id, Arc::new(Mutex::new(job))))
        .is_err()
    {
        return UNKNOWN_HANDLE;
    }
    if let Ok(mut b) = state.lock() {
        b.owner = Some(job_id);
    }
    if !hold {
        let loaded = e.loaded.clone_for_service();
        let require_hardware = e.require_hardware;
        let adapter_index = e.adapter_index;
        // Spawn and register under the context lock. `rs_gpu_close` takes the
        // worker list under that same lock, so this worker is either joined by
        // the close or the close was refused while its job was pending; it
        // cannot be pushed after the close has already returned.
        let worker = std::thread::spawn(move || {
            if cancel_requested(job_id) {
                publish(job_id, JobState::Cancelled, &[]);
                return;
            }
            let arity = loaded.bundle.input_arity as usize;
            let inputs = synth_inputs(arity, n as usize, seed as i64 as u32);
            execute_job(job_id, &loaded, &inputs, require_hardware, adapter_index);
        });
        e.workers.push(worker);
    }
    job_id
}

/// Run a submitted job and publish its terminal event.
///
/// Under the device feature the completion is published from the queue's
/// submitted-work-done callback, so it is tied to the runtime event that says
/// the dispatch and its staging copy finished — not to the worker returning.
/// The `Ok` arm below still publishes, which covers a zero-work submission
/// (no callback fires) and is a no-op when the callback already published.
fn execute_job(
    job: i64,
    loaded: &crate::LoadedBundle,
    inputs: &[Vec<u32>],
    require_hardware: bool,
    adapter_index: Option<usize>,
) {
    #[cfg(feature = "device")]
    {
        let selection = crate::device::Selection {
            name: None,
            index: adapter_index,
        };
        match crate::run::run_with_completion(loaded, inputs, require_hardware, &selection, move |words| {
            publish(job, JobState::Complete, words);
        }) {
            Ok(out) => {
                publish(job, JobState::Complete, &out.output);
                return;
            }
            Err(crate::run::RunFailure::Unavailable(_)) if !require_hardware => {
                // Optional mode may still use the reference path below.
            }
            Err(crate::run::RunFailure::Unavailable(_)) => {
                publish(job, JobState::Failed(DEVICE_UNAVAILABLE), &[]);
                return;
            }
            Err(crate::run::RunFailure::Refused(_)) => {
                publish(job, JobState::Failed(DEVICE_REFUSED), &[]);
                return;
            }
        }
    }
    #[cfg(not(feature = "device"))]
    let _ = adapter_index;
    if require_hardware {
        publish(job, JobState::Failed(DEVICE_UNAVAILABLE), &[]);
        return;
    }
    let out = crate::reference::Evaluator::new(
        &loaded.bundle.kernel_ir,
        crate::reference::Limits::default(),
    )
    .map_err(|_| BUNDLE_REFUSED)
    .and_then(|mut evaluator| evaluator.run(inputs).map_err(|_| LIMIT_REFUSED));
    match out {
        Ok(words) => {
            publish(job, JobState::Complete, &words);
        }
        Err(code) => {
            publish(job, JobState::Failed(code), &[]);
        }
    }
}

fn cancel_requested(job: i64) -> bool {
    jobs()
        .lock()
        .ok()
        .and_then(|map| map.get(&job).cloned())
        .and_then(|j| j.lock().ok().map(|j| j.cancel_requested.is_some()))
        .unwrap_or(false)
}

/// Publish the single terminal event for a job. Later or reordered events are
/// ignored: the first terminal state is the one owner, and a completion is
/// never overwritten by a cancellation or a loss.
fn publish(job_id: i64, state: JobState, output: &[u32]) -> bool {
    let job = match jobs().lock().ok().and_then(|m| m.get(&job_id).cloned()) {
        Some(j) => j,
        None => return false,
    };
    let mut j = match job.lock() {
        Ok(j) => j,
        Err(_) => return false,
    };
    if j.state != JobState::Pending {
        return false;
    }
    // The words and the terminal transition are one critical section (G15a):
    // a receipt reader takes this same lock before copying, so it can never
    // observe `COMPLETE` while the buffer still holds the previous bytes.
    // Lock order: job, then buffers map, then buffer state.
    run_test_hook(TestPoint::PublishBeforeCopy, job_id);
    let output_handle = j.output;
    let buffer = buffers().lock().ok().and_then(|m| m.get(&output_handle).cloned());
    if let Some(buffer) = buffer {
        if let Ok(mut b) = buffer.lock() {
            if state == JobState::Complete {
                let written = output.len().min(j.bytes / ELEMENT_BYTES as usize);
                for (i, word) in output.iter().take(written).enumerate() {
                    let start = i * ELEMENT_BYTES as usize;
                    b.bytes[start..start + ELEMENT_BYTES as usize]
                        .copy_from_slice(&word.to_le_bytes());
                }
            }
            b.owner = None;
        }
    }
    j.state = state;
    true
}

fn lookup_job(context: i64, job: i64) -> Result<Arc<Mutex<Job>>, i64> {
    let job = jobs()
        .lock()
        .map_err(|_| UNKNOWN_HANDLE)?
        .get(&job)
        .cloned()
        .ok_or(UNKNOWN_HANDLE)?;
    let (owner, generation) = {
        let j = job.lock().map_err(|_| UNKNOWN_HANDLE)?;
        (j.context, j.generation)
    };
    if owner != context {
        return Err(UNKNOWN_HANDLE);
    }
    match lookup(context) {
        Ok(entry) => {
            let current = entry.lock().map_err(|_| UNKNOWN_HANDLE)?.generation;
            if current != generation {
                Err(STALE)
            } else {
                Ok(job)
            }
        }
        // The context was closed: the job is from an older generation. The
        // completion is quarantined with the job record, never silently
        // re-attributed to a new context.
        Err(_) => Err(STALE),
    }
}

/// Submit one request against `output`. The buffer must be `n * 4` bytes and
/// becomes owned by the job until a terminal event. Returns a job identity or
/// a negative status.
#[no_mangle]
pub extern "C" fn rs_gpu_submit(context: i64, output: i64, n: i64, seed: i64) -> i64 {
    begin_job(context, output, n, seed, false)
}

/// Create a pending job without a worker. Test-only lifecycle control: the
/// event is published explicitly by [`service_test_publish`]. This function is
/// not exported as a symbol and cannot be reached from a module.
#[doc(hidden)]
pub fn service_test_submit(context: i64, output: i64, n: i64, seed: i64) -> i64 {
    begin_job(context, output, n, seed, true)
}

/// Publish a terminal event for a job created by [`service_test_submit`].
/// `state` is `COMPLETE`, `CANCELLED`, or a negative error status. Returns 0
/// when this call is the single owner of the event, `STALE` when a terminal
/// event already exists.
#[doc(hidden)]
pub fn service_test_publish(job: i64, state: i64, output: &[u32]) -> i64 {
    let terminal = if state == COMPLETE {
        JobState::Complete
    } else if state == CANCELLED {
        JobState::Cancelled
    } else if state < 0 {
        JobState::Failed(state)
    } else {
        return INVALID_ARGUMENT;
    };
    if publish(job, terminal, output) {
        0
    } else {
        STALE
    }
}

/// Observe a job without consuming it.
#[no_mangle]
pub extern "C" fn rs_gpu_poll(context: i64, job: i64) -> i64 {
    let job = match lookup_job(context, job) {
        Ok(j) => j,
        Err(code) => return code,
    };
    let state = match job.lock() {
        Ok(j) => j.state,
        Err(_) => return UNKNOWN_HANDLE,
    };
    match state {
        JobState::Pending => PENDING,
        JobState::Complete => COMPLETE,
        JobState::Cancelled => CANCELLED,
        JobState::Failed(code) => code,
    }
}

/// Copy a completed job's result once. A second call is `STALE`: the receipt
/// is consumed, and the linear Austral token is what keeps that a source rule.
#[no_mangle]
pub extern "C" fn rs_gpu_result(
    context: i64,
    job_id: i64,
    out_ptr: *mut u8,
    cap: i64,
) -> i64 {
    if cap < 0 || (cap > 0 && out_ptr.is_null()) {
        return INVALID_ARGUMENT;
    }
    let job = match lookup_job(context, job_id) {
        Ok(j) => j,
        Err(code) => return code,
    };
    // Checking the receipt, copying the words and consuming the receipt are one
    // critical section (G15a). Splitting them let two readers both see an
    // unclaimed receipt, and let a reader observe `COMPLETE` before the
    // publisher had copied the words.
    let mut j = match job.lock() {
        Ok(j) => j,
        Err(_) => return UNKNOWN_HANDLE,
    };
    match j.state {
        JobState::Pending => return BUSY,
        JobState::Cancelled => return STALE,
        JobState::Failed(code) => return code,
        JobState::Complete => {}
    }
    if j.result_taken {
        return STALE;
    }
    let bytes = j.bytes;
    // Too small a destination is retryable: it does not consume the receipt.
    if cap < bytes as i64 {
        return LIMIT_REFUSED;
    }
    run_test_hook(TestPoint::ResultBeforeCopy, job_id);
    let buffer = buffers().lock().ok().and_then(|m| m.get(&j.output).cloned());
    let buffer = match buffer {
        Some(b) => b,
        None => return STALE,
    };
    {
        let b = match buffer.lock() {
            Ok(b) => b,
            Err(_) => return UNKNOWN_HANDLE,
        };
        if bytes > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(b.bytes.as_ptr(), out_ptr, bytes);
            }
        }
    }
    j.result_taken = true;
    bytes as i64
}

/// Request cancellation. The request is recorded but the job stays `PENDING`
/// until a terminal event exists; neither mode frees the buffer.
#[no_mangle]
pub extern "C" fn rs_gpu_cancel(context: i64, job: i64, mode: i64) -> i64 {
    if mode != CANCEL_WAIT && mode != CANCEL_DEVICE_WORK {
        return INVALID_ARGUMENT;
    }
    let job = match lookup_job(context, job) {
        Ok(j) => j,
        Err(code) => return code,
    };
    let outcome = match job.lock() {
        Ok(mut j) => {
            if j.state != JobState::Pending {
                return STALE;
            }
            j.cancel_requested = Some(mode);
            0
        }
        Err(_) => UNKNOWN_HANDLE,
    };
    outcome
}

/// Close a handle. Idempotent only for the failure code: a second close returns
/// `UNKNOWN_HANDLE`, so a stale identity cannot be mistaken for a live one.
/// Closing while a job is pending is refused (`BUSY`): the worker still holds
/// the context and its buffers, so freeing them would race the device.
#[no_mangle]
pub extern "C" fn rs_gpu_close(handle: i64) -> i64 {
    let entry = match lookup(handle) {
        Ok(e) => e,
        Err(code) => return code,
    };
    // Take the worker handles under the same lock that refuses a pending job,
    // then join them after the context is unreachable: a worker whose job is
    // already terminal may still be finishing the device call that published
    // it, and the device must be dropped before this call returns.
    let workers = {
        let mut e = match entry.lock() {
            Ok(e) => e,
            Err(_) => return UNKNOWN_HANDLE,
        };
        if let Some(live) = e.live_job {
            if job_pending(live) {
                return BUSY;
            }
        }
        // Marked before the handle leaves the registry: a submission holding
        // this entry then refuses instead of starting a worker (G15a).
        e.closed = true;
        std::mem::take(&mut e.workers)
    };
    match contexts().lock() {
        Ok(mut map) => {
            if map.remove(&handle).is_none() {
                return UNKNOWN_HANDLE;
            }
        }
        Err(_) => return UNKNOWN_HANDLE,
    }
    // The context is no longer reachable, so its buffer payloads are
    // released. The records stay (with their generation) so a number that
    // belonged to this context reports `STALE` rather than being mistaken for
    // an unknown handle. A job that was pending would have been refused above;
    // its record also stays and reports `STALE` from now on.
    if let Ok(map) = buffers().lock() {
        for buffer in map.values() {
            if let Ok(mut b) = buffer.lock() {
                if b.context == handle {
                    b.bytes = Vec::new();
                    b.owner = None;
                }
            }
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    0
}

/// Version of the contract this provider implements. A module can call it
/// before opening anything; the value is compared by the deployment, not
/// silently accepted as a capability.
#[no_mangle]
pub extern "C" fn rs_gpu_service_version() -> i64 {
    SERVICE_VERSION
}

impl crate::LoadedBundle {
    /// Service handles share a validated bundle; loading is immutable, so the
    /// clone is a `PathBuf`/`String` copy, not a second device allocation.
    fn clone_for_service(&self) -> crate::LoadedBundle {
        crate::LoadedBundle {
            dir: self.dir.clone(),
            bundle: self.bundle.clone(),
            wgsl: self.wgsl.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_two() {
        assert_eq!(rs_gpu_service_version(), 2);
    }

    #[test]
    fn bad_arguments_are_refused_before_loading() {
        assert_eq!(rs_gpu_open(2, -1), INVALID_ARGUMENT);
        assert_eq!(rs_gpu_open(0, -2), INVALID_ARGUMENT);
        let long = vec![b'x'; MAX_PATH_BYTES + 1];
        assert_eq!(
            rs_gpu_open_at(long.as_ptr(), long.len() as i64, 0, -1),
            INVALID_ARGUMENT
        );
    }

    #[test]
    fn missing_bundle_is_refused() {
        let path = b"/tmp/avm-g14-definitely-not-a-bundle";
        assert_eq!(
            rs_gpu_open_at(path.as_ptr(), path.len() as i64, 0, -1),
            BUNDLE_REFUSED
        );
    }

    #[test]
    fn unknown_and_closed_handles_are_refused() {
        assert_eq!(rs_gpu_run(999_999, 1, 0), UNKNOWN_HANDLE);
        assert_eq!(rs_gpu_close(999_999), UNKNOWN_HANDLE);
        assert_eq!(rs_gpu_buffer_new(999_999, 1), UNKNOWN_HANDLE);
        assert_eq!(rs_gpu_poll(999_999, 999_999), UNKNOWN_HANDLE);
        assert_eq!(rs_gpu_cancel(999_999, 999_999, CANCEL_WAIT), UNKNOWN_HANDLE);
    }

    #[test]
    fn oversized_requests_are_refused_before_execution() {
        assert_eq!(rs_gpu_run(1, -1, 0), LIMIT_REFUSED);
        assert_eq!(
            rs_gpu_run(1, MAX_ELEMENTS as i64 + 1, 0),
            LIMIT_REFUSED
        );
    }
}
