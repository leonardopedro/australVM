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

## Baseline snapshot (D00.1, preserved: 2026-11-19, x86_64)

Kept verbatim as the first measurement. Its logs are in
`docs/distributed/baseline/2026-11-19-x86_64/`; the live paths under
`baseline/` now hold the re-measured run described below.

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

## Baseline snapshot (D00.1b, 2026-11-21, aarch64)

Same commands, a host where the offline dependency cache is populated. Nothing
was changed in either repository to obtain this: no lockfile edit, no network,
`--locked --offline` throughout.

| Item | Value |
|---|---|
| Date | 2026-11-21 |
| Host architecture | aarch64 |
| Rust | rustc 1.99.0 (b940084d7 2026-09-28), cargo 1.99.0 (5f94df478 2026-08-27) |
| australVM HEAD | `3ad01bacf073` plus this record's own edits |
| unfer HEAD | `b0aaa7c`, clean |
| Hydro checkout | `../hydro` at `92e6451bb997872928384323481bc18b22ef850b`, clean, not built |
| Bridge features | default; `test-stubs` for the lib suite |
| Proof tools on PATH | `why3`, `mm0-rs`, `mm0-c` absent (gaps below) |
| `UNFER_SYMBOLS` / `ZENODO_SYMBOLS` | 92 `uk_*` / 5 `uz_*` (unchanged from the x86_64 run) |

Test counts from that run, per log: `unfer_protocol` 4+51+2 (four targets);
`unfer_identity` 6+1; `unfer_data` 51+4+1; `unfer_edge --features audit` 84+1;
`unfer_consensus --lib` 111; `phase6_testnet` 4 (loopback TLS, 2-live-of-3);
bridge `--features test-stubs` 65. All exit 0, zero failures.

### What the installed consensus adapter actually is (D00.2 inspection)

Read from the cached `rust-quepaxa-0.1.0` and `unfer_consensus/src/net.rs`, not
from the plan's description:

- The value is **`u64`, not bytes**. `NetworkConsensusHandler<V, C, S, E>`
  requires `V: Clone + Ord + Send + Sync + 'static`
  (`network/node_runtime.rs:45,57`), and the adapter instantiates it as
  `StateMachine<u64>` with payloads held in `SharedLedger`; `NOOP_VALUE`
  (`u64::MAX`) fills committed gaps (`net.rs:47,305`). Decision 4's
  `ReplicatedValue::Command(bytes)` would therefore *replace* this adapter, not
  slot into it — D10 has to price that, and it is why the plan says to confirm
  bounds first.
- A wire codec adds serde bounds on top: `V: serde::Serialize` and
  `V: Ord + serde::Deserialize` (`replica.rs:41-42`, `runtime.rs:46-47,74-75`).
- Durable submission identity already exists in the dependency and is narrower
  than the plan's: `SubmissionJournal<V>` keys on `client_id: [u8; 16]` plus
  `request_id: u64` (`network/dedupe.rs:125`), and `ExactlyOnceExecutor` has
  `export_checkpoint`/`import_checkpoint` that **default to `StorageError`**
  (`dedupe.rs:41-55`). A durable exactly-once executor here must implement both;
  the default is a loud refusal, not a silent in-memory success.
- Replay/dedup durability in the adapter is one JSON file written by
  `write` + `rename` with **no `fsync`** (`net.rs:66-115`, `LedgerFile::save`),
  bounded at `MAX_JOURNAL_BYTES = 16 MiB` (`net.rs:51`), and a corrupt file fails
  loudly instead of resetting to empty. Fire-and-forget durability of the
  *committed* log is what D07/D13 must fix before claiming restart safety.

## Baseline commands (D00.2)

Each row is a command actually run from the stated repository. `exit` is `gap`
when the environment, not the code, prevented a result; the log names the
cause. The checker validates this block's schema, uniqueness and evidence files.

<!-- baseline:begin -->
id	command	exit	evidence
B00-1	U: cargo test --locked --offline -p unfer_protocol	0	docs/distributed/baseline/unfer_protocol.log
B00-2	U: cargo test --locked --offline -p unfer_identity	0	docs/distributed/baseline/unfer_identity.log
B00-3	U: cargo test --locked --offline -p unfer_data	0	docs/distributed/baseline/unfer_data.log
B00-4	U: cargo test --locked --offline -p unfer_consensus --lib	0	docs/distributed/baseline/unfer_consensus_lib.log
B00-5	U: cargo test --locked --offline -p unfer_edge --features audit	0	docs/distributed/baseline/unfer_edge_audit.log
B00-6	U: cargo test --locked --offline -p unfer_consensus --features network --test phase6_testnet	0	docs/distributed/baseline/unfer_phase6_testnet.log
B00-7	A: cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs --lib	0	docs/distributed/baseline/bridge.log
B00-8	A: git diff --check	0	docs/distributed/baseline/git-diff-check.log
<!-- baseline:end -->

### Named gaps

B00-2 … B00-7 were gaps on 2026-11-19 for two environment reasons, both now
resolved by measurement rather than by changing anything: `cc v1.6.0` was missing
from that host's offline cache (`error: failed to download 'cc v1.6.0'`), and
that working tree had a modified `safestos/cranelift/Cargo.lock`, so `--locked`
refused. On 2026-11-21 both conditions are gone — the cache has the crate and
the lockfile is clean — and the same commands pass. The 2026-11-19 logs are kept
under `baseline/2026-11-19-x86_64/`; they are evidence of an environment gap,
never of a failing suite.

Still open, and still gaps rather than passes:

- **No prover or MM0 checker on the interactive PATH.** `why3`, `mm0-rs` and
  `mm0-c` are absent (mm0 tools exist under `/nix/store`). Any D20/D21 evidence
  must be produced from a shell that carries them.
- **Hydro is checked out but not built.** `../hydro` is at
  `92e6451bb997872928384323481bc18b22ef850b` (`hydro_lean` on Lean 4.33.1); the
  plan reads it as a design reference only, so no build was attempted and none is
  claimed.
- **The committed-log durability gap is real, not a tool gap.** B00-4/B00-6 pass
  with the ledger's `write`+`rename` and no `fsync`; a passing suite does not
  measure crash durability. D07/D13 own that witness.

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

Measured 2026-11-21 (the 2026-11-19 run reported `6 named gap(s)`; the block
above now carries the re-measured exits):

```
$ python3 scripts/check-distributed-contracts
[pass] D00 contracts: 8 contract row(s), 8 baseline row(s) (0 named gap(s)),
       all paths and evidence files present
$ python3 scripts/check-distributed-contracts --selftest
[pass] D00 selftest: 6 mutation control(s) refused
$ # end-to-end controls: a duplicate row appended inside the block and an
$ # inventory naming docs/nope.rs each exit 1 naming the offending line
```

## Task status

| Task | Status | Note |
|---|---|---|
| D00 | complete (re-measured 2026-11-21) | this record, the inventory and the checker; all seven suites green on this host, the remaining gaps named above |
| D01–D26 | not started | D01 begins in the sibling `unfer` repository. The plan's §3 authorizes a coordinated change there for a task that names both repositories, but the patch is not written yet; its failing witness is the first substep |
