# 028 — Distributed request state

D16's first total transition: a finite request lifecycle (`Idle`, `Pending`,
`Applied`, `Rejected`) with explicit `Submit`, `MatchingResponse`, `Retry` and
`CancelWait` events.

`001-lifecycle` copies `examples/distributed/RequestState.{aui,aum}` and runs it
through the CPS JIT (`--use-cps-jit`). The expected stdout is the same finite
event table the Rust reference in `examples/distributed/RequestStateReference.rs`
produces; `scripts/check-request-state` runs both and diffs them. A wrong request
leaves `Pending`; a wrong digest rejects; a repeated response is a no-op;
`CancelWait` keeps the request `Pending` (cancellation is not rollback); and the
`maximum_int64` vector exercises the numeric bound without arithmetic.

The transition carries a `Liquid_Requires` contract, so the existing totality
gate checks it for loops, recursion and foreign calls. The refusals live in the
existing `020-liquid-syntax` suite.
