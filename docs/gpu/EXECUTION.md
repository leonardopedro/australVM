# GPU kernel backend — execution record

This is the compact execution log for [PLAN_gpu_kernels.md](../../PLAN_gpu_kernels.md).
It records what was actually run, in which environment, and what remains open.
It is not a claim of GPU execution for any task that has not produced adapter
evidence.

## Baseline (G00, 2026-11-17)

| Item | Recorded value |
|---|---|
| australVM revision | `d22256120b0fba1edf2e32541a77a371c549f098` (branch `master`) |
| Pre-existing dirty paths (preserved, unrelated) | `safestos/cranelift/src/adapters.rs` (modified), `safestos/cranelift/tests/scalar_abi.rs` (untracked) |
| Host architecture | `x86_64`, kernel `6.18.48-amd64-desktop-rolling` |
| OCaml | 5.3.0 (opam switch, `~/.opam/5.3.0`) |
| dune | 3.22.0 |
| Rust | `rustc`/`cargo` 1.99.0 |
| Nix | 2.33.0 (used only for the documented `.#ocaml`/`.#lean` profiles) |
| Python | 3.12.13 |
| SHA-256 for OCaml bundles | `digestif` 1.3.0 (opam-installed for this work; declared in `dune-project` and `flake.nix`) |
| GPU toolchain observed | `nvidia-smi` reports one `NVIDIA GeForce GTX 1060 with Max-Q Design`; `/dev/dri/card0,card1,renderD128,renderD129` exist. **This is not an adapter probe**: G09 owns enumeration and classification. |
| wgpu/Naga | not present in this checkout's locks before G07. The isolated `safestos/gpu` crate pins 29.0.4 per the contract. |
| Cargo cache | crates registry cache is populated (offline resolution possible for cached crates). |

### Baseline build/test commands

```sh
dune build lib/ bin/ test/        # exit 0 (opam dune 3.22.0)
```

The build emitted only pre-existing C stub warnings and the two known
menhir shift/reduce conflicts. No pre-existing build failure blocks this work.

### Command profiles (unchanged from the plan)

```sh
nix develop "git+file://$PWD#ocaml" -c make all
nix develop "git+file://$PWD#ocaml" -c dune exec test/GpuFrontendTest.exe
nix develop "git+file://$PWD#ocaml" -c dune exec test/GpuObligationsTest.exe
cargo test --locked --manifest-path safestos/gpu/Cargo.toml
cargo test --locked --manifest-path safestos/gpu/Cargo.toml --features device
cargo test --locked --manifest-path safestos/gpu/Cargo.toml --features service
cargo test --locked --manifest-path safestos/gpu/Cargo.toml --features "device service"
scripts/gpu-service-jit          # self-hosted GPU runner: v1 JIT launch + denial
python3 scripts/check-gpu source|optional|required [--require-proof]
python3 scripts/gpu-benchmark --repeats 5 [--adapter-index N]
python3 scripts/gpu-obligations check BUNDLE [--require-proof]
nix shell ".#why3" ".#alt-ergo" -c python3 scripts/gpu-obligations check BUNDLE --require-proof
```

## Task table

| Task | Status | Source/contract identity | Commands and evidence | Gaps/next action |
|---|---|---|---|---|
| G00 | implemented | this file; `docs/gpu/KERNEL_V1.md` | `git rev-parse HEAD`, `dune build lib/ bin/ test/` exit 0 | adapter probe deferred to G09 by design |
| G01 | implemented | `lib/Compiler.ml` `prepare_module`; `examples/gpu/pointwise/` | `dune exec test/GpuFrontendTest.exe` | none |
| G02 | implemented | `lib/gpu/Gpu_ir.ml`, `Gpu_bundle.ml` | `dune exec test/GpuIrTest.exe` | SHA-256 is Digestif; Rust reproduction is G07 |
| G03 | implemented | `lib/gpu/Gpu_subset.ml` | `dune exec test/GpuSubsetTest.exe` | corpus grows in G08 |
| G04 | implemented | `lib/gpu/Gpu_intrinsics.ml` | `dune exec test/GpuSubsetTest.exe`, `GpuFrontendTest.exe` | intrinsic identity is resolved from the compiled canonical Pervasive module |
| G05 | implemented | `lib/CliParser.ml`, `lib/CliEngine.ml`, `Compiler.ml` | `dune exec test/GpuCliTest.exe` (3 tests); `scripts/gpu-source-check` publishes and validates real bundles | publication requires the G07 checker, which is now wired through `AUSTRAL_GPU_CHECKER` |
| G06 | implemented | `lib/gpu/Gpu_wgsl.ml` | `dune exec test/GpuWgslTest.exe` | goldens check rendering only |
| G07 | implemented | `safestos/gpu/` | `cargo test --manifest-path safestos/gpu/Cargo.toml` (12 tests); `avm-gpu validate`/`reference` on `scripts/gpu-source-check` bundles | device feature lands in G09; no GPU execution claimed |
| G08 | implemented | `examples/gpu/pointwise/`, `corpus.tsv`, `scripts/gpu-source-check`, `test-programs/suites/022-gpu/` | `python3 scripts/gpu-source-check` → `[pass] G08 source/C/IR agree: 156 cases over 14 entries (seed 20261118)`; three semantic mutations detected and a `while` refused; `python3 test-programs/runner.py 022-gpu` PASS | fixed-seed corpus is bounded, not exhaustive; the helper-elision control is a source-level mutation, while the checker's helper audit is covered by the crate tests |
| G09 | implemented | `safestos/gpu/src/device.rs`, optional `wgpu =29.0.4` | `cargo test --features device`; `avm-gpu probe --list` enumerates Intel UHD 630 (Vulkan, integrated), GTX 1060 (Vulkan, discrete), llvmpipe (Vulkan, cpu) and a GL copy of the GTX; default `probe` is Intel; `--adapter NAME`/`--adapter-index N` select explicitly and validate (unknown/ambiguous/out-of-range refused, exit 2 for conflicting flags) | default selection is wgpu's first suitable adapter; the selector is how a specific adapter is named, and `required` still refuses a software adapter |
| G10 | implemented | `safestos/gpu/src/run.rs`, CLI `run` | `cargo test --features device` (19 unit tests, 4 preflight); `avm-gpu run /tmp/g10.avkg --inputs … --require-hardware` on the Intel UHD 630/Vulkan: `add` = `[0,3,5]`, `difference` = `[4294967294,3,4294967294]`, N=1024 equal to `reference`, N=0 dispatch 0; both bit-exact with `avm-gpu reference` | `submit` timing includes first-use pipeline creation (150 ms); no async service or throughput claim |
| G11 | implemented | `scripts/gpu-hardware-check` | `scripts/gpu-hardware-check` → device agrees with **C and IR** at sizes 0,1,63,64,65,127,128,129,1024 (22,414 elements over 14 entries); `add` at N=1,048,576 agrees with IR; guard/index/binding/profile mutations refused pre-submission; swapped-subtract and reversed-comparison source mutations disagree on device; unequal/arity/malformed/oversized inputs refused; Intel UHD 630/Vulkan, ~52 s | oversized **dispatch** (rather than oversized input) is exercised by `run::tests::preflight_refuses_limits_before_buffers` with injected adapter limits, not by the hardware script |
| G12 | implemented | `scripts/check-gpu`, `scripts/gpu-benchmark`, `.github/workflows/gpu.yml`, `examples/gpu/pointwise/README.md`, `safestos/gpu/tests/device_end_to_end.rs` | `scripts/check-gpu source` → `source=pass`, `shader-validation=pass`, `device-kind=not-probed`, `execution=not-run`, `proof=gap`; `optional` → source+Naga pass, physical Intel probe, `execution=not-run`; `required` → source+Naga pass, `device-kind=physical`, `execution=pass` after the full G11 corpus/controls, exit 0; the same required mode with a CPU-only checker exits 1 (`device-kind=unavailable`), while optional exits 0 with `[gap]`; CPU crate tests 16 unit; device build 19 unit + 2 integration; baseline `add` N=1024 Intel 20,496 el/s execution / 5,214 el/s process, GTX 1060 1,641,691 / 6,281, phases in `examples/gpu/pointwise/README.md` | at G12 time `proof=gap` was the M2 boundary; G13 now supplies the Why3 lane (`proof=pass` when why3/alt-ergo are present) and leaves the MM0 certificate as its own gap; benchmark has no speedup threshold and process wall includes unattributed startup; oversized dispatch is still covered by the injected-limits unit test, not the hardware script |
| G13 | implemented (Why3), MM0 gap | `lib/gpu/Gpu_obligations.ml`, `scripts/gpu-obligations`, `docs/gpu/OBLIGATIONS.md` | `dune exec test/GpuObligationsTest.exe` (5 tests: actual bundle accepted; changed plan, removed guard, changed extent and tampered shader refused); `nix shell ".#why3" ".#alt-ergo" -c python3 scripts/gpu-obligations check /tmp/g10.avkg --require-proof` → `[pass] Why3 memory-plan obligations: 8 goal(s) valid with alt-ergo`, `[gap] MM0 certificate: mm0-rs/mm0-c not on PATH`; `scripts/check-gpu source --require-proof` → `proof=pass (Why3 memory-plan goals valid)` | MM0 is statements-only: PC03's composition is applied to the plan in `mm0/proof_chain/gpu_plan.mm0` (PC06), but the plan's residual premises O1–O5 are not discharged for an artifact, so `gpu_memory.mm0` says `certificate: [gap]` and no theorem is claimed; runtime extents/fresh output/invocation ids are recorded premises, not proved; general shader/model correspondence is PC05 work |
| G14 | implemented | `safestos/gpu/src/service.rs`, `safestos/gpu/tests/service.rs`, `examples/gpu/service/`, `umbrella/modules/GpuService.aum` | `cargo test --features service` → 5 unit + 2 integration; `cargo test --features "service device"` → 3 integration incl. `a_physical_required_run_is_a_device_launch`; `austral compile umbrella/modules/GpuService.aum --target-type=tc` exit 0; `scripts/gpu-service-jit` builds the Pointwise bundle, launches through the manifest gate and requires `RESULT 56` plus the omit-manifest `UK-4001` denial; measured with a system-linked umbrella → `RESULT 56`, `umbrella: matches --expect 56`, exit 0; the Nix-linked umbrella reaches the same result after `vulkan-loader`/Mesa are put on its loader path; the same run with the manifest omitted → `UK-4001: Authorization denied — 'anon' is not granted 'rs_gpu_open'` before opening the bundle | The JIT host is Nix-linked, so its loader does not search the host's `/lib` and reports no adapter unless `vulkan-loader` and a Mesa ICD are supplied; that is a deployment requirement, and the module requests `require_hardware=1` rather than hiding the gap with a reference fallback. Handles are raw `Int64`; linear job/buffer wrappers and async completion are G15. |
| G15 | implemented (mock lifecycle; device completion from the submitted-work-done callback) | `safestos/gpu/src/service.rs`, `safestos/gpu/tests/service.rs`, `stdlib/gpu/Jobs.{aui,aum}`, `test-programs/suites/024-gpu-jobs/` | `cargo test --locked --manifest-path safestos/gpu/Cargo.toml --features service` → 21 unit + 11 integration, 0 failed (9 v2 mock lifecycle tests); `cargo test … --features "service device"` → 24 unit + 13 integration, 0 failed, incl. `a_physical_submit_completes_each_context_with_its_own_result` on the default Intel UHD 630 and on `AVM_GPU_ADAPTER_INDEX=1` NVIDIA GTX 1060 (two contexts each, every result matching its own seed); `./austral compile stdlib/gpu/Jobs.aui,stdlib/gpu/Jobs.aum --target-type=tc` exit 0; `python3 test-programs/runner.py 024-gpu-jobs` → 4 PASS (use-after-submit, double receipt, read before complete, linear leak) | Device completion is published from `Queue::on_submitted_work_done` (readback and publication both inside the callback), so the worker cannot manufacture one by returning; `rs_gpu_close` joins the context's workers so teardown cannot overlap that thread. The physical test still proves lifecycle and per-context ownership on hardware, not that two dispatches overlapped. `GpuService.aum` and its manifest stay on v1; a module using `Jobs` must add the v2 grants. The linear rule that a context cannot close while a buffer is live is runtime `BUSY`, not a type error. |

## Limitations

- No task above claims GPU execution except where a dispatch and readback
  record exists (G10, G11 and the G14 JIT launch). G09 alone is adapter
  discovery, never execution.
- `nvidia-smi` output is inventory evidence only. wgpu's default
  `request_adapter` enumerates the Vulkan **Intel UHD Graphics 630**
  (integrated) adapter first, so the recorded G09–G11 records are Intel/Vulkan.
  `avm-gpu probe --list` also sees the GTX 1060 (Vulkan, discrete) and a
  GL copy of it; `--adapter`/`--adapter-index` select explicitly, and a dispatch
  on `--adapter-index 1` (GTX 1060) produced `[0,3,5]` for `add`. The wgpu
  record is the one that counts, not the OS inventory.
- G10/G11 execution is reproducible with:
  `AUSTRAL_GPU_CHECKER=$PWD/safestos/gpu/target/debug/avm-gpu _build/default/bin/austral.exe compile examples/gpu/pointwise/Pointwise.aui,examples/gpu/pointwise/Pointwise.aum --target-type=wgsl --entrypoint=Pointwise:add --output=/tmp/g10.avkg`
  then `avm-gpu run /tmp/g10.avkg --inputs <file> --require-hardware [--adapter-index N]`
  and `avm-gpu reference` on the same input. Outputs agree; the JSON records
  the actual backend/device, dispatch count and separate prepare/upload/
  submit/readback timings.
- M2's first release boundary is the source gate plus the G11 hardware record.
  G13's Why3 obligations are implemented and reported as `proof=pass` when the
  prover is available. The MM0 certificate remains a separate `[gap]`: PC06
  applies the PC03 composition to the plan (`docs/proof/GPU_PLAN.md`), but its
  residual premises O1–O5 are not discharged for an artifact, and `check-gpu`
  never turns Why3 into an MM0 pass. G14's ABI, grant gate and physical JIT launch are recorded; G15's v2 ABI,
  linear wrappers, mock lifecycle and two-context physical submit/result on both
  available Vulkan GPUs are recorded, and device completion is now published from wgpu's
  submitted-work-done callback (with the context's workers joined at close); G16 and later remain pending.
- CPU CI (`gpu-source`) asserts `device-kind=not-probed` and
  `execution=not-run`; only the dispatch-only `gpu-required` job on a
  self-hosted `gpu` runner runs required mode.
- Baseline is a measurement, not a target: no speedup threshold is enforced,
  no comparison against unrelated physics workloads is made, and `submit` can
  contain first-use driver pipeline work.
- G14's JIT record uses `umbrella/modules/GpuService.aum` with
  `rs_gpu_open(1, -1)` and the provider from `examples/gpu/service/provider`.
  A system-linked umbrella (host Cargo/glibc) sees the host Vulkan libraries
  directly and needs only the bridge path:

  ```sh
  AVM_GPU_BUNDLE=/tmp/avm-g14.avkg \
  LD_LIBRARY_PATH="$PWD:$PWD/safestos/cranelift/target/release" \
  umbrella/target/release/umbrella --austral ./austral --cratedir /tmp/g14provider \
    --manifest /tmp/g14-manifest.toml --entry compute --expect 56 \
    umbrella/modules/GpuService.aum
  ```

  It printed `RESULT 56` and `umbrella: matches --expect 56`, exit 0. The
  Nix-linked umbrella resolves `libvulkan` through the Nix loader, not the
  host's `/lib`, so it needs the loader and Mesa ICD explicitly:

  ```sh
  LOADER=$(nix build --no-link --print-out-paths nixpkgs#vulkan-loader)
  MESA=$(nix build --no-link --print-out-paths nixpkgs#mesa)
  AVM_GPU_BUNDLE=/tmp/avm-g14.avkg \
  VK_DRIVER_FILES=$MESA/share/vulkan/icd.d/intel_icd.x86_64.json \
  LD_LIBRARY_PATH="$PWD:$PWD/safestos/cranelift/target/release:$LOADER/lib:$MESA/lib" \
  umbrella/target/release/umbrella --austral ./austral --cratedir /tmp/g14provider \
    --manifest /tmp/g14-manifest.toml --entry compute --expect 56 \
    umbrella/modules/GpuService.aum
  ```

  That run also printed `RESULT 56` and exit 0. The adapter was the Intel UHD
  630 used by the G10/G11 records; `require_hardware=1` means this is a device
  launch, not the reference evaluator. A deployment without a visible Vulkan
  loader/ICD gets `-6`, never a silent fallback. Service handles are raw
  `Int64`; G15's `stdlib/gpu/Jobs` is the linear v2 surface and is recorded in
  its own row. This v1 launch and its grant denial are unchanged by G15.

  The self-hosted `gpu-required` job runs `scripts/gpu-service-jit` after the
  device corpus. On a Nix-linked runner it needs
  `AVM_GPU_JIT_LD_LIBRARY_PATH` pointing at a Vulkan loader and ICD (the same
  deployment requirement as the manual record); the script fails rather than
  skipping when no adapter is visible.
- wgpu/Naga device support is optional (`--features device`) and is not part of
  the default compiler build.
