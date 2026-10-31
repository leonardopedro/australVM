# mm0-pairing

The verified `.mm0` companion for the LOGOS CNL certificate, plus the
generator and the regression runner that pin down the **mm0/mm1
statement-pairing rule**.

## The pairing rule (empirically verified with mm0-c)

An `.mm0` file is the human-readable *spec* paired against the proof-carrying
`.mmb` binary. `mm0-c MMB < MM0` cross-checks them:

| `.mm1` source | `.mmb` stream | `.mm0` statement required? |
|---|---|---|
| `sort` / `provable sort` | `Sort` (flags checked) | yes, in stream order |
| `term` / `def` | `Term/Def` | yes, with bodies |
| `abstract def` | `Term/Def` | yes, **bodyless** `def` (no `abstract` keyword) |
| `axiom` | `Axiom` | yes |
| `pub theorem` | `Thm` | yes, statement only (proofs stripped) |
| plain `theorem` | `LocalThm` | **no** - must not appear at all |
| `local def` | `LocalDef` | **no** - must not appear at all |
| `delimiter`, `notation`, `prefix`, `infixl`/`infixr` | not in stream | consumed, never paired - but their content still matters, because later statements must parse (dropping `notation sb` breaks `peano5`'s `[0 / x]`) |
| lisp forms, `do` blocks, `@...` annotations, `--` comments | none | dropped |

Ground truth: `peano.mm0`'s 55 `theorem` statements exactly equal
`peano_ref.mmb`'s 55 public `Thm` entries, and its axioms/terms match
one-for-one.

## Files

| file | role |
|---|---|
| `gen_mm0.py` | generates an `.mm0` from an `.mm1` (public statements only, stream order) |
| `verify_pairing.py` | one-shot runner: regen + positive + rebuild-from-source + 10 negative controls + 1 tolerated variant |
| `full2.mm1` / `full2.mmb` / `full2.mm0` | the verified pair: 61 public statements (2 sort, 32 term/def, 22 axiom, 5 theorem), 563 local thms, 2 local defs. The 5 theorems are `ifppos`/`ifpneg` plus the 3 `pub theorem logos_cnl_*` certificate claims, which are proved entirely by local `addcom`/`add01`/`addS1` induction proofs |
| `head2.mm1`, `prelude.mm1`, `build_prelude.py` | provenance: the parts `full2.mm1` was assembled from |
| `cnl_goal.mm1`, `ind_test.mm1` / `.mmb` | CNL goal shape and the induction-proof variants (their theorems stay local inside `full2.mm1`) |
| `peano_ref.mmb` | `mm0-rs compile` of upstream `examples/peano.mm1` (ground-truth reference) |

## Reproduce

Requires `mm0-c` and `mm0-rs` on `PATH` (build from github.com/digama0/mm0).

```sh
python3 verify_pairing.py
```

Expected: every check reports `[ok]`, ending with
`16/16 checks passed - pairing rule verified` (exit 0).

What it asserts:

1. `gen_mm0.py` deterministically reproduces the checked-in `full2.mm0`.
2. `mm0-c full2.mmb < full2.mm0` passes.
3. A fresh `mm0-rs compile full2.mm1` also verifies against the checked-in
   `.mm0` (source/binary/spec mutually consistent).
4. Ten broken variants are **rejected**: axioms removed, `ax_1`/`ax_2`
   swapped, public theorems removed, an extra local theorem statement
   inserted, CNL claims removed, `infixl eq` moved after its uses, public
   `def eq` dropped, `delimiter` dropped, sort modifier weakened, and
   `notation sb` dropped.
5. One variant is **accepted** (adjacent notation lines swapped), pinning
   the property that notation lines are never stream-paired. Notably
   tolerated too, but not asserted: a bodyless `def` where the `.mmb` has a
   body (mm0-c does not cross-check def bodies).
6. Optional: upstream `peano.mm0` pair (set `MM0_EXAMPLES` if the mm0
   checkout is not at `/tmp/mm0`).
