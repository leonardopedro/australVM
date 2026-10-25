# memsafe: the linearity ledger and its MM0 certificate

**Category**: feature
**Date**: 2026-10-25
**Status**: implemented

## What

`LinearityCheck` now emits a *ledger* of its linear-resource and borrow
decisions (`--emit-linearity-ledger=<path>`, `lib/LinearityLedger.ml`), and
`scripts/ledger2mm0` turns that ledger into MM0 proofs over
`docs/memsafe.mm0`, re-verified by the frozen `mm0-c` root of trust.
`docs/memsafe.md` is the plan rewritten against the tree (`docs/lean4mm0.md`'s
shape): what the generic Strategy-B plan proposed, what already existed, what
this iteration added, and what is deferred with its prerequisite named.

## Why

The checker was a complete linearity *and* borrow checker that discarded every
decision it made. That made it unfalsifiable from outside: the only evidence a
module was linear-safe was the absence of an error. The ledger makes those
decisions first-class facts, and the certificate makes them machine-checked by
a verifier nobody in this repo can edit informally.

Two decisions keep it honest rather than decorative:

- **Branch paths.** The checker walks both arms of an `if` from the same
  table, so a flat trace would show a double consumption. Arms are recorded as
  paths, and resolution emits one timeline per (variable, maximal path) — so
  branch consistency, previously a table comparison in OCaml, becomes a
  property each path's certificate has to satisfy.
- **No grounded class.** Every theorem is a `run_cons` chain; the artifact
  pins `trust-split: structural=17 grounded=0`. Unlike the Lean-side
  certificates there is nothing the emitter evaluated and the verifier merely
  echoed.

## How verified

- `python3 scripts/ledger2mm0 verify` — drift, `mm0-rs` (`4 sorts, 33
  term/def, 30 ax/thm`), `mm0-c` (exit 0), and the negative anchor: a proof
  paired with a mutated timeline is rejected (`failed to unify: trans st_idle
  e_cons ?n =?= trans st_owned e_cons st_idle`).
- `python3 scripts/ledger2mm0 selftest` — the emitter's audit refuses a trace
  whose last event was dropped (resource never released); no toolchain needed.
- `test/LinearityLedgerTest.ml` — 9 cases: scope inheritance, arm separation,
  maximal-path only, nested branches, JSON shape, and the two wire contracts
  (`event_token` ↔ emitter, `event_rule` ↔ theory).
- `scripts/verify-invariants` — eight `memsafe:` items, including "the pinned
  ledger is in sync with the fixture and the checker" (regenerates from
  `mm0/fixtures/linearity_ledger.aum`) and "the fixture exercises every
  automaton edge" (derived from `docs/memsafe.mm0`, so an unreachable
  transition fails the gate). 52 passed / 0 gaps / 0 failed.
- CI: the OCaml `test` job re-derives the ledger (the only job that builds
  `austral.exe`); the `phase-d` job runs `ledger2mm0 verify` (the only job with
  the mm0 toolchain).

## Findings recorded in the artifact

- `t_unbor_idle` exists because `TBorrow`'s second branch resets the original
  to `Unconsumed` when an inner read borrow ends while the outer is live — the
  checker does not count borrow depth. The idempotent edge makes the genuine
  trace checkable without enabling any new `consume`; the looseness is written
  down in `docs/memsafe.mm0`'s header, `docs/memsafe.md` §5.1, and the
  emitter's `EDGES` table rather than smoothed away.
- A write-reference *parameter* is discharged by the function's exit, not by a
  scope: without a hook at the end of the body its ledger showed it as never
  released (docs/memsafe.md §5.2).

## Not covered

The certificate is about the trace, and the trace comes from the (untrusted)
checker: parser and C-lowering bugs are outside this argument. Liquid
refinements stay with the Why3/z3 gate (`docs/LIQUID.md`), and a C99 semantics
is deferred with its prerequisite named in `docs/memsafe.md` §7.
