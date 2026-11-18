# PC02 shared word and map-memory semantics

This is the first executable semantic layer for the GPU proof chain. It gives
the source clamp, the KernelIR and the shader model one common vocabulary
without claiming the later reflection or lowering edges.

The specification is `mm0/proof_chain/word.mm0`; the checked proofs are
`mm0/proof_chain/word.mm1`. Independent finite controls live in
`scripts/pc02-controls`.

## What is in the model

The admitted scalar fragment needs three things:

1. a 32-bit unsigned value carrier with a total order;
2. an interpretation of the source `clamp` condition over that order;
3. the admitted map memory, with one value per index.

`word.mm0` declares a sort `u32` and a `ule` order with meet and join. The
order is required to be a lattice:

| Declaration | Meaning |
|---|---|
| `ule_refl` | `x <= x` |
| `umeet_lb_l`, `umeet_lb_r` | `meet x y` is below each argument |
| `umeet_greatest` | any common lower bound is below `meet x y` |
| `ujoin_ub_l`, `ujoin_ub_r` | each argument is below `join x y` |
| `ujoin_lub` | `join x y` is below any common upper bound |
| `uzero_le` | `uzero` is below every word |

`ueq` is equality with reflexivity, symmetry, transitivity and the one
rewriting rule the clamp proof uses (`ueq_sub_le`).

These are **foundational** laws of the chosen order theory. They are not
application facts: none mentions `source_clamp`, `mget`, or a GPU plan.

## Clamp denotation

The real source function is `Pointwise.clamp` in
`examples/gpu/pointwise/Pointwise.aum`:

```austral
function clamp(x: Nat32): Nat32 is
    if x > 100 then
        return 100;
    else
        return x;
    end if;
end;
```

On a total order this is `min x 100`, so the model defines:

```text
source_clamp(x) = umeet x u100
```

`u100` denotes the source literal 100. The comparison direction is visible in
the definition: the reversed source condition (`x < 100`) denotes
`max x 100`, not another name for the same function. The finite control runs
that reversed denotation and exhibits `x = 101` as a counterexample to the
upper bound.

The two PC02 source properties are proved for every element of the carrier:

```text
clamp_upper : ule (source_clamp x) u100
clamp_lower : ule uzero (source_clamp x)
```

`clamp_upper` is the meet's right lower bound; `clamp_lower` is
`umeet_greatest` with `uzero_le` for both arguments. A changed order law, a
changed literal, or a definition that no longer unfolds to the meet no longer
unifies with the proof term.

## Map memory

`Mem` is the admitted map plan:

```text
mget (mput m i v) i = v
uindep i j => mget (mput m i v) j = mget m j
```

`uindep` is a side condition, not arithmetic. It is kept abstract because this
layer has no index model and no launch-bound theorem; PC05 owns the actual
index relation. The theorems `map_read_after_write`, `map_write_frame` and
`map_write_then_read` are proved from those two definitions. The capacity-3
toy in `mm0/store_projection_toy.mm0` is a different memory model and is not
identified with this one.

The map model deliberately does **not** claim:

- that the source clamp's result is the value the shader stores;
- that `uindep` holds for a launch;
- that the guard makes every access in bounds;
- that a 32-bit overflow wraps the way the carrier does. There are no
  arithmetic operations in this layer to overflow.

Those are PC04/PC05/PC06 edges.

## Trust boundary and finite interpretation

The general proofs are parametric in any carrier satisfying the order and map
laws. The independent control `scripts/pc02-controls` interprets the carrier
over actual 32-bit unsigned values and checks, at the boundary values and a
fixed pseudo-random sample:

- `meet` is `min`, `join` is `max`, and both bounds hold;
- `source_clamp` agrees with `100 if x > 100 else x`;
- the reversed condition (`max`) violates the upper bound at `x = 101`;
- a signed interpretation of `0xFFFFFFFF` would be `-1`, but the unsigned
  order is the one used by the model; the control refuses the signed reading;
- the map write/read and frame equations hold for independent indices, and
  the same-index read is the written value.

Finite tests are controls, not the general proof. The general claim is the
MM0 theorem over the whole carrier; the control can find a bad interpretation
or a changed comparison, but cannot stand in for `mm0-c`.

## Reproduce

```sh
nix shell '.#mm0' -c bash -c '
  mm0-rs compile mm0/proof_chain/word.mm1 /tmp/word.mmb &&
  mm0-c /tmp/word.mmb < mm0/proof_chain/word.mm0
'
python3 scripts/pc02-controls
```

`scripts/verify-invariants` runs both when the tools are present, plus a
negative control that flips `clamp_upper`'s conclusion and requires `mm0-rs`
to refuse the existing proof.

## Still open

- PC02's Boolean/conditional object language is represented here by its
  denotation (`min`), not by a separate conditional evaluation relation.
- The word carrier's cardinality and bit-level representation are interpreted
  by the finite control, not proved from a foundation.
- Source reflection is PC04: this file says what `clamp` means, not that the
  parsed Austral body is that function.
- PC03 relation/trace composition is a separate layer over this one:
  [COMPOSITION.md](COMPOSITION.md) assembles `mm0/proof_chain/relation.mm0`
  after this specification and proves the conditional and discharged two-edge
  compositions. It adds no axiom and does not change anything here.
