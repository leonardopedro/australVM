# GPU jobs (G15): the v2 buffer/job surface from Austral

`JobsDemo.aum` is the deployment half of G15. It imports the linear wrappers
in `stdlib/gpu/Jobs` and drives one complete buffer/job cycle through the
JIT:

1. `open(1, -1)` — required hardware, default adapter;
2. `new_buffer(&context, 8)` — one owned output buffer;
3. `submit(&context, buffer, 8, 5)` — the add bundle with `input0[i] = 5 + i`,
   `input1[i] = i`;
4. `poll`/`poll_pending` until the job is terminal;
5. `read(result, out, 32)` — copy the completed result into an allocated
   32-byte buffer (the add bundle's output is eight 4-byte words);
6. `free_buffer`, `close` — and every refusal path returns the linear tokens
   too, so the module type-checks linearly with no leak.

`compute` returns the byte count `read` reported, so the JIT's `--expect`
compares evidence from the device readback (32 bytes copied out of the
completed result) rather than the service's status. A refusal returns the
negative service status instead.

## Run it

```bash
scripts/gpu-jobs-jit
```

The script builds the Pointwise `add` bundle, builds the provider `cdylib`,
runs the module through `umbrella` with `module.toml` as the grant manifest,
and requires `RESULT 32`. It then reruns without a manifest and requires the
`UK-4001` denial for `rs_gpu_open` before the bundle opens. As with
`examples/gpu/service`, a Nix-linked host needs a Vulkan loader and ICD on
`LD_LIBRARY_PATH`; `AVM_GPU_JIT_LD_LIBRARY_PATH` supplies them.

`module.toml` grants the complete v2 surface:

```text
rs_gpu_open, rs_gpu_close, rs_gpu_buffer_new, rs_gpu_buffer_write,
rs_gpu_buffer_read, rs_gpu_buffer_free, rs_gpu_submit, rs_gpu_poll,
rs_gpu_result, rs_gpu_cancel
```

The whole `Jobs` module is compiled, so even wrappers this demo does not call
name their symbol at compile time.

Drop one and JIT compilation fails with `UK-4001` before the operation runs.

## JIT gaps this example found

Two lowering gaps made a module that returns union cases and reads constants
unrunnable, and both are fixed:

- a union constructed in tail position (`return Complete(…)`) went through
  `declare_import("__union_new", argc)` instead of the inline allocation, so
  two cases with different field counts declared `__union_new` with two
  incompatible signatures. Fixed in `safestos/cranelift/src/cps.rs`; the
  regression test is
  `tail_position_union_constructions_of_different_arity_compile`.
- a module-level `constant` is emitted as a zero-argument function, but a
  reference to it lowered to a local `Var` with the qualified name
  (`Jobs::PENDING`), which the JIT cannot resolve. References are now calls to
  the unqualified function (`lib/Compiler_cps.ml`).

The pointer intrinsics the wrappers use are lowered too: `au_calloc`
(`allocateBuffer`) is a new native symbol with zeroing and overflow checks, and
`au_free` (`deallocate`) was already registered; `sizeof` now reports real
scalar and pointer widths instead of 0.
