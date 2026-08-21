# Seed note: test discipline

**Category**: testing
**Status**: implemented

The australVM test surface runs via
`cargo test --features "ecmascript,test-stubs" --lib` (83 tests). Tests that
touch kernel-global stores (audit trail, action queue, meter, vault) serialize
on per-store `*_TESTS_LOCK`s. Clippy baseline: 21 warnings (18 pre-existing
errors). `cargo fmt` is not a gate (pre-existing diffs).
