# Pointwise GPU kernels — authoring and release evidence (M2)

This fixture is the first-release WGSL target: one Austral module pair compiles
to a validated bundle, and the same generated shader is checked by the Rust
bundle checker, evaluated by the KernelIR reference evaluator, and (when a
physical adapter is available) dispatched on a GPU.

The authoritative admission contract is
[docs/gpu/KERNEL_V1.md](../../../docs/gpu/KERNEL_V1.md); the execution log is
[docs/gpu/EXECUTION.md](../../../docs/gpu/EXECUTION.md). This page is the
practical authoring/release note. It does not replace either.

## Installation and locked dependencies

The GPU work is an isolated crate. The default build is CPU-only; wgpu is
optional and is never added to the compiler/bridge default features.

```sh
# CPU-only checker: strict bundle parsing, canonical digests, Naga validation,
# KernelIR reference evaluation.
cargo test --locked --manifest-path safestos/gpu/Cargo.toml

# Device build: pinned wgpu 29.0.4 + pollster, Vulkan/GL/Metal/DX12 backends.
cargo test --locked --features device --manifest-path safestos/gpu/Cargo.toml
cargo build --locked --features device --manifest-path safestos/gpu/Cargo.toml

# The OCaml compiler must be built against a fresh cranelift bridge:
nix develop "git+file://$PWD#ocaml" -c make all
```

`Cargo.lock` in `safestos/gpu/` pins Naga/wgpu 29.0.4 and the rest of the
checker's dependency graph; use `--locked` so a release record cannot silently
drift to a different shader compiler. Compiler links need the bridge release
artifact (`make bridge` or `make all`) on `LD_LIBRARY_PATH`:

```sh
export LD_LIBRARY_PATH="$PWD:$PWD/safestos/cranelift/target/release"
```

## Authoring commands

The module pair is `Pointwise.aui` + `Pointwise.aum`. `Pointwise:add` is a
two-input kernel; `Pointwise:identity` is the smallest one-input kernel.

```sh
# Compile one entry to a staged bundle (the checker validates Naga + wrapper).
AUSTRAL_GPU_CHECKER=$PWD/safestos/gpu/target/debug/avm-gpu \
  _build/default/bin/austral.exe compile \
  examples/gpu/pointwise/Pointwise.aui,examples/gpu/pointwise/Pointwise.aum \
  --target-type=wgsl --entrypoint=Pointwise:add --output=/tmp/add.avkg

# Check the bundle and evaluate the KernelIR (no GPU needed).
avm-gpu validate /tmp/add.avkg
avm-gpu reference /tmp/add.avkg --inputs /tmp/in.json

# Dispatch the validated WGSL; --require-hardware refuses a software adapter.
avm-gpu run /tmp/add.avkg --inputs /tmp/in.json --require-hardware
```

`/tmp/in.json` is strict JSON of the form `{"inputs":[[...],[...]]}`; every
element must be an unsigned 32-bit integer and both arrays must have equal
length. The output is `{"inputs":[[0,1,2],[0,2,3]]}` → `[0,3,5]`.

Adapter selection is explicit and validated. Without a selector, wgpu's own
`request_adapter` chooses the first suitable adapter (on this host, Vulkan
Intel UHD 630). `probe --list` prints every adapter in index order; names are
case-insensitive substrings and must match exactly one adapter:

```sh
avm-gpu probe --list
avm-gpu probe --require-hardware --adapter-index 1
avm-gpu run /tmp/add.avkg --inputs /tmp/in.json --require-hardware --adapter-index 1
```

On this host index 1 is the Vulkan `NVIDIA GeForce GTX 1060 with Max-Q Design`
(a GL copy of it is also enumerated, which is why the name selector is
ambiguous and the index is the precise handle). An out-of-range index, an
unknown name, an ambiguous name, or mixing `--adapter` with `--adapter-index`
is refused; there is no silent fallback.

## Gate modes

`scripts/check-gpu` never collapses its results into one `verified` value:

```sh
scripts/check-gpu source     # CPU-only: C/IR corpus + Naga validation
scripts/check-gpu optional   # source + adapter probe; gap allowed (exit 0)
scripts/check-gpu required   # source + physical-device corpus; gap is exit 1
```

Every run prints separate `source=`, `shader-validation=`, `device-kind=`,
`execution=` and `proof=` fields. `source` mode needs no GPU and no `device`
build. `optional` prints `[gap]` and exits 0 when hardware is unavailable.
`required` converts the same condition into exit 1 and runs
`scripts/gpu-hardware-check` (C + IR + device at sizes 0…1024, MAX stress,
structural/semantic controls, preflight refusals).

CPU CI (`.github/workflows/gpu.yml` → `gpu-source`) asserts
`device-kind=not-probed` and `execution=not-run`, so a CPU pass cannot present
as GPU coverage. The `gpu-required` job is dispatch-only and runs on an
explicitly labelled self-hosted GPU runner; there is no `ubuntu-latest`
fallback.

## Admitted subset (v1)

From KERNEL_V1: one or two `Nat32` parameters, one `Nat32` result, mapped over
equal-length input arrays to one output array; immutable scalar computation,
conditionals, acyclic local helpers, and exact `Nat32` modular/bitwise
intrinsics. No traps, casts, recursion, loops, foreign IO, pointers, shared
memory, barriers, or arbitrary storage indexing. The compiler generates the
identity-index memory plan and the guard; authors do not write the wrapper.

## Diagnostics

| Symptom | Meaning |
|---|---|
| `source=pass`, `device-kind=not-probed` | CPU lane only; no GPU claim |
| `[gap] ... hardware not executed` | optional mode with no/software adapter; allowed |
| `required mode: a physical adapter is required; no skip` | required mode saw no physical adapter (exit 1) |
| `adapter is cpu, not a physical GPU` | a software adapter cannot satisfy required mode |
| `input arrays have unequal lengths` | preflight refusal before any buffer exists |
| `input arity is 2, the request carries 1` | request shape does not match the bundle |
| `N = … exceeds the release limit 1048576` | oversized input, refused pre-dispatch |
| `dispatch of … workgroups exceeds the adapter maximum` | oversized dispatch (tested with injected limits) |
| `no adapter matches "…"; available: …` | invalid explicit selector; lists valid choices |
| `adapter selector "…" is ambiguous` | use `--adapter-index` |
| `shader/pipeline validation failed: …` | Naga or pipeline rejected the emitted WGSL |

## Measured baseline (2026-11-18)

Same generated `Pointwise:add` WGSL, N=1024, five `avm-gpu run` invocations
after one compile; medians except the one-time compile/validate/reference
phases. Measured with `scripts/gpu-benchmark`.

| Phase | Intel UHD 630 (Vulkan) | GTX 1060 (Vulkan) |
|---|---:|---:|
| compile (Austral → bundle) | 181.36 ms | 178.34 ms |
| validate (strict + digests + Naga + wrapper) | 3.34 ms | 4.61 ms |
| reference (KernelIR evaluation) | 5.21 ms | 5.26 ms |
| prepare (adapter/device/pipeline) | 27.03 ms | 31.27 ms |
| upload | 0.37 ms | 10.95 ms |
| submit (includes any first-use driver work) | 48.93 ms | 0.44 ms |
| readback | 1.03 ms | 0.18 ms |
| process wall (per invocation) | 196.38 ms | 163.03 ms |
| execution elements/s | 20,496 | 1,641,691 |
| end-to-end elements/s (process wall) | 5,214 | 6,281 |

There is **no required speedup threshold** for v1 and no comparison against
unrelated physics workloads. The numbers record a baseline; the process wall
also contains fixed startup cost that the phase fields do not attribute, and
first-use driver pipeline compilation can land in `submit` rather than
`prepare`. `--repeats` and `--adapter-index` change only which baseline is
recorded.

## Not covered by this release (M2)

- GPU execution on CPU-only CI is not covered and is not implied. The recorded
  hardware runs are Intel UHD 630/Vulkan and GTX 1060/Vulkan.
- G13's Why3 lane proves the index/bounds, guard, flatten-injectivity and
  distinct-write goals for the actual bundle; `check-gpu` prints `proof=pass`
  when why3/alt-ergo are available and `proof=gap` otherwise. The MM0
  certificate is still a separate `[gap]`, and input extents, fresh output and
  hardware invocation ids remain recorded runtime premises. Non-alias is not
  proved; it is trusted runtime allocation.
- The wrapper audit and renderer/model correspondence remain trusted code.
  Naga validation is not a proof of semantic equivalence.
- No async service, throughput SLO, CUDA backend, MLIR backend, or NPU target.
- A software adapter (llvmpipe) can be enumerated but cannot satisfy
  `--require-hardware` or `check-gpu required`.
