# Seed note: liquid application obligations + MM0 integrity gates

**Category**: feature
**Date**: 2026-10-25
**Status**: implemented

## What

Two plan documents were re-grounded against this tree and then executed:
`docs/cas.md` (whole-terrain verification) and `docs/lean4mm0.md` (Lean →
DeltaNets → MM0).

Refinement layer (`lib/liquid/`):

- **§5.6 APPLICATION** — one `§5.6` goal per call to a contracted callee, its
  `Requires` restated over the actuals. Without it a caller could violate a
  precondition and every emitted goal still held, because the callee's contract
  entered the `.mlw` only as an axiom.
- **T5** — `TotalityCheck` states "refined bindings are immutable" itself
  (assignment and write-borrow of a contract-named parameter), on top of
  `TypingPass`'s existing parameter-assignment rejections.
- **§5.9 FOLD (result half)** — `Liquid_Invariant` became an obligation instead
  of a comment.
- **T3** — corrected in the header: structural patterns only, by construction at
  `Tast` (no guard field exists).
- **`why3_ops`** — TRL→Why3 connective translation, pinned by a golden and
  gated by `why3 prove --type-only`.
- **`pragma Liquid_Fold`** — the measure pass now honours the marker, not just a
  naming convention.
- **Arithmetic actuals** — `term_of` renders `pervasive::trapping{Add,Subtract,
  Multiply}` (both `TFuncall` and `TMethodCall`) and resolves the `TLetTmp`
  temporary the desugarer hoists a compound argument into, so `atLeast(n - 1)`
  yields `forall n:int. n >= 1 -> n - 1 >= 0`.

MM0 (`scripts/lean2mm0`, `mm0/*`):

- the trust split is a pinned budget that fails generation when it widens;
- `selftest` mutates each trust anchor and requires a refusal;
- each artifact states the theory rules its proofs actually use, and
  `verify-invariants` requires every used rule to be a declared `term`.

DeltaNets reducer fixes (`unfer/logos`): `alloc_node` no longer recycles forwarding
stubs, `commute_dup_through` terminates both outputs of each fresh Dup, and the
ΔI/ΔA insertion renames uses instead of leaving unconnected ports.

## Why

Each item was a claim the documents made and the code did not keep: an obligation
never emitted, a marker never read, a trust split counted only in prose. The rule
applied throughout is under-claiming — a missing obligation is visible work, a
fabricated one rejects a correct program — so every widening came with a gate
that fails when it drifts.

## How verified

- `dune runtest` — 18 suites green, 8 cases in `LiquidVCTest` (goldens
  `vc-callsite-and-invariant`, `vc-operators`, `vc-arithmetic`).
- `python3 test-programs/runner.py` — all e2e suites pass, including
  `020-liquid-syntax/014-application-and-invariant`.
- `scripts/verify-invariants` with Why3 on `PATH` — 38 passed, 0 gaps, 0 failed
  (`why3 prove --type-only` over every golden, extraction-match for
  `LiquidMeasure`, `lean2mm0 check`/`selftest`/`verify`).
- `cargo test -p logos` — 304 lib tests green (5 reducer tests added).

## Frozen

This note is archived and frozen (dsh notes policy).
