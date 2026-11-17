# rust_kv — capability demo status

`examples/modules/rust_kv` is the cap-std capability fixture named in
`docs/UMBRELLA_PRODUCTION_REVIEW.md` (UR01). It currently contains a manifest,
granted data and this status demo. It has **no Rust implementation**, so it has
no capability execution to demonstrate.

## What `run_demo.sh` reports

`./run_demo.sh` checks the manifest declarations (`name`, `archetype`, `entry`,
`fs` grant) and then reports the capability stage as `UNSUPPORTED`, naming the
path that remains uncovered:

```
UNSUPPORTED: capability execution is not implemented
   uncovered path: examples/modules/rust_kv has no Rust cdylib, so no
   read of data/store.jsonl, no RESOLVE_BENEATH traversal refusal and no
   hot-swap grant decision was performed.
```

It exits **3** (not implemented). It never prints `PASS` or `ALL TESTS PASSED`
while that stage is uncovered, and it does not build anything: the previous
script's optional `modhost` build was not evidence of a capability operation and
has been removed rather than kept as a stand-in.

The previously printed transcript is kept in
`fixtures/printed_success.txt`. It is not a success path: `HAR02` requires the
guard to reject it.

## Existing tests this demo references

These are real unit tests, and the demo does not run them, so they are not
presented as demo execution:

- `safestos/cranelift/tests/capstd_tests.rs` — `CapFs` reads and the
  `RESOLVE_BENEATH` traversal refusal, with the `capstd` feature enabled
  (`cargo test --features test-stubs,capstd --test capstd_tests`).
- the umbrella manifest swap-grant set comparison (it compares grants; it does
  not enforce IO).

## The guard

`scripts/test-umbrella --case HAR02` is the acceptance case:

1. the real demo must exit nonzero, make no success claim, and name the
   uncovered capability path;
2. the printed-success fixture must be rejected by the same oracle;
3. a mutated copy of the script whose uncovered-stage function is replaced by
   `echo "PASS: capability execution"` must be caught — if the mutation stops
   producing a success claim the guard itself fails, so it cannot pass vacuously.

## What replaces this

U22 supplies the real pipeline: a compiled module that reads `data/store.jsonl`
through cap-std, refuses `../../etc/passwd`, and rejects a hot-swap that
escalates the `fs`/`net` grant sets. That is what will make a success result
mean something; adding a fake Rust implementation here would not.
