# GPU service adapter (G14; v2 buffers/jobs in G15)

This is the narrow, versioned service contract a manifest-gated Austral module
uses to launch a compiled GPU bundle. It is an ordinary `rs_*` foreign
namespace: the JIT’s `check_call_permission` decides each call from
`module.toml`, exactly like `uk_*` and `uz_*`. No `au_*` symbol or privileged
path bypasses that check.

## ABI

All identities are opaque `Int64`; no wgpu type, Rust layout or device pointer
crosses the boundary.

```text
rs_gpu_service_version() -> 1

rs_gpu_open(require_hardware: Int64, adapter_index: Int64) -> handle
rs_gpu_run(handle: Int64, n: Int64, seed: Int64) -> checksum | negative status
rs_gpu_close(handle: Int64) -> 0 | negative status
```

`rs_gpu_open` reads the deployment-selected bundle path from the
`AVM_GPU_BUNDLE` environment variable. The module cannot choose a path, so it
cannot ask the adapter to open an arbitrary file. `adapter_index = -1` uses the
default adapter; a non-negative value is the explicit selector validated by
G09.

A Nix-linked host loads `libvulkan` through the Nix loader, which does not
search the host's `/lib`. Deployment must therefore supply a Vulkan loader and
an ICD (for example the Nix `vulkan-loader` and `mesa` outputs) on
`LD_LIBRARY_PATH` and `VK_DRIVER_FILES`. Without them a required-hardware
request returns `-6`; it never silently falls back.

`rs_gpu_run` synthesises the bundle’s inputs (`input0[i] = seed + i`,
`input1[i] = i`), runs **exactly one** request, and returns the wrapping sum of
the output words masked to 63 bits. The explicit `n` is bounded by the v1
release limit.

Negative statuses:

| Code | Meaning |
|---|---|
| `-1` | invalid argument (bad boolean/selector/path length/UTF-8) |
| `-2` | bundle refused (load, digest or validation failure) |
| `-3` | unknown or already-closed handle |
| `-4` | request outside the v1 element/limit contract |
| `-5` | a device was present but the request was refused |
| `-6` | no physical adapter for a required-hardware request |

A required-hardware request never falls back to the reference evaluator.
With the `device` feature, a non-required request uses the device when one is
available and falls back to the validated reference path only when no adapter
exists; the Rust integration test records which path is available.

## v2: buffers and jobs (G15)

`rs_gpu_service_version()` returns `2`. The v1 symbols above are unchanged; a
module that only needs a one-shot checksum keeps using them. The additive
surface is:

```text
rs_gpu_buffer_new(context, elements) -> buffer
rs_gpu_buffer_write(context, buffer, offset_bytes, ptr, len) -> 0 | status
rs_gpu_buffer_read(context, buffer, offset_bytes, ptr, cap) -> bytes | status
rs_gpu_buffer_free(context, buffer) -> 0 | status

rs_gpu_submit(context, output_buffer, n, seed) -> job
rs_gpu_poll(context, job) -> PENDING(0) | COMPLETE(1) | CANCELLED(2) | status
rs_gpu_result(context, job, out_ptr, cap) -> bytes | status
rs_gpu_cancel(context, job, mode) -> 0 | status
```

Statuses added by v2 are `-7` `BUSY`, `-8` `STALE` and `-9` `DEVICE_LOST`;
`-10` `TIMEOUT` is reserved for the device path. A buffer owned by a live job
refuses reads, writes and free with `BUSY`; a job from an older context
generation, a second result copy and a duplicate completion event all return
`STALE`. `rs_gpu_close` refuses with `BUSY` while a job is pending, then frees
buffer payloads and keeps the identities so their handles report `STALE`.
Cancellation (`mode` 0 = cancel-wait, 1 = cancel-device-work) records the
request but leaves the job `PENDING`; the buffer is released only after a
terminal event.

The Austral side is `stdlib/gpu/Jobs.{aui,aum}`: opaque linear `Context`,
`OwnedBuffer`, `InFlightJob`, `PendingJob`, `CompletedResult` and
`CancelledJob`, with union outcomes that return the buffer on every refusal.
Those wrappers are not yet imported by a deployment module; a module that uses
them must add the v2 grants to its manifest.

## Authorization and ownership

`examples/gpu/service/module.toml` grants the three v1 `rs_gpu_*` symbols.
Omitting a grant fails JIT compilation with `UK-4001` before the operation
runs; that is the negative control in the recorded evidence. G15's linear
`OwnedBuffer`/`PendingJob` wrappers are in `stdlib/gpu/Jobs`, and their v2
symbols need the same kind of manifest grant; this example's manifest stays on
v1 because `GpuService.aum` still calls only v1.

The provider is `examples/gpu/service/provider` (a `cdylib`). Native trusted
adapters are still trusted code: this is not an OS sandbox, and the manifest
authorizes a call, not an arbitrary native capability.

## Evidence

- `safestos/gpu/src/service.rs` — ABI, handle table, statuses; 21 unit tests
  with `--features service` (v1 plus v2 generation/ownership checks).
- `safestos/gpu/tests/service.rs` — open/run/close against a real bundle; bad
  lengths and stale handles refused; nine v2 mock lifecycle tests (buffers,
  one owner per completion, busy close, stale/forged identities, cancellation
  and timeout quarantine, loss is not completion, a second context advancing
  while the first job is pending); with `--features "service device"` a
  physical-required run launches the add bundle, and
  `a_physical_submit_completes_each_context_with_its_own_result` submits on two
  live contexts and checks each result (or records an explicit gap when
  `AVM_GPU_ALLOW_NO_DEVICE=1`). `AVM_GPU_ADAPTER_INDEX=N` selects an adapter
  explicitly; measured on both the Intel UHD 630 (default) and the NVIDIA GTX
  1060 (`=1`), both Vulkan. On the device path `COMPLETE` is published from
  wgpu's submitted-work-done callback (`crate::run::run_with_completion`), and
  `safestos/gpu/tests/device_end_to_end.rs` checks that hook receives exactly
  the readback `run` returns. `AVM_GPU_STRESS_RUNS=N` re-runs the two-context
  cycle in N child processes, so a teardown that races process exit fails the
  test instead of passing and crashing afterwards.
- `stdlib/gpu/Jobs.{aui,aum}` — linear wrappers; `austral compile …
  --target-type=tc` exit 0. `test-programs/suites/024-gpu-jobs/` holds four
  compile-time fixtures that must fail to type-check (use-after-submit, double
  receipt, read before complete, linear leak), each with pinned diagnostics.
- `umbrella/modules/GpuService.aum` — the Austral host module type-checks and
  calls the service through the JIT with `rs_gpu_open(1, -1)`. The measured
  physical launch returned `RESULT 56`/exit 0 on the Intel UHD 630: a
  system-linked umbrella sees host Vulkan directly, while the Nix-linked host
  needs `vulkan-loader` and a Mesa ICD. Without those, the Nix loader sees no
  adapter and the module returns `-6`; it does not fall back to reference.
- Grant-denial evidence (manual, recorded in `docs/gpu/EXECUTION.md`):
  `umbrella --entry compute` without `--manifest` fails with
  `UK-4001: ... 'rs_gpu_open'` before opening the bundle.
- Physical JIT launch: `scripts/gpu-service-jit` builds the Pointwise `add`
  bundle, runs the committed module through the manifest-gated JIT and
  requires `RESULT 56`; the same script omits the manifest and requires the
  `UK-4001` denial. A system-linked host needs only the bridge on
  `LD_LIBRARY_PATH`; a Nix-linked host also needs `vulkan-loader` and a Mesa
  ICD, supplied through `AVM_GPU_JIT_LD_LIBRARY_PATH`. The CI
  `gpu-required` job runs this on the self-hosted GPU runner. Exact commands
  and the manual record are in `docs/gpu/EXECUTION.md`.
