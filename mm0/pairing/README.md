# mm0-pairing

Verified `.mm0` companions for the LOGOS CNL certificate, plus the generator
and the regression runner that pin down the **mm0/mm1 statement-pairing
rule**.

## The pairing rule (empirically verified with mm0-c)

An `.mm0` file is the human-readable *spec* paired against the proof-carrying
`.mmb` binary. `mm0-c MMB < MM0` cross-checks them:

| `.mm1` source | `.mmb` stream | `.mm0` statement required? |
|---|---|---|
| `sort` / `provable sort` | `Sort` | yes, in stream order |
| `term` / `def` | `Term/Def` | yes, with bodies |
| `abstract def` | `Term/Def` | yes, **bodyless** `def` (no `abstract` keyword) |
| `axiom` | `Axiom` | yes |
| `pub theorem` | `Thm` | yes, statement only (proofs stripped) |
| plain `theorem` | `LocalThm` | **no** - must not appear at all |
| `local def` | `LocalDef` | **no** - must not appear at all |
| `delimiter`, `notation`, `prefix`, `infixl`/`infixr` | not in stream | consumed, never paired; only keep ones attached to public names (`notation pim`, `prefix nf` are dropped because `pim`/`nf` are local) |
| lisp forms, `do` blocks, `@...` annotations, `--` comments | none | dropped |

Ground truth: `peano.mm0`'s 55 `theorem` statements exactly equal
`peano_ref.mmb`'s 55 public `Thm` entries, and its axioms/terms match
one-for-one.

## Files

| file | role |
|---|---|
| `gen_mm0.py` | generates an `.mm0` from an `.mm1` (public statements only, stream order) |
| `verify_pairing.py` | one-shot runner: regen + positives + rebuild-from-source + 5 negative controls |
| `full2.mm1` / `full2.mmb` / `full2.mm0` | base verified pair (58 public statements, 563 local thms, 2 local defs) |
| `full3.mm1` / `full3.mmb` / `full3.mm0` | certificate pair: same + 3 `pub theorem logos_cnl_*` claims proved by local `addcom`/`add01`/`addS1` (public `Thm` 2 -> 5, `LocalThm` unchanged at 563) |
| `head2.mm1`, `prelude.mm1`, `build_prelude.py` | provenance: the parts `full2.mm1` was assembled from |
| `cnl_goal.mm1`, `ind_test.mm1` / `.mmb` | CNL goal shape and the induction-proof variants |
| `peano_ref.mmb` | `mm0-rs compile` of upstream `examples/peano.mm1` (ground-truth reference) |

## Reproduce

Requires `mm0-c` and `mm0-rs` on `PATH` (build from github.com/digama0/mm0).

```sh
python3 verify_pairing.py
```

Expected: every check reports `[ok]`, ending with
`N/N checks passed - pairing rule verified` (exit 0).

What it asserts:

1. `gen_mm0.py` deterministically reproduces the checked-in `.mm0` files.
2. `mm0-c full2.mmb < full2.mm0` and `mm0-c full3.mmb < full3.mm0` both pass.
3. Fresh `mm0-rs compile` output of each `.mm1` also verifies against the
   checked-in `.mm0` (source/binary/spec mutually consistent).
4. All five broken variants are **rejected**: axioms removed, `ax_1`/`ax_2`
   swapped, public theorems removed, an extra local theorem statement
   inserted, CNL claims removed.
5. Optional: upstream `peano.mm0` pair (set `MM0_EXAMPLES` if the mm0
   checkout is not at `/tmp/mm0`).

Original investigation result (2026-10-31): positive pairs exit 0;
all five negatives fail as expected (exit 255/1), so the checker is not
passing vacuously.
