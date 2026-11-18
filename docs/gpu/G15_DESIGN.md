# G15 design: linear jobs over the GPU service

Status: implemented. G14's synchronous `rs_gpu_run` stays as it was; G15 adds
the additive v2 buffer/job ABI, the linear `stdlib/gpu/Jobs.{aui,aum}`
wrappers, deterministic mock completion tests, and physical device tests of
the v2 lifecycle. On the device path a job's `COMPLETE` is published from
wgpu's `Queue::on_submitted_work_done` callback, after the staging copy is
mapped and read, so callback-timed asynchrony is now the implemented path, not
an open item. The mock lifecycle remains the deterministic evidence for
ordering cases (timeout, reordered events, loss) that hardware does not
produce on demand.

Evidence at this revision:

- `cargo test --locked --manifest-path safestos/gpu/Cargo.toml --features service`
  → 21 unit + 11 integration, 0 failed; the nine v2 integration tests are
the mock lifecycle listed below.
- `cargo test … --features "service device"` → 24 unit + 13 integration + 3
  device, 0 failed; the physical v2 test submitted on two live contexts and
  each `rs_gpu_result` matched its own seed, both on the default adapter
  (Intel UHD 630/Vulkan) and with `AVM_GPU_ADAPTER_INDEX=1` (NVIDIA GTX
  1060/Vulkan, discrete). `device_end_to_end::completion_hook_receives_the_
  readback_from_the_work_done_event` checks that the same work-done hook
  receives exactly the readback `run` returns, once.
- `austral compile stdlib/gpu/Jobs.aui,stdlib/gpu/Jobs.aum --target-type=tc`
exit 0.
- `python3 test-programs/runner.py 024-gpu-jobs` → 4 failure fixtures PASS
(use-after-submit, double receipt, read before complete, linear leak).

## Why the current ABI is not enough

`rs_gpu_*` v1 is deliberately narrow:

```text
rs_gpu_open(require_hardware, adapter_index) -> context
rs_gpu_run(context, n, seed) -> checksum
rs_gpu_close(context) -> 0
```

The service synthesises inputs and folds the output, so no buffer crosses the
ABI and there is no completion event a module can observe. That is sufficient
for G14's "one launch" evidence, but it cannot express:

- a caller-owned input/output buffer;
- an outstanding job whose completion is later than submission;
- a job identity that survives a stale context or a device reset;
- cancellation as distinct from waiting.

G15 therefore adds an **additive v2** namespace. `rs_gpu_service_version`
returns `2`; v1 calls remain and keep their meaning. A module that only needs
the one-shot checksum does not migrate.

## ABI (implemented in `safestos/gpu/src/service.rs`)

All identities are opaque `Int64`; all lengths are explicit; no wgpu type or
device pointer crosses the boundary.

```text
rs_gpu_buffer_new(context, elements) -> buffer
rs_gpu_buffer_write(context, buffer, offset_bytes, ptr, len) -> 0 | status
rs_gpu_buffer_read(context, buffer, offset_bytes, ptr, cap) -> bytes | status
rs_gpu_buffer_free(context, buffer) -> 0 | status

rs_gpu_submit(context, output_buffer, n, seed) -> job
rs_gpu_poll(context, job) -> PENDING | COMPLETE | CANCELLED | error
rs_gpu_result(context, job, out_ptr, cap) -> bytes | status
rs_gpu_cancel(context, job, mode) -> 0 | status
```

`PENDING`/`COMPLETE`/`CANCELLED` are `0`/`1`/`2`, not a success checksum.
`rs_gpu_service_version()` returns `2`; v1 `rs_gpu_run` is unchanged.

Statuses added by v2: `-7` `BUSY` (buffer owned by a live job, close refused),
`-8` `STALE` (old generation, consumed receipt, duplicated event) and `-9`
`DEVICE_LOST`; `-10` `TIMEOUT` is reserved for the device path. A job handle
encodes `(context, generation, sequence)` internally — the module only ever
sees an opaque positive `Int64`. `rs_gpu_buffer_write` and `rs_gpu_buffer_read` copy
immediately; the pointer is never retained. A buffer belongs to exactly one
context and its generation.

The FFI layer does not decide ownership. It only reports the runtime state;
the Austral linear types below make misuse a source error, and the runtime
identity check makes a forged/stale `Int64` a refusal.

## Austral linear state machine

New module `stdlib/gpu/Jobs.{aui,aum}`:

```text
Context          -- one open service context; close consumes it
OwnedBuffer      -- writable/readable storage owned by one Context
InFlightJob      -- submitted, completion not yet observed
PendingJob       -- a job whose completion may be polled
CompletedResult  -- a matched completion with result bytes still retained
CancelledJob     -- cancel acknowledged, allocations quarantined until release
```

Privileged record constructors stay private to the module; the unions below
are public so callers can match them. The operations are linear:

```text
submit(ctx: borrow Context, buffer: OwnedBuffer, n, seed) -> SubmitOutcome
-- SubmitOutcome = Submitted(InFlightJob) | SubmitRefused(OwnedBuffer, status)

poll(job: InFlightJob)          // and poll_pending: PendingJob -> PollOutcome
    -> Pending(PendingJob) | Complete(CompletedResult)
     | Cancelled(CancelledJob) | Failed(OwnedBuffer, status)

read(result: CompletedResult, address, cap) -> Read(OwnedBuffer, bytes | status)
cancel(job, mode) -> CancellationRequested(PendingJob) | CancelRefused(OwnedBuffer, status)
release(job: CancelledJob) -> OwnedBuffer
free_buffer(ctx: borrow Context, buffer: OwnedBuffer) -> status
```

Every refusal path that could otherwise lose the buffer returns it (or the
submission returns `SubmitRefused` with it), so the linearity checker accepts
the module and the runtime's `BUSY`/`STALE` refusal cannot leak the token.
`read` copies at most `cap` bytes and returns the retained buffer on both
success and failure.

`submit` consumes the output buffer. `poll` consumes the in-flight token and
returns either the same job in `Pending` or a completed/cancelled receipt;
there is no operation that clones either token. A second receipt requires a
second `poll` on a token that no longer exists, so it cannot type-check.

`CompletedResult` retains the buffer until `read` or `release`; `CancelledJob`
retains it until the device acknowledges cancellation or a context reset. The
runtime keeps a quarantine entry for every allocation whose completion/abort
has not been observed, so closing a context cannot free memory the device may
still touch.

## Runtime identities and completion

A job token carries:

```text
(context_id, context_generation, job_sequence)
```

`context_generation` changes on close/reset; a token from an older generation
is `STALE`, even if the numeric `Int64` is reused. The registry stores the
generation and sequence, not just a hash, so a forged token with a plausible
number still has to match a live entry.

`COMPLETE` is emitted only from the adapter's completion event (the wgpu
submission's submitted-work-done callback or the equivalent polled fence),
after the result buffer is visible. A pure in-memory state transition is not a
completion; that was the defect the plan calls out in the old Virtio record
transition. Device loss returns `LOST`, never `COMPLETE`; a timeout returns
`PENDING`/`TIMEOUT` and leaves the allocation quarantined.

Only one outstanding job per context is admitted initially. A second
`submit` while the first is live returns `BUSY` at the ABI and is not
representable as a well-typed `submit` on a borrowed context whose linear
token is still live. Capacity changes later need an explicit bound, not an
unbounded queue.

## Cancellation

`CancelMode` separates:

- `CancelWait`: the caller wants the token back only after the device
  acknowledges the cancellation; the allocation stays quarantined until then.
- `CancelDeviceWork`: request the adapter cancellation/drop; the token becomes
  `CancelledJob` only on acknowledgement or reset.

Neither mode frees the buffer. A timeout after `CancelDeviceWork` is still
pending/quarantined, not a successful release.

## Execution model

The blocking `queue.submit`/`map`/`poll` work runs on a dedicated worker per
context (or a small pool with one worker per context). The controller thread
never blocks on the adapter while holding the registry lock. `rs_gpu_poll` is a
non-blocking observation of the worker's published state; `rs_gpu_result`
copies only after `COMPLETE`.

The controller can submit a second independent context while the first job is
running; that is the G15 "second request advances while the first runs" test.

## Authorization

The new symbols are ordinary `rs_gpu_*` names in the same manifest namespace:

```toml
kernel = [
  "rs_gpu_buffer_new", "rs_gpu_buffer_write", "rs_gpu_buffer_read",
  "rs_gpu_buffer_free", "rs_gpu_submit", "rs_gpu_poll",
  "rs_gpu_result", "rs_gpu_cancel",
]
```

No `au_*` privileged route is added. A missing grant fails at JIT compile time
with `UK-4001`, before any buffer or job is created. `rs_gpu_open_at` remains a
trusted-deployment convenience, not a module capability.

## Tests and fixtures

Compile-time (Austral) fixtures in `test-programs/suites/024-gpu-jobs/`, each
with a pinned `austral-stderr.txt`:

- `001-use-after-submit` — `free_buffer` after `submit` moved the buffer:
  `The variable buffer has already been consumed.`
- `002-double-receipt` — the same `CompletedResult` passed to two receipts.
- `003-read-before-complete` — `read` on an `InFlightJob`: expected
  `CompletedResult`, got `InFlightJob`.
- `004-leak-linear-buffer` — return without consuming an `OwnedBuffer`.

These fail type checking; they are source fixtures, not runtime checks. A
closing-a-context-with-a-live-buffer fixture cannot be written as a type error
because `Context` and `OwnedBuffer` are separate tokens; that case is the
runtime `BUSY` below.

Runtime mock lifecycle in `safestos/gpu/tests/service.rs` (test-only
`service_test_submit`/`service_test_publish`, not exported symbols):

- buffer write/read roundtrip, submit -> pending -> complete -> Result, with
exactly one receipt (`buffers_roundtrip_and_a_completed_job_writes_its_result`);
- repeated/reordered publication leaves one owner (`repeated_and_reordered_completion_events_keep_one_owner`);
- a second live job on one context is `BUSY` (`a_second_live_job_is_refused_with_busy`);
- a second context submits and completes while the first job is pending, and
  identities do not cross contexts (`a_second_context_advances_while_the_first_job_is_pending`);
- stale generation and forged identities are `STALE`/`UNKNOWN_HANDLE`
  (`stale_generation_and_forged_identities_are_refused`);
- close during a pending job is refused (`close_during_a_pending_job_is_refused`);
- cancellation keeps the buffer quarantined until its event
  (`cancellation_keeps_the_buffer_quarantined_until_the_event`);
- a mock timeout is terminal, is never overwritten by a late completion, and
  returns the allocation (`a_timeout_is_terminal_and_returns_the_allocation`);
- a mock device loss is never `COMPLETE` (`a_mock_loss_is_never_a_completion`).

Physical device lifecycle (`--features device`, requires an adapter):

- two live contexts submit (`rs_gpu_submit` spawns each worker), both reach
  `COMPLETE`, and each `rs_gpu_result` returns that job's own words
  (`a_physical_submit_completes_each_context_with_its_own_result`; measured
  64 + 32 result bytes on both the Intel UHD 630 and the NVIDIA GTX 1060).
  `AVM_GPU_ADAPTER_INDEX` selects a specific adapter; the default is wgpu's
  first suitable one. The test is ordering-tolerant: it does not claim the two
  dispatches overlapped on the device, only that a second live request was
  accepted and each completion kept its own ownership.

The device completion path is `crate::run::run_with_completion`: the readback
happens inside the queue's submitted-work-done callback, and that callback is
where the service publishes `COMPLETE`. The worker therefore cannot publish a
completion by returning; it only publishes failures it observed before a
submission existed (or when no submission was needed). Because publication can
precede the worker's own return, `rs_gpu_close` joins the context's worker
threads before it reports success, so device teardown never overlaps a live
thread (this was measured: without the join the two-context test segfaulted
intermittently at process exit, 2 of 9 runs; with it, 16 of 16 runs passed
across the Intel and NVIDIA adapters). `AVM_GPU_STRESS_RUNS=N` repeats that
cycle one process per run (`a_physical_submit_child_cycle`, spawned by the
test), because the crash lives in the exit path and a loop inside one process
cannot see it: 20 runs passed with the join and a control with the join
removed failed 2 of 30 with SIGSEGV. The mock lifecycle remains the
deterministic evidence for ordering the hardware does not produce on demand,
and a mock success is never presented as hardware evidence.

## Migration and compatibility

- `SERVICE_VERSION` is `2`; v1 symbols keep their v1 behavior.
- `GpuService.aum` stays on v1; `stdlib/gpu/Jobs` is the v2 surface, and no
  deployment module uses it yet.
- `examples/gpu/service/module.toml` still grants only the v1 symbols; a module
  using `Jobs` must add the v2 grants (listed above) or fail `UK-4001`.
- `scripts/gpu-service-jit` continues to cover the v1 JIT path; it does not
  exercise the async lifecycle, and no gate was weakened to make G15 pass.

## Resolved questions

1. `OwnedBuffer` carries bytes only; `new_buffer` still takes elements and
   records `elements * 4`, exposed as `buffer_length`.
2. `PendingJob` carries no clock token; timeout policy stays with the module,
   and the runtime reports `TIMEOUT`/`PENDING` without releasing the buffer.
3. A context reset has no separate release proof: allocations stay quarantined
   until the job reaches a terminal state, and `close` refuses while one is
   pending (`BUSY`).
4. One worker thread per submitted job, publishing into the shared registry
   (device completion from the queue's work-done callback); the controller only
   observes with `rs_gpu_poll` and never blocks on the adapter. The context
   keeps the handles and `rs_gpu_close` joins them, because a job can be
   terminal while its worker is still inside the callback.
