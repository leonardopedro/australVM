# 023 — Liquid index closure (WF3)

`PLAN_liquid_types.md` L11a: a contract's index function must be
total-fragment-admissible, and "transitively" means the *whole closure* — an
unannotated helper reached from a contract is audited too. The compiler pass is
`lib/liquid/LiquidIndexClosure.ml`; the refusal names the rule, the typed call
path and the offending statement's source location.

These cases go through the ordinary end-to-end path, so they pin the diagnostic
a user sees rather than the module's internals:

- `001-accept-pure-helper` — a contract calls a pure, acyclic helper. It must
  compile, run and print.
- `002-refuse-loop-in-helper` — the helper contains a `while`, which the
  fragment excludes; the message must name `spin` and the call path.
- `003-refuse-embed-under-an-argument` — the forbidden `@embed` sits in a
  *nested* call argument. The refusal must blame `peek`, the inner function,
  because reporting the outer call would name the wrong definition.
