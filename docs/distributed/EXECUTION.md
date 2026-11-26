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

- **Prover availability is partial.** `mm0-rs` and `mm0-c` are installed under
  `~/.local/bin` on this host and produced the D20 transcript; `why3` is still
  not on the interactive PATH, and no Lean induction shell is configured. D21's
  MM0/Lean induction therefore remains open rather than unverified by accident.
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
stage-bridge`, and the gate is 179 passed / 4 explained gaps / 0 failed
(`docs/distributed/evidence/d09-verify-invariants.log`). `make test` itself (1.5 h
of drift controls) was not re-run in full; the structural order is what changed,
and it is what the above checks.

That 178/4/0 is measured *after* this branch merged origin's compiler update
(`lib/Compiler_cps.ml`, `lib/CliEngine.ml`, `lib/gpu/*`): while the merge was in
flight the gate failed W5 "the certificate's cps digest is the bridge's digest of
the emitted bytes", because `mm0/flux1d_certificate.mm0` had been emitted before
the last `lib/` change. That compiler change's own follow-up re-emitted the
certificate, and the failure is gone without anything here touching CPS
emission. The four remaining gaps are the explained ones (W5 object arch, W9.1
RV64, W9.3 Lean, and PC05a/lean2mm0 when the mm0 tools are not on `PATH`).

## D09 — trusted artifact identity and instance capabilities (2026-11-22, steps 1–3 of 4)

A `umbrella/src/{project,trust,main}.rs`, `safestos/cranelift/src/{capstd_mod,module,lib}.rs`;
tests `umbrella/tests/{policy,cli,entry}.rs`, `safestos/cranelift/tests/capstd_tests.rs`.

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

### Capabilities: handles, not paths (steps 2–3)

`ModuleHost::load` now constructs one `capstd_mod::caps::InstanceCaps` per
instance and installs it under the instance id:

* the **directory** capability is rooted at `<module-dir>/.state`, canonicalised
  once at construction, so a symlinked state directory cannot widen it later;
  absolute paths and any `..` component are refused before I/O, and a resolved
  parent that escapes the root is refused too;
* the **network** capability addresses exactly `[module] node` — the one peer the
  instance serves (D08). No node means no peers, and a peer outside that set is
  refused *before* anything is delivered, so the adapter has no way to dial an
  arbitrary host;
* a handle is an opaque `u64` issued to one module. Presenting it as another
  module is refused, ids are never reused, and `close` ends the capability.

The `rs_cap_*` boundary (registered in `RS_FFI_SYMBOLS`, so the manifest gate
gates each verb by name like any other `rs_*` call) takes the caller from the
current instance context rather than from an argument, refuses a revoked or
replaced instance before looking at any handle, checks every length against a
1 MiB bound, copies buffers in and out (nothing borrowed is retained), and wraps
each call in `catch_unwind` — unwinding into JIT'd code is undefined, so a panic
returns `-101` instead. A short output buffer reports the required length as a
negative number, so a caller cannot mistake truncation for a full frame.

| Witness | Asserts |
|---|---|
| `handles_are_instance_scoped_bounded_and_never_reach_a_denied_target` | relative read/write works; `..` and absolute paths refused; another module's handle refused; oversized write refused; unconfigured peer refused with **no sink line**; accepted frame is pending until settled; cancelled request never reaches the sink; closed handle is gone and ids are not reused |
| `the_rs_boundary_uses_the_current_instance_and_refuses_a_stale_one` | no caller → `-1`; the installed instance's `rs_cap_dir_open`/`write`/`read` work; short buffer → negated length; absolute path → negative code; a revoked instance → `-3` before any handle |

```
$ cargo test --test capstd_tests   # 6 passed
$ cargo test                       # EXIT=0 (lib 70, capstd 6, modhost_cli 42, module_tests 12)
```

### The Austral demo (step 4)

`umbrella/modules/distributed_io/` is a module that can only touch the
filesystem through a capability handle: it declares `rs_cap_dir_open` and
`rs_cap_write`, opens the handle the host issues, and writes `note.txt` under
its state directory. `DistributedIo.aum` is compiled by the **ordinary
front-end** (`gen_cps.sh` → `austral compile … --emit-cps=module.cps`), and the
resulting CPS is what `modhost` loads, so the run goes through the same manifest
gate as any hosted module.

Two details worth knowing about the shape:

* the buffer verbs take their two addresses as 64-bit words, because a
  `Rust_Ownership` declaration with an `Address[Nat8]` parameter lowers to a
  generated `<name>__adapter` wrapper, and the host has no wrapper to resolve.
  What the host checks is unchanged: non-null pointer, bounded length, path
  contained in the state directory.
* the module host did not register `rs_*` at all — `compile_cps_binary` copied
  `au_*`, `uk_*` and `uz_*` only — so a granted `rs_*` call failed with
  `can't resolve symbol`. That gap is fixed here; registration is still not
  permission, the manifest gate decides by name.

| Witness | Asserts |
|---|---|
| `the_granted_module_writes_through_its_capability` | grant → `compute: 25` and `note.txt` contains `hello from distributed_io`, written through the handle |
| `without_the_grant_the_call_is_refused_and_nothing_is_written` | removing `rs_cap_write` → exit 1, UK-4001 naming the verb, and no file on disk |
| `run_demo.sh` | the same two runs from the shell, against the release binary; refusal leaves nothing behind |

```
$ cargo test --test distributed_io   # 2 passed
$ bash umbrella/modules/distributed_io/run_demo.sh   # granted: 25 bytes; ungranted: UK-4001, nothing written
$ cargo test                         # EXIT=0 (lib 70, capstd 6, distributed_io 2, modhost_cli 42)
$ bash scripts/verify-invariants     # 179 passed, 4 explained gap(s), 0 failed
$ nix develop … -c dune build @runtest   # EXIT=0
```

### Not claimed

This is a digest pin, not a signature: it says the artifact is the one the
project approved, not who approved it or what the code does. `trusted.exports`
is read from `nm -D --defined-only` — a host without `nm` is refused rather than
passed — and it checks that a symbol exists, not its signature; C has no
signature in the symbol table, so the ABI must still match by construction.

The capability tier is the Tier 1 coupling this file opens with: a module that
can call `std::fs` or `TcpStream` directly is *not* contained by refused paths or
a peer allowlist, and hard isolation stays the VM tier's job. The transport that
would settle a pending request is not here — `settle` is called by tests, and the
sink is a file in the state directory, so what is proven is the boundary's
refusals and ownership, not a live network path. Only the *directory* capability
is exercised end to end from Austral: nothing yet sends a frame, so
`rs_cap_send`/`query`/`cancel` are proven from Rust only. The umbrella's own
capability wiring (`capabilities` in the project document: bind an instance,
install its set, run the entry inside that scope) compiles and is covered by the
schema tests, but no test drives it through a real run — the demo's witness is
`modhost`. `untrusted-native` remains U26's tier and is still refused by
preflight.

## D10 — canonical replicated commands and domain-separated signatures (2026-11-23, steps 1a–3 of 3)

U **new** `unfer_protocol/src/distributed.rs`, `unfer_protocol/src/lib.rs`;
tests `unfer_protocol/tests/distributed_wire.rs`.

### Why the value had to change

QuePaxa agrees on a `u64`. The command itself waits in `net.rs`'s
`LedgerSnapshot::pending`, so a committed id names a `BTreeMap` entry on *this*
process: a restarted peer, or a peer that never saw the submission, holds the
slot and not the work. `ReplicatedValue` replaces that value with the command's
bytes and gives "nothing here" its own variant instead of the reserved id
`NOOP_VALUE = u64::MAX` — no integer is carved out of the command space, and
there is no process-local id left to resolve.

### The frame

```text
magic(4) = "UFDC" | version(u16 BE) | kind(u8) | body_len(u32 BE) | body
kind 0 (noop):    body_len = 0
kind 1 (command): body = schema(u16 BE, never 0) | payload
```

Every byte has one encoding: fixed widths, big-endian integers, no map, no
float, no `serde_json` rendering to disagree about. `body_len` is checked
against `MAX_BODY_BYTES` (1 MiB payload + the schema tag) from the header alone,
before a buffer is reserved, so a 4 GiB claim costs a refused decode and not an
allocation.

| Rule | Refusal |
|---|---|
| wrong magic / version / kind | `BadMagic` / `UnsupportedVersion` / `UnknownKind` (the version is never guessed at) |
| declared body larger than the bound | `BodyTooLong`, measured from the header |
| no-op with bytes, command without a schema tag, schema 0 | `NoopWithBody` / `CommandBodyTooShort` / `ZeroSchema` |
| fewer bytes than declared, at any prefix | `Truncated { needed, had }` |
| a second frame behind the first, one byte of junk | `TrailingGarbage` (but `decode_frame` returns the first frame and its length, so a stream reads both) |
| more unfinished bytes than one frame can carry | `BufferTooLong` on the streaming decoder |

`FrameDecoder::push` takes arbitrary chunks and returns every complete frame in
order, so partial and coalesced frames are the same code path.

### The signature domain

A signature covers `"unfer/distributed/command/v1"`, then
`len(u16) | field` for **deployment, audience, schema, context**, then
`len(u32) | frame`. Each element is length-tagged rather than delimiter-joined,
so moving a byte across a field boundary changes the preimage (a test moves one
byte from `deployment` into `audience` and asserts the two preimages differ).
`SignatureVersion::Legacy` is the pre-D10 layout — `sha256(frame)`, no domain —
kept so already-signed history still verifies; nothing new may use it.

### The dependency's own limits (measured from `rust-quepaxa` 0.1.0)

| Limit | Value | Where |
|---|---|---|
| wire version | `WIRE_VERSION: u16 = 4` | `network/wire.rs:7` |
| deployment identity | `DeploymentId([u8; 16])` | `network/wire.rs:10` |
| transport RPC frame | `DEFAULT_MAX_FRAME = 8 MiB`, length-prefixed `u32`, empty frame refused | `network/transport.rs:26`, `:590`, `:625` |
| recorder/runtime snapshots | postcard, `StorageEnvelope` version-tagged | `network/codec.rs:125` |
| submission journal (ours) | `MAX_JOURNAL_BYTES = 16 MiB` | `unfer_consensus/src/net.rs:52` |
| batch ceilings (dependency defaults) | `max_batches_per_request`, `max_batch_bytes = 4 MiB`, `max_fetch_ids` | `network/batch.rs:278-287` |

This wire's `MAX_FRAME_BYTES` is 1 MiB + 13 bytes, inside the transport's 8 MiB
frame, so one replicated command is one RPC frame with nothing to fragment at
the transport layer.

### Witnesses (`unfer_protocol/tests/distributed_wire.rs`, 12)

| Witness | Asserts |
|---|---|
| `both_decoders_accept_the_golden_vectors_and_re_encode_them` | a second decoder written independently of the module (explicit cursor) agrees with `decode_exact` on every pinned vector, and re-encoding reproduces the bytes |
| `the_golden_vectors_use_every_declared_width` | the vectors pin the magic, the version and a header-only no-op |
| `every_split_point_of_every_vector_streams_to_the_same_value` | every two-chunk split *and* byte-at-a-time streaming yield the same value, with nothing left buffered |
| `coalesced_frames_decode_in_order` | three frames in one push and in 3-byte chunks decode to the same sequence |
| `every_truncation_is_refused_and_names_what_it_needs` | every prefix is `Truncated`, and a truncated tail makes `finish()` refuse |
| `trailing_garbage_is_refused_and_two_frames_are_not_one` | one junk byte is refused; two frames are two frames |
| `an_oversized_body_is_refused_from_the_header_alone` | `MAX_BODY_BYTES + 1` and `u32::MAX` are refused before any payload exists |
| `malformed_headers_are_refused_by_the_rule_they_break` | bad magic, version, kind, no-op-with-body, short body, schema 0 |
| `a_command_is_bounded_at_construction_and_at_decode` | exactly-at-the-bound round-trips; one byte more is refused; an empty payload is legal |
| `a_frame_for_another_deployment_audience_schema_or_context_has_another_preimage` | all four domain fields separate |
| `domain_separation_is_length_tagged_not_delimited` | a shifted field boundary changes the preimage; an over-long field is refused by name |
| `the_legacy_version_is_sha256_of_the_frame_alone` | legacy is `sha256(frame)` and ignores the domain; the new version differs from it |

```
$ cargo test -p unfer_protocol --offline   # EXIT=0: lib 353, cell_tool 4,
                                           # distributed_wire 12, protocol 51, doctests 3
$ cargo check -p unfer_consensus --offline # EXIT=0
```

Evidence: `docs/distributed/evidence/d10-unfer_protocol_tests.log`,
`d10-unfer_consensus_check.log`.

### The replicated value is what `net.rs` agrees on (step 3a)

U `unfer_consensus/src/net.rs`.

`NOOP_VALUE = u64::MAX` is gone. `NetworkCluster::build` names its gap filler
with `.with_noop_value(ReplicatedValue::Noop)`, and the submission channel, the
`StateMachine<ReplicatedValue>` impl and the pump all carry the value.

A submission becomes a value whose payload is `id(u64 BE) |
canonical_json(ConsensusTransaction)` under `LEDGER_TX_SCHEMA = 1`
(`encode_ledger_value`/`decode_ledger_value`). The reservation id travels
*inside* the payload, so a decision is self-describing: `append` learns which
submission it applied from the bytes, and needs no map to look it up.

| Before | Now |
|---|---|
| `append(Vec<u64>)`, looked up in `LedgerSnapshot::pending` | `append(Vec<ReplicatedValue>)`, decoded from the value |
| a peer that never held the payload holds the slot and not the work | any replica applies the decision from its own bytes |
| `pending` doubles as payload store and durability record | `outbox` is **only** this node's pre-proposal durability; it is drained by the decision and never read to apply one |
| unknown id: silently skipped | unknown schema, short payload, undecodable bytes: refused by name, nothing committed |

The renamed field keeps `#[serde(alias = "pending")]`, so a pre-D10 snapshot
loads with its queued submissions in the outbox and re-proposes them instead of
dropping work it had already acknowledged. `append` also removes an applied id
from the outbox and de-duplicates by committed id, which is what makes a
re-delivered decision (or the second replica sharing one ledger) a no-op.

Witnesses in `net.rs`: a committed value applies on a ledger that never queued
it (and its receipt answers with the slot the payload landed at); the outbox is
drained by the decision and is not needed to apply it; an unknown schema, a
payload shorter than its id and undecodable bytes are each refused with nothing
committed; a no-op decision commits nothing and is not an error.

```
$ cargo test -p unfer_consensus --offline --features network --lib          # 162 passed
$ cargo test -p unfer_consensus --offline --features network --test phase6_testnet
                                                                            # 4 passed
$ cargo test -p unfer_consensus --offline --features network --test request_identity
                                                                            # 6 passed
$ cargo test -p unfer_consensus --offline --features network --test power_cut
                                                                            # 4 passed
```

Evidence: `docs/distributed/evidence/d10-net-lib.log`,
`d10-net-phase6_testnet.log`, `d10-net-request_identity.log`,
`d10-net-power_cut.log`.

### Step 3b: `signing.rs` signs the domain-separated preimage

U `unfer_consensus/src/signing.rs`. `canonical_frame` is now the unsigned
canonical JSON on its own; `canonical_bytes` is `sha256(frame)` over it,
byte-for-byte what it was.

| Piece | Behaviour |
|---|---|
| `CONSENSUS_SIGNING_DOMAIN` | `deployment = "unfer"`, `audience = "unfer-consensus"`, `schema = "consensus-transaction"`, `context = "consensus"` |
| `sign_transaction` | signs `signing_preimage(domain, canonical_frame(tx))` — the domain-separated layout |
| `sign_transaction_legacy` | still produces `sha256(frame)`; kept only to witness or reproduce pre-D10 history |
| `accepted_messages` | `[domain-separated, legacy]`, tried in that order |
| `verify_transaction` / `verify_transaction_in` | accept either preimage, so signatures made before this patch keep verifying |
| `verify_identity_op_as` | same dual acceptance, so a *new* identity op still authenticates at the ordered point against the stored key |
| `verify_arctic_threshold` | same dual acceptance; the aggregate signatures the test helpers build cover `canonical_bytes`, so they verify through the legacy branch |

Witnesses added: a legacy signature still verifies (and equals the legacy
signer byte for byte); a new signature is *not* the legacy one, equals the
domain-separated signer, verifies in its own domain, and is refused when the
deployment, audience, schema or context is changed to anything else; altered
bytes are refused under both layouts.

The operation's `signature` field is 64 bytes and carries no version tag, so
verification cannot know which layout a signature used and must try both. That
is why legacy acceptance is a *compatibility window*, not a permanent rule:
retiring it needs a deployment-wide signal that no pre-D10 signature remains,
and until then a legacy signature over any frame that also verifies as a
domain-separated one is accepted (the domain-separated one is checked first).

```
$ cargo test -p unfer_consensus --offline --features network --lib   # 162 passed (9 signing)
$ cargo check -p unfer_identity -p unfer_taler -p unfer_data --offline # EXIT=0
```

Evidence: `docs/distributed/evidence/d10-net-lib.log`,
`d10-consumers_check.log`.

### What is not done yet

* no live transport carries these frames yet (D11/D17).
* `Legacy` signature acceptance cannot be retired until a deployment-wide
  cutover signal exists; today the verifier tries both layouts.

## D11 — one replica per process (2026-11-23, steps 1–3 of 3)

U `unfer_consensus/src/net.rs`; **new** `unfer_consensus/src/bin/unfer_node.rs`,
`unfer_consensus/tests/independent_nodes.rs` and the fixture configs that test
writes; `unfer_consensus/Cargo.toml` (bin + test targets, both
`required-features = ["network"]`).

### Startup belongs to a process, not to a harness (step 1)

| Piece | What it decides |
|---|---|
| `DeploymentConfig` | deployment id, lane, members (replica, address, certificate), accepted clients. Validated **before anything binds**: id non-zero, ≥2 members, replicas `1..=n` in order, addresses distinct, no certificate shared between two roles, client ids non-zero, unique and declared |
| `NodeConfig` | which replica this process is, its own certificate **and key** (the key the deployment file never carries), an absolute `state_dir`, the RPC timeout |
| `StateDirLock` | an exclusive lock on `<state_dir>/node.lock`, held for the process's lifetime. A second process pointed at a live directory fails with `locked by another node process`; the kernel releases it on a kill, so a restart is not blocked |
| `SharedLedger::open` | a ledger with **no** submission pump. A standalone node receives complete values over TLS, so `allocate` refuses with a named error instead of queueing into a pump that is not there |
| `start_node` / `start_replica` | one startup path for a server, recorder, runtime and state machine. `NetworkCluster` is now the in-process harness: two `start_replica` calls over one shared ledger and directory, plus the local pump |

`unfer_node` has three commands. `serve --config node.json [--ready ready.json]`
starts a node, publishes the address it actually bound (so a config may ask for
port 0) and stops on SIGINT/SIGTERM. `submit --config client.json --to ADDR
--id N --tx tx.json` is a **client**: it frames the value itself (`id` inside
the payload, D10) and prints the outcome as JSON. `digest --config node.json`
reads a node's durable committed log without taking its lock, which is how the
test compares what two processes applied.

Configuration is JSON with hex DER inside the file. That is deliberate: a
fixture is self-contained, so two processes never resolve a shared path — which
is exactly the shared state D11 forbids.

### Three processes, disjoint state, identical digests (step 2)

The test starts three processes with three state directories, three
certificates and three ports, submits one transaction through node 1 and one
through node 2, then compares `digest` output:

| Check | Measured |
|---|---|
| each process publishes the port it was configured with | 3/3 |
| digests after two commits | all three equal |
| each directory holds only its own `node-<id>` and its own `ledger.json` | yes; no sibling replica's state |
| a second `serve` on a live state directory | refused: `locked by another node process` |
| a client config for deployment 2 submitting into deployment 1 | refused, non-zero exit |
| the in-process harness still passes its suites | `phase6_testnet` 4, `request_identity` 6, `power_cut` 4 |

### Stop one node, catch up on restart (step 3)

1. Kill node 3 (SIGKILL), submit a third transaction through node 2. Both
   survivors commit it; node 3's digest still reads `2:…` — a stopped process
   does not apply what it missed.
2. Restart node 3 on the same directory, address and identity: it rebinds the
   port, and is one commit behind (`3:d5a7…` against the survivors' `4:…` once
   the fourth decision lands).
3. Submit a fourth transaction through node 1. The rejoined node converges on
   the survivors' digest `4:6b0e9fc5…`, which is the digest of the **whole**
   log — so it applied the decision it missed while it was down, not just the
   new slot.

```
$ cargo test -p unfer_consensus --offline --features network --lib        # 167 passed
$ cargo test -p unfer_consensus --offline --features network --test independent_nodes
                                                                           # 1 passed
$ cargo test -p unfer_consensus --offline --features network --test phase6_testnet   # 4
$ cargo test -p unfer_consensus --offline --features network --test request_identity # 6
$ cargo test -p unfer_consensus --offline --features network --test power_cut        # 4
```

Evidence: `docs/distributed/evidence/d11-lib.log`,
`d11-independent.log` (its stderr line carries both digests),
`d11-phase6_testnet.log`, `d11-request_identity.log`, `d11-power_cut.log`.

### What is measured, and what is not

* **Catch-up needs a decision — superseded, see the follow-up below.** As
  measured here, the dependency informs a replica of decisions it missed
  (`InformDecisions`) only when the cluster next decides, so a node restarted
  while the cluster was idle stayed behind until then. The follow-up below
  removes that dependency on a new decision; this paragraph is kept as the
  measurement it was, not as current behaviour.
* **No checkpoint export/import was needed**, so none was added: the recorder
  and runtime snapshots plus the informed decisions reconstruct the log, and the
  digest equality in the test is what proves it.
* **Five configuration refusals are unit witnesses** in `net.rs`: zero
  deployment id, members not consecutive from 1, two replicas on one address, a
  relative state directory, and an occupied one (lock). A sixth — a client the
  deployment does not declare — covers the client side.
* **The harness is not a deployment.** `NetworkCluster` still runs two replicas
  over one directory and one ledger on purpose; it is a labelled fixture, so the
  failure modes it tests stay reproducible in one test binary.

### Follow-up (2026-11-23): an idle cluster pushes, a rejoined replica no longer waits

The gap above was that catch-up rode on the next decision. It no longer does:

* every replica runs a `catch_up_loop` (`net.rs`) beside its server. Every
  `CATCH_UP_INTERVAL` (200 ms) it takes its own committed head and asks each peer
  `Status(head)`. A status probe answers "do you have this slot", so the peer's
  frontier is found by **binary search** between what is already known and `head`
  — a peer behind by one slot costs the same probes as a peer behind by a
  million.
* it then sends the missing decisions through the transport's own
  `InformDecisions` request, in batches of `CATCH_UP_BATCH` (256). The peer's
  recorder records them and its state machine applies them in slot order, exactly
  as it would a live decision, so its ledger advances and persists.
* the decisions come from the shared `NetworkConsensusHandler` the replica
  already applies with (one `Arc`, `SharedSubmissionHandler`), not from a second
  copy that could disagree with it. Nothing new is submitted, and the cluster
  stays idle.

Measured: after a replica was stopped, two decisions taken without it, and the
process restarted, it reached the survivors' digest **3:…** with **no further
submission** (`tests/independent_nodes.rs`), and `process/slow-replica-idle-catch-up`
records the same for the process fixture. The crash matrix's slow-replica case is
now an idle-catch-up assertion, not a "submit once more" one.

Limits: this is the peers doing the looking, so a node whose peers are *all* down
still waits for one to return (there is nothing to push it); and the probe/push
cadence is a timer, measured 200–400 ms on this host, not an event.
* SIGKILL is used here only to stop a process, not to inject a storage fault;
  the crash matrix over these processes is D13.

## D12 — deterministic fault scheduler over the production transition core (2026-11-23, steps 1–3 of 3)

**New** `unfer_consensus/src/sim.rs` and
`unfer_consensus/tests/distributed_schedules.rs`; `unfer_consensus/src/lib.rs`
(publishes the module), `src/storage.rs` and `src/node.rs` (two test seams, see
below).

### The node is real; the environment is scheduled (step 1)

The harness does not reimplement the node. Each replica is a `ConsensusNode`
over its own real `TransitionLog`, and the things a test cannot control from
outside become explicit inputs:

| Input a process really gets | How the harness supplies it |
|---|---|
| which committed decisions this replica has learned | `EngineView::horizon`: `get_log` reveals only slots up to it |
| whether it can reach the quorum to propose | `EngineView::writable`: a partitioned replica's `submit` is refused by the engine, not by the node |
| what the next journal append returns | `TransitionLog::inject` via `ConsensusNode::inject_storage_fault` (`BeforeWrite`), which is the existing D07 fault point, re-compiled for the library so an integration test can arm it |
| whether the process is running | `Crash` drops the node; `Restore` re-opens it over the same directory **and the same engine view**, so a restore-then-deliver is a catch-up |
| duplicate and reordered requests | `Retry` re-sends the same request id and the same signed payload; `Deliver` is per replica, so replicas apply the same log at different times |

The consensus engine is an assumption in this harness, stated rather than
tested: one `Cluster` holds the decided log, and the harness decides who has
learned what. Application order is an input. Ordering correctness is D11's
QuePaxa dependency, not this task's claim.

### Nondeterminism is registered (step 2)

Every choice is recorded in a `Trace` with a class and a stable site id, and the
classes are kept apart because a failure in one is a different claim:
`Content` (which request, and whether it is a fresh or duplicated encoding — a
bad client), `Scheduling` (deliver/partition/crash/restore — a bad host), and
`Fuel` (how many steps a bounded run may take — a truncated run mistaken for a
pass, so a run that stops on the bound says so). The generator is SplitMix64, not
OS entropy: the same seed and the same action list reproduce the same trace and
the same state fingerprint, which the replay test asserts, including that a
*different* seed changes nothing when the action list is fixed.

### The fixed domain, its bounds, and what pruning means (step 3)

| Bound | Value |
|---|---|
| Clients | 2 (each with its own keypair, DID and durable request sequence) |
| Replicas | 3 (each with its own directory, journal and engine view) |
| Requests | 3 — an authorized mint (applies), a mint by the non-authority client (refused by the node itself, slot advances), and an identity create (applies through a second ledger) |
| Action alphabet | 9 fixed actions (3 admits, 1 retry, 3 delivers, 1 partition, 1 crash) |
| Depth | **4** in the default run, **5** and **6** in the `#[ignore]`d deep run (`UNFER_D12_DEPTH`) |
| Unpruned state space | 7381 / 66430 / 597871 prefixes for depth 4 / 5 / 6 |
| Executed, pruned, leaves | depth 4: **1342** executed, **545** pruned as equivalent, **648** leaves; depth 5: **6904**, **2898**, **3239**; depth 6: **33769**, **14617**, **15400** |
| Violations | **none** at any of the three depths |
| Runtime | depth 4 **21 s**, depth 5 **153 s** on the host filesystem; depth 5 **110 s** and depth 6 **720 s** with the journals on tmpfs (`d12-schedules.log`) |
| Seeded walks (past the bound) | 32 runs × 12 steps, **30 distinct** final states, no violation |

The deep run is `a_deep_search_to_depth_five_finds_no_violation`, `#[ignore]`d
because it takes minutes — not because its numbers are optional: they are the
ones in this table, and the commands are in `d12-schedules.log`. The nightly
`.github/workflows/deep-schedules.yml` in U runs it (depth 5 by default, depth 6
on dispatch) and writes the bounds line into the run summary, so the number here
can be re-derived from a green run object instead of a transcript.

**What it cost, and what was cut.** The first measurement of this domain took
325 s at depth 5. Profiling the per-prefix cost found it was not the schedule at
all: `SparseMerkle::empty()` rebuilt the 256-level default hash chain on every
ledger construction, twice per replica, and in a debug build that is 3.9–4.9 ms —
24 ms of every prefix, 90 % of the run. The chain depends only on the depth, so
it is now computed once per process and shared (`certs.rs::default_chain`); the
same numbers came out at 21 s and 153 s, which is also a check that the cache did
not change a single ledger root (the `certs` unit tests compare them).

What remains is the protocol under test: each applied transition writes a journal
frame and `sync_data`s it, so a prefix costs roughly `depth × one sync`. That is
why the medium is named in every row above — tmpfs is where depth 6 fits in a
nightly window (**720 s**, 33769 prefixes), and it is the same protocol against a
fast device, not a weaker one. It is *not* a power-loss claim: nothing in D12
speaks to device-level durability, which is D13's subject and is measured there
with real processes and real `chmod`/corruption.

Pruning is by `Sim::state_key` — decided log, per-replica watermark, liveness and
state digest. Two prefixes with the same key are treated as admitting the same
continuations, which holds only because the transition core is deterministic and
every input beyond that key is an action in the trace. Retention pruning and a
lost journal would break the equivalence; this domain does neither, and that is
why the bound is a bound and not a proof.

### The four guards, minimized

Each guard is a schedule that a design missing the guard would fail. The
schedule is minimized (one-removal-per-pass, repeated) and the minimized form is
re-run, so the recorded witness is the shortest one the harness found:

| Guard | Script → minimal | What the production core did on the minimal schedule |
|---|---|---|
| dedup | 3 → **3** | two committed entries for one mint (`2` slots), one application: supply **100**, both replicas applied 2 of 2 |
| rejection | 3 → **2** | unauthorized mint refused at slot 1 (**UK-7001**), watermark **1**, not halted, supply 0 |
| restart | 7 → **4** | a replica restarted over its journal applied 2 of 2 slots and holds the peer's digest `2:204b0b8d…` |
| revoke | 8 → **4** | a replica that was down for create *and* revoke replays both in log order: revoked, applied 2, digest `2:221b71e0…` equal to its peer |

`saves`: the traces are written to `docs/distributed/evidence/d12-traces/` (one
`.trace` per guard, plus `bounded-search.txt` and `witnesses.txt`). The
`UNFER_D12_TRACES` variable is what puts them there, so the evidence is
regenerated rather than copied by hand.

### What the harness observed and the docs must not smooth over

A storage failure at slot 1 leaves the replica **halted with watermark 0** while
its in-memory certificate ledger already shows the evaluated mint (supply 100,
applied 0, UK-5011, `journal_error` set). Nothing is acknowledged and the
watermark does not move, but the memory view of a halted node is not a state
anyone may read. The test then proves the durable path does not double-count:
crash, restore, and the slot applies exactly once.

**Follow-up (2026-11-23): that memory can no longer be served.** The question was
whether D11's `SharedLedger` — the other applying path — could serve state a node
had evaluated but not recorded. Two findings, both now fixed rather than argued:

* the ledger had the **same** divergence: `append` extended the in-memory bank
  and the id → slot map and only then wrote `ledger.json`, so a failed snapshot
  left slot *n* readable through `committed()`, `committed_since()` and
  `receipt()` while the file did not have it. `append` now rolls the bank, the
  map and the outbox back to the durable prefix when the snapshot fails, and
  `allocate` drops a reservation it could not persist. Measured:
  `a_decision_that_cannot_be_persisted_is_not_served` (read-only state dir →
  `committed() == 0`, receipt `Unknown`, retry commits once) and
  `a_reservation_that_cannot_be_persisted_is_not_kept`.
* a `ConsensusNode` still exposes its memory through plain accessors, which cannot
  fail on their own. It now has `servable()` — an error while halted or while the
  last journal write failed — and the one serving surface in this workspace,
  `DataPublisher::resolve`, checks it: on a halted node the memory still holds the
  content (`node.content(&cid)` is `Some`) and `resolve` returns `None`
  (`a_halted_node_does_not_serve_its_in_memory_state`).

So the answer to the question asked is: not any more. A stopped node's memory is
reachable only by code that ignores the refusal, and both surfaces that a caller
could serve from now refuse.

```text
$ cargo test -p unfer_consensus --offline --features network
unittests src/lib.rs          167 passed
tests/distributed_schedules.rs 7 passed
tests/independent_nodes.rs     1 passed
tests/phase6_testnet.rs        4 passed
tests/power_cut.rs             4 passed
tests/request_identity.rs      6 passed
                                                            EXIT=0
```

Evidence: `docs/distributed/evidence/d12-schedules.log` (the run above with its
report lines), `d12-suites.log`, `d12-traces/`.

### What this is not

* A bounded search is not an all-run proof: 1342 prefixes of 7381 at depth 4,
  6904 of 66430 at depth 5 and 33769 of 597871 at depth 6, alphabet 9, plus the
  32 walks — that is the whole claim.
* The consensus engine is an assumption. Nothing here says anything about
  QuePaxa's ordering or liveness.
* No real transport, filesystem or OS scheduler is involved; `FaultPoint` is the
  existing D07 injection point, not a new fault model.
* Wall-clock timers are not modelled: the harness has no time, only steps, which
  is why the trace's classes are content, scheduling and fuel and not "deadline".

## D13 — abrupt-crash and storage-recovery fault matrix (2026-11-23, steps 1–3 of 3)

**New** `unfer_consensus/tests/crash_matrix.rs` (eleven cases),
`unfer_consensus/tests/support/mod.rs` (the process fixture D11 and D13 share;
`tests/independent_nodes.rs` now uses it, with the same assertions),
`scripts/distributed-recovery` and `docs/distributed/evidence/d13-recovery.tsv`.

### Two kinds of evidence, deliberately kept apart

| Kind | How it is produced | What it can claim |
|---|---|---|
| Deterministic boundary | the D07 journal fault points (`BeforeWrite`, `HalfWrite`, `AfterWrite`), plus a real `chmod` and a real corrupt byte | repeatable statements about the crash protocol at an exact point |
| Process crash | three `unfer_node` processes over TLS, `SIGKILL` at chosen offsets, restarted on the same directory | what a killed process does with files the kernel already has |

Neither is a power-loss measurement. A `SIGKILL` returns nothing from the page
cache that a disk with a volatile write cache might lose; no case here claims
otherwise, and the plan's D13 distinction is preserved rather than blurred.

### The six boundaries, and what each case measured

| Plan boundary | Case | Measured outcome |
|---|---|---|
| before admission persistence | `fail-before-write` | applied **0**, halt `UK-5011`, `journal_error` set; restart applies slot 1 **once** (`applied=1 identities=1`) |
| during the frame write | `fail-half-write` | **119** torn bytes on disk, recovery discards them (`applied=0`), then applies once |
| after persistence, before reply | `fail-after-write` | uncertain sync halts; either the frame survived or it did not — `identities=1` in both worlds, never two |
| after commit, before apply | `process/kill-mid-submit` ×5 | kill at 0/2/5/10/20 ms into a submission; every retry settled, `slots` grew by **exactly one** in all five |
| after apply, before the snapshot | `process/slow-replica-snapshot` | a killed replica's snapshot read `1:` while the survivors had `3:` — the snapshot is written independently of the commit |
| while a snapshot is published | `corrupt-record`, `conflicting-checkpoint` | a corrupt record is refused (`record checksum does not match its payload`), never replaced by an empty log; a journal that disagrees with the decided log halts at the named slot |

Additional matrix rows, because they belong to the same protocol:

* `permission` — the journal made read-only for this user: the next open is
  refused with `Permission denied`, so a node cannot start on a log it cannot
  write. A mode change *while the writer is open* does not stop it, because
  access is fixed at open: that is recorded as the property of this design, not
  as a passing test. Given the journal back, `applied=2 identities=2`.
* `refusal` — a refused entry (UK-6003 on replay) is durable, advances the
  slot, and is replayed rather than re-decided: `applied=2 identities=1`.
* `journal-format` — the framing the matrix relies on: one record of 189 bytes
  recovers as `records=1 last_slot=1 torn=0`.
* `halted-state-fence` (follow-up to D12) — the ledger path of the matrix now
  serves only what it persisted: a snapshot that fails rolls the in-memory bank,
  id map and outbox back, and a `ConsensusNode` exposes `servable()` so a halted
  node's evaluation is not served (`docs/distributed/evidence/d13-halted-state-fence.log`).

### Quorum, and what a minority cannot do

With two of three processes killed, a submission to the survivor **failed**
(`submit_ok=false`) and the committed count did not move (`slots=1->1`). Healing
the two replicas and retrying the same request identity produced exactly one
slot (`2:…`), identical on all three. No coordinated write happened without a
quorum, in either the refusal or the retry.

### Slow replica, restated as a recovery claim

Node 3 stopped while two more decisions were taken: its snapshot still read
`1:` against the survivors' `3:`. Restarted on the same directory it read `1:`
again — the log it did have, never an empty substitute — and then, with the
cluster still idle, its peers pushed the two missed decisions and it held the
**whole** log `3:…`, equal to theirs (`process/slow-replica-idle-catch-up`). One
further decision then moved all three to `4:…`. The lag is a real, stated
property — a replica's snapshot trails its commits — and the repair comes from
its peers, not from a new decision and not from its own poll loop.

```text
$ cargo test -p unfer_consensus --offline --features network --test crash_matrix
11 passed (8 deterministic, 3 process), 0 failed
$ scripts/distributed-recovery --check
distributed-recovery: fixture matches (24 case line(s), 0 gap(s))
```

Evidence: `docs/distributed/evidence/d13-recovery.tsv` (one line per case, with
the values this host measured), `d13-crash_matrix.log` (the full run, including
each assertion's output). `scripts/distributed-recovery` regenerates the fixture
and `--check` compares the *shape* of the run — which cases ran and what each
claimed — never the digests, which belong to one run.

### What this is not

* Not a power-loss claim, and not a filesystem-model claim: both need a storage
  model this task does not build (D13's second paragraph names them).
* The consensus engine's behaviour under partition is an assumption the harness
  supplies (a submission without a quorum is refused by the transport); only the
  *application* side is measured.
* Catch-up no longer needs a decision — the idle-push follow-up in D11's section
  is what made the slow-replica case here pass without a new submission — but it
  does need a live peer: if every peer of a rejoined replica is down, only the
  next decision will move it.
* The kill offsets are chosen, not exhaustive: five points around a submission
  are not every interleaving, and the deterministic cases cover the boundaries
  the offsets cannot place exactly.

### D14 — Ordered fencing for scheduled jobs

A job claim used to be a generation counter held in the calling process. That is
sufficient for one worker and useless for two: the counter a partitioned worker
holds says nothing about what the committed state believes, and a lease-style
answer would need synchronised clocks in a system whose premise is that they are
not. So the token is a fence, and the committed state owns it.

**The fence is `(deployment, resource, epoch, generation)`.** `epoch` is the
reset point — it advances only when `generation` would overflow, or when a caller
asks for it explicitly — and `generation` advances once per claim transition.
Fences are ordered lexicographically on that pair, so "stale" means "not the
current one", never "old in seconds". `FenceTable` in `unfer_consensus/src/jobs.rs`
holds the current fence and holder per resource; a `ConsensusNode` owns one and
rebuilds it from its journal (`storage::JournalRecord::Fence`), so a restart
returns the fences that were written and no others.

| Transition | What it does | What it refuses |
|---|---|---|
| claim | issues the next fence for an idle resource | a resource someone holds (`RESOURCE_HELD`) |
| renew | a fresh generation for the same holder | any fence that is not current |
| release | makes the current fence idle | a stale worker releasing a newer claim |
| reclaim | takes over a *released* resource | a resource that was never released |
| fire / disable | settles under the current fence | a stale or replayed completion |

Round-robin stays where it was: `LeaderLease` is a *preference* about who will
claim next. Correctness rests on the fence, so a wrong preference costs a refused
claim, not a double fire.

**The enforcement point is the sink, and only if it can enforce.** `FencedSink`
is the interface: `OrderedMockSink` rejects any fence it has already passed, so
an effect that was ordered after another cannot land before it — even if the node
that admitted it never saw the newer claim. A sink *without* that interface
cannot be protected by a fence, and this record does not claim one: D15 is where
an unenforceable sink gets a durable outbox and reconciliation instead of a
promise. The same honesty applies to the journal: `settle_job` reports "the effect
reached the sink but the settle could not be recorded" as its own error rather
than as a clean failure, because retrying that blindly would apply the effect
twice.

**What is tested** (`unfer_consensus/tests/ordered_fencing.rs`, 8 cases, plus 10
unit cases in `jobs.rs`):

```text
$ cargo test -p unfer_consensus --test ordered_fencing
two_partitioned_workers_cannot_both_lead ......... ok   B is refused with RESOURCE_HELD
delayed_old_completion_is_refused ................ ok   the old fence cannot commit
claim_and_settle_replay_are_refused .............. ok
generation_overflow_advances_the_epoch ........... ok   (7,MAX) -> (8,1), never a wrap
restart_rebuilds_fences_and_job_state ............ ok   claim survives, fired stays fired
stale_release_of_a_newer_claim_is_refused ........ ok
fence_transitions_are_durable_and_recover_last_wins  ok   3 records, last wins
a_fenced_sink_enforces_order_without_asking_the_node  ok
```

**The first symbolic invariant target for D21** is the one the table above makes
concrete: *for every resource and every settle, the fence under which the effect
was applied is the fence the committed state held when the settle was recorded.*
A stale settle is not a lost update to be repaired later; it is a transition the
state refuses, which is why the target is expressible as an invariant over the
committed fence rather than over a lease interval.

**Replicated claims (the transport follow-up).** The fence table above is per
node, so a claim still has to reach every replica for two workers to get one
answer. `unfer_protocol::JobOp` now carries `Register`, `Claim`, `Renew`,
`Release`, `Fire` and `Disable` as signed `ConsensusTransaction::JobOp` entries.
The fence is **derived from the committed slot order** by every replica; a
command's `expected: JobFence` is only a compare-and-swap precondition (it names
the fence the caller believes is current), never the fence the state will issue.
A `Claim` therefore cannot choose its own epoch or generation. The application
path (`node.rs::apply_job_op`) writes no `JournalRecord::Fence`: replaying the
transition log reconstructs the same claims, so a restart cannot refuse a claim
against itself or trip the D07 comparison.

`unfer_node job --config node.json --job <name>` replays a node's durable ledger
through the same application node and prints `state`, `holder`, `epoch` and
`generation`. It reads the snapshot without taking the running node's lock, like
`digest`.

**Measured over the TLS transport** (`unfer_consensus/tests/independent_nodes.rs`,
three processes, two live replicas, `--features network`): two workers submit a
claim for the same job concurrently through different nodes. Slots 2 and 3 are
both committed; exactly one applies and the other is a durable refusal. All three
processes report the same holder, fence generation `1`, and digest. A `Fire`
with `expected generation 2` (never issued) is committed and leaves the job
claimed; a `Fire` with the current `(1,1)` moves it to fired. Evidence
`docs/distributed/evidence/d14-job-transport.log`; the reproducibly measured
subset is `1 passed, 0 failed`, and the full consensus suite on the same tree is
`220 passed, 0 failed`.

**What this is not.** Not a wall-clock lease: no code path in `jobs.rs` or
`node.rs` reads a clock, and the plan's exclusion ("wall-clock leases being safe
under arbitrary clock skew") is what that buys. Not enforcement for sinks that
cannot enforce — named above, and left to D15. Not a replicated *effect*: the job
lifecycle is in the log, but the sink call and its acknowledgement are still
D15's outbox; a `Fire` records the committed lifecycle decision, not proof that
an external effect happened.

### D15 — Durable outbox for approval-controlled effects

**Files (unfer):** `unfer_ffi/src/outbox.rs` (new, ~700 lines incl. tests), `unfer_ffi/src/handles.rs`, `unfer_ffi/src/lib.rs`, `unfer_ffi/Cargo.toml` (+`sha2 = "0.10"`). **australVM:** evidence `docs/distributed/evidence/d15-outbox.log`.

**What changed.** The outbox sits *under* the existing action queue; `ActionRecord` is unchanged and an applied action still ends `Approved` with a resolved record. The middle of `uk_action_apply` is now a durable protocol:

1. **Intent before dispatch.** `EffectIntent` (request identity, principal, effect, resource, epoch, payload digest) is appended and flushed to the durable store before any sink is called.
2. **Approval bound to the bytes.** `ApprovalBinding` names request, payload digest (`sha256` of canonical JSON), resource and epoch; resolution re-checks all four, so an approval replayed against an altered payload is refused before the sink sees it. The same check runs again immediately before the call, catching a revocation that happened while the operator was reading.
3. **Three outcomes, three records.** `OutboxRecord::{Planned, Dispatching, Acknowledged, Retryable, Uncertain, Refused, Reconciled}`. `Acknowledged` is the sink saying it applied; `Retryable` the sink saying it did not and may be asked again; `Uncertain` is the honest one (marker went out, no ack) and waits for `Outbox::reconcile`.
4. **Idempotency key never changes.** A retry — including the one a restart performs after a crash between sink call and ack — carries the first attempt's key, so a dedup sink applies once. A non-dedup sink goes to `Uncertain` and is never replayed automatically.

**Sink contract.** `EffectSink` exposes `deduplicates`, `enforces_fence`, `apply`. A fence (D14) is required only when the sink can enforce it; a fence a sink ignores is refused rather than promised. `DispatchPolicy`/`AllowAll` is the policy hook at the execution point.

**Recovery.** `handles::recover_durable` scans the trailing dispatch markers and marks them unknown-outcome via `outbox::unresolved_in`. A recovery bug found and fixed this turn: `recover_durable` minted a new handle per record, leaving a resolved action twice (pending ghost + approved); now one handle per id, latest record wins.

**FFI mapping.** `Storage` failure → `Code::STORAGE_FAILED` (5011); `Uncertain`/`Unrecorded` → `UNKNOWN_OUTCOME` (1010); approval/policy/terminal → `CALL_DENIED` (4001). A resolved-record failure *after* a durable ack is 5011, not 1010 (the ack is the durability point). `uk_action_apply` uses the outbox when a durable store exists; the RAM-only inline path is retained.

**What is tested** (`unfer_ffi --lib`, 188 passed, 0 failed; outbox subset 13 passed):

```text
$ cargo test -p unfer_ffi --lib --offline outbox
outbox::tests::intent_is_durable_before_any_sink_is_called .... ok
outbox::tests::approval_covers_request_resource_and_epoch_too . ok
outbox::tests::approval_replay_on_an_altered_payload_is_refused_before_the_sink . ok
outbox::tests::the_idempotency_key_covers_payload_and_request_identity . ok
outbox::tests::a_fence_reaches_a_sink_that_enforces_order .... ok
outbox::tests::a_fence_is_refused_before_a_sink_that_cannot_enforce_it . ok
outbox::tests::crash_after_sink_success_before_ack_does_not_duplicate_at_a_dedup_sink . ok
outbox::tests::no_dedup_sink_enters_reconciliation_instead_of_being_retried . ok
outbox::tests::reusing_a_request_identity_for_a_different_payload_is_refused . ok
outbox::tests::revoked_worker_fails_at_the_execution_point_and_never_reaches_the_sink . ok
outbox::tests::retryable_failure_is_recorded_apart_from_acknowledgment . ok
outbox::tests::open_checked_refuses_an_unreadable_effect_record . ok
tests::approved_action_dispatches_through_the_outbox_end_to_end . ok
```

End-to-end: JSONL store, `outbox_kinds == ["planned","dispatching","acknowledged"]`, restart recovers `Approved` (not unknown-outcome), second apply returns `-4005` (`ACTION_ALREADY_RESOLVED`). `resolved_checkpoint_failure_surfaces_unknown_outcome` updated to `fail_on: 4` with a comment naming the outbox flush sequence (checkpoint 1, planned 2, dispatching 3, ack 4, resolved 5). Full consensus suite green (217 passed, 0 failed).

**What this is not.** Not a second approval system: `uk_gate_approve`/`uk_action_apply` are unchanged. Not a delivery guarantee: it records what happened and says so when the record is missing. Not enforcement for sinks that cannot enforce — those go to reconciliation. Epoch 0 is used for local actions that have no authority epoch; this is documented, not invented.

### D16 — One total Austral distributed transition

**Files (australVM):** `examples/distributed/RequestState.{aui,aum}` (new), `examples/distributed/RequestStateReference.rs` (new), `test-programs/suites/028-distributed-request-state/` (new), `scripts/check-request-state` (new), `test-programs/runner.py` (suite wiring). **unfer:** none. **Evidence:** `docs/distributed/evidence/d16-request-state.log`.

**The state machine.** The finite lifecycle is `Idle`, `Pending`, `Applied`, `Rejected`, with explicit `Submit`, `MatchingResponse`, `Retry` and `CancelWait` events. Every non-idle state carries the stable request id and the payload commitment (`held_request`, `held_digest`), so a response is checked against the identity and bytes the state actually accepted. `CancelWait` leaves the request `Pending`: cancellation is a local decision to stop waiting, not rollback. A matching response applies; a wrong request id leaves `Pending`; a matching id with a different digest rejects; a repeated response is a no-op. No arithmetic is performed, and the `maximum_int64` vector checks the bound without overflow.

**The total fragment.** `step` carries `Liquid_Requires(Contract => "true")`, so the existing `TotalityCheck` gate checks it for T1 (no recursion), T2 (no `while`/`for`) and T4 (no foreign calls). No validator extension was needed: the transition uses only records, unions, `case`, `if` and return. `main` is outside the fragment and only prints the fixed vectors. The refusals are pinned as compiler fixtures: `002-refuse-foreign` (T4), `003-refuse-loop` (T2), `004-refuse-recursion` (T1).

**Differential execution.** `scripts/check-request-state` compiles the same module through the CPS JIT (`--use-cps-jit`), compiles the independently written Rust reference, runs both on the fixed vectors, and diffs stdout. The reference is not generated from the C output. Measured: `12 vectors agree`; source digests in the evidence log (`RequestState.aum` `ed591b03…`, `RequestState.aui` `0573cae4…`, reference `d3ac6034…`). The `restart-resume` vector rebuilds `Pending` from the durable request id and digest and continues; continuation depends on no other state. `python3 test-programs/runner.py 028` passes all five cases (lifecycle, source-copy, and the three totality refusals).

**What this is not.** Not a proof of the protocol: it is a total transition and a differential test on finite vectors. Compiler correctness remains a named trust boundary, as does the C compiler. The module does not call the network or a sink; it decides the lifecycle state, and D14/D15 carry that decision through consensus and the outbox.

### D17 — Linear local wrappers for authenticated remote resources

**Files (australVM):** `stdlib/net/Distributed.{aui,aum}` (new),
`safestos/cranelift/src/distributed.rs` (new), `examples/distributed/RequestTransport.aum`
(new), `test-programs/suites/022-distributed/` (new), `scripts/check-distributed-wrappers`
(new), `umbrella/modules/DistributedRequest.aum`, `umbrella/tests/distributed_request.rs`
and `umbrella/tests/distributed_tls.rs` (new), `umbrella/Cargo.toml` (mutual-TLS test
dependencies), `test-programs/runner.py` (suite wiring). **unfer:** none. **Evidence:**
`docs/distributed/evidence/d17-distributed-wrappers.log`,
`d17-tls-harness.log` and `d17-replay-audit.log`.

**The wrappers.** `PendingRequest`, `ResultReceipt`, `RemoteResource` and
`AuthorityToken` are linear records over process-local host handles. The handle is
never serialized: `(request_id, digest, resource, deployment, owner, epoch)` lives
in the host authority, and a wrapper can only be produced by a foreign verb. The
states are explicit — submit and retry consume the request and return the same
wire identity; wait consumes it and returns either the same request or one
receipt; cancel-wait stops only the local wait; every state has a disposal path.
The host authority in `distributed.rs` keeps the D14 fence generation and the
D15 `planned`/`dispatching`/`acknowledged` outbox sequence for each submitted
request. This is the host authority itself, not the live QuePaxa transport or the
durable outbox.

**Measured.** `bash scripts/check-distributed-wrappers` compiles
`RequestTransport` through the CPS JIT and links the generated C against the
release bridge; its 8-line stdout is diffed (`transport vectors agree`). The
same script runs the host authority's 22 tests: forged handles, wrong owner,
wrong deployment, retired epoch, replayed wire token, mismatched digest, retry
identity/fence, numeric/capacity guards, the fence/outbox path, and the replay
and receipt-lifetime cases below (`test result: ok. 22 passed`).
`python3 test-programs/runner.py 022-distributed` passes 5/5 compile-time
fixtures (copy, use-after-submit, double receipt, wrong-state access, leak). The
umbrella integration test compiles the module through the compile-only
front-end and executes `run` in-process (`test result: ok. 1 passed`). Source
digests are in the evidence log; the release bridge defines 18 `au_dist_*`
symbols.

**Live harness.** `distributed::Transport` is the FFI seam. Installing one with
`distributed::set_transport` makes every `au_dist_*` verb dispatch as an
`Operation` to that transport instead of the process's `Authority`; both sides
share `apply`, so a remote authority runs exactly the same validation. The
`distributed_tls` integration test generates a per-run CA plus server and client
certificates, configures rustls with mutual client verification, spawns a child
process that owns the `Authority`, and runs `DistributedRequest.aum` in the
parent through the front-end and JIT. Every verb crosses a 41-byte framed TLS
message and returns an 8-byte status; the module's `au_dist_respond` crosses the
same path, so no response is injected in-process. The child rejects an
unexpected EOF and exits only after the client's TLS `close_notify`. Measured:
`test result: ok. 2 passed` (the harness plus its child entry), with the existing
in-process tests still passing. The harness also sends the replay refusals over
the same socket after the module run: stale wire adoption (`-15`), duplicate
request id (`-2`), tombstone capacity (`-14`), replayed response (`-11`),
retired-epoch cleanup (`NO_TOKEN`/`NO_REQUEST`) and the old-epoch token refusal
(`-6`). The child's request-id cap is lowered to 2 for the harness so the
capacity case does not need 65k round trips; the production default remains
65,536. This is loopback
mTLS with process isolation, not a consensus or durability layer.

**Adversarial replay audit.** The authority now distinguishes a *resource
incarnation* from its caller-supplied `(resource, deployment, owner, epoch)`
tuple. Releasing a resource and introducing the same ID again gives the new
resource a fresh incarnation, so stale tokens and requests cannot be carried
forward:

* `token_wire` twice returns `STATUS_NO_TOKEN`; a second `adopt_wire` of the same
  bytes returns `STATUS_REPLAY`, and a second `authorize` of the adopted handle
  returns `STATUS_NO_TOKEN`.
* A wire captured before release cannot be adopted against the re-introduced
  resource (`STATUS_STALE_RESOURCE`); release also drops that incarnation's
  local tokens, requests and request-id tombstones, so forged local handles fail
  with `STATUS_NO_TOKEN`/`STATUS_NO_REQUEST`.
* Retiring an epoch drops that epoch's tokens, requests, receipts, wires and
  request-id tombstones. Old handles fail with `STATUS_NO_TOKEN`/
  `STATUS_NO_REQUEST`, and issuing a token for the old epoch returns
  `STATUS_RETIRED_EPOCH`; the first request in the new epoch starts clean.
* A request id names one action while its `(resource, incarnation, epoch)`
  lives. Reusing it for an active request, a second live resource, a completed
  request, or a disposed request returns `STATUS_DUPLICATE`; releasing the
  resource or retiring the epoch frees it.
* A response is accepted once (`STATUS_OK`); a replay returns
  `STATUS_ALREADY_SENT`, and after the terminal `wait` consumes the request it
  returns `STATUS_NO_REQUEST`. Incarnation overflow is refused before insertion
  (`STATUS_OVERFLOW`).

The tombstone set is bounded at 65,536 ids per authority; a full set fails
closed with `STATUS_CAPACITY` and release/retirement reclaims it. The wire cache
is bounded at 65,536 values: under pressure stale wires are evicted first, while
a cache full of live wires refuses serialization with `STATUS_CAPACITY` without
consuming the local token. Release drops receipts created by that incarnation,
so an unreleased receipt cannot outlive its resource. A rejected response yields
a `RESULT_REJECTED` receipt on `wait`; `cancel_wait` alone does not mint one and
leaves the request pending. Both terminal receipts and pending requests are
purged by release/retirement. Tombstones remain process-local, not a durable
request book; the D06 consensus book remains the durable identity authority.
The audit transcript is `docs/distributed/evidence/d17-replay-audit.log`.

**Replay.** `token_wire` consumes the local token and `adopt_wire` accepts each
wire id once. The test serializes a token, adopts it, then replays the same bytes
and gets `STATUS_REPLAY`; the original handle is already gone, so there are not
two authorized owners. Remote ownership transfer remains out of scope: resource
exclusivity is the ordered authority's, not the copied bytes'.

**What this is not.** The live harness reaches one D17 `Authority` directly over
loopback mutual TLS; it is not the QuePaxa transport, does not order requests
through consensus, and adds no crash recovery or durable outbox. The in-process
authority remains the default when no transport is installed. The compile-time
fixtures pin linearity diagnostics, not compiler soundness; compiler correctness
remains a named trust boundary.

### D18 — Enforceable distributed verification profiles

**Files (australVM):** `safestos/cranelift/src/verification.rs` (new), `safestos/cranelift/src/{module,distributed,federation}.rs`, `safestos/cranelift/src/bin/modhost.rs`, `safestos/cranelift/src/lib.rs`, `lib/Compiler_plugin.{ml,mli}`, `lib/Compiler.{ml,mli}`, `lib/CliParser.{ml,mli}`, `lib/CliEngine.ml`, `lib/liquid/LiquidWhy3.ml`, `lib/liquid/LiquidCheck.ml`, `umbrella/src/{frontend,main}.rs`, `test/CliParserTest.ml`, `test/CliEngineTest.ml`, `scripts/check-verification-profile` (new). **unfer:** none. **Evidence:** `docs/distributed/evidence/d18-verification-profiles.log`.

**A requirement the host reads.** A versioned `[verify]` table (`version = 1`) may state `liquid`, `totality`, `protocol`, `schema`, `proof_tier` (`none|smt|mm0`) and `distributed` (`off|optional|required`). Unknown keys and versioned fields without `version = 1` are refused, so a typo cannot look like an enforced policy. `distributed = "required"` also requires `[module] profile = "strict"`, required liquid and totality, protocol/schema identities, and proof tier `smt` or `mm0`. The legacy L8 table (`liquid` only, no version) keeps its old `liquid.ok` gate unchanged.

**Evidence is separate from the claim.** A `verification.ok` sidecar records `version`, `liquid`, `totality`, `transitions`, `covered`, `protocol`, `schema`, `proof_tier`, `identity`, `artifact`, `authorizer` and `deployment`. `ModuleManifest::load` and hot swap run the same `VerificationRequirements::check_evidence` gate, and `modhost verify` exposes it directly. A required deployment fails on a missing sidecar, a short transition cover (`8/9`), a stale protocol/schema, a lower proof tier, a stub DID, a non-SHA-256 artifact id, `allow-all`, or `local-consensus`; an optional profile with no sidecar succeeds with a visible `[gap]` note. The compiler threads the required posture explicitly: `--liquid-required` sets `Compiler_plugin.liquid_required`, `LiquidWhy3.required()` reads it before its environment fallback, and `LiquidCheck` invokes the prover when either required mode or `AUSTRAL_LIQUID_VERIFY` asks. The umbrella front-end unions the CLI flag with the manifest posture rather than relying on the caller's environment.

**Refinement at the FFI edge.** The D17 authority validates request ids/digests (`STATUS_INVALID`), checks generation with `checked_add` (`STATUS_OVERFLOW`), and caps the outbox at 64 (`STATUS_CAPACITY`) before minting or mutating state; these are the length/counter/epoch refinements this fragment supports, and they run before any unsafe boundary. Non-federated artifact identities are now SHA-256 with an explicit `sha256:` prefix and a labelled stub identity; federation exposes `create_consensus_node_strict`, which refuses `LocalConsensus`, and strict evidence refuses stub DIDs at the manifest gate. Local development modes remain available and labelled (`[gap]`, `authorizer = "allow-all"` is refused only when strict).

**Measured.** `bash scripts/check-verification-profile` passes: required mode fails closed for a missing prover (exit 127) and for a prover that refuses `deterministic_goal`; optional mode exits 0 with `liquid: [gap]`; the complete strict profile is accepted at `9/9`, while the missing sidecar, an `8/9` cover, stale protocol/schema, `proof_tier = none`, a stub DID, a `deadbeef` artifact id, `allow-all` and `local-consensus` are each refused. `verification::` 10/10, `distributed::` 22/22 (the D17 replay and lifetime audits added twelve) and `federation::` 3/3 unit tests pass; `cargo test --manifest-path umbrella/Cargo.toml --offline` passes all suites; `dune build lib/ bin/ test/` and `CliParserTest`/`CliEngineTest` pass. Source digests and the exact transcript are in the evidence log.

**What this is not.** The sidecar is a claimed result from the named tools, not an independently proof-checked certificate. SMT success remains Why3/solver evidence and finite transition coverage covers only the enumerated cases; no solver or MM0 checker is invoked by this gate. Compiler correctness remains a named trust boundary.

### D19 — Channel contracts and a production hook census

**Files (unfer):** `unfer_protocol/src/channel_contracts.rs` (new), `unfer_protocol/src/lib.rs`, and the D19 marker comments in `unfer_consensus/src/{net,node,sim}.rs` and `src/bin/unfer_node.rs`; `unfer_consensus/src/net.rs` now constructs the live contract. **australVM:** `scripts/check-channel-contracts` (new), `docs/distributed/channel-hooks.tsv` (new), `safestos/cranelift/src/channel_contracts.rs` (new host probe), `stdlib/net/ChannelContract.{aui,aum}` (new), `examples/distributed/ChannelContractProbe.aum` (new), `scripts/check-distributed-wrappers`, `docs/distributed/contracts.tsv`, this record. **Evidence:** `docs/distributed/evidence/d19-channel-contracts.log`.

**Contracts are values, not call-site comments.** `ChannelContract` records the producer and consumer locations, schema name/version, ordering scope, retry lifetime, authentication, loss/reordering model, a bounded capacity with a witness, delivery mode, optional completeness frontier, and whether the decision is monotone. `OrderingScope::PerConnection` covers only `RetryScope::WithinConnection`; a reconnect retry has to name `TotalOrder` or state that it is unordered. `ordered_channel()` returns its marker only when the ordering scope covers the retry lifetime.

**The constructors refuse the unsafe promotions.** `CommutativeFold::new` accepts addition, max, min and union and refuses subtraction and division. `Delivery::at_least_once().promote_to_exactly_once(None)` is refused; a promotion needs a `DuplicateSuppression` and a per-connection suppressor is refused even when bounded. `UsageCap` refuses a zero bound, and `CompletenessGuarantee::from_local_timeout` is a refusal, not a frontier; a nonmonotone contract without `OrderedClose`, `Watermark` or `Quorum` is rejected. `VoteTally` counts distinct eligible authenticated members, so three packets from one member cannot satisfy a two-member quorum (the D19 counterexample).

**The census ties hooks to the inventory.** `docs/distributed/channel-hooks.tsv` lists 17 production send/receive/timer/retry/storage/simulator hooks across four files, each with a `// D19-HOOK: Hxx` marker and a contract row. `scripts/check-channel-contracts` checks both directions (inventory row to marker, marker to inventory row) and scans production code for known primitives that have no marker nearby. It also refuses duplicate IDs, unknown kinds, unknown contracts, wrong-file markers and an unregistered source hook. The scan is shallow by design: it proves the site is listed, not that the contract's semantic fields are true.

**The live path uses it.** `consensus_live_submission_contract()` is one definition in `unfer_protocol`, shared by `start_replica`, `submit_client` and the in-process harness. All three refuse to proceed unless the contract yields its ordered-channel and exactly-once markers; the host bridge constructs the same contract for the Austral wrapper. The wrapper holds an opaque `LiveSubmissionContract`; `provesOrdered` and `provesExactlyOnce` ask the host, and a forged or released handle is refused. The example prints `contract ordered=1`, `contract exactly_once=1`, `contract release=0`.

**Measured.** `cargo test --offline -p unfer_protocol --lib channel_contracts` passes 14/14; the full `unfer_protocol` lib is 367/367. The consensus live-contract test passes, and a negative test proves the gate refuses a per-connection-ordering contract and an at-least-once contract instead of only checking the one hard-coded live value. The bridge `channel_contracts` tests pass 2/2. `scripts/check-distributed-wrappers` still reports 22 bridge, 1 umbrella and 2 live-TLS tests, plus 18 `au_dist_*` and 4 `au_channel_*` symbols; its Austral probe compiles and executes. `scripts/check-channel-contracts --selftest` refuses all seven mutation controls, and the live census reports 17 hooks across four files with every marker registered. `scripts/check-distributed-contracts` reports all contract rows and evidence present.

**What this is not.** Only the consensus submission path and its Austral probe construct a contract; other boundaries in the census remain named but not yet wired. The hook census is marker-based and pattern-assisted; a new hook written in an unlisted form can escape it until the pattern or the site is added. The Austral wrapper reports host-checked markers but does not itself send or order a message. Completeness frontiers remain a caller-supplied premise, not a consensus proof.

### D20 — Finite request trace certificate in MM0

**Files (australVM):** `docs/distributed.mm0` (new theory), `scripts/distributed2mm0` (new), `mm0/distributed_trace.{mm0,mm1}` (new generated pair), `mm0/fixtures/distributed_request_trace.json` (new pinned trace), `scripts/verify-invariants`, `scripts/mm0-comparator`. **Evidence:** `docs/distributed/evidence/d20-distributed-trace.log`.

**What the certificate says.** One pinned trace of 10 events: open(7,11,1), submit, persist, accept, release; open(8,12,1), submit, persist, reject, release. The state carries `(request, digest, epoch)`, and persistence is a state (`st_pending` vs `st_durable`), not a comment. `accept`/`reject` have no edge from `st_pending`, and their event must carry the request's exact identity. The proof is one `run_cons` per event, so `mm0-c` re-checks every transition. The artifact is `5 sorts, 21 term/def, 10 ax/thm`; the trust split is `structural=10 grounded=0 certificates=1`, with no `assumed` leaves.

**Mutations are load-bearing, not decorative.** The emitter's audit refuses response request-id, digest and epoch changes, event reordering, and an omitted persistence boundary. `verify` then mutates the committed certificate text and requires `mm0-rs` to reject each mutant; a mutation that fails to change the artifact is a failure. The clean pair is accepted by `mm0-rs` and `mm0-c`.

**Measured.** `scripts/distributed2mm0 selftest` refuses all five trace mutations; `scripts/distributed2mm0 check` confirms the pair is in sync; `scripts/distributed2mm0 verify` reports the clean certificate accepted and all five mutants rejected. `scripts/verify-invariants` now runs the drift check and, when the toolchain is present, the verify half. `scripts/mm0-comparator` registers the emitter for the owner's approval workflow.

**What this is not.** This is a **finite trace certificate**, not an all-run proof or induction. It does not prove the consensus engine's ordering, the D17 authority's identity rules, or that the pinned trace is the only execution the code can produce. The theory is small enough to audit by eye; its missing edges are the refusals above. Production statement approval remains the owner's `mm0-comparator` workflow, and the artifact is not yet in `mm0/approved/`.

### D21 — Symbolic invariant (finite model check; induction open)

**Files (unfer):** `unfer_consensus/tests/fence_invariant.rs` (new). **australVM:** evidence `docs/distributed/evidence/d21-fence-invariant.log`.

**What is checked.** A finite-state model of the D14 fencing state machine (claim/renew/release/reclaim/fire, generation advance with epoch overflow, fence comparison) is explored by BFS over **every reachable state and every outgoing transition** — a complete model check of the finite abstraction, not a sampled trace:

```text
fence invariant: states=2116 transitions=507840 settled=3312 violations=0
conformance: 4800 steps compared, 0 disagreements, 178 stale tokens presented, 0 accepted
conformance coverage: 6 operations, 181 stale tokens presented, 0 accepted
```

Three guard mutations are applied and each is caught with a witness: `StaleSettle → I1`, `WrapGeneration → I2`, `StaleRelease → I3`. Differential conformance compares the model against the real `JobQueue` over 4800 steps with 0 disagreements. Two precision fixes were required and are recorded: the real successor rule is `generation + 1` up to `u64::MAX` (conformance passes `top = u64::MAX`, the model check passes `MAX_GEN = 3`), and "a release accepted on a stale fence" is a transition property checked in `explore`, not a state invariant.

**The three D21 claims targeted:** repeated matching request preserves its result; authority epoch never decreases; a stale fence never changes the protected state. The fence-invariant test machine-checks the third (stale fence) concretely; the first two remain as stated targets for the symbolic proof.

**What this is not.** This is **not** the MM0/Lean induction over arbitrary runs: the symbolic proof of initialization + one-step preservation + induction over finite runs (D21 step 2) and the audit of theory consistency against rejected axiom candidates (step 3) are still open, and are recorded as such rather than substituted with sampled traces. D20 now supplies one finite MM0 trace certificate, but a single trace is not the induction premise; D21 still needs a symbolic transition relation and a reviewed induction rule. The model check is a consistency check over its finite domain, not a general consistency proof, and bitwidth premises must match execution (they do: `u64`).

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
| D09 | verified within stated scope | `umbrella` pins artifact identity before build and load; the host issues per-instance directory and single-peer network handles; `rs_cap_*` bounds, copies and contains them; an Austral module compiled by the stock front-end writes through the handle, and the same module without the grant is refused with nothing written. Gaps named above (no live transport, directory capability only end to end, umbrella capability binding untested) |
| D10 | verified within stated scope (steps 1a–3) | canonical versioned frame + explicit no-op + domain-separated preimage (`unfer_protocol` 353+4+12+51+3); `net.rs` replicates `ReplicatedValue`, applies from the decision, keeps only a local outbox (`unfer_consensus` lib 162, phase6 4, request_identity 6, power_cut 4); `signing.rs` signs the domain-separated frame and still verifies legacy layout; evidence `docs/distributed/evidence/d10-*.log`. Gaps: no live transport carried these frames when this row was written (D11 added one), and legacy signature acceptance cannot be retired without a cutover signal |
| D11 | verified within stated scope (steps 1–3) | one replica per process from a validated `DeploymentConfig`/`NodeConfig`, an exclusive state-directory lock, a `SharedLedger` with no submission pump, and `unfer_node serve|submit|digest`; three processes with disjoint state commit identical digests, a second process on a live directory is refused, and a killed node converges on the survivors' full log after the next decision (`d11-independent.log`); harness suites still green (phase6 4, request_identity 6, power_cut 4) |
| D12 | verified within stated scope (steps 1–3) | `sim.rs` drives real `ConsensusNode`s over real journals, with delivery, quorum membership, journal outcome, crash and restore as explicit recorded inputs; bounded search of the fixed domain (2 clients, 3 replicas, 3 requests, 9 actions) executed 1342 of 7381 prefixes at depth 4 (545 pruned, 648 leaves, 21 s), 6904 of 66430 at depth 5 (2898 pruned, 3239 leaves, 153 s) and 33769 of 597871 at depth 6 (14617 pruned, 15400 leaves, 720 s on tmpfs), finding no violation at any depth; the 24 ms/prefix that dominated the first run was a per-ledger rebuild of the sparse-Merkle default chain, now computed once per process; 32 seeded walks of 12 steps reached 30 distinct states with no violation; four guards (dedup, rejection, restart, revoke) are witnessed by minimized schedules (3→3, 3→2, 7→4, 8→4) re-run against the production core; evidence `docs/distributed/evidence/d12-schedules.log`, `d12-suites.log`, `d12-traces/`. Limits: the consensus engine is an assumption, the search is bounded (not an all-run proof), and the halted-node in-memory divergence observed at a storage fault was handed to D13 and is now fenced (see the follow-up in this section) |
| D13 | verified within stated scope (steps 1–3) | eleven cases: eight deterministic boundaries (journal `BeforeWrite`/`HalfWrite`/`AfterWrite`, a read-only journal, a corrupt record, a conflicting checkpoint, a refusal replay, the journal framing) and three real-process matrices (SIGKILL mid-submit at 0/2/5/10/20 ms, quorum loss and heal, slow-replica catch-up); each acknowledged slot survived, every logical request grew the log by at most one slot, no failed domain operation applied, and a corrupt journal was refused rather than replaced by empty state; evidence `docs/distributed/evidence/d13-recovery.tsv` (24 case lines, 0 gaps), `d13-crash_matrix.log`, regenerated by `scripts/distributed-recovery`; the D12 follow-up adds the halted-state fence (`d13-halted-state-fence.log`). Limits: a process crash is not a power-loss claim, the consensus engine's partition behaviour is an assumption, and a replica's snapshot trails its commits until the cluster informs it, though the idle-push follow-up in D11's section now repairs a rejoined replica without a new decision |
| D14 | verified within stated scope (steps 1–5 + transport) | scheduled jobs are *ordered fenced* to the plan's tuple, not a local counter. The committed state owns one current `JobFence { deployment, resource, epoch, generation }` per resource, stored in the ledger snapshot (`net.rs` `LedgerSnapshot.jobs`, mutated only through `SharedLedger::with_jobs` under the ledger lock and persisted as one coherent snapshot). Transitions are explicit: `claim` at the current epoch (a replay by the same holder is idempotent, no generation bump), `renew` (new generation, same epoch), `release` (re-queue and supersede), `reclaim` (restart takeover), `take_over` (a later epoch supersedes a partitioned leader) and `settle`/`disable`; only the current fence (fence equality plus holder) settles the job or mutates its protected resource, so an old worker may keep computing but cannot commit. Ordering is `(epoch, generation)`; generation overflow is refused, never wrapped; no transition reads a clock; `FencedSink` models a sink *with* an enforcement interface and refuses a stale fence (a sink without one is not claimed to be fenced). Step 5 gives that command a durable submission path: `SharedLedger::allocate_job` persists it in a `job_outbox` (part of the same snapshot), proposes it to the pump, re-proposes it after a restart, and `append` clears the pending entry by content when the decision applies it — the same durability rule the transaction outbox follows. Step 4 makes the claim an ordered *command*: `JobCommand` (claim, take_over, renew, release, reclaim, settle, disable) is encoded under its own schema (`JOB_COMMAND_SCHEMA = 2`) as a replicated value, and `SharedLedger::append` dispatches by schema so both ledger transactions and job commands flow through the one ordered apply; a refused command is a deterministic no-op on every replica. Evidence: `unfer_consensus` job suite (17 tests, including `two_partitioned_workers_both_believing_they_lead`, `restart_reclaims_the_claimed_job`, `stale_release_cannot_release_a_newer_claim`, `claim_replay_by_the_same_holder_is_idempotent`, `generation_overflow_is_refused_not_wrapped`, `sink_rejects_a_stale_fence`), the node-level release/retry test, the network-feature witnesses `net::tests::the_job_fence_is_owned_and_durably_held_by_the_committed_ledger` (a claim survives a reopen, then a reclaim makes the old fence stale) and `net::tests::a_job_command_converges_across_replicas_and_a_refusal_mutates_nothing` (same command stream → identical fence on two ledgers; a superseded settle changes neither), `net::tests::an_allocated_job_command_is_durable_and_cleared_when_decided` (a pending claim survives a reopen and is re-proposed, then a decided one is not), and the full `unfer_consensus --features network --lib` suite (184 passed). First symbolic invariant target for D21: *a stale fence never changes the protected state*, over the claim/renew/release/reclaim/take_over/settle/disable vocabulary, with `FencedSink` as the executable witness that a rejected stale mutation appends no effect. Limits: the proposal path is the in-process submission pump (as the ledger path is); TLS transport contention now exercised (`d14-job-transport.log`: two processes contending commit both claims but only one applies; all three replicas report the same holder, generation 1 and digest); no external sink driven end to end (that dispatch is D15's), and no external sink is driven end to end — that dispatch is D15's; enforcement beyond the deterministic ordering is the sink's, not this module's |
| D15 | verified within stated scope (steps 1–3) | effects go through a durable dispatcher, not inline. `unfer_ffi/src/dispatch.rs` commits an `EffectIntent` (request identity, principal, resource, epoch, effect, Observe/Mutate kind, payload digest) to the new `dispatches` stream and flushes it **before** any sink is called; the approval is an `Approval { request_id, resource, epoch, payload_digest }` that must bind the committed intent exactly, so a replayed approval for an altered payload, another resource or another epoch is refused; policy is rechecked at the execution point (the existing sensitive-forward latch is the policy wired into `uk_action_apply`), and the outcome is recorded as `Acknowledged`, `Retryable` or `Uncertain` — distinct durable states, never collapsed into "done". Retries reuse one idempotency key derived from the intent identity; `Uncertain` may be retried only at a sink whose `dedups()` is true, and an uncertain irreversible action at a non-dedup sink moves to `Reconciling` and is not replayed. `uk_action_apply` is the existing effect path that now uses the dispatcher end to end (when a durable store is configured; without one there is no durable commit, so the in-memory path remains), with `ActionRecord` gaining `kind` and `epoch` (serde-defaulted) recorded at submission. Evidence: 11 dispatcher unit tests (`approval_replay_on_an_altered_payload_is_refused`, `a_revoked_worker_fails_at_the_execution_point`, `crash_after_sink_success_before_acknowledgment_does_not_duplicate_at_a_dedup_sink`, `an_uncertain_irreversible_effect_enters_reconciliation_and_is_not_replayed`, `a_retry_reuses_the_same_idempotency_key`, `dispatch_without_a_durable_commit_is_refused_and_never_reaches_a_sink`, …), two FFI tests (`action_apply_dispatches_through_the_durable_dispatcher`, `a_sensitive_latched_principal_is_refused_at_the_execution_point`), `unfer_ffi` lib 188 passed, `unfer_protocol` lib 353 passed. Limits: the only sink is the in-process `local-inprocess` sink (no external network sink yet, so "at least one existing effect path" is the action queue's local effect), `uk_action_submit`'s immediate observe/vetted path still applies inline, reconciliation of a non-dedup uncertain action has no operator tooling beyond the durable `Reconciling` record, and existing `ActionRecord`s without `kind`/`epoch` read as Mutate/0. |
| D16 | verified within stated scope (steps 1–3) | **new** `examples/distributed/RequestState.{aui,aum}`, `RequestStateReference.rs`, `test-programs/suites/028-distributed-request-state/`, `scripts/check-request-state`; U `test-programs/runner.py`. Finite `Idle/Pending/Applied/Rejected` lifecycle with stable request id and payload digest; `step` is a contracted total function (no recursion, loops or foreign calls) accepted by the existing gate. The same 12 event vectors (including restart continuation) run through the CPS JIT and an independent Rust reference with identical stdout (`12 vectors agree`); source digests are recorded. Three compiler fixtures pin T4 (foreign call), T2 (loop) and T1 (recursion) refusals; `python3 test-programs/runner.py 028` passes 5/5. Compiler correctness and the C compiler remain named trust boundaries. |
| D17 | verified within stated scope (steps 1–3 + live TLS harness) | **new** `stdlib/net/Distributed.{aui,aum}`, `safestos/cranelift/src/distributed.rs`, `examples/distributed/RequestTransport.aum`, `test-programs/suites/022-distributed/`, `scripts/check-distributed-wrappers`, `umbrella/modules/DistributedRequest.aum`, `umbrella/tests/distributed_request.rs`, `umbrella/tests/distributed_tls.rs`; U `test-programs/runner.py`, `umbrella/Cargo.toml`. Linear wrappers over host handles; `submit`/`retry` consume the request and preserve wire identity, `wait` returns pending or one receipt, and every state has an explicit disposal/cancel-wait. Five compile-time fixtures pin copy, use-after-submit, double receipt, wrong-state access and leak refusals; the host authority rejects forged handles, wrong owner/deployment, retired epochs and replayed wire tokens; resource incarnations, scoped bounded request-id tombstones, a bounded stale-wire cache, retired-epoch cleanup and receipt release close replay and lifetime leaks (`22 passed`, including the D17 replay audit). The positive transport agrees on 8 lines through the CPS JIT/C bridge, and the umbrella compile-only front-end plus in-process JIT executes the authored module (`1 passed`). The live harness installs a `distributed::Transport`, spawns a child `Authority`, connects both processes with per-run mutual TLS, and runs the same module with every `au_dist_*` verb crossing the socket (`2 passed`, clean TLS shutdown); `python3 test-programs/runner.py 022-distributed` passes 5/5; source digests and 18 `au_dist_*` symbols are recorded. Remote ownership transfer is out of scope; the harness reaches one authority directly, not through QuePaxa ordering or a durable outbox; compiler correctness remains a trust boundary. |
| D18 | verified within stated scope | **new** `safestos/cranelift/src/verification.rs`, `scripts/check-verification-profile`; U `module.rs`, `distributed.rs`, `federation.rs`, `modhost.rs`, compiler/CLI/liquid seams, umbrella posture. Versioned `[verify]` requirements and a separate `verification.ok` sidecar are enforced at load, swap and `modhost verify`; unknown/versionless fields are refused. `--liquid-required` is an explicit compiler input. Required mode fails on a missing or refusing prover, optional mode reports `[gap]`; strict evidence refuses an incomplete cover, stale protocol/schema, low proof tier, stub DID, non-SHA-256 artifact, `allow-all` and `local-consensus` (`10/10` verification tests). D17 counters/ids are checked before unsafe FFI (`STATUS_INVALID`/`OVERFLOW`/`CAPACITY`/`STALE_RESOURCE`, `22/22 distributed`); non-federated artifact ids are SHA-256 (`3/3 federation`). SMT verdicts remain solver evidence and the sidecar is not a checked certificate. |
| D19 | verified within stated scope (live submission path + Austral probe; other boundaries pending) | **new** `unfer_protocol/src/channel_contracts.rs`, `safestos/cranelift/src/channel_contracts.rs`, `stdlib/net/ChannelContract.{aui,aum}`, `examples/distributed/ChannelContractProbe.aum`, `scripts/check-channel-contracts`, `docs/distributed/channel-hooks.tsv`; U `unfer_consensus/src/{net,node,sim}.rs`, `src/bin/unfer_node.rs`. Contract values carry producer/consumer, schema, ordering/retry scopes, authentication, fault model, bounded capacity, delivery, completeness and decision kind. Constructors refuse per-packet quorum, unordered subtraction, at-least-once promotion without request-identity dedup, per-connection dedup for exactly-once, local-timeout completeness and nonmonotone decisions without a frontier (`14/14`). `start_replica`/`submit_client` construct the live contract and refuse without both markers; the bridge exposes 4 `au_channel_*` verbs and the Austral example prints ordered/exactly-once/release. The census registers 17 hooks and refuses 7 mutation controls. Other boundaries are listed but not wired; `unfer_protocol` lib is 367/367. |
| D20 | verified within stated scope | **new** `docs/distributed.mm0`, `scripts/distributed2mm0`, `mm0/distributed_trace.{mm0,mm1}`, `mm0/fixtures/distributed_request_trace.json`. A 10-event request trace is a run of the state-bound automaton; `mm0-rs` compiles `5 sorts, 21 term/def, 10 ax/thm`, `mm0-c` accepts every transition, and the trust split is `structural=10 grounded=0 certificates=1`. Five mutations (response id, digest, epoch, event order, omitted persistence) are refused by the emitter audit and by `mm0-rs` against the mutated certificate. Limits: finite trace only, not an all-run or induction proof; consensus ordering and D17 identity remain assumptions; the artifact awaits owner approval |
| D21 | finite model verified; induction open | U `unfer_consensus/tests/fence_invariant.rs`; A `docs/distributed/evidence/d21-fence-invariant.log`. BFS explores **2116 states / 507840 transitions / 3312 settles** with 0 violations, and differential conformance compares 4800 steps against the real `JobQueue` with 0 disagreements (178 stale tokens, 0 accepted). Mutations `StaleSettle`, `WrapGeneration`, `StaleRelease` each produce a named witness. The third target (stale fence cannot change protected state) is machine-checked; the repeated-request and epoch-monotonicity targets plus the MM0/Lean induction and axiom audit remain open. D20 now exists but is a finite trace, not the induction premise |
| D14–D26 | in progress | D14 transport claims verified; D15 durable outbox verified; D16 total request transition verified; D21 finite model check verified (induction/MM0 open); D17 verified; D18 verified; D19 live submission contract + Austral probe verified; D20 finite trace certificate verified; D22 next optional tier |
