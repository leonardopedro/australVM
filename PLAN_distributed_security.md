# Distributed security development plan: australVM + unfer

**Status: proposed, 2026-11-15. D00 (reproducible baseline + contract
checker) is complete as of 2026-11-19 — see
[docs/distributed/EXECUTION.md](docs/distributed/EXECUTION.md). D01–D26 are
not started.**
This plan covers both repositories as one project. It follows a source review
of the local Hydro checkout, including Hydro Lean, and incorporates the owner's
instruction to use Rust libraries through `umbrella` and `cap-std` where useful.
The [review](docs/DISTRIBUTED_SECURITY_REVIEW.md) records source evidence,
repository revisions, existing strengths, and findings R01–R12.

The recommended sequence improves authentication, encryption, application
atomicity, retry semantics and the existing QuePaxa integration first. It then
adds typed distributed boundaries, deterministic fault exploration and explicit
proof coverage. The goal is an incremental extension of existing guarantees.

## 1. Intended result and explicit limits

A distributed module should be able to submit an authorized operation, lose a
connection, restart, retry, and recover the same accepted or rejected result.
Replicas should apply the same authenticated committed prefix to the same
discrete state. A revoked or superseded worker should be unable to mutate that
state through stale authority. Local linear ownership and total transition
functions should help establish these properties without pretending that a
network message is a unique linear object.

The first supported profile has fixed membership, authenticated peers, crash
and recovery faults, arbitrary finite message delays and connection failures.
Applications tolerate duplicate requests, reordered requests across connections,
and fragmented frames. Quorum loss suspends coordinated writes. QuePaxa's
agreement and authenticated transport remain explicitly trusted dependencies;
their implementation is not proved by this work.

| Guarantee | Meaning and required evidence | Does not imply |
|---|---|---|
| Local resource safety | Existing Austral linearity/borrow checks; exact FFI ownership contracts; linear request/receipt wrappers | Unique copies of bytes across hosts |
| Total transition | Finite input, no general recursion/IO, checked integer ranges, bounded folds | Termination of the service or eventual network response |
| Authorized mutation | Receiver validates caller, delegation, audience, operation, resource and authority epoch at the ordered execution point | That a valid signature grants every operation |
| Retry safety | One request identity binds one payload; durable result and dedup state survive restart | Exactly-once packet delivery or arbitrary external side effects |
| Replica agreement | Same validated committed prefix gives the same control state and outcomes | Bitwise equality of arbitrary GPU/float computations |
| Distributed exclusion | Every protected sink rejects a stale monotonically increasing fence | Wall-clock leases being safe under arbitrary clock skew |
| Merge convergence | Proven laws for specifically registered metadata merges | Mergeable ownership, double-spend prevention or revocation under partition |
| Progress | Separate theorem/test under stated quorum, fairness, delivery and storage assumptions | Bounded response time in an asynchronous partition |

Two consequences are worth fixing before the tasks. **A guarantee proved over all
schedules is a safety guarantee.** Hydro's own Lean development states the reason
(`hydro_lean/internal/FIDELITY.md`, "Scope of the Runtime Model"): its denotation
admits more behaviours than any schedule realizes, and the transfer it needs is
machine ⊆ denotation, which makes a contract stronger and never unsound. The same
direction leaves progress unproved, so the last row of the table is reachable
only under D23's explicit fairness, delivery, quorum and storage assumptions —
never as a corollary of "proved for every schedule". An absence of received
messages is not a decision until a completeness frontier says so (D19).

**A guarantee whose truth depends on batching needs a named premise.** Hydro's
quorum API guarantees membership and the threshold under every batching, and its
completeness only under an explicit usage cap on responses per key (its finding
B3); the cap is a field of the contract, not a comment at the caller. The same
holds here for any guarantee that depends on how arrivals are grouped — dedup
windows, watermark completeness, "at most one vote per member per ballot". Each
must be a named premise with a witness, which is what D19's registry and D24's
assumption list carry. Deduplication at the sender does not discharge it.

Postpone Byzantine consensus, dynamic membership, live ownership migration,
cross-cluster transactions, arbitrary distributed shared memory, a new general
dataflow DSL, a full Hydro port and a proof of Cranelift. These require separate
designs after the first vertical slice works. Threshold signatures are an
authorization mechanism; they do not turn crash-fault consensus into BFT.

## 2. Architecture and language responsibilities

```text
Austral module
  existing linear checks + pure total transition + optional required refinements
        |
        | scalar / borrowed buffer ABI; local opaque handles only
        v
umbrella / modhost: pinned trusted Rust adapter
  per-instance caller context + host-created directory/network capabilities
  bounded codec + stable request identity + durable submission status
        |
        | authenticated versioned envelope; full bounded payload initially
        v
unfer node, independent process and storage
  authenticate/admit -> existing QuePaxa -> ordered validate/apply/reject
  durable result + dedup + applied slot + outbox intent in one recovery unit
        |
        +-> fenced effect dispatcher -> sink enforcing fence and idempotency key
        |
        +-> trace/evidence -> MM0 checks / optional Lean model -> claim manifest
```

Use Rust for TLS, filesystem durability, cryptographic APIs, bounded codecs,
queues, cancellation and adapters to existing crates. Prefer the existing
`unfer_consensus`, `unfer_data`, `unfer_protocol`, `unfer_ffi` crates; create
small modules before creating a second runtime. `umbrella` remains an integration
host that invokes Cargo; rustc does not become part of the OCaml compiler.

Use total Austral for a small executable transition specification: one-request
lifecycle, authority-epoch comparisons, discrete request validation and selected
merge logic. Existing Rust ledgers can stay Rust. Their guarantees must be
described as Rust behavior tested against a model unless a stronger checked
correspondence is added. Do not move large Rust ledgers to Austral just to label
them verified.

`cap-std` supplies explicit host capabilities to audited Rust adapters. The host
alone opens ambient roots, sockets, key material and clocks. Trusted in-process
Rust can bypass this discipline; arbitrary build scripts, native libraries and
unsafe code require an isolation tier. Network grants name bounded destinations
and methods, not arbitrary URLs; validate resolved addresses, redirects and
reconnects at the adapter. A raw FFI pointer, kernel handle or JIT address is
never a remote resource identifier.

### Protocol decisions fixed for the first implementation

1. Sign a domain-separated, versioned canonical envelope. Include deployment
   identity, sender identity/incarnation, stable request ID, recipient/service,
   operation/resource, authority/policy epoch, payload digest and schema version.
   Bind every field on which receiver authorization depends. Reject unsupported
   versions and oversize/deep payloads before allocation or expensive proof work.
2. Retries reuse a request ID and exact canonical payload. A new business action
   gets a new ID even if its content is identical. Same ID with different bytes
   is a conflict. Retry after timeout queries durable status; timeout is not abort.
3. Use typed `Submitted`, `Committed`, `Applied`, `Rejected`, `Unknown` statuses.
   Keep submission IDs, consensus slots and application positions distinct.
   Cancellation stops waiting/retrying locally; it cannot retract committed work.
4. Start with full bounded command bytes as QuePaxa's value. The installed
   `rust-quepaxa` 0.1.0 supports generic `V: Clone + Ord + Send + Sync` in
   `NetworkConsensusHandler`; transport codecs add serialization bounds. A small
   `ReplicatedValue::{Noop, Command(bytes)}` avoids a shared numeric-ID lookup.
   Confirm all actual trait/codec bounds in D10. Defer large-payload CAS references
   until an availability protocol is specified; never truncate a CID to a u64.
5. Distinct authenticated member identities count toward quorums. Duplicate
   packets, retries, signer aliases and stale membership epochs add no votes.
   The consensus dependency owns consensus voting; application quorum utilities
   must separately enforce these rules.
6. Order revocation and fence changes with the operations they protect. A stale
   replica cannot acknowledge a fresh coordinated mutation without the required
   prefix/freshness evidence. Partitioned replicas may serve explicitly stale
   reads with a watermark; they cannot grant exclusive authority.
7. A deterministic domain rejection consumes its committed slot and records its
   reason. Corrupt storage, invalid consensus evidence or a missing required
   payload halts recovery. An application rejection cannot silently drop work or
   permanently block the log suffix.
8. Separate business state from external effects. Persist an outbox intent in the
   state transaction; deliver with a stable sink idempotency key and fence. If a
   sink cannot enforce deduplication, report an uncertain result and require
   reconciliation. Do not promise automatic exactly-once execution for that sink.
9. Bound frame size, nested decoding, inbox/outbox occupancy, batch size, retry
   state, retained results and proof-verification work. Backpressure is an
   explicit result. A partial batch remains retryable with the same IDs.
10. Retain existing ABI and diagnostics. Introduce versioned additive Rust APIs
    and optional manifest sections. Update all shared registries when adding an
    operation; do not add `uk_*` solely to avoid using the existing `rs_*` seam.

## 3. Instructions for the executing model

Read this plan, the review, and the applicable `AGENTS.md` before editing. `A`
below means australVM; `U` means the sibling unfer repository. A task listing
both explicitly authorizes a coordinated change within this plan's scope when
the owner asks to execute the plan. Read Hydro as a design reference; keep it
unchanged. This document does not request implementation during the review turn.

Execute the earliest unfinished task whose dependencies passed. Each numbered
substep is a separate bounded patch if necessary. Normally touch at most three
production files plus their tests per patch; split a task that exceeds this.
Use existing helpers and language constructs before inventing a framework.
Do not silently simplify a security condition to get a test or proof green.

For each patch:

1. Record the pre-existing dirty files and inspect the named functions. Preserve
   unrelated numerical/PCNL work in unfer. Never reset a repository to get clean
   tests. Check symbol/manifest schemas against source, not historical counts.
2. Add the smallest meaningful failing regression or counterexample for the
   behavior being changed, then fix it. Keep temporary expected failures outside
   the normal passing suite until repaired. Assert state/output properties,
   including refused-operation state equality, rather than implementation text.
3. Run the task's targeted check. After the local checks pass, run the relevant
   crate/compiler gate once. Record command, toolchain, exit status, evidence
   path and remaining gaps. Missing optional tools are a gap; missing evidence
   required by the deployment profile refuses that deployment.
4. Update the execution record below. Propose a conventional commit message;
   commit only if the execution session authorizes commits. Never regenerate
   held-out approved statements as part of fixing a proof. Stage scratch
   approvals for owner review using the existing comparator workflow.
5. If a proof needs a new axiom, weaker premise, larger trusted budget, changed
   statement, or a dependency rewrite, record the issue and a concrete failing
   witness. Continue independent tasks; reserve that design change for review.

Avoid broad dependencies and research excursions. Pin a new library only when
an existing one cannot satisfy a named requirement, and test the adapter seam.
Simulation before live-network debugging; falsification before proof generation.

### Command profiles

Commands below are templates to run from the stated repository. D00 verifies
them and records actual local prerequisites. A missing cache/dependency is a
reported environment gap, not permission to weaken tests or update lockfiles.

```bash
# U: run within ../unfer so its aarch64 .cargo/config.toml applies.
cargo test --locked --offline -p unfer_consensus --lib
cargo test --locked --offline -p unfer_identity
cargo test --locked --offline -p unfer_data
cargo test --locked --offline -p unfer_protocol
cargo test --locked --offline -p unfer_edge --features audit
cargo test --locked --offline -p unfer_consensus --features network --test phase6_testnet

# A: every dune/make build runs inside nix develop, per AGENTS.md.
nix develop "git+file://$PWD#ocaml" -c make bridge
nix develop "git+file://$PWD#ocaml" -c make all
nix develop "git+file://$PWD#ocaml" -c make test
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs
python3 scripts/check-links
bash scripts/verify-invariants
git diff --check
```

Use the default prover shell, or an explicitly working prover environment, for
required Why3 proofs. The `#ocaml` fallback can build but cannot certify missing
prover results. Lean proofs use the existing version-matched exporter shell;
do not enter a heavy Sail-model rebuild for an unrelated task, and do not widen
that shell to Hydro's Lean 4.33.1/Mathlib for a distributed task (D22). New untracked
Nix inputs are not visible to a git flake until included in its source snapshot;
prepare that snapshot explicitly, without committing unrelated changes.

For bridge edits rebuild the release `.so` before OCaml integration checks.
Follow unfer's `docs/TEST_LOCKS.md` for FFI tests. A single serialized green run
is not evidence that a lock-order/concurrency regression is fixed. Hydro itself
need not build for this plan; if it is built, obey its Cargo metadata and
nightly-format instructions and keep its Lean 4.33.1 separate.

## 4. Delivery sequence and dependency map

| Milestone | Tasks | Exit condition |
|---|---|---|
| M0: local hardening | D00–D04 | Identity, nonce, RPC and refusal-atomicity regressions pass |
| M1: trustworthy local distributed boundary | D05–D09 | Typed outcomes, durable retry identity, scoped runtime authority and Rust capability adapter |
| M2: independent replicas | D10–D13 | Separate processes/storage, deterministic replay and abrupt-restart evidence |
| M3: protected module workflow | D14–D19 | Fenced job/effect workflow, linear remote handles, total validator, enforced profile and explicit channel contracts |
| M4: checked evidence | D20–D24 | Trace certificates, selected symbolic invariants, qualified deployment claims and negative controls |
| M5: measured extension | D25–D26 | Selected metadata merge and bounded-resource/performance acceptance |

Critical path:

```text
D00 -> D01,D02,D03,D04
D01,D04 -> D05 -> D06 -> D07
D03,D05 -> D08 -> D09
D05,D06,D07 -> D10 -> D11
D05 -> D12; D07,D11,D12 -> D13
D06,D11,D12 -> D14 -> D15
D05,D06 -> D16; D08,D09,D14,D16 -> D17
D16,D17 -> D18; D10,D16 -> D19
D12,D16 -> D20; D14,D20 -> D21
D07,D12,D16,D19 -> D22; D10,D13,D20,D22 -> D23
D02,D03,D08,D13,D15,D17,D18,D19,D20,D21,D23 -> D24
D19,D21 -> D25; D24 -> D26
```

These are dependencies, not instructions to spawn agents. Work sequentially by
default. D22/D23's Lean extension is a separate evidence tier: its absence must
not be presented as proof of network refinement. M4 can ship the narrower MM0
and tested-runtime tier with those claims explicitly absent; it cannot advertise
the Lean-backed tier until both tasks pass.

## 5. Task cards

Paths marked **new** are proposed; all other paths exist in the reviewed tree.
Proposed test names below are acceptance targets to implement, not existing tests.

### D00 — Capture a reproducible baseline (small)

**Read/edit:** A `docs/DISTRIBUTED_SECURITY_REVIEW.md`; U
`docs/TEST_LOCKS.md`, `Cargo.lock`, `unfer_consensus/Cargo.toml`; **new** A
`docs/distributed/EXECUTION.md` and `scripts/check-distributed-contracts`.

1. Record all three repository revisions, dirty files, architecture, enabled
   features, actual symbol registry sizes, existing test commands and available
   proof tools. Preserve the original baseline separately from later updates.
2. Run focused baseline suites for consensus, identity, data, protocol and the
   bridge. Inspect the locked QuePaxa generic value, storage/replay and checkpoint
   contracts in the cached source; note what the embedding application must do.
3. Start a stdlib-only checker that validates the execution record/contract
   inventory schema, duplicate IDs and referenced existing files. Future tasks
   extend this checker; do not create placeholder passing security checks.

**Accept:** baseline has actual logs or named gaps for each command; schema
checker rejects a duplicate entry and missing file. **Do not:** copy stale test
counts, clean user changes, update lockfiles, or claim new security guarantees.

### D01 — Bind identity authority on submit and replay (small patches)

**Depends:** D00. **U:** `unfer_consensus/src/{signing,identity,node}.rs`,
`unfer_identity/src/lib.rs`, their existing tests.

1. Reproduce a victim DID update/revoke signed by an attacker-supplied key, and
   a stale revoke. Verify refusal leaves the original identity unchanged.
2. Enforce self-certifying DID/key equality on creation. Enforce current stored
   authority and monotone operation sequence on update/revoke. Define duplicate
   replay consistently with D05/D06; do not invent unsigned key rotation.
3. Verify signed transactions at ordered application/recovery as well as
   ingress. Separate signature integrity from state-dependent authorization so
   a rotation/revocation ordered in the log affects subsequent operations.

**Accept:** attacker operations, mismatched DID creation, stale revoke and
tampered replay fail; legitimate lifecycle passes through LocalConsensus.
Run identity/signing/node tests plus `cargo test -p unfer_identity` with D00's
locked environment. **Proof target:** only the currently authorized key can
change an identity; historical replay uses historical ordered authority.

### D02 — Version and repair encrypted content envelopes (small patches)

**Depends:** D00. **U:** `unfer_data/src/{crypto,store,blueprint,publisher}.rs`;
versioned serialization types and data tests.

1. Demonstrate that two envelopes in one KeyRing epoch, and repeated use of one
   DataKeypair, invoke encryption with the same key and chunk nonce. Characterize
   existing legacy ciphertexts before changing serialization.
2. Use a library-backed, domain-separated per-object key context with a fresh
   cryptographic object salt/identifier; chunk counters are unique within that
   derived key. Authenticate envelope version, object identity, epoch, chunk
   index, total/chunk length and recipient context as appropriate. Check bounds,
   RNG failure and counter exhaustion. Never implement an AEAD primitive here.
3. Roundtrip and swap/tamper tests cover object, epoch, index, chunk omission,
   truncation and recipient mismatch. Legacy reads are a labelled compatibility
   path; new writes always use the secure format. Document that root retention
   is not cryptographic deletion. Keep key creation outside consensus transitions.

**Accept:** repeated-object tests exercise distinct per-object key contexts;
cross-object chunk substitution and metadata tampering fail; retained legacy
fixtures read only through the intended path. Run unfer_data tests. A
freshness sampling test supplements the library/RNG assumption; it is not a
proof that collisions are impossible.

### D03 — Authenticate capability RPC and constrain promise ownership (small patches)

**Depends:** D00. **U:** `unfer_edge/src/{main,caprpc,config}.rs`, RPC tests.

1. Resolve an authenticated session/peer to a caller context at the trusted
   ingress. Reuse existing edge authentication when available; document the
   precise credential-to-principal mapping. Reject unauthenticated remote
   requests; reject non-loopback capability exposure until this is configured.
2. Require caller and owner/issuer authority for mint, resolve and revoke.
   Resolve only an existing owned pending promise, once, with endpoint/grants
   bounded by its parent. Authenticate requests independently of IDs.
3. Test two principals, guessed IDs, foreign revoke, invented promise ID,
   resolve twice, revoked parent, and requested grant escalation.

**Accept:** A's credential cannot operate B's capabilities; the admin principal
is never a default for remote requests. Run edge tests with `audit`. **Do not:**
trust a caller-supplied principal header or rely on secret numeric IDs.

### D04 — Make domain refusal atomic (small patches)

**Depends:** D00. **U:** `unfer_consensus/src/certs.rs` then each other ledger
individually; `node.rs` only where needed for state fingerprints.

1. Add `burn_refusal_preserves_state`: valid first input then invalid second,
   repeated input, foreign-owned input and sum overflow. Compare UTXOs,
   nullifiers, root, supply and any result bookkeeping before/after refusal.
2. Fully validate burn before applying its changes. Use transfer's existing
   validation-before-mutation structure where applicable.
3. Audit auction, mathbond, market and attribution refusal paths. Add regression
   cases only for distinct risks found; avoid a speculative ledger rewrite.

**Accept:** each refused operation is a state no-op; success preserves existing
business invariants. Run affected ledger tests and consensus library tests.
This is domain atomicity; D07 provides crash atomicity.

### D05 — Separate submission, commitment and application (medium; split in three)

**Depends:** D01,D04. **U:** `unfer_consensus/src/{engine,node,net}.rs`,
`unfer_identity/src/lib.rs`, `unfer_data/src/publisher.rs`; **new**
`unfer_protocol/src/submission.rs` plus registry/export wiring.

1. Add distinct ID/slot/watermark types and status/result records. Keep existing
   local APIs through documented adapters; network callers get a receipt and
   query/wait operation. Timeout returns an uncertain pending/unknown state.
2. Apply contiguous committed entries deterministically. A domain rejection
   records a stable outcome and advances the slot; local corruption/storage
   failure halts. Test bad entry between two good entries and tampered/gapped
   history. Check any attempted partial mutation using D04's state comparison.
3. Update DidManager and content publication so their network paths report
   completion only after application, including typed refusal. Validate bounded
   waits and cancellation; preserve synchronous local behavior.

**Accept:** Local and network receipts cannot be confused with committed slots;
invalid domain entry does not wedge a good suffix; pending identity is not
reported as created. Run consensus/identity/data/protocol tests. No new C ABI
symbol is required at this stage.

### D06 — Durable logical request identity and safe retention (medium)

**Depends:** D05. **U:** `unfer_consensus/src/{idempotency,node}.rs`, submission
types; **new** request-status tests in `unfer_consensus/tests/`.

1. Use `(deployment, principal, client-incarnation, request-sequence)` or an
   equivalently specified durable ID. Persist the client's incarnation/sequence
   before transmitting; server admission records the payload digest with it.
2. Cache accepted and deterministic rejected results. Same ID/same bytes yields
   the same result; same ID/different bytes conflicts; different IDs/equal bytes
   remain distinct actions subject to the business ledger's own idempotence.
3. Replace unconditional prune with an acknowledged retention floor/tombstone
   scheme. An ID below a retained admission floor is refused even at a new log
   slot and after restart. Unacknowledged clients cannot force unbounded RAM:
   apply a quota and explicit backpressure/expiry policy.

**Accept:** nonconsecutive retry, retry after prune, restart, rejected retry,
concurrent submissions, independent client incarnations and equivocation tests
pass. State/result/floor persistence is integrated atomically in D07; do not
advertise crash-safe retry before that dependency passes.

### D07 — One durable application transition and honest failure semantics (medium)

**Depends:** D06. **U:** `unfer_consensus/src/net.rs`; **new**
`unfer_consensus/src/storage.rs`; existing durable backend code only in separate
patches with its own tests.

1. Replace independently sampled bank/pending/counter snapshots with one
   serialized storage transaction or one recoverable append record containing
   slot, canonical command, decision/result, dedup update and resulting state
   version. Replay is idempotent by consensus slot and request ID.
2. Use a single writer and explicit crash protocol: write complete framed/checksummed
   record, synchronize, publish; snapshot via synchronized temporary file,
   rename and parent-directory sync as required by the supported platform.
   Distinguish corruption, a torn trailing record and an uncertain write result.
3. Wire into QuePaxa's StateMachine contract: the generic library cannot atomically
   commit its runtime store with application state and requires replay-safe
   execution by slot. Define a recovery order and reject mismatched checkpoints.
4. Repair the existing Loro snapshot path's documented durability level or label
   it accurately, without asserting all backends have identical transactions.
   Reuse `DurableStore` only where its actual atomicity meets this protocol.

**Accept:** fault injection before/after each write/sync/publish boundary gives
either the old state or one fully recoverable transition; duplicate execution
after runtime-store failure changes nothing. An acknowledged durable result
survives restart. Uncertain failures never become false definite refusals.

### D08 — Scope caller context and enforce dynamic remote policy (medium)

**Depends:** D03,D05. **A:** `safestos/cranelift/src/{auth,cps,module}.rs`;
U caller-context/GrantSet seams in `unfer_ffi/src/handles.rs` and protocol types.

1. Test two loaded modules with disjoint grants and interleaved compilation and
   calls. Audit existing FFI caller-context checks. Bind an immutable instance
   identity at compilation/handle creation; eliminate reliance on whichever
   process-global principal was installed most recently for the new profile.
2. Keep the static import gate. Add a context-bearing runtime wrapper for the
   distributed/effect boundary, checked by the receiving service on each use.
   Retain module, user/delegator and remote node identity as distinct fields.
3. Test policy revocation after compilation, failed load preserving previous
   context, hot-swap with in-flight requests, principal spoofing and arbitrary
   external-symbol imports. For the strict profile admit only declared ABI
   registry entries; prefix-based exemptions are not ambient foreign-call grants.

**Accept:** module A never borrows B's grants; stale authority is refused at the
receiver even with already compiled code; UK-4001 behavior remains stable.
Run targeted bridge/FFI tests, rebuild bridge release, then compiler integration.
Do not generalize this result to every legacy native call without evidence.

### D09 — Add an explicit Rust adapter through umbrella (medium)

**Depends:** D08. **A:** `umbrella/src/main.rs`, `safestos/cranelift/src/capstd_mod.rs`;
**new** `umbrella/modules/distributed_io/` and a small Austral positive/negative demo.

1. Validate trusted artifact identity/profile before building or loading it.
   Treat building an arbitrary crate as executing build scripts. Pin approved
   Cargo.lock/features/library digest; resolve exact ABI exports and signatures.
   Reject invalid deployment manifests before Cargo/dlopen when no trusted
   build is authorized. Initializers are part of trusted native execution.
2. Host constructs a directory capability rooted in the instance state directory
   and a network capability restricted to its configured peers. The adapter
   receives handles, never ambient roots or permission to dial arbitrary hosts.
3. Implement bounded send/query/wait/cancel over existing Rust protocol code,
   exposing borrowed buffers/scalars/opaque local handles via `rs_*`. Audit panic
   containment, checked lengths and ownership; no retained borrowed pointer.
4. Test denied grant with an observable mock sink, sibling-path traversal,
   unauthorized peer/redirect, handle use by another module and dropped request.
   Keep hard isolation for untrusted native code a separate deployment tier.

**Accept:** umbrella exercises a real Rust library through the ordinary compiler
and manifest gate; missing grant produces no sink call. The new demo states its
trust assumptions and takes explicit paths, not developer-machine paths.

### D10 — Replicate complete bounded commands and validate wire contracts (medium)

**Depends:** D05,D06,D07. **U:** `unfer_consensus/src/net.rs`; **new**
`unfer_protocol/src/distributed.rs` and codec tests; existing signing code.

1. Define canonical versioned command bytes and `ReplicatedValue` with an
   explicit no-op variant. Pass full bounded values through the generic locked
   QuePaxa interfaces; remove dependence on one shared payload map. Record the
   dependency's actual serialization, ordering, frame-size and journal limits.
2. Domain-separate signatures with deployment/audience/schema/context. Establish
   golden canonical byte vectors, explicit integer widths and unknown-field/tag
   rules. Legacy wire signatures remain a separate compatibility version.
3. Validate before allocation, decode partial/coalesced frames, reject trailing
   garbage, invalid UTF-8 where applicable, depth/length overflows and malformed
   signatures. Do not equate TLS peer identity with transaction authority.

**Accept:** two independent decoders accept the golden vectors; fragmentation at
every byte boundary and truncated/oversized frames behave as specified. A
command signed for another deployment/service or altered after signing is
refused. No command can commit an unresolved process-local numeric value ID.

### D11 — Run independent QuePaxa-backed node processes (medium)

**Depends:** D10. **U:** refactor `unfer_consensus/src/net.rs`; **new**
`unfer_consensus/src/bin/unfer_node.rs`, `tests/independent_nodes.rs` and fixture config.

1. Separate single-node startup from test-cluster orchestration. Each process
   owns its recorder/runtime store, application store, request results and TLS
   identity. Deployment ID, member identities and addresses come from validated
   configuration. The existing fast harness remains as a labelled test fixture.
2. Launch three separate processes with disjoint state directories and no shared
   payload files, Arc, memory map or application lock. Submit through different
   nodes, wait for a specified slot and compare canonical state/result digests.
3. Restart one node and catch up from authenticated committed history/checkpoints.
   Implement and test StateMachine checkpoint export/import if needed; refuse
   unknown or conflicting deployment/schema identities.

**Accept:** a certificate mint/transfer sequence and one rejected operation yield
identical independent state and results. Stop the submitting process before
other application nodes replay; survivors still possess the full payload.
The test fails if any node secretly reads another node's application storage.

### D12 — Deterministic fault scheduler over the production transition core (medium)

**Depends:** D05. **U:** **new** `unfer_consensus/src/sim.rs`,
`tests/distributed_schedules.rs`; existing node/engine seams.

1. Extract/reuse a deterministic event step returning state + effect intents;
   adapters provide explicit inputs for delivery, timer, retry, storage outcome,
   crash and restore. Preserve business logic instead of reimplementing it in a
   toy simulator. Treat the external consensus engine as an assumption in this
   harness; application order is an explicit input.
2. Register stable nondeterminism-site IDs. Capture seed and choice trace;
   maintain separate classes for content choice, scheduling and bounded fuel.
   Inject duplicate/reordered requests, connection partitions and stale workers.
3. Explore a small fixed domain of two clients, three nodes, two requests and
   bounded schedules; save and minimize counterexamples. State the exact bounds
   and state-space pruning equivalence. A bounded search is not an all-run proof.

**Accept:** same trace gives identical outputs/state; known unsafe variants of
dedup, revoke and rejection handling produce concrete traces; repaired variants
survive the recorded corpus. Empty/no-progress traces cannot be the only tests.

### D13 — Abrupt-crash and storage-recovery fault matrix (medium)

**Depends:** D07,D11,D12. **U:** independent-node tests, storage fault hooks,
**new** `scripts/distributed-recovery` and result fixtures.

1. Cover crash before admission persistence, after persistence before reply,
   after commit before apply, after apply before runtime checkpoint, after result
   persistence before response, and while a snapshot is published.
2. Use subprocess kill/restart for real processes and injected IO failures for
   deterministic boundaries. Test disk-full/permission failure, corrupted
   committed record, torn tail, conflicting checkpoint and slow replica catch-up.
3. Partition a minority and the former leader; assert no successful coordinated
   write without quorum. Heal, retry stable IDs, compare every result and root.

**Accept:** no acknowledged durable operation disappears, no logical request
applies twice, no failed domain operation partially applies, and recovery never
silently substitutes empty state. Distinguish process-crash measurements from
power-loss claims; model/storage evidence must support the latter separately.

### D14 — Upgrade scheduled jobs to ordered fencing (medium)

**Depends:** D06,D11,D12. **U:** `unfer_consensus/src/{lease,jobs,node}.rs`;
protocol command registry and tests.

1. Keep round-robin selection as a scheduling preference. Introduce an ordered
   claim command yielding `(deployment, resource, epoch, generation)` as a fence,
   plus holder and validity rules. The committed state owns the current fence.
2. Protected state mutations and mock external sinks reject stale fences.
   Renew, release, reclaim and restart are explicit transitions; no correctness
   argument depends on synchronized wall clocks. Do not promise an enforceable
   fence for a sink that has no enforcement interface.
3. Test two partitioned workers both believing they lead, delayed old completion,
   claim replay, generation overflow, restart and stale release of a newer claim.

**Accept:** only the current fence can settle the job or mutate its protected
resource. The old worker can continue computing but cannot commit a stale result.
Record the first symbolic invariant target for D21.

### D15 — Durable outbox for existing approval-controlled effects (medium)

**Depends:** D14,D07. **U:** existing action queue in `unfer_ffi/src/handles.rs`,
durable stream types; **new** small dispatcher module and sink-contract tests.

1. Persist effect intent with request/result/state transition; dispatch only
   after durable commit. Keep Observe/Mutate classification, approval tokens,
   delegation limits and the sensitive-forward latch intact.
2. Bind approval to exact payload digest, resource, request identity and epoch.
   Recheck revocation/policy at the specified execution point. Record durable
   sink acknowledgment, uncertain outcome or retryable failure separately.
3. A sink with dedup receives the same idempotency key on every retry and enforces
   the fence. A sink without dedup enters reconciliation on uncertain delivery.
   Never automatically replay an uncertain irreversible action as a fresh action.

**Accept:** crash after sink success before acknowledgment does not duplicate the
effect at a dedup-capable sink; approval replay on altered payload fails; revoked
worker fails; unknown sink outcome is visible. At least one existing effect
path uses the dispatcher end to end; no parallel approval system is introduced.

### D16 — One total Austral distributed transition (small then medium)

**Depends:** D05,D06. **A:** **new** `examples/distributed/RequestState.{aui,aum}`
and fixtures; `lib/liquid/TotalityCheck.ml` only for demonstrated missing checks.
**U:** existing logos validation and a reference test adapter.

1. Start with finite states `Idle/Pending/Applied/Rejected` and explicit events
   for submit, matching response, retry and cancel-wait. State stores stable
   request identity and payload commitment. Cancellation does not mean rollback.
2. Use the existing supported total fragment. If records/unions/folds cannot be
   certified by an existing tool, report the exact unsupported construct before
   extending a deterministic emitter. Do not encode protocol states as unrelated
   polynomial equalities and claim the protocol was proved.
3. Execute the same finite event vectors through Austral JIT, the Rust adapter
   and an independently written small reference function. Include wrong request,
   wrong digest, repeated response, restart continuation and numeric bounds.

**Accept:** totality validator rejects loops, foreign IO and recursion in the
transition; accepted fixture executes the lifecycle correctly. Differential
tests and a source digest document the Rust/Austral correspondence; compiler
correctness remains a named trust boundary.

### D17 — Linear local wrappers for authenticated remote resources (medium)

**Depends:** D08,D09,D14,D16. **A:** **new** `stdlib/net/Distributed.{aui,aum}`,
`test-programs/suites/022-distributed/`; umbrella adapter. **U:** authenticated
resource/token validation alongside existing capability RPC.

1. Design abstract linear `PendingRequest`, `ResultReceipt`, `RemoteResource`
   and authority-token wrappers over host-managed handles. Constructors remain
   privileged; serialization uses authenticated resource/deployment IDs and
   epochs, never local integer handles/pointers.
2. Submit consumes the local request object; retry consumes and returns the same
   PendingRequest with unchanged wire identity; wait returns pending or a single
   consumed receipt path. Provide explicit disposal/cancel-wait for every state.
3. Test compile-time copy/use-after-consume/wrong-state calls and runtime forged
   IDs, wrong owner, wrong deployment, retired epoch and captured-token replay.
   Remote ownership transfer is out of scope: resource exclusivity remains at
   the ordered authority, despite copied wire bytes.

**Accept:** positive end-to-end module works through umbrella; negative Austral
fixtures fail with pinned linearity diagnostics; replayed serialized bytes cannot
create a second authorized owner. Run compiler suite in nix develop and adapter
integration after rebuilding the release bridge when needed.

### D18 — Make distributed verification profiles enforceable (medium)

**Depends:** D16,D17. **A:** `lib/liquid/LiquidWhy3.ml`, compiler CLI/plugin
seams, `safestos/cranelift/src/{module,federation}.rs`; **new** profile fixtures.

1. Define optional versioned manifest requirements for total transition coverage,
   liquid results, protocol/schema identity and required proof tier. Thread the
   existing required mode into the compiler invocation explicitly; a manifest
   field ignored by the compiler is not enforcement.
2. Add refinement obligations for lengths, queue capacities, counters, epochs
   and numeric conversions in the supported fragment. Use runtime validation
   before treating untrusted input as refined. SMT success remains Why3/solver
   evidence unless independently proof-checked; record that trust honestly.
3. Strict distributed mode rejects missing/skipped/unknown prover results,
   stub DIDs, DefaultHasher artifact IDs, AllowAll and an implicit LocalConsensus
   deployment. Keep intentional local development modes available and labelled.

**Accept:** remove the prover or corrupt a goal and required deployment fails;
optional mode reports a gap. All network artifact IDs use a specified
cryptographic digest. Overlength/overflow inputs are refused before unsafe FFI.

### D19 — Register location, ordering, retry and completeness contracts (medium)

**Depends:** D10,D16. **U:** existing op/symbol registries; **new**
`unfer_protocol/src/channel_contracts.rs`; A checker from D00.

1. For each supported boundary record producer/consumer location, schema,
   ordering scope, duplicate scope, delay/loss assumptions, authentication,
   capacity, and any completeness/watermark rule. Distinguish FIFO per live TCP
   connection from logical request retries across reconnects.
2. Expose opaque Rust marker/constructor APIs and matching Austral wrappers only
   for the contracts actually enforced. An unordered fold requires commutativity;
   arbitrary repeated deliveries require idempotence too. Consecutive-only
   destuttering is insufficient for nonconsecutive request retries. A guarantee
   that holds only under a usage cap or a batching rule takes that cap as an
   explicit contract field with a witness (the `cqKeyCount … ≤ max` pattern in
   Hydro's `Hydro/Std/Quorum.lean`), so a caller that cannot establish it does
   not get the guarantee; a comment at the call site is not a premise.
3. Add a registry census for every production send/receive/timer/retry/storage
   hook and simulator site. Changes require matching inventory and test/model
   coverage. The census catches missing sites, not semantic equivalence by itself.

**Accept:** packet-count quorum, unproved unordered subtraction, a completeness
or dedup guarantee stated without its usage cap, and promotion of an at-least-once
channel to exactly-once fail the appropriate constructor/check.
Counterexample: repeated votes from one authenticated member never form quorum.
Nonmonotone decisions such as close/absence require an ordered close or explicit
completeness frontier, never a local timeout interpreted as global knowledge.

### D20 — Certify finite distributed traces in MM0 (medium)

**Depends:** D12,D16. **A:** **new** `docs/distributed.mm0`,
`scripts/distributed2mm0`, `mm0/distributed_trace.{mm0,mm1}` and pinned trace fixture;
reuse comparator, budget and mutation helpers from existing scripts.

1. Define a small explicit state/transition vocabulary covering request identity,
   outcome stability and authority epoch. Every trusted rule has a model
   interpretation and a named trust-budget entry. Start with one lifecycle trace.
2. Emitter independently replays input events and produces a chain proof against
   the fixed theory. Run mm0-rs then mm0-c; artifact reports subject/source hashes,
   assumptions, trusted leaves, exercised transition kinds and exact trace length.
3. Mutate response request ID, payload hash, epoch, event ordering and omitted
   persistence boundary. A relevant mutation must be rejected and must actually
   apply. A no-op mutation or rejection for unrelated syntax is a failed control.

**Accept:** deterministic regeneration, checker acceptance and all negative
controls pass. Label this **finite trace certificate**, not all-run safety.
Prepare approved statements in a scratch root; production approval remains the
owner's existing comparator workflow. Trace checking alone need not block the
development profile but missing required checks blocks a certified profile.

### D21 — Prove selected symbolic invariants and lift over runs (medium, bounded scope)

**Depends:** D20,D14. **A:** distributed theory/emitter, approved-statement
scratch output; U executable state/witness fixtures.

1. Select three claims: repeated matching request preserves its result;
   authority epoch never decreases; a stale fence never changes the protected
   state. State initial-state constraints and every reachable transition case.
2. Prove initialization and one-step preservation over symbolic state, then
   induction over arbitrary finite runs. Provide a valid execution with a real
   accepted mutation and a distinct rejected stale mutation so premises are
   satisfiable and the claim is not vacuous.
3. Audit theory consistency against an executable finite model and rejected
   axiom candidates, following the toy-witness pattern. This finite witness is a
   consistency check over its domain, not a general consistency proof. No axiom
   may simply assert one of the target protocol guarantees.

**Accept:** all transition cases covered, theorem dependencies and trusted rules
listed, owner-reviewable statements stable, real guard mutations break the proof.
If the current MM0 fragment cannot express the induction, isolate that extension
as a reviewed subtask; do not substitute sampled traces. Bitwidth/range premises
must match execution. Remaining source/lowering correspondence is still explicit.

### D22 — Small Lean distributed semantics and axiom audit (optional stronger tier)

**Depends:** D07,D12,D16,D19. **A:** **new** `proofs/distributed/` with a small Lake
project pinned to the existing exporter-compatible Lean version, and a build gate.

1. Write import-light definitions of the production transition vocabulary,
   node-local state, in-flight messages, crash/restore and explicit schedules.
   Reuse finite lists and total steps. Do not import Hydro Lean or take Mathlib as
   an incidental dependency: this tree's exporter shell is Lean 4.30.0 and the
   Hydro checkout is pinned at Lean 4.33.1 with Mathlib, so an import would force
   a toolchain migration for a library whose value here is its discipline, not
   its size. Mirror one module at a time. Document each source/model seam.
2. Prove the selected invariant under every modeled step, including stutter,
   duplicate delivery, refusal, crash and recovery. Separate consensus agreement
   and cryptographic assumptions as named predicates/premises.
3. Add headline axiom reports and reject sorryAx, native_decide/ofReduceBool,
   unreviewed axioms, unsafe/extern substitution and unaudited implementation
   replacements. Tie the decision-site census to D19. Count the compiler seams
   the way Hydro's gate does (`hydro_lean/internal/GATE.md` §4): a grep for
   `implemented_by`/`@[extern]`/`native_decide`/`unsafe`/`ofReduceBool` that must
   be empty, and every `@[csimp]` substitution carrying a kernel-checked
   `f = fImpl` and a reviewed count. A green build whose theorems are about
   definitions the compiler rewrote is the failure this catches and an axiom
   report does not.

**Accept:** clean build and non-vacuous executable witnesses, explicit theorem
dependencies, all-run safety only for the stated model. A manual Rust/Austral
mirror remains trusted even with a source hash. The existing `lean2mm0-stmts`
may index the statements; its output is never counted as MM0 proofs of them.

### D23 — Relate bytes, process steps and the model (optional stronger tier)

**Depends:** D10,D13,D20,D22. **A:** Lean codec/trace lemmas and adapter evidence;
U canonical codec and process trace tests.

1. Prove roundtrip and bounded incremental-decoding properties for the small
   selected envelope fragment; model authenticated sender attribution as a
   separately declared assumption. Map successful production decode to that
   vocabulary and count unmapped variants as gaps.
2. Relate one process activation to the modeled step using the explicit
   refinement relation, then compose across arbitrary allowed interleavings.
   Include retries/reconnects and crash/recovery; Hydro's FIFO-prefix model alone
   does not cover arbitrary duplicate requests after reconnect.
3. Differentially replay real captured byte traces, fragmentation and malformed
   input through implementation and model. State whether agreement is proved,
   tested, or manually reviewed at each edge. Add conditional progress only with
   explicit scheduling, delivery, quorum and storage assumptions.

**Accept:** byte-to-step safety theorem for the chosen modeled fragment, decoder
and trace mutations fail, and at least one progressing run and one stalled but
safe partition run are witnessed. No theorem is advertised about unmodeled Rust
code or arbitrary OS/TLS behavior.

### D24 — Bind deployment claims to artifacts and gate them (medium)

**Depends:** M0–M3 and D20,D21; D22,D23 additionally for the stronger tier.
**A:** existing fingerprint/comparator/invariant scripts, `module.rs`, umbrella
loader; **new** `docs/distributed/CLAIMS.md`, claim manifest and CI job.

1. Manifest binds source, CPS, target-specific object capture, Rust library and
   transitive native/build dependency identity, Cargo.lock/features, runtime
   version, grants/policy epoch, wire schema, theory, approved statements and
   proof artifacts. Canonicalize serialization and verify every digest at load.
2. Evidence class per claim is one of runtime-tested, finite-trace-certified,
   symbolic-invariant-proved or model-refinement-proved, with named assumptions
   and gaps. A model-refinement-proved claim also names the seams it relies on:
   the substitution count with each `f = fImpl`, the empty-grep for
   `implemented_by`/`@[extern]`/`native_decide`/`unsafe`, and the trusted
   Rust-to-model mirror. A digest is identity evidence; it is not a proof of
   semantics.
3. Gate deployment by required class, signature and freshness. Mutate one field
   at a time: library byte, grant, schema, theorem premise, proof, target, runtime
   version or source; refuse or explicitly invalidate the affected evidence.
4. Extend existing invariant gates and CI with focused fast checks and a separate
   bounded network/recovery lane. Missing tools print gaps in development and
   refuse profiles requiring their evidence. Preserve existing W3 pending counts
   and approved statements; do not mark approval as implementation completion.

**Accept:** one command builds/checks an example deployment and prints its exact
claim set; all required negative controls refuse; uncertified examples never
silently inherit stronger claims. Document key trust, rollback/policy-epoch
rules, and how an owner reviews scratch comparator approvals.

### D25 — Improve one existing metadata merge (optional extension)

**Depends:** D19,D21. **U:** existing Loro durable/board/cooperation metadata
selected after source inspection; A merge specification and proofs.

1. Choose a low-risk metadata projection whose intended behavior already needs
   concurrent merge, such as append-only evidence annotations with authenticated
   unique IDs. Do not merge balances, unique job claims, approvals or ownership.
2. Specify join, identity and ordering; prove associativity, commutativity,
   idempotence and bounded representation behavior. Handle deletion/tombstones
   with an explicit stable frontier; disconnected replicas cannot resurrect
   collected records under the claimed contract.
3. Test duplicates, partitions, different merge orders, forged authorship and
   stale import after compaction against the existing backend.

**Accept:** one existing user workflow gains measured convergence and stable
bounded-state behavior; unproved arbitrary merge operators remain unavailable.
Reuse Loro instead of creating another general CRDT engine.

### D26 — Resource bounds, rollout and operator acceptance (medium)

**Depends:** D24; D25 only if included in deployment. **Both:** existing module
examples, runbooks and CI; **new** bounded distributed benchmark script.

1. Publish a reproducible demo using the existing certificate workflow and a
   fenced scheduled job through the Austral/umbrella path. Include positive
   request, domain rejection, dropped response/retry, revoked worker, partition,
   abrupt restart and recovery, with exact expected results.
2. Measure max queue/storage growth, admission/backpressure, catch-up time,
   per-request policy/crypto/proof overhead and end-to-end latency against the
   baseline on the same machine. Define budgets from measured baseline and
   intended limits; do not invent throughput targets from Hydro's benchmarks.
3. Add rolling upgrade compatibility within the fixed schema policy, explicit
   migration/offline upgrade for incompatible storage, rollback refusal across
   retired authority epochs, key/cert rotation runbook and storage corruption
   recovery procedure. Membership changes remain out of scope.

**Accept:** repeated fault demo produces stable results with bounded resources;
slow/down peers do not cause unbounded retries; every advertised guarantee has
an evidence link and an operational limitation. Run final affected suites once
and preserve evidence. Do not run unrelated physics/GPU suites for documentation
or transport-only patches.

## 6. Minimum useful release and cost control

The first user-visible release is M0–M3 with runtime-tested evidence: secure
identity/RPC/encryption, atomic domain refusal, typed requests, durable retry
results, independent nodes, stale-worker fencing, one effect path, and the
Austral/umbrella demo. It can be useful while stronger theorem work continues.
Do not gate basic hardening on completion of a large formalization.

The first proof release adds D20/D21 and D24's claim manifest. D22/D23 are
reserved for a stronger, explicitly narrower modeled fragment; they must not
delay P0 fixes. D25 is optional until an existing workflow needs it. Compiler
backend correctness and Byzantine consensus remain separate projects.

For a cheaper model, begin with D00 and D01. Each subsequent task should consume
the previous task's fixture and one recorded counterexample. Tasks D05, D07,
D08, D10, D21 and D23 are review boundaries: require a concrete design/evidence
record before expanding scope. Routine implementation within the agreed plan
can continue; owner statement approval is the existing explicit exception.

Do not estimate calendar time from model speed. Track patches completed,
counterexamples eliminated, verification time, trusted-rule growth, and residual
gaps. A task taking repeated attempts without reducing a concrete failure should
be split or escalated with its minimized witness.

## 7. Execution record and next-task prompt

All tasks start pending. Create the actual ledger in D00; this template is not
evidence of work performed.

| Task/patch | Status | Source revisions + dirty inputs | Targeted checks/logs | Evidence class | Remaining assumptions/gaps |
|---|---|---|---|---|---|
| D00 | implemented with gaps | A `3ad01bac`+; U `b0aaa7c`, clean | [`docs/distributed/EXECUTION.md`](docs/distributed/EXECUTION.md), `scripts/check-distributed-contracts --selftest` | structural checker | The 2026-11-19 rows that were `gap` for a missing `cc` cache are re-measured on this aarch64 host; the suites still not run keep their named gap |
| D01 | verified within stated scope | U `b0aaa7c` + this patch; A unchanged | `unfer_consensus --lib` 116 passed, `unfer_identity`, `phase6_testnet`, `unfer_data`; logs under `docs/distributed/evidence/` | runtime-tested | Identity authority is enforced at the ordered point; other op kinds still verify at ingress only, and unsigned key rotation is refused rather than implemented |
| D02 | verified within stated scope | U `b0aaa7c` + this patch | `unfer_data --lib` 62, `--test integration` 4, `unfer_ffi --lib` 175; logs under `docs/distributed/evidence/` | runtime-tested (negative control for the old nonce) | Per-object key context and authenticated metadata; v1 envelopes refused, not migrated; publisher key distribution is D10/D11 |
| D03 | verified within stated scope | U `b0aaa7c` + this patch | `unfer_edge --features audit` 89 + 1; `docs/distributed/evidence/d03-unfer_edge_audit.log` | runtime-tested | Capability RPC is loopback-only and every registry verb is owner-checked; the HTTP layer still has one principal (A9 auth seam) |
| D04 | verified within stated scope | U `b0aaa7c` + this patch | `unfer_consensus --lib` 121; `docs/distributed/evidence/d04-unfer_consensus_lib.log` | runtime-tested | Burn is validate-then-commit and four ledgers pin whole-state refusal; crash atomicity remains D07's |
| D05 | verified within stated scope | U `b0aaa7c` + this patch | `unfer_protocol --lib` 351, `unfer_consensus --lib` 129, `--features network --test phase6_testnet` 4, `unfer_identity` 7+1, `unfer_data --lib` 63; `docs/distributed/evidence/d05-*.log` | runtime-tested (real 2-of-3 QuePaxa cluster, including restart) | Receipts cannot be mistaken for slots; a domain refusal advances the slot and a gap halts; identity/publishing settle before reporting. Durable request identity is D06, crash atomicity D07 |
| D06 | verified within stated scope | U `14e0a7a` + this patch | `unfer_consensus --test request_identity` 6, `--lib` 152, `unfer_protocol` 353+4+51+3, `unfer_identity` 7+1, `unfer_data` 63+4+1, `unfer_edge --features audit` 89+1; `docs/distributed/evidence/d06-*.log` | runtime-tested | One request id names one action: same bytes replays the settled result, different bytes is UK-8101, at-or-below the retained floor is UK-8102, a full book is UK-8103. The book is in memory; the bindings become durable in D07 |
| D07 | verified within stated scope | U `14e0a7a`, `249c53c` + this patch | `unfer_consensus --lib` 152, `--test power_cut` 4, `--features network --test phase6_testnet` 4, `unfer_ffi` 175+55+1; `docs/distributed/evidence/d07-*.log`, `d08-unfer_consensus_power_cut.log` | runtime-tested (fault injection at each boundary, plus SIGKILL of a real writer on tmpfs) | One framed, fsynced log; torn tail is truncated, a bad checksum is corruption, a failed sync is uncertain; replay compares recorded outcomes and halts on a mismatch. Not an atomic commit with QuePaxa's own runtime store |
| D08 | verified within stated scope | A this patch (bridge release rebuilt; both derived `.so` copies staged) | `cargo test` 0 failures (`--lib` 66, `module_tests` 12, `cps_tests` 14, `modhost_cli` 15 + 3 + 41/1 ignored), `bash scripts/verify-invariants` 179 passed / 4 explained gaps / 0 failed, dune `@runtest` EXIT=0; `docs/distributed/evidence/d08b-*.log` | runtime-tested (two loaded modules, revocation, failed load, hot-swap, real peer-bound call through the binary) | An immutable per-instance identity replaces the process-global principal; grants merge instead of replacing; revocation/swap retire an identity; strict profile admits only declared ABI symbols; `call_from_peer` is the production receiver and compares the transport's peer with the instance's declared node. The peer is operator-supplied until D11/D17; revocation is enforced at the host boundary, not inside compiled code |
| D09 | verified within stated scope (steps 1–4) | A this patch (bridge release rebuilt) | `umbrella` `cargo test` EXIT=0 (`--lib` 59, 31 integration); `cargo test` EXIT=0 (`--lib` 70, `capstd_tests` 6, `module_tests` 12, `modhost_cli` 42/1 ignored); `docs/distributed/evidence/d09-*.log` | runtime-tested (capability refusals, ownership, revocation at the FFI boundary) | Build/load are pinned to an approved artifact (lock digest + exact features before cargo, library digest + ABI exports before dlopen); each instance gets an opaque directory handle rooted at its state dir and a single-peer network handle, with bounded, copied, panic-contained `rs_cap_*` verbs attributed to the D08 context. No live transport, and no Austral demo yet (step 4) |
| D10 | verified within stated scope (steps 1a–3) | U `unfer_protocol/src/distributed.rs`, `unfer_consensus/src/net.rs`, `unfer_consensus/src/signing.rs` | `cargo test -p unfer_protocol --offline` EXIT=0 (lib 353, `cell_tool` 4, `distributed_wire` 12, `protocol` 51, doctests 3); `cargo test -p unfer_consensus --offline --features network --lib` 162 passed; `--test phase6_testnet` 4, `--test request_identity` 6, `--test power_cut` 4, all EXIT=0; `cargo check -p unfer_identity -p unfer_taler -p unfer_data --offline` EXIT=0; `docs/distributed/evidence/d10-*.log` | measured | Canonical versioned frame (`UFDC`, v1) with an explicit no-op, a validate-before-allocate decoder (1 MiB body bound), every-truncation/split/coalesce coverage and a second independent decoder on the golden vectors. `net.rs` now replicates `ReplicatedValue` (payload = reservation id + canonical transaction JSON) and applies a decision from its own bytes, so a replica that never queued the submission still applies it; the shared payload map is demoted to a local outbox that is drained by the decision and never read to apply one, and a pre-D10 snapshot's queued work is read through `serde(alias = "pending")` and re-proposed. `signing.rs` signs the domain-separated frame (deployment/audience/schema/context) and still verifies the legacy `sha256(frame)` layout, since the 64-byte signature field carries no version tag. Gaps: no live transport carries these frames (D11/D17); legacy acceptance needs a cutover signal before it can be retired |

The D00 record is the ledger: every later task appends its row there with the
command, exit, evidence path and the assumption it leaves behind.

Allowed statuses: pending, in progress, implemented with gaps, verified within
stated scope, owner statement approval pending. “Verified” requires a precise
claim and checker, not a build alone. Add a link to the relevant failure witness
and mutation controls for each security change.

Copy this prompt when assigning one patch to a cheaper model:

```text
Execute task Dxx, substep N, from PLAN_distributed_security.md.
Read the task, its dependencies' execution records, the relevant review finding,
and applicable AGENTS.md. Inspect only the named implementation seams first.
Preserve existing dirty files. Reproduce the named failure, make the smallest
repair, and run the targeted acceptance check plus the affected existing suite.
Do not weaken statements, replace approved files, raise trust budgets, or expand
the protocol to get green. Report the concrete change, commands and exit results,
evidence paths and residual assumptions. Update the execution record. If this
substep cannot fit a small patch, propose the next bounded split with its witness.
```

## 8. Design references and attribution

Sources were read from Hydro commit
`92e6451bb997872928384323481bc18b22ef850b`: `hydro_lang` stream/location types,
`hydro_std/src/quorum.rs`, `lattices`, and `hydro_lean`'s grades, decisions,
process/trace semantics, quorum example, fidelity notes and gate. The detailed
[reference map](docs/DISTRIBUTED_SECURITY_REVIEW.md#4-what-hydro-contributes)
states which ideas transfer and where the reference's scope ends. Hydro's local
LICENSE is Apache-2.0. This plan adapts design ideas; no source code is copied.

**Re-checked against the checkout on 2026-11-21** (HEAD exactly
`92e6451bb997872928384323481bc18b22ef850b`, tree clean, no `target/`, so nothing
was built). Every path the review's §4 table cites exists; `hydro_lean` is 115
`.lean` files and ~70k lines on Lean `v4.33.1`; the only library allowed to carry
`sorry` is the tutorial-step library (`lakefile.toml`, `internal/GATE.md` §2).
Four qualifications the plan's tasks now encode: the denotation is larger than
what schedules realize, so proved-over-all-schedules is a safety statement only;
the quorum contract's completeness holds under a usage cap, not universally
(finding B3); the machine is a semantics of the combinators, not of the code
`hydro_lang` generates, so the module-to-Rust mirror is review plus
`#nondet_census`, not proof; and lossy transport, dynamic membership, adequacy
and the benchmark are outside the model. Its findings B1–B6 (duplicate P1a
replies forming a fake quorum, every-tick recommit and rebase, late responses at
`min < max`, a payload at a collected slot, the checkpoint treated as executed,
and the `count > f` skip) are the failure classes decisions 5 and 7 here are
written against, found by falsification (`lake exe falsify`) rather than by
proof. Hydro's gate is the model for D22 step 3 and D24 step 2: 140 headline
axiom reports each within `[propext, Classical.choice, Quot.sound]`, `sorryAx`
and `ofReduceBool` counts at 0, an empty grep for compiler seams, and nineteen
`@[csimp]` substitutions each with a kernel-checked equation.

Existing project plans remain authoritative for their own features:
[Rust interop](PLAN_valen.md), [liquid types](PLAN_liquid_types.md),
[kernel certificates and ISA gaps](PLAN_sota_kernel_verification.md),
[package/CAS machinery](PLAN_avm_pkg.md), and
[driver/VM isolation](docs/spectrum_plan.md). This work does not relabel their
open obligations as solved by distributed testing.
