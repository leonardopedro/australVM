# Seed note: harness waterfall + guards (H5/H6/H9)

**Category**: feature
**Date**: H5–H9 stages
**Status**: implemented

## What
The loopback chokepoint is a composable waterfall of `symbol/*` listeners
(emit/waterfall/parallel/serial): audit → grant → latch → meter → posture →
guard → dispatch. Each stage added one listener without replacing the C ABI:
- H5: waterfall refactor-of-record.
- H6: deadline guard (UK-4603) composing with the meter.
- H9: strict-posture pause (UK-4501) reusing the S21 approval lane.

## Why
S21/S25/S26 were `if`-chains that could not be composed or tested in isolation.
The waterfall pins registration order = enforcement order with a regression
test, and the emitted UK codes stayed byte-identical.

## How verified
- `cargo test --features "ecmascript,test-stubs" --lib` (83 tests, incl.
  `loopback_listener_registration_order_is_enforcement_order`).
- `loopback_meter_and_guard_compose_with_disjoint_vocabularies`,
  `strict_posture_pauses_mutators_and_admits_enders`.

## Frozen
This note is archived and frozen (dsh notes policy).