# Distributed security execution record

Plan: [PLAN_distributed_security.md](../../PLAN_distributed_security.md).
Review: [DISTRIBUTED_SECURITY_REVIEW.md](../DISTRIBUTED_SECURITY_REVIEW.md).
Contract inventory: [contracts.tsv](contracts.tsv) (validated by
`scripts/check-distributed-contracts`).

This record is the D00 baseline. It is a snapshot, not a claim: numbers below
are measured on the host named here, and a `gap` is an environment or tooling
absence with the command and log that produced it. Task cards append their own
entries as they land; nothing here may be upgraded from `gap` to a security
claim without the evidence the plan requires.

## Baseline snapshot (D00.1)

| Item | Value |
|---|---|
| Date | 2026-11-19 |
| Host architecture | x86_64 |
| Rust | rustc 1.99.0 (b940084d7 2026-09-28), cargo 1.99.0 (5f94df478 2026-08-27) |
| australVM HEAD | `98fcece5b8e8bc193a60a1d160f49f5798d243c8`, 17 dirty path(s) |
| unfer HEAD | `492cbedc49dc048ef1f614f28854f8bf89bb3757`, clean |
| Hydro checkout | absent in this workspace (named gap; the plan only reads it as a design reference) |
| Proof tools on PATH | `why3` absent, `mm0-rs`/`mm0-c` absent (present only in the Nix store); `gcc` and `nix` present |
| Bridge features | default `cedar,unfer-kernel,tier1-rs`; `test-stubs` for tests |
| `UNFER_SYMBOLS` | 92 entries, all `uk_*` (measured from `safestos/cranelift/src/lib.rs`, not copied from old prose) |
| `ZENODO_SYMBOLS` | 5 entries, all `uz_*` |

The historical "21 `uk_*`" figure in older plans is stale; the registry census is
the source of truth and the plan's instruction to check schemas against source
is why this baseline records the measured counts.

## Baseline commands (D00.2)

Each row is a command actually run from the stated repository. `exit` is `gap`
when the environment, not the code, prevented a result; the log names the
cause. The checker validates this block's schema, uniqueness and evidence files.

<!-- baseline:begin -->
id	command	exit	evidence
B00-1	U: cargo test --locked --offline -p unfer_protocol	0	docs/distributed/baseline/unfer_protocol.log
B00-2	U: cargo test --locked --offline -p unfer_identity	gap	docs/distributed/baseline/unfer_identity.log
B00-3	U: cargo test --locked --offline -p unfer_data	gap	docs/distributed/baseline/unfer_data.log
B00-4	U: cargo test --locked --offline -p unfer_consensus --lib	gap	docs/distributed/baseline/unfer_consensus_lib.log
B00-5	U: cargo test --locked --offline -p unfer_edge --features audit	gap	docs/distributed/baseline/unfer_edge_audit.log
B00-6	U: cargo test --locked --offline -p unfer_consensus --features network --test phase6_testnet	gap	docs/distributed/baseline/unfer_phase6_testnet.log
B00-7	A: cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs --lib	gap	docs/distributed/baseline/bridge.log
B00-8	A: git diff --check	0	docs/distributed/baseline/git-diff-check.log
<!-- baseline:end -->

### Named gaps

- **`cc v1.6.0` is not in the offline Cargo cache.** B00-2 … B00-6 stop before
  compiling: `attempting to make an HTTP request, but --offline was specified`.
  This is an environment gap, not a test result; the plan forbids weakening
  tests or updating lockfiles to work around it. Re-run with the dependency
  cache populated before treating those suites as measured.
- **The bridge refuses `--locked`.** `safestos/cranelift/Cargo.lock` is already
  modified in the working tree (from the merge), so `--locked` aborts before
  the build. The lockfile was not regenerated here; B00-7 is a gap until the
  owner decides whether that lock change is intended.
- **No prover or MM0 checker on the interactive PATH.** `why3`, `mm0-rs` and
  `mm0-c` are absent (mm0 tools exist under `/nix/store`). Any D20/D21 evidence
  must be produced from a shell that carries them; an absent prover is a gap,
  never a pass.
- **Hydro is not checked out.** D22/D23 can only be design references here.

## Contract inventory (D00.3)

`contracts.tsv` begins the inventory the later tasks extend: one row per
distributed boundary site, with `repo` (`A` for this repository, `U` for
`../unfer`), a repository-relative `path`, a status from
`pending|tested|gap|refused` and an evidence file or `-`. Every row today is
`pending`: D00 records the sites, it does not assert a contract over them.

`python3 scripts/check-distributed-contracts` validates the header and value
vocabulary, rejects a duplicate ID in either the inventory or the baseline
block, requires every referenced path and evidence file to exist, and re-runs
the same validation over mutated copies in `--selftest` (duplicate ID, unknown
status, missing file, duplicate baseline row, missing baseline evidence,
malformed header) so a checker that cannot fail is itself a failure.

Measured 2026-11-19:

```
$ python3 scripts/check-distributed-contracts
[pass] D00 contracts: 8 contract row(s), 8 baseline row(s) (6 named gap(s)),
       all paths and evidence files present
$ python3 scripts/check-distributed-contracts --selftest
[pass] D00 selftest: 6 mutation control(s) refused
$ # end-to-end controls: a duplicate row appended inside the block and an
$ # inventory naming docs/nope.rs each exit 1 naming the offending line
```

## Task status

| Task | Status | Note |
|---|---|---|
| D00 | complete | this record, the inventory and the checker; baseline gaps named above |
| D01–D26 | not started | D01 begins in the sibling `unfer` repository; this execution session may read it but must not write it, so the next patch needs an explicit coordinated change there |
