# `map-sum-u32` — the fixed reduction (G22a)

G22a defines the one reduction the compiler will emit as a WGSL kernel, and
checks its *CPU schedule* before any shader exists. Nothing here dispatches a
device: G22b emits the staged WGSL, G22c runs it and reads back the final word.
The point of fixing the definition first is that the emitter, the Rust
schedule and the C oracle then have one arithmetic to agree with, instead of
each inventing its own pass counts and tail rule.

## The operation

`map-sum-u32` — the proposed `--gpu-operation` value; the flag itself arrives
with G22b's emitter — compiles one `Nat32 → Nat32` mapper from the Austral
source and computes an explicit **modular sum modulo 2³²** over the mapper's
image of a validated input array:

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

The reduction artifact is versioned and deliberately has **two** identities:

- a **program identity** — mapper IR, the `wrapping-add-u32` operator and its
  identity, the group size, scratch/resource requirements, the maximum input
  length; this is the same for every run; and
- a **launch-plan identity** — the per-run pass counts, per-pass input/output
  extents and binding roles, derived from the validated input and pinned to the
  program identity by digest.

A v1 map-only consumer is rejected by the profile (`reduce-u32-v1`), not
silently interpreted as a helper; ordinary v1 bundle bytes and their
validation are untouched by this card.

The roles are `mapper-input` for pass 0 and `partial-input` for later passes;
a declaration that disagrees with the derived plan is refused, as are wrong
pass counts, wrong extents, a wrong identity, and an oversized input.

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

- No barrier, workgroup-scratch or dispatch semantics: those are G22b/G22c and
  are validated there, not inferred from this schedule.
- No device execution, timing or bandwidth.
- The four-elements-per-lane variant (`i = group*256 + lane + 64*j`) is not
  implemented here; it is a later tuning patch and must repeat the coverage and
  tail checks if it replaces this schedule.
