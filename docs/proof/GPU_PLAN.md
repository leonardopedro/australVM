# PC06 — the composition applied to G13's memory plan

[COMPOSITION.md](COMPOSITION.md) proves the shared composition
`compose_bound`: two equality edges transport any bound `b`. This page records
the second application of that theorem, to G13's GPU memory plan, and states
exactly what the application does and does not discharge.

Specification: `mm0/proof_chain/gpu_plan.mm0`; proofs: `.mm1`;
checker: `scripts/pc06-obligations`.

## The plan's edges

G13's admitted plan moves one index from the invocation to the buffer slot:

```text
edge_plan_flat i j = (i = j)      -- flattening preserves the index
edge_plan_slot j t = (j = t)      -- the slot store writes it unchanged
```

Both are *definitions*. With them, PC03's `compose_bound` gives the composed
write bound at an arbitrary launch size:

```text
plan_write_bound        : edge_plan_flat i j → edge_plan_slot j t → i ≤ n → t ≤ n
plan_write_bound_256    : the same at i ≤ 256 → t ≤ 256
```

`plan_write_bound_256` is the same proof application as `plan_write_bound`, so
the launch bound is a parameter of the shared theorem. The checker requires the
two statements to be identical after renaming the bound, and requires both the
clamp artifact (`compose_discharged`) and the GPU plan to apply
`compose_bound` — one shared algorithm theorem, two lowering artifacts, plus
the same artifact at a new concrete bound.

The assembled theory is `3 sorts, 19 term/def, 29 ax/thm`; `gpu_plan.mm0` adds
no axiom and the assembled specification is checked by `mm0-c`.

## Residual obligations

The composition is conditional on `hb`, the plan's guard/extent fact. That fact
is **not** available in MM0: it is Why3 arithmetic over the emitted plan, or a
runtime premise recorded in the manifest. Each residual keeps its evidence
class; none is counted as an MM0 discharge.

| Id | Obligation | Evidence class |
|---|---|---|
| O1 | generated flat index is representable | Why3 goal `index_representable` (separate route, not MM0) |
| O2 | guard implies the element/index bound | Why3 goal `guard_implies_bound`; consumed as `plan_write_bound`'s `hb` |
| O3 | active distinct invocations write distinct indices | Why3 goal `distinct_active_writes` + runtime distinct invocation ids |
| O4 | every input binding has extent n | runtime premise recorded in the manifest (trusted at bind time) |
| O5 | the fresh output does not alias an input | runtime allocation premise (trusted; no alias construction in v1) |

The numbering matches `lib/gpu/Gpu_obligations.ml`'s emitted `-- obligation N:`
slots, and `scripts/pc06-obligations` fails if a slot or this table drifts.

## What this does not claim

- `gpu_memory.mm0` remains `certificate: [gap]` with `proves: 0 theorems`. The
  plan theory is attached to it by name, not admitted as its certificate: a
  compiled bundle's artifact is statements-only until its residual premises are
  discharged *and* a reflection edge ties the emitted plan to these edges.
- O1, O3, O4 and O5 are not composed here. O3 additionally needs the distinct
  invocation-id premise; O4/O5 are runtime facts with no MM0 model yet.
- The Why3 goals are evidence for the arithmetic, never an automatic MM0
  discharge. `scripts/gpu-obligations` reports the two routes separately.
- No safety or progress property beyond the single write bound is claimed.

## Commands

```sh
nix shell '.#mm0' nixpkgs#python3 -c python3 scripts/pc06-obligations
python3 scripts/pc06-obligations --plan <emitted>/gpu_memory.mm0
```

`--plan` checks an artifact emitted by `scripts/gpu-obligations emit`: it must
be statements-only, carry the five obligation slots, record its theorem count,
and stay `[gap]`. A forged `-- proves: 1 theorems` record is refused (the
checker exercises that on a synthetic artifact every run). `scripts/verify-invariants`
runs the default check.
