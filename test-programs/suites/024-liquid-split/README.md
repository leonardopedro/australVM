# 024 — Liquid split pilot (L11c)

`PLAN_liquid_types.md` L11c, for the one audited range wrapper: a `Window` with
`base`/`off`/`len`, `length`/`offset` that really read those slots, and a
`split` whose body the checker verifies (`left.off = offset(w)`,
`left.len = k`, `right.off = offset(w) + k`, `right.len = length(w) - k`).

- `001-accept-split` — a literal split. `0 <= 3` and `3 <= length(w) = 8` are
  decided by arithmetic, so the call is discharged without a prover, the halves
  are consumed in the branch, and the program runs. Its `element_width`
  declaration makes the byte relation computable: `AUSTRAL_LIQUID_EVIDENCE=<path>`
  on this compile writes the L11d.1 evidence, whose ranges are
  `[10,13)`/`[13,18)` and `[80,104)`/`[104,144)` bytes
  (`test/LiquidSplitPilotTest.ml` holds it to that).
- `002-refuse-wrong-total` — the right half keeps `length(w)` instead of
  `length(w) - k`, so the halves would not partition the input. Refused at the
  definition, before any caller is checked.
- `003-refuse-unproved-precondition` — a caller whose own `Requires` only says
  `0 <= k`; `k <= length(w)` is not implied, so the split is refused. The pilot
  has no prover, and an unknown obligation is not a pass.
