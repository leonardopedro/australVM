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
cargo test --locked --manifest-path safestos/gpu/Cargo.toml
cargo test --locked --manifest-path safestos/gpu/Cargo.toml --features device
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
| G09 | implemented | `safestos/gpu/src/device.rs`, optional `wgpu =29.0.4` | `cargo test --features device` (13 tests); `cargo run --features device -- probe`; `avm-gpu probe` → `{"backend":"Vulkan","name":"Intel(R) UHD Graphics 630 (CFL GT2)","device_type":"integrated-gpu"}`, exit 0; `probe --require-hardware` exit 0 | adapter evidence is from wgpu enumeration, not `nvidia-smi`; no kernel has been dispatched yet |
| G10–G23 | pending | — | — | ordered extensions; no GPU execution claimed |

## Limitations

- No task above claims GPU execution except where an adapter record exists; none
  does in this baseline.
- `nvidia-smi` output is inventory evidence only. G09's probe currently
  enumerates a Vulkan **Intel UHD Graphics 630** (integrated) adapter and does
  not see the GTX 1060, so adapter evidence and OS inventory disagree; the
  wgpu record is the one that counts. No dispatch/readback record exists, so
  G10's execution acceptance remains open.
- wgpu/Naga device support is optional (`--features device`) and is not part of
  the default compiler build.
