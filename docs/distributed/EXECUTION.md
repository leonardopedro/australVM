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

## D01 — identity authority bound on submit and replay (2026-11-21)

U `unfer_consensus/src/{identity,signing}.rs`, U `b0aaa7c` (clean before the
patch; the patch is the only local change). No A code changed.

### The failure, stated as the tests that now pass

`identity::tests` (all five added in this patch, each one a witness that was
red before the registry change):

| Witness | Was | Now |
|---|---|---|
| `create_cannot_claim_another_dids_name` | a create naming Alice's DID with the attacker's key was accepted, because `Create` only checked the DID was unused | refused `INVALID_SIGNATURE`; a DID is `did:unfer:<hex key>` and cannot name a key it does not own |
| `attacker_update_under_its_own_key_is_refused` | `verify_transaction` authenticates an identity op under `op.signing_key`, so an attacker-signed update *verified*; the registry then applied it, moving Alice's endpoint | refused; state compared byte for byte before and after |
| `attacker_revoke_under_its_own_key_is_refused` | same path, `Revoke` set `revoked = true` with no key check at all | refused; `resolve` still returns Alice |
| `stale_revoke_is_refused` | `Revoke` assigned `entry.seq = op.seq` unconditionally, so a replayed revoke with `seq = 2` ended a DID already at `seq = 5` | refused `DUPLICATE_TRANSACTION`; state unchanged |
| `unsigned_create_is_refused` | no signature check inside the registry, so a tampered op applied if it arrived already committed | refused `INVALID_SIGNATURE` |

### The repair

- `signing::verify_identity_op_as(op, pubkey)` verifies an identity op against a
  caller-supplied key. `verify_transaction` must trust `op.signing_key` (ingress
  has no state); the registry must not.
- `IdentityRegistry::apply_identity_op` is now the enforcement point, at the
  ordered application position recovery also passes through: create requires
  `did == did:unfer:hex(signing_key)` and a valid self-signature; update and
  revoke require `signing_key` equal to the *stored* authority, a signature by
  that authority, and a strictly increasing `seq` (revoke included). No unsigned
  rotation: an op carrying a different key is refused rather than silently
  ignored, which is what the old `Update` did with `op.signing_key`.
- Ingress verification is unchanged, and deliberately so: the attacker's op is
  authentic under its own key, which is why this belongs at the ordered point
  and why a test asserts that `verify_identity_op_as` accepts the forgery while
  the registry refuses it.

### Evidence

```
$ cargo test --locked --offline -p unfer_consensus --lib          # 116 passed
$ cargo test --locked --offline -p unfer_identity                 # 6 + 1 passed
$ cargo test --locked --offline -p unfer_consensus --features network --test phase6_testnet  # 4 passed
$ cargo test --locked --offline -p unfer_data                     # passed
```

Logs: `evidence/d01-unfer_consensus_lib.log`,
`evidence/d01-unfer_identity.log`, `evidence/d01-phase6_testnet.log`,
`evidence/d01-data.log`. All exit 0.

### What this does **not** claim

- Non-identity transactions still verify only at ingress; this patch makes the
  identity path state-dependent and leaves the other ledgers to their own tasks
  (D03, D14).
- Key rotation is refused, not implemented. Adding it is a protocol change with
  its own witness and is not smuggled in as “update”.
- `duplicate replay consistently with D05/D06`: a replayed identity op is
  refused by the DID's own sequence number, which is durable in the registry;
  the submission-level request identity (D06) is untouched.

## D02 — version and repair the encrypted content envelopes (2026-11-21)

U `unfer_data/src/{crypto,store,blueprint,publisher}.rs`, `src/lib.rs`,
`tests/integration.rs`; U `b0aaa7c` + this patch. No A code changed.

### The failure, characterized before the change

The old scheme had two defects that compound:

- `crypto.rs` built the GCM nonce as eight zero bytes followed by the chunk
  index (old lines 49-51), so the nonce is a function of the index alone.
- `store.rs::KeyRing::encrypt_envelope` used **one** epoch key for every cell
  (old line 79-83), and `blueprint.rs::encrypt_stored_cell`/`publisher.rs`
  derived one AES key per keypair with no per-object salt. There was no
  associated data at all: version, epoch, object identity, chunk count and
  length were unauthenticated, and a body could be re-attached to another
  object's metadata under the same key.

Consequence: two envelopes in one KeyRing epoch encrypted chunk 0 under the
same key and the same nonce. That is a GCM nonce reuse, not a theoretical
weakness — the commutative keystream lets a reader recover
`P1 ⊕ P2` and produce a valid tag for a modified chunk.

Legacy ciphertexts: `CellEnvelope` was never serialized with a version (it is
an in-memory struct), and the old format authenticated no metadata. Version 1
therefore cannot be distinguished or safely opened under the new derivation,
so it is **refused by name** rather than mis-decrypted; there is no silent
fallback.

### The repair

- `crypto::ObjectKey` = domain-separated SHA-256 of the epoch/transport key and
  the object identifier (§D02.2's per-object key context). The identifier is
  either fresh from `OsRng` (`ObjectKey::fresh`, `seal_object`) or derived from
  the plaintext digest for content-addressed callers (`content_object_id`,
  `seal_object_with_id`). A failed RNG read is an error, never a zero salt.
- Every chunk is sealed with AAD `version ‖ epoch ‖ object_id ‖ object_len ‖
  chunk_count ‖ index` under domain `unfer-cell-chunk-v2`. Bounds checked before
  the cipher is touched: `object_len ≤ 2^40`, `chunk_count ≥ 1`,
  `index < chunk_count`, and the chunk count must fit the `u32` counter.
- `SealedObject` carries `version`, `object_id`, `object_len`, `chunk_count` and
  the chunks, so a bare `Vec<Vec<u8>>` can no longer travel without the metadata
  it was authenticated against. `open_object` refuses a foreign version, a
  chunk-count disagreement and a reassembled length that differs from the bound
  length.
- `KeyRing::encrypt_envelope` seals each cell under a fresh object context;
  `encrypt_stored_cell`/`decrypt_stored_cell` take the same treatment; the
  publisher derives a content-addressed context and keeps its own chunk size.
  No AEAD primitive is implemented here: AES-256-GCM and SHA-256 both come from
  the existing dependencies.

### Evidence

| Witness | Asserts |
|---|---|
| `crypto::tests::two_objects_in_one_epoch_do_not_share_a_nonce` | distinct object ids, distinct chunk-0 ciphertexts for identical plaintext in one epoch |
| `crypto::tests::shared_object_context_repeats_the_nonce_which_is_the_old_bug` | negative control: with a shared object id the ciphertext *is* identical, so the test above is not vacuous |
| `crypto::tests::chunk_cannot_be_spliced_between_objects` | a chunk moved between objects fails authentication |
| `crypto::tests::epoch_and_length_are_authenticated` | replay into another epoch and a shortened declared length both fail |
| `crypto::tests::version_one_envelope_is_refused` | the legacy format is refused with the reason |
| `crypto::tests::out_of_range_index_and_oversize_object_are_refused` | counter and length bounds |
| `crypto::tests::content_addressed_sealing_derives_a_stable_object_id` | digest-derived id round-trips, different cells differ |
| `store::tests::two_envelopes_in_one_epoch_do_not_share_a_nonce` | the KeyRing-level version of the same witness |
| `store::tests::envelope_body_cannot_be_moved_to_another_object` | same length, same epoch key, foreign body refused |
| `blueprint::tests::repeated_use_of_one_keypair_does_not_repeat_a_nonce` | the `DataKeypair` path |
| `blueprint::tests::stored_cell_body_cannot_be_spliced` | body moved under one keypair is refused |

```
$ cargo test --locked --offline -p unfer_data --lib            # 62 passed
$ cargo test --locked --offline -p unfer_data --test integration # 4 passed
$ cargo test --locked --offline -p unfer_ffi --lib             # 175 passed (vault uses CellEnvelope)
```

Logs: `evidence/d02-unfer_data_lib.log`,
`evidence/d02-unfer_data_integration.log`, `evidence/d02-unfer_ffi_lib.log`.
All exit 0.

### What this does **not** claim

- No recipient/transport redesign: `DataPublisher`'s self-DH convention still
  discards the secret key, so its ciphertext is not decryptable by a third
  party. D02 fixes nonce reuse and metadata binding; key *distribution* is
  D10/D11's transport work.
- The derived (content-addressed) identifier means re-sealing the same plaintext
  under the same key reproduces the same ciphertext. That is a repeated message,
  not a nonce reused under different data; the random-id path is the default for
  store and blueprint seals.
- Envelope version 1 objects are refused, not migrated. Nothing durable stored
  them (in-memory struct), so there is no migration to write.

## D03 — authenticate capability RPC and constrain promise ownership (2026-11-21)

U `unfer_edge/src/{caprpc,main}.rs`; U `b0aaa7c` + this patch.

### What the ingress could and could not authenticate

Measured, not assumed: `unfer_edge` has **no session authentication** — the
`x-principal` header is documented as trusted (A9), and `/api/cap/*` attributed
every request to the admin principal (`admin::admin_principal()`). D03 therefore
does two separate things and says which is which:

1. **Ingress (implemented, tested):** `/api/cap/*` now refuses any peer whose
   address is not loopback, and refuses an unknown peer. `capability_peer_allowed`
   is a pure predicate, tested for IPv4/IPv6 loopback, a private address, the
   unspecified address and `None`. The real session's `client_addr()` feeds it.
   A deployment on `0.0.0.0` no longer hands out capability authority; per-session
   principals still need the S22 seam, which is a named gap, not a claim.
2. **Registry (implemented, tested):** owner checks now hold for every verb, so
   two local principals are separated even while the HTTP layer has one:
   `new_promise` records its owner and its grant ceiling; `resolve_promise` needs
   an *existing* promise, owned by the caller, on the promise's own endpoint,
   resolving once, with grants ⊆ the promise's; `revoke` needs the caller to own
   the id (or the pending promise) and returns false otherwise; the HTTP revoke
   route turns that false into 403 instead of 200.

### Witnesses (`caprpc::tests`, `main::tests`)

| Witness | Asserts |
|---|---|
| `another_principal_holds_no_authority_over_a_capability` | bob cannot resolve alice's promise, revoke alice's cap, or invoke it; alice still can, afterwards |
| `invented_or_reused_promise_id_is_refused` | a guessed id resolves nothing; a resolved promise cannot resolve again |
| `resolution_cannot_escalate_grants_or_swap_endpoint` | `admin` added to an `echo` promise is refused; another endpoint is refused; the refusals do not consume the promise |
| `revoked_promise_cannot_be_resolved` | cancelling the promise removes the authority |
| `capability_surface_is_loopback_only` | loopback v4/v6 allowed only; private, unspecified and unknown peers refused |

```
$ cargo test --locked --offline -p unfer_edge --features audit   # 89 + 1 passed
```

Log: `evidence/d03-unfer_edge_audit.log`, exit 0.

### Gap this does **not** close

`/api/cap/*` still attributes every loopback request to the admin principal.
A and B sharing loopback are only separated by the registry's owner checks, not
by credentials. That is A9's auth seam (S22), and a cryptographic session
principal is a separate task, not something D03 can honestly claim.

## D04 — make domain refusal atomic (2026-11-21)

U `unfer_consensus/src/{certs,auction,mathbond,mathbond_market,attribution}.rs`;
U `b0aaa7c` + this patch.

### The bug: a burn mutated while it validated

`apply_burn` spent each input **inside** its validation loop. A burn whose first
input was valid and whose second was nonexistent, foreign, repeated or
overflowing returned `Err` after the first input was already removed from the
UTXO set, recorded as spent and deleted from the SMT — a refused operation that
changed consensus state. It is now validate-then-commit like `apply_transfer`
(which already had the right shape): dedup, ownership/amount via the read-only
`spend_input`, `checked_add` over the whole input set, then one commit loop.

A second, distinct finding came out of the fingerprint itself: `total_supply()`
used `sum()`, which **panicked** on overflow once two `u64::MAX` mints existed.
A reporting accessor must not panic on state the ledger accepted, so it folds
with `saturating_add` and says why.

### Witnesses

`certs::tests::burn_refusal_preserves_state` — four refusal shapes (valid first +
ghost second, the same input twice, foreign-owned second input, `checked_add`
overflow), each compared against a *whole-ledger* fingerprint: UTXOs and their
amounts/owners/blindings, the spent set, the SMT root, supply and unspent count.
The succeeding case is asserted too, so the test is not just "refusals fail".

### Audit of the other four ledgers

Each got the same whole-state refusal test, since "add regression cases only for
distinct risks found" needs a check to find them:
`auction::tests::refusal_preserves_state` (below floor, self-bid, unknown lot
close), `mathbond::tests::refusal_preserves_state` (zero amount, overfunding,
unissued bond), `mathbond_market::tests::refusal_preserves_state` (more shares
than held, zero deposit, invented pool), `attribution::tests::refusal_preserves_state`
(non-author approve, invented credit, offer over an unregistered item). **All
four already validate before mutating** — no ledger rewrite was made, and the
tests pin that shape against regression.

```
$ cargo test --locked --offline -p unfer_consensus --lib   # 121 passed
```

Log: `evidence/d04-unfer_consensus_lib.log`, exit 0.

### Scope

This is domain atomicity within one applied op. Crash atomicity (a torn write
during the commit) is D07's, and the ledger's `write`+`rename` without `fsync`
still stands between here and a restart-safety claim.

## D05 — separate submission, commitment and application (2026-11-21)

**New** U `unfer_protocol/src/submission.rs`; U
`unfer_consensus/src/{engine,node,net}.rs`, `unfer_identity/src/lib.rs`,
`unfer_data/src/publisher.rs`; U `b0aaa7c` + this patch.

### The confusion that was there

`ConsensusEngine::submit` returned one `u64` that callers used as both a
receipt id and a log position. That is true on `LocalConsensus` (submission *is*
commitment there) and false on `NetConsensus`, which reserves an id from
`next_id` before QuePaxa runs and appends the payload at whatever slot the
decision lands in. `DidManager` and the publisher then reported success as soon
as `submit` returned, so a network caller could be told a DID was created while
the entry was still only *pending*.

### What is now distinct

| Type | Meaning |
|---|---|
| `SubmissionId` | the receipt identity `submit` returns; not a position |
| `Slot` | the committed position the payload landed at |
| `Watermark` | the highest slot **applied** to this node's state |
| `ReceiptState` | `Pending`, `Committed(slot)`, `Refused{slot, refusal}`, `Unknown(reason)` |
| `Refusal` | a domain verdict as (code, message), equal iff the same entry was refused the same way |
| `HaltReason` | `Gap{expected, found}` / `OutOfOrder` — a property of the *history*, not the entry |

The engine gained `receipt(id)` and a bounded `wait(id, timeout)`; a deadline
returns `Unknown(TimedOut)`, never success and never a refusal. `ConsensusNode`
gained `submit_tracked`, `settle(id, timeout)` (wait, apply, then report the
settled outcome), `watermark()`, `refusal(slot)`, `halted()` and `sync_report()`.
`submit` and `sync` keep their shapes as documented adapters.

On `NetConsensus` the id→slot mapping is durable (`LedgerSnapshot.committed_ids`,
`serde(default)` so a pre-existing snapshot still loads), because a restarted
node must still be able to answer what happened to a submission id it handed out
before the restart.

### Application semantics, and what changed in them

Previously any `Err` from applying a committed entry came back out of `sync()
and stopped the prefix there, which meant one domain refusal wedged every good
entry committed behind it — and the watermark then sat *before* an entry the
engine had already committed, forever. Now:

* a **domain refusal** is recorded against its slot and the slot advances;
* a **gap or backwards slot** halts the node: it applies nothing further, keeps
the reason, and refuses later `sync` calls rather than silently skipping the
history it could not apply;
* each committed entry is **re-verified** before application. Ingress
  verification only covers what this node submitted; without this, an entry
  arriving through the log with a stripped signature would apply.

Two existing tests changed because the contract changed, and their names say
what they now assert: `invalid_certificate_op_rejected_before_log` became
`refused_certificate_op_advances_the_slot_with_a_typed_outcome` (the old name
claimed a rejection *before* the log, which is not where it ever happened), and
`market_resolve_forgery_rejected` now asserts a typed refusal at its slot
instead of `sync().unwrap_err()`.

### Witnesses

| Witness | Asserts |
|---|---|
| `a_refused_entry_between_two_good_ones_does_not_wedge_the_suffix` | 3 slots applied, watermark 3, refusal at slot 2, both good mints present and the refused 500 absent (supply 150) |
| `a_refused_entry_leaves_the_ledger_fingerprint_unchanged` | whole-`CertificateLedger` `Debug` fingerprint identical before/after (D04's comparison reused) |
| `a_gap_in_committed_history_halts_without_applying_anything_past_it` | `HaltReason::Gap{expected:2, found:3}`, watermark 1, supply 100, and a second `sync` still refuses |
| `a_tampered_entry_is_refused_on_replay` | a signature flipped after signing is refused `INVALID_SIGNATURE` at application, supply 0 |
| `settle_applies_the_entry_it_reports_as_committed` | `Committed` implies `total_supply() == 100` — the state, not just the log |
| `settle_reports_a_refusal_rather_than_committed` | unauthorized mint settles as `Refused` with UK-7001 at slot 1 |
| `a_receipt_bounded_wait_reports_unknown_not_success` | an unknown id settles as UK-1010 |
| `local_receipt_is_its_own_slot` | `LocalConsensus` receipt is the slot; an unreserved id is not |
| `pending_creation_is_not_reported_as_created` | `DidManager` over an engine that never commits returns UK-1010, `applied_seq() == 0`, DID absent |
| `publication_is_not_reported_before_application` | `DataPublisher` over the same engine returns UK-1010, nothing applied |

```
$ cargo test --locked --offline -p unfer_protocol --lib          # 351 passed
$ cargo test --locked --offline -p unfer_consensus --lib         # 129 passed
$ cargo test --locked --offline -p unfer_consensus --features network --test phase6_testnet  # 4 passed
$ cargo test --locked --offline -p unfer_identity                # 7 + 1 doc passed
$ cargo test --locked --offline -p unfer_data --lib              # 63 passed
```

Logs: `evidence/d05-unfer_{protocol,consensus,identity,data}*.log`, all exit 0.
The `phase6_testnet` run matters twice over: it drives a real 2-of-3 QuePaxa
cluster through the new id→slot receipt path, and its restart test proves the
mapping survives a full-cluster restart.

### Not claimed

Local receipt/commitment/application separation is **not** evidence of
Byzantine ordering; QuePaxa's own guarantees are unchanged and untested here.
D06 owns durable *request* identity (a caller retrying across a crash), and D07
owns crash atomicity — `settle` says "applied to this node", not "durable on
disk". Halt classification here is by history shape only; D07 adds the other
kind (an infrastructure failure during application) via `Code::is_infrastructure`.

## D06 — durable logical request identity and safe retention (2026-11-22)

**New** U `unfer_consensus/tests/request_identity.rs`; U
`unfer_consensus/src/{idempotency,node}.rs`; U `unfer_protocol/src/{codes,submission}.rs`;
U `14e0a7a` + this patch.

### Why a content key cannot answer a client

The H7 guard (`IdempotencyStore`) deduplicates delivery: "did this content already
apply?". A client asks something else after a timeout — "did *my request* apply?"
A content key answers that only if the caller can reproduce the exact bytes, and
two different callers asking for the same bytes are two actions the ledger must
still be able to distinguish.

### What is now named and durable

| Type | Meaning |
|---|---|
| `RequestId` | `(deployment, principal, incarnation, sequence)` — the caller's own name for one action |
| `ClientSequence` | the durable `(incarnation, sequence)` the client resumes from |
| `AdmissionBook` | per-client retained window of `Admission { digest, outcome }` |
| `AdmittedOutcome` | `Pending{submission}` / `Accepted{submission, slot}` / `Rejected(refusal)` |
| `AdmissionVerdict` | `Fresh` / `Duplicate(outcome)` / `Conflict{admitted, offered}` / `BelowRetention{floor}` / `AtCapacity{capacity}` |

The client persists the sequence **before** returning the id
(`ClientRequests::next` → `RequestSequenceStore::save`); `FileSequenceStore`
makes that durable with write + fsync + rename + directory fsync. A store failure
does not advance the sequence, so the id that is eventually sent is the one that
became durable — otherwise a crash between "allocated" and "sent" would burn an
id, and the retry that followed would be a *different* action.

Server side, `ConsensusNode::submit_request` is the admission point: the same id
with the same bytes replays the recorded submission (or its recorded rejection),
the same id with different bytes is UK-8101 and nothing is applied, an id at or
below the retained floor is UK-8102, and a full book is UK-8103.

### Retention is a floor, not a schedule

`IdempotencyStore::prune_before` dropped guards on a schedule, which is safe only
while nobody retries. Once a request id is forgotten the server cannot tell a
retry from a first delivery, so the replacement is an **acknowledged floor**:
`acknowledge(&id, through)` forgets the records and keeps the tombstone, and a
request at or below the floor is refused (UK-8102) rather than applied a second
time. Unacknowledged clients cannot force unbounded memory: the book has a
capacity and answers `AtCapacity` (UK-8103) instead of evicting a record a
client may still retry.

### Witnesses

| Witness | Asserts |
|---|---|
| `a_retry_with_the_same_bytes_replays_the_recorded_result` | duplicate returns the recorded `Accepted{submission, slot}` |
| `a_retry_with_different_bytes_is_a_conflict` | `Conflict{admitted, offered}` and no second admission |
| `below_the_retained_floor_a_retry_is_refused_not_reapplied` | `BelowRetention{floor: 3}`; sequence 4 is still fresh |
| `unacknowledged_clients_hit_the_quota_instead_of_evicting` | `AtCapacity{2}`; acknowledging a floor frees room |
| `independent_incarnations_are_distinct_sequence_spaces` | the same sequence in a new incarnation is a different request |
| `a_client_store_failure_does_not_advance_the_sequence` | the retried id is sequence 2, the first that became durable |
| `a_restarted_client_resumes_its_persisted_sequence` | the stored incarnation wins over the argument |
| `a_file_backed_client_sequence_survives_a_reopen` | reopen continues at sequence 2 |
| `a_retried_request_is_submitted_once_and_lands_in_one_slot` | one watermark, one slot, the same receipt twice |
| `a_conflicting_retry_is_refused_and_applies_nothing` | UK-8101 and the watermark does not move |
| `a_settled_rejection_is_replayed_to_the_retrying_client` | a cached refusal, not a second submission |
| `different_request_ids_with_identical_bytes_are_distinct_actions` | two ids, two slots |
| `a_request_below_the_acknowledged_floor_is_refused_after_restart_of_retention` | UK-8102; the retained neighbour still answers |
| `a_client_cannot_exhaust_memory_by_inventing_incarnations` | the third incarnation of a two-incarnation principal is refused; retiring frees exactly one |
| `the_scope_budget_is_counted_separately_from_open_records` | two empty windows fill the scope budget, and retiring frees one |
| `a_client_that_invents_incarnations_is_bounded_and_can_retire_a_run` | at the node: UK-8103, another principal unaffected, retire then accepted |

```
$ cargo test --locked --offline -p unfer_consensus --test request_identity  # 6 passed
$ cargo test --locked --offline -p unfer_consensus --lib                   # 152 passed
$ cargo test --locked --offline -p unfer_protocol                          # 353 + 4 + 51 + 3 passed
$ cargo test --locked --offline -p unfer_identity                          # 7 + 1 passed
$ cargo test --locked --offline -p unfer_data                              # 63 + 4 + 1 passed
$ cargo test --locked --offline -p unfer_edge --features audit              # 89 + 1 passed
```

### Not claimed

The book is **in memory** in D06 (`AdmissionBook` is `Serialize`, and D07 makes the
bindings durable). Until the binding write succeeds, a retry after a crash is a
fresh admission; what makes that safe rather than duplicating an effect is the
ledger's own content-key guard plus D07's durable request records, not this
table.

A client that keeps inventing incarnations used to buy a fresh admission
namespace per request, and each namespace is a retained window that outlives its
records (the floor is the only thing that makes a forgotten id safe to refuse).
Windows are therefore bounded twice: `DEFAULT_INCARNATIONS_PER_PRINCIPAL` per
`(deployment, principal)` and `DEFAULT_ADMISSION_SCOPES` globally, with a
`retire_incarnation` call as the only way to regain headroom. Retirement is the
client's declaration that it will never retry under that run, because the window
takes its floor with it — so a client that retires a run it might still retry
loses the protection, and a client that does not retire is refused at the limit
rather than served indefinitely. The bound is policy, not a proof about client
behaviour: this is still one node's memory, and the numbers are constants, not a
negotiated quota.

## D07 — one durable application transition and honest failure semantics (2026-11-22)

**New** U `unfer_consensus/src/storage.rs`; U `unfer_consensus/src/{net,node}.rs`,
U `unfer_ffi/src/durable/loro.rs` (separate patch `249c53c`); U `14e0a7a` + this
patch.

### The state that never existed

`SharedLedger` persisted `bank`, `pending` and `committed_ids` by reading three
separate locks in sequence. A crash between two of those reads published a
snapshot the ledger never had; the worst case was a payload already removed from
`pending` and not yet in `bank`, i.e. a lost transaction recorded as a clean
snapshot. On top of that, `fs::write` + `rename` with no `fsync` can survive the
rename and lose the bytes.

Now all three live behind one `Mutex<LedgerState>`, the snapshot is taken under
that lock, and the write is tmp → fsync → rename → directory fsync. A decision is
acknowledged to the pump only after its snapshot is durable.

### The log

`storage::TransitionLog` frames every record as
`| magic | version | length | crc32 | payload |` and the protocol for a record is
**write the whole frame, sync, publish**. Two record kinds share one writer:

* `TransitionRecord { slot, state_version, command_digest, outcome, dedup }` —
  one applied slot: what was decided, and the exactly-once guard it installed;
* `RequestRecord { id, digest, submission, outcome, slot }` — the client binding,
  written when an outcome settles and **before** the caller is told.

Recovery distinguishes the three cases the plan asks for:

| Case | Behaviour |
|---|---|
| incomplete trailing frame | torn tail: truncated, and `torn_tail_bytes` reports it |
| complete frame, bad magic/version/length/CRC | corruption: refused, file untouched |
| complete frame written, `sync` failed | `StorageError::Uncertain` — never reported as "definitely not recorded" |

### Recovery order and mismatched checkpoints

`ConsensusNode::open` recovers the log, rehydrates the request bindings into the
admission book, and turns the transition records into a **comparison script**:
the next `sync()` replays the engine log and every already-recorded slot must
reproduce the recorded command digest and outcome. A slot that reproduces a
different decision is a mismatched checkpoint — the node halts with
`HaltReason::Failed`/UK-5011 instead of merging two histories. A transition that
cannot be written halts at that slot, so the watermark never passes an unrecorded
entry. An outcome that cannot be recorded is reported as
`Unknown(Unrecorded)`, never as a refusal.

The consensus library persists its own runtime store separately and cannot
commit it in the same transaction; the order is therefore "runtime store first,
then this log, replay by slot", and execution is idempotent by slot. That is a
recovery *order*, not an atomic cross-store commit, and it is documented as such
in `storage.rs`.

### Failure classification

`ApplyFailure` splits an application error by code family (`Code::is_infrastructure`:
UK-1010, UK-1011, UK-3002, UK-5000, UK-5011). A domain code is a verdict on the
entry (record and advance); an infrastructure code means "this node could not
tell" and halts. This is the classification D05 left as a documented gap.

### Witnesses

| Witness | Asserts |
|---|---|
| `records_survive_a_reopen` | 3 records, `last_slot` 2, no torn tail |
| `a_torn_tail_is_discarded_and_the_log_stays_usable` | 1 record survives, `torn_tail_bytes > 0`, the log continues at slot 3 |
| `a_complete_frame_with_a_bad_checksum_is_corruption_not_a_torn_tail` | `Corrupt` and the damaged bytes are left in place |
| `an_uncertain_write_is_reported_as_uncertain` | `Uncertain`, and recovery still publishes the complete frame |
| `a_write_failure_before_the_frame_leaves_nothing` | 1 record, no torn tail |
| `slots_must_increase_and_request_slots_must_be_known` | repeated slot and a request naming an unknown slot are both `Corrupt` |
| `a_partial_header_at_eof_is_torn` | 2 bytes are a torn tail, not corruption |
| `a_recorded_transition_survives_a_restart_and_is_compared_not_reapplied` | state rebuilt by replay, supply 150, watermark 2 |
| `a_mismatched_journal_halts_instead_of_merging_two_histories` | `Failed{slot 1}`, UK-5011, "journal recorded", watermark 0 |
| `a_torn_transition_write_halts_at_that_slot` | halt at slot 1, watermark 0, the next sync also refuses |
| `an_acknowledged_request_result_survives_a_restart` | the retry reuses the recorded submission, watermark 1 |
| `an_unrecordable_request_result_is_reported_as_unrecorded_not_refused` | `Unknown(Unrecorded)`, no halt |
| `a_storage_failure_is_a_halt_and_a_domain_error_is_a_refusal` | UK-5011 halts, UK-7002 refuses |

```
$ cargo test --locked --offline -p unfer_consensus --lib                        # 152 passed
$ cargo test --locked --offline -p unfer_consensus --features network --test phase6_testnet  # 4 passed
$ cargo test --locked --offline -p unfer_ffi                                    # 175 + 55 + 1 passed
```

`phase6_testnet` still drives a real 2-of-3 cluster through the consolidated
snapshot, including the full-cluster restart, so the network path is exercised
and not only compiled.

### Not claimed

This is not an atomic commit across the consensus library's runtime store and
application state; the runtime store can be ahead and recovery replays by slot.
`fsync` is only as good as the platform's block device. The unit tests inject the
failure *at* each protocol boundary (`FaultPoint::BeforeWrite`/`HalfWrite`/
`AfterWrite`); `tests/power_cut.rs` additionally kills a real child process
(`SIGKILL`) that is appending to a journal on `/dev/shm`, which is what proves
the recovery path sees bytes left by a process that never got to clean up:

| Witness | Asserts |
|---|---|
| `a_killed_writer_leaves_a_journal_that_recovers_and_continues` | five rounds, each recovered prefix is contiguous from slot 1, and the log accepts the next append |
| `a_half_frame_from_a_dead_process_is_discarded_not_reported_as_corruption` | an aborted writer's partial frame is a torn tail, not corruption |

Measured: round recoveries of 17, 12, 52, 24 and 92 transitions with 0 torn tail
bytes — the kill lands between appends far more often than inside one, which is
why the half-frame case is forced explicitly instead of hoped for. `/dev/shm` is
tmpfs, so `sync_data` is a no-op there: this test verifies the framing, ordering
and truncation protocol under abrupt death, not that a disk honoured an fsync.
Durability against media loss rests on the syscall sequence, not on this test.
The Loro snapshot patch makes that file durable; it does not claim the same
transaction for other backends.

## D08 — scope caller context and enforce dynamic remote policy (2026-11-22)

A `safestos/cranelift/src/{auth,module,cps}.rs`; bridge release rebuilt.

### The authority that belonged to whoever loaded last

The JIT gate answered "who is calling?" with two process globals: a single
`DEPLOYMENT_PRINCIPAL` string and a single installed `AuthorizationEngine`.
`ModuleHost::load` set both, so with two modules loaded:

* every call was attributed to the module whose manifest loaded **last**, so
  module A borrowed module B's grants and A's own grants vanished the moment B
  loaded (the engine was *replaced*, not merged);
* a module that failed to compile left the new engine — and the new principal —
  installed, so a failed load was also a policy change;
* there was no way to say "this compiled module no longer has authority": the
  grant was a property of the process, not of the instance.

### An identity instead of a global

`auth::InstanceContext` is bound once per load and is immutable: `instance` (an
allocated id, never reused), `module` (the manifest name grants are keyed by),
`delegator` (the user, when a host supplies one), `node` (the remote endpoint, in
a **separate** field), `epoch`, and `strict`. The loader

1. binds the identity, merges the module's grants into the live engine (instead
   of swapping it in over the other modules'),
2. enters that identity's caller scope for the compile, so `check_call_permission`
   attributes every gated call to *this* module, and
3. on failure restores the previous engine and revokes the identity it just
   bound.

Liveness is a registry lookup (`instance` + `epoch`): `revoke_instance` removes
an entry and `rebind_instance` moves the epoch, and both make an
already-compiled module's authority stop working without touching its code.
`ModuleHost::call` checks liveness before the function pointer — which is where
a revocation can be enforced, since compiled code does not re-check itself.
`ModuleHost::swap` compiles the replacement under the current identity, then
rebinds it: the pre-swap context is retired, and a swap that fails leaves both
the old code and the old identity live. The process-global deployment principal
is deliberately **not** set by `load` any more: loading a module must not
redefine the authority of code compiled outside it. `safestos_load_auth_manifest`
still sets it for the legacy OCaml path, and UK-4001's wording is unchanged.

### Strict profile: a prefix is not a grant

`[module] profile = "strict"` makes the gate require the callee to be a
**declared ABI symbol** as well as granted. The declaration set is derived from
the same tables the JIT builder registers (`UNFER_SYMBOLS`, `ZENODO_SYMBOLS`,
`RS_FFI_SYMBOLS`, plus the `au_*`/`__*` builtins), so a symbol the JIT can
resolve is a symbol the profile can name, and a name a manifest invented cannot
become a call even if it is in the grant list. An unknown profile value is a
error, not a silent downgrade, and a hot-swap may not change the profile (that is
a re-load).

### The receiver side of the boundary

`auth::check_remote_use(from_peer, context, action, resource)` takes the peer
identity the transport authenticated **separately** from the context it is
checking, and requires `context.node == from_peer`: a token cannot certify its
own node. It then checks liveness at the receiver and the grant at the receiver's
own policy, so a stale copy of the sender's grants buys nothing.

### Witnesses

| Witness | Asserts |
|---|---|
| `d08_instances_are_isolated_revocable_and_retired_by_swap` | A and B keep disjoint grants and identities; a failed load leaves both intact; a revoked instance cannot be called while its neighbour can; a swap retires the old context and a refused swap changes nothing |
| `d08_strict_profile_is_explicit_and_cannot_be_swapped_away` | strictness reaches the loaded identity, an unknown profile is refused, and a hot-swap to a laxer manifest is rejected |
| `an_instance_context_is_the_authority_and_a_stale_one_is_refused` | the JIT gate uses the scope, not the global principal; revocation and rebinding both refuse; a strict module refuses a *granted but undeclared* symbol; spoofed peer and node-less contexts are refused |

```
$ cargo test --lib                # 66 passed
$ cargo test --test module_tests  # 11 passed
$ cargo test --test cps_tests     # 14 passed
$ cargo test                      # 19 suites, 0 failures (modhost_cli 41, 1 ignored)
$ nix develop "git+file://#ocaml" -c make bridge   # release .so rebuilt, EXIT=0
$ bash scripts/verify-invariants  # 177 passed, 3 explained gap(s), 0 failed
$ nix develop "git+file://#ocaml" -c dune build @runtest  # EXIT=0
```

### Not claimed

Revocation is enforced at the **host** boundary and at the receiver of a
remote/distributed call, not inside the JIT-compiled function body: the gate is
a compile-time decision, and an already-running function is not interrupted.
The A-side compiler integration (dune `@runtest`, with the rebuilt release
artifact on `LD_LIBRARY_PATH`) passes; it asserts that the bridge still loads and
the compiler suites are unmoved, not that the OCaml path exercises the new
identity. Legacy native calls outside `uk_*`/`uz_*`/`rs_*` are not generalized
into this result.

The peer a remote call arrives from is operator-supplied at the front door
(`modhost --peer`): the transport that would *authenticate* it is D11/D17's, so
what this proves is that a call is refused when the loaded instance serves a
different node, serves none, is ungranted for the entrypoint, or has been
revoked — not that some network connection's identity was verified.

### The receiver (follow-up, 2026-11-22)

`check_remote_use` had no caller outside its tests. `ModuleHost::call_from_peer`
is now that caller, and it is the only host entry point that takes a peer
identity, so a request cannot name its own node:

* `[module] node = "…"` binds the instance to the node it serves; a swap may not
  change it (that is a re-load, like a profile change).
* The receiver refuses a context that names no node, one whose node is not the
  authenticated peer, one that is not live at this receiver, and one whose
  module is not granted the entrypoint it was asked to serve.
* `modhost host <dir> --call <ep> --peer <node>` is the front door to it.

| Witness | Asserts |
|---|---|
| `d08_a_remote_call_is_served_only_for_the_declared_peer` | the declared peer is served; another peer, a local module, an ungranted entrypoint and a revoked instance are each refused; a swap to a different node is refused |
| `peer_calls_are_served_only_for_the_declared_node` | the same through the binary: `--peer` matches → result; `--peer` differs → exit 1 with UK-4001; a module with no node → refused; the local path still works |

```
$ cargo test --test module_tests   # 12 passed
$ cargo test --test modhost_cli    # 15 passed
$ cargo test                       # EXIT=0 (lib 66, cps_tests 14, modhost_cli 15, module_tests 12)
```

### Gate ordering (follow-up, 2026-11-22)

The earlier `make test` stop was not a test failure but a stale *derived copy*:
`verify-invariants` W5 compares the repository-root `.so` with the release
artifact, and `test-locked` staged it only inside its own recipe — which runs
after the drift controls have already run a full gate over the tree. Any release
rebuild therefore turned every control red on a tree that was current.
`make stage-bridge` now deploys both copies (root and `$(BRIDGE_DIR)`), `bridge`
calls it, and `drift-controls`, `drift-controls-all` and `test-locked` take it as
an order-only prerequisite, so "rebuilt" and "staged" are one event.

Measured: `make bridge` EXIT=0 with both copies byte-identical to the artifact
(`cmp`), `make -n drift-controls` shows the two `cp -f` lines before
`scripts/w3-drift-controls`, an emptied root copy is healed by `make
stage-bridge`, and the gate is 177 passed / 3 explained gaps / 0 failed.
`make test` itself (1.5 h of drift controls) was not re-run in full; the
structural order is what changed, and it is what the above checks.

One tree-state caveat, recorded because the numbers above are otherwise read as
"HEAD is green": those runs were taken on 2026-11-22 ~15:00, before this branch
merged origin's compiler update (`lib/Compiler_cps.ml` +363 lines,
`lib/CliEngine.ml`, `lib/gpu/*`, with a two-line certificate refresh). After that
merge the gate reports one failure — W5 "the certificate's cps digest is the
bridge's digest of the emitted bytes" — because `mm0/flux1d_certificate.mm0` was
emitted before the last `lib/` change, so the recorded digest names an older
compiler build. Re-emitting it belongs to that compiler change, not to D08/D09:
nothing here touches CPS emission, and the certificate is a proof artifact whose
refresh should carry the author's own measurement. The remaining gaps are the
same explained ones (W5 object arch, W9.1 RV64, W9.3 Lean, plus PC05a/lean2mm0
when the mm0 tools are not on `PATH`).

## D09 — trusted artifact identity through umbrella (2026-11-22, step 1 of 4)

A `umbrella/src/{project,trust,main}.rs`; tests `umbrella/tests/{policy,cli,entry}.rs`.

### Why a pin, and why two of them

`umbrella` decides three separate permissions (build, load, call) *before* the
first stage runs. What it did not decide was **which artifact** the first two
permissions are for: authorizing a build authorized whatever the crate happened
to contain, and authorizing a load authorized whatever cdylib path was named.
Building a crate executes its build scripts and `dlopen` runs the library's
initializer, so both are native execution.

A project that authorizes either now has to pin the identity in a `trusted`
block, and the document is refused at parse time without one:

| Field | Checked | Against |
|---|---|---|
| `trusted.lock_sha256` | before `cargo build` | the crate's `Cargo.lock` (a different dependency graph is a different build) |
| `trusted.features` | before `cargo build` | the features the caller asked for (an added feature is a different build) |
| `trusted.library_sha256` | after the build, before `dlopen` | the bytes about to be mapped |
| `trusted.exports` | after the build, before `dlopen` | the library's dynamic symbol table |

A wrong-length digest, an empty export list, or an unknown field is a document
error — not a weaker trust decision. The library digest is checked before the
export list: if the bytes are not the pinned ones, their symbols say nothing.

### Witnesses

| Witness | Asserts |
|---|---|
| `a_drifted_lock_is_refused_before_anything_runs` | one byte of lock drift refuses the build, and the refusal names the pin |
| `an_added_feature_is_refused_even_with_a_pinned_lock` | the pinned set is exact; a command-line extra is refused |
| `a_project_without_a_pin_is_refused_rather_than_trusted` | both the parser and the gate refuse an unpinned build/load |
| `a_build_without_a_pinned_artifact_is_refused` / `an_unknown_trusted_field_is_named` | the document rules, by field name |
| `a_library_that_is_not_the_pinned_one_is_refused_and_its_exports_never_consulted` | digest mismatch stops before the symbol table |
| `an_unreadable_library_is_a_refusal_not_a_pass` | an artifact that cannot be hashed is not trusted |

```
$ cargo test --lib           # 59 passed
$ cargo test                 # EXIT=0 (6 integration binaries: 5+6+4+7+6+3 passed)
```

### Not claimed

This is a digest pin, not a signature: it says the artifact is the one the
project approved, not who approved it or what the code does. `trusted.exports`
is read from `nm -D --defined-only` — a host without `nm` is refused rather than
passed — and it checks that a symbol exists, not its signature; C has no
signature in the symbol table, so the ABI must still match by construction.
Steps 2–4 of D09 are not started: the instance-rooted directory capability and
the peer-restricted network capability, the bounded `send`/`query`/`wait`/`cancel`
surface over the Rust protocol code, and the Austral positive/negative demo do
not exist yet, so nothing here shows a module being *given* those handles.
`untrusted-native` remains U26's tier and is still refused by preflight.

## Task status

| Task | Status | Note |
|---|---|---|
| D00 | complete (re-measured 2026-11-21) | this record, the inventory and the checker; all seven suites green on this host, the remaining gaps named above |
| D01 | verified within stated scope | identity authority bound at the ordered point; five witnesses above; the limits listed there |
| D02 | verified within stated scope | per-object key context, authenticated metadata, version refusal; eleven witnesses and the ffi vault suite |
| D03 | verified within stated scope | capability RPC loopback-only at ingress; registry owner/grant checks per verb; the auth seam (A9) stays a named gap |
| D04 | verified within stated scope | burn is validate-then-commit; four refusal fingerprints; `total_supply` no longer panics on overflow |
| D05 | verified within stated scope | receipt/slot/watermark are distinct; a domain refusal advances the slot, a gap halts; identity and publishing settle before reporting |
| D06 | verified within stated scope | a client-named request replays its settled result, conflicts are refused, retention is an acknowledged floor and a full book backpressures |
| D07 | verified within stated scope | one framed, fsynced log; torn tail vs corruption vs uncertain; replay compares recorded outcomes and halts on a mismatch; an abrupt-kill child on tmpfs recovers with a contiguous prefix and continues |
| D08 | verified within stated scope | per-instance immutable identity replaces the process-global principal; grants merge instead of replacing; revocation and swap retire an identity; strict profile admits only declared ABI symbols; `call_from_peer` is the production receiver and checks against the peer the transport authenticated |
| D09 | in progress (step 1 of 4) | `umbrella` pins the artifact identity — Cargo.lock digest and feature set before cargo runs, library digest and ABI exports before dlopen — and refuses a build/load authorization with no pin; capabilities (steps 2–3) and the Austral demo (step 4) are not started |
| D08–D26 | not started | D08 is next (scoped caller context, dynamic remote policy) |
