# Seed note: loopback waterfall architecture

**Category**: architecture
**Status**: implemented

The loopback chokepoint models each call as a `symbol/*` event through
registered `SymbolListener`s. Registration order = enforcement order
(audit → grant → latch → meter → posture → guard → dispatch), pinned by test.
A listener delegates via `next()` or owns a decision by returning a terminal
`Flow`. Dispatch modes: waterfall (first owner wins), parallel (most
restrictive), serial (last terminal). See ecma.rs.
