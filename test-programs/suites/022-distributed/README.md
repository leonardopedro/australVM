# 022 — Distributed linear wrappers

D17's compile-time half. Each fixture copies `stdlib/net/Distributed.{aui,aum}`
and feeds the same module a different forbidden use, so the pinned diagnostic is
the wrapper type's contribution rather than a generic syntax error:

- `001-copy` — assigning a `PendingRequest` to a second binding and using both.
- `002-use-after-submit` — using the original binding after `submit` consumed it.
- `003-double-receipt` — using the original binding after `wait` consumed it.
- `004-wrong-state` — calling the receipt accessor on a pending request.
- `005-leak` — returning with a live request instead of disposing it.

The positive half is `scripts/check-distributed-wrappers`: it runs
`examples/distributed/RequestTransport.aum` through the CPS JIT and the bridge,
then runs the host authority's forged-ID, wrong-owner, wrong-deployment,
retired-epoch and replayed-token tests. The umbrella-hosted positive module is
`umbrella/modules/DistributedRequest.aum`, exercised by
`umbrella/tests/distributed_request.rs`.
