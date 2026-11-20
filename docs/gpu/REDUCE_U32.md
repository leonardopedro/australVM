# `map-sum-u32` — the fixed reduction (G22a–G22c)

G22a defines the one reduction the compiler will emit as a WGSL kernel and
checks its *CPU schedule*; G22b stages the WGSL and audits its structure;
G22c dispatches it, reads the final word once and compares it with the G22a
oracles on a physical adapter. The point of fixing the definition first is
that the emitter, the Rust schedule and the C oracle then have one arithmetic
to agree with, instead of each inventing its own pass counts and tail rule.

Measured on 2026-11-20 (RTX 1060, Vulkan): all nine pinned sizes, including
the four-pass 1 048 576 case, equal the C oracle bit for bit (`run-reduce
exit 0`, `dispatch_count` 0/1/2/2/2/2/2/3/4).

## The operation

`map-sum-u32` — the `--gpu-operation` value G22b added to the WGSL target —
compiles one `Nat32 → Nat32` mapper from the Austral source and computes an
explicit **modular sum modulo 2³²** over the mapper's image of a validated
input array:

```
mapSum(f, [x0 … x(n-1)]) = Σ f(xi)  (mod 2³²)
```

- **Identity:** `0`. Empty input returns `0` without creating a zero-size GPU
  binding and without calling the mapper.
- **One word out**, one word per partial in between. There is no scalar
  readback per element and no CPU fold of the full array.
- **Wrapping add**, exactly the `Nat32` modular-add semantics the ordinary
  C/JIT paths use; the oracle uses `f(x) = 3x + 1`, so `f(0) = 1 ≠ 0`. A
  short-tail implementation that *maps* padding instead of adding the identity
  therefore changes the sum — that is the control, not a coincidence.

## The schedule

The partition is

```
i = 64 * group + lane,    0 <= lane < 64
```

so `(group, lane) → i` is total on the valid index space and each valid index
has exactly one pair. A pass over `n > 0` elements produces `ceil(n/64)`
outputs. Pass 0 reads the caller's input and applies the mapper; every later
pass reads the previous pass's partial sums and only adds. A pass that
produces one element is the last one.

| input `n` | pass outputs |
|---|---|
| 0 | *(no pass)* |
| 1 … 64 | `[1]` |
| 65 … 4096 | `[ceil(n/64) …]` e.g. 65 → `[2,1]`, 4096 → `[64,1]` |
| 4097 | `[65,2,1]` |
| 1 048 576 (v1 max) | `[16384,256,4,1]` |

At most **four passes** are needed at the v1 maximum; a singleton still runs
pass 0, and empty input runs none. The last tile of a pass is short: its
missing lanes contribute the identity and are never mapped.

The host derives every count from the *validated* input with checked
arithmetic and refuses an input over 1 048 576 elements. Byte capacities are
`count × 4`; the total allocation is the caller's input plus every pass's
output, with distinct storage per pass (nothing is aliased or reused). Scratch
is one workgroup of 64 `u32` words (256 bytes), independent of `n`.

## Artifact identities

The reduction artifact (`reduce.json` + `reduce.wgsl`, schema
`austral.gpu.reduction.v1`) carries the **program identity**: the mapper IR
and its digest, the `wrapping-add-u32` operator and its identity, group size
64, 256 bytes of scratch, the maximum input length, both entry names/roles
and digests, and the program text/digest. A v1 map-only consumer is rejected
by the profile (`reduce-u32-v1`), not silently interpreted as a helper;
ordinary v1 bundle bytes and their validation are untouched by this card.

The **launch plan** is not in the artifact. The host derives pass counts,
extents and roles from the validated input with `Gpu_reduce.schedule` (and
`reduce::derive_plan` in Rust), so a bundle cannot assert its own counts. The
roles are `mapper-input` for pass 0 and `partial-input` for later passes.

## G22b — the staged WGSL and its audit

`map-sum-u32` publishes `reduce.json`/`reduce.wgsl` instead of the pointwise
`kernel.json`/`kernel.wgsl`; publishing runs `avm-gpu validate-reduce` on the
staged directory and refuses to rename it into place unless the checker's
schema, IR digest, WGSL digest and program digest agree with what was staged.

The shader has two entries in one module, each a 64-lane workgroup:

- `map_sum` (pass 0) reads `input0[i]`, guarded by
  `if (i >= arrayLength(&input0))`, storing the identity `0u` in a tail lane
  and `mapper(input0[i])` otherwise;
- `add_partials` (later passes) performs the same guarded load but stores
  `input0[i]`; it never calls the mapper again.

Each entry then runs strides `32,16,8,4,2,1` (`if (lane < stride)` combines
`scratch[lane]` with `scratch[lane + stride]`), with one `workgroupBarrier()`
after the load and after every stride, and lane 0 alone writes
`output0[wid.x] = scratch[0u]`. Scratch is `array<u32, 64>`. No lane can skip
a collective and no group writes more than one slot.

Naga validates both entries. That is necessary but not sufficient: a shader
can drop a guard, skip a barrier, change a stride or let every lane write and
still type-check. `safestos/gpu/src/reduce_audit.rs` therefore checks the
fixed structure directly — entry names/count, workgroup size, binding roles,
scratch, guard and identity counts, barrier count (14), stride order and
multiplicity, the single lane-0 group write and the absence of
`output0[lane]`. Its tests mutate one property at a time and require the
audit to name it: removed load guard, skipped barrier, wrong stride,
`lane == 1u` writer, per-lane group write, and a `read_write` input binding.
The pointwise wrapper audit is untouched; this profile has its own audit.

Measured 2026-11-20: `dune exec test/GpuReduceWgslTest.exe` → 4 tests OK
(entries/workgroup, guard/strides/barriers, determinism + WGSL digest,
metadata is a program not a plan); `cargo test --offline --lib` → 41 passed
(7 of them audit mutations); publishing
`examples/gpu/reduce/Reduce.aum --entrypoint=Reduce:mapper` succeeds and the
emitter's `program_sha256` equals the checker's `026a410a…`.

## G22c — dispatch and the measured run

`avm-gpu run-reduce REDUCTION --inputs FILE` reuses provider selection, error
scopes and limits. Preflight derives the schedule and checks every pass's
extent, the input binding, the group count and the workgroup size against the
adapter. One pipeline per entry is created from the validated module; each
pass binds the previous pass's output as its input, all passes are encoded in
one command buffer and submitted once in order, the final word is copied to a
tiny staging buffer, and completion is observed before the single readback.
Intermediate buffers are retained until then, so a failing run keeps its
comparison artifacts. Empty input requests no device dispatch and returns the
identity.

Measured on 2026-11-20 with `avm-gpu run-reduce … --require-hardware
--adapter-index 1` (NVIDIA GTX 1060, Vulkan — a physical discrete adapter):

| n | passes (`dispatch_count`) | device output | C oracle |
|---|---|---|---|
| 0 | `[]` (0) | 0 | 0 |
| 1 | `[1]` (1) | 37036 | 37036 |
| 63 | `[1]` (1) | 264878151 | 264878151 |
| 64 | `[1]` (1) | 3737067680 | 3737067680 |
| 65 | `[2,1]` (2) | 2287662604 | 2287662604 |
| 4095 | `[64,1]` (2) | 3539391591 | 3539391591 |
| 4096 | `[64,1]` (2) | 1596073984 | 1596073984 |
| 4097 | `[65,2,1]` (3) | 3321096364 | 3321096364 |
| 1048576 | `[16384,256,4,1]` (4) | 2720530432 | 2720530432 |

The 65-element run moved 272 bytes in two passes and reported prepare 186.1
ms (first-use pipeline creation), submit 0.30 ms and readback 0.11 ms; these
are single cold observations, not a benchmark or a throughput claim.

## The four-elements-per-lane variant (`map-sum-u32-x4`)

A second operation, not a replacement: `--gpu-operation=map-sum-u32-x4`
selects profile `reduce-u32-x4-v1`, and both artifacts remain comparable. A
group still has 64 lanes but covers 256 elements: lane `lane` of group `group`
maps `i = group*256 + j*64 + lane` for `0 <= j < 4`, accumulating four guarded
loads into one `acc` before the same 32,16,8,4,2,1 tree. A short tail skips an
out-of-range chunk, so the identity is added for exactly the missing indices
and no padding is ever mapped. The pass counts shrink accordingly:

| input `n` | lane1 outputs | x4 outputs |
|---|---|---|
| 0 | none | none |
| 1 … 256 | `ceil(n/64)` | `[1]` |
| 257 | `[5,1]` | `[2,1]` |
| 4097 | `[65,2,1]` | `[17,1]` |
| 1 048 576 | `[16384,256,4,1]` | `[4096,16,1]` |

The structural audit has its own x4 form (four guard/chunk pairs, offsets
64/128/192, the same barrier/stride/write checks) and its tests mutate each
property. The emitter suite pins the offsets, because a first version emitted
`i0 + 64` three times and produced a wrong sum at N=65; the audit now refuses
that shape.

Measured 2026-11-20 on the GTX 1060 (Vulkan, physical): all nine pinned sizes
plus the boundary sizes 255, 256 and 257 agree bit for bit with the C oracle
(or the serial oracle for the boundary rows, which the pinned table does not
contain); 1 048 576 uses three passes instead of four. `cargo test --offline
--lib` → 56 passed (x4 schedule, tail control, partition coverage, x4 audit
mutations).

## Comparison with map + full readback + CPU fold

`avm-gpu compare-reduce REDUCTION --map BUNDLE --inputs FILE [--iterations N]`
runs the staged reduction and the pointwise mapper on the same adapter, reads
back the whole mapped array, folds it on the CPU with wrapping add, and
requires the two to agree. `--iterations` re-submits each prepared pipeline so
the reported `warm_submit` is warm; the first submission is included in
`submit_total`.

Measured at N = 1 048 576, 5 iterations, GTX 1060:

| path | bytes moved | warm submit | readback | passes |
|---|---|---|---|---|
| `map-sum-u32` | 4 260 888 | 0.1606 ms | 0.4956 ms | 4 |
| `map-sum-u32-x4` | 4 210 760 | 0.1289 ms | 0.2265 ms | 3 |
| map + full readback + CPU fold | 12 582 912 | 0.0688 ms | 4.9356 ms | 1 |
| (x4 comparison run) | 12 582 912 | 0.0690 ms | 5.3719 ms | 1 |

The reduction moves about a third of the bytes and its readback is one word
instead of four megabytes; the pointwise map's single dispatch has a lower
submit time. These are single observations, not a benchmark.

## The hardware lane

`scripts/gpu-reduce-hardware` compiles both reductions plus the pointwise map,
then for 12 sizes (including 255/256/257 and the maximum) runs
`reference-reduce` and `compare-reduce --require-hardware --iterations 5`,
requiring the reference to match the pinned table where a row exists and the
device, the fold and the reference to agree everywhere. A missing adapter is
`[gap]` exit 77; `scripts/check-gpu required` runs this lane after its pointwise
corpus and converts the gap into failure. `scripts/check-gpu source` publishes
both artifacts and validates them with the CPU checker, so the artifact and
audit path is exercised even where no GPU exists. `.github/workflows/gpu.yml`
greps both `G22b reduction bundle validated (…)` lines in the source job and
both `G22c reduction hardware …` lines plus `reduction hardware lane complete`
in the self-hosted GPU job.

## Oracles and the checks

Three implementations read the same definition:

- `lib/gpu/Gpu_reduce.ml` — the plan: `schedule`, `capacity_bytes`,
  `total_bytes`, `partition_index`, `program_text`/`program_id` and
  `launch_plan_text`/`launch_plan_id`.
- `safestos/gpu/src/reduce.rs` — a serial oracle (`serial_map_sum`) and the
  tiled schedule (`tiled_map_sum`), the plan validator, and the shared tests.
- `examples/gpu/reduce/` + `scripts/gpu-reduce-u32` — the C backend oracle: an
  Austral mapper and an independent serial modular sum, compared against
  `expected.tsv`.

The generator is fixed and documented so all three evaluate the same input:

```
value(i) = i * 2654435761 + 12345  (mod 2³²)
f(x)     = 3x + 1                  (mod 2³²)
```

`expected.tsv` is the pinned serial sum for `0, 1, 63, 64, 65, 4095, 4096,
4097, 1048576`; `0` and `1` are hand-checkable (`0`, and `3·12345+1 = 37036`).

Measured on 2026-11-19 (x86_64):

```
python3 scripts/gpu-reduce-u32
[pass] G22a C oracle: 9 sizes (0, 1, 63, 64, 65, 4095, 4096, 4097, 1048576)
       agree bit-for-bit with the pinned serial sum

dune exec test/GpuReduceTest.exe
Ran: 8 tests … OK            # scheduling, refusals, capacities, coverage, ids

cd safestos/gpu && cargo test --offline --lib reduce
running 10 tests … test result: ok. 10 passed
```

The Rust tests cover the tiled schedule against the serial oracle at every
size, the padding control (`mapping_padding_would_change_the_sum`), partition
coverage, each plan refusal, no mapper call on empty input, and the C oracle
table for both the serial and the tiled implementations. The OCaml tests pin
the pass lists, the four-pass bound, the capacities, the partition and the two
identities. `scripts/check-gpu source` runs the C oracle, so CI exercises it.

## What this does not claim

- The structural audit checks the emitted text, not a general correspondence
  between WGSL and the ISA: it is evidence about *this* fixed collective, not
  a shader semantics proof.
- The device table is one physical adapter and one input per size. It is not a
  benchmark, a bandwidth measurement or a claim about other GPUs; timings are
  single cold runs and include first-use pipeline creation.
- The four-elements-per-lane variant is implemented as its own operation; it
  repeats the coverage and tail checks and is measured, but only as one more
  fixed reduction — not as a general tuning space.
