# 027 — liquid descent (L11e)

The opt-in fragment from `docs/LIQUID_BEND_PLAN.md` §L11e: a function marked
`pragma Liquid_Descent` promises that every recursive call consumes one
immutable inductive argument as a proper constructor subterm. The checker
recognises that relation in the typed tree (including the temporaries typing
introduces for a `case` scrutinee), keeps L3's T1 for every other edge, and
rejects effects, rebuilt values and a self-call on the original argument.

- `001-agrees-with-fold` runs the ordinary CPU path (`--target-type=c`) and
  prints the same measure from a `Liquid_Fold` function and a `Liquid_Descent`
  function. The recursive-union fixture used by the checker cannot be emitted
  as C — its by-value tail has no C representation — so the executable
  correspondence is measured on the Int64 measure the fold combinator already
  uses. The proper-subterm cases themselves are `test/LiquidDescentTest.ml`,
  at `--target-type=tc`.
- `002-refuse-original` and `003-refuse-rebuilt` are the two negative cases:
  the same recursive union with the self-call changed to the original value
  and to a freshly constructed value. Both are refused at the typed pass,
  before codegen.

`AUSTRAL_LIQUID_DESCENT_EVIDENCE=<path>` makes the compiler write a
line-oriented evidence record: the extracted rank sanity values, the index
evaluator's step budget, the fold function in the module, and one line per
marked function with its resolved subterm binder, self-call count, deepest
case nesting and verdict. Those numbers are measurements, not a termination
proof; the decrease relation is syntactic and the decreasingness theorem is
the extracted `liquid_measure.mlw` obligation.
