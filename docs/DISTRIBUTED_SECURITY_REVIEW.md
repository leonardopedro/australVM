# Distributed security review: australVM + unfer

Reviewed 2026-11-15. Scope: the module compiler/runtime in australVM, the
federation/control plane in `../unfer`, and the design and proof machinery in
`../hydro`, including `hydro_lean`. Execution plan:
[PLAN_distributed_security.md](../PLAN_distributed_security.md).

This is a source review, not a penetration test or a rerun of the repositories'
test suites. The findings below identify concrete code paths and distinguish
observed implementation from proposed guarantees. Reproduction tests belong to
the first implementation tasks. No implementation is delivered by this review.

## 1. Snapshot and scope

| Repository | HEAD reviewed | Working tree at review |
|---|---|---|
| australVM | `95eb3bafb52d66a9618ff201fd4704ef8119773c` | Clean before this documentation change |
| unfer | `d5123ae3e62f3670e42aec3872760ac6859a5749` | Existing tracked and untracked numerical/PCNL work; reviewed as found, left untouched |
| hydro | `92e6451bb997872928384323481bc18b22ef850b` | Clean |

The host is aarch64. Tool availability was inspected; Rust is 1.99.0. Builds,
network deployments, prover runs, and adversarial reproductions were not run.
Historical test counts in READMEs are not fresh measurements for this review.

The review follows the compiler's linearity/liquid seams, JIT authorization,
module hosting and Rust interop; unfer's identity/signing, log application,
idempotency, jobs, transport and persistence, capability RPC, content encryption,
and existing proof tools. It does not audit all numerical kernels, all proxy
routes, the QuePaxa dependency's implementation, or every unsafe FFI call.

## 2. Foundations to retain

| Existing mechanism | Evidence | Useful extension |
|---|---|---|
| Linear ownership and borrow checking, with emitted decision ledgers | `lib/LinearityCheck.ml`, `lib/LinearityLedger.ml`, [memsafe](memsafe.md), `scripts/ledger2mm0` | Linear local handles for pending requests, receipts and authority; checked runtime identities across nodes |
| Opt-in total, pure refinement fragment and Why3 obligations | `lib/liquid/TotalityCheck.ml`, `LiquidWhy3.ml`, [LIQUID](LIQUID.md); `../unfer/logos/src/austral_codegen/validate.rs` | Total event transitions and bounded message validators, with explicit machine-integer bounds |
| CPS opcode registry, coverage and code fingerprints | `lib/Cps_codec.ml`, `safestos/cranelift/src/isa.rs`, `scripts/isa-census`, `scripts/cps-fingerprint` | Bind deployment evidence to source, CPS, Rust library, policy, schema and target |
| Small MM0 checking pipeline, trust budgets, approved statements, mutations | `scripts/au2mm0`, `ledger2mm0`, `mm0-comparator`; [verification plan](../PLAN_sota_kernel_verification.md) | Distributed transition certificates, invariant preservation, held-out assumptions |
| Signed transactions, deterministic application ledgers, threshold minting | `../unfer/unfer_consensus/src/{node,signing,certs,auction,mathbond,mathbond_market,attribution}.rs` | Authenticate and validate at every replica; treat ordered rejection as an outcome |
| QuePaxa TLS integration and restart harness | `../unfer/unfer_consensus/src/net.rs`, `tests/phase6_testnet.rs` | Independent payload stores, separate processes, abrupt-crash tests |
| Durable event streams and session replay | `../unfer/unfer_protocol/src/durable.rs`, `../unfer/unfer_ffi/src/durable/`, `../unfer/prob_kernel/src/{event,session}.rs` | Atomic recovery of state, deduplication, results and external-effect intent |
| Capability RPC, attenuating grants, approval and sensitive-data controls | `../unfer/unfer_edge/src/caprpc.rs`, `../unfer/unfer_protocol/src/types.rs`, `../unfer/unfer_ffi/src/handles.rs` | Authenticated remote callers, receiver-side policy and authority epochs |
| Rust integration without embedding rustc in the compiler | [umbrella](../umbrella/README.md), `safestos/cranelift/src/capstd_mod.rs`, `safestos/rsffi/` | Rust adapters for networking, durable storage, cryptography and codecs |

These are valuable mechanisms with different scopes. A frontend linearity
certificate is about recorded decisions. A content digest identifies bytes.
Neither alone proves JIT memory safety or a network protocol's correctness.

## 3. Findings and priorities

Priority P0 means fix before enabling a remote distributed profile; P1 means
required for the first supported distributed deployment; P2 means a subsequent
extension. Findings are based on source inspection unless explicitly stated.

### R01 — Identity signatures are not bound to the named DID (P0)

`../unfer/unfer_consensus/src/signing.rs::verify_transaction` uses the
`IdentityOp.signing_key` supplied in the operation. It does not compare that
key to the operation's self-certifying DID or the registered identity.
`identity.rs::apply_identity_op` accepts Create's key and does not check
Update/Revoke's signing key against the stored one. Revoke also lacks Update's
stale-sequence check.

Together these paths permit an operation naming another DID to be signed under
the submitter's own key and pass the visible checks. Add adversarial tests
through `ConsensusNode`, including replay; enforce the existing self-certifying
DID rule and registered authority. Key rotation needs an explicit protocol if
introduced later. See D01.

### R02 — Reusable encryption paths reuse AES-GCM nonces (P0)

`../unfer/unfer_data/src/crypto.rs::encrypt_chunk` uses a nonce consisting of
eight zero bytes and a chunk index. `store.rs::KeyRing::encrypt_envelope` uses
one epoch key across multiple envelopes and restarts chunk indices at zero.
`blueprint.rs::encrypt_stored_cell` similarly permits reusing a keypair.
Distinct messages can therefore reuse a key/nonce pair. A fresh key in
`publisher.rs::publish` does not repair these reusable paths.

Introduce a versioned envelope with a fresh per-object key derivation context
and authenticated metadata, using established cryptographic libraries. Keep
legacy reads explicit and prohibit insecure legacy writes. Do not describe
`KeyRing`'s retention API as cryptographic erasure: it retains the root from
which old keys are derivable. D02 covers this.

### R03 — Capability RPC currently attributes requests to the admin (P0)

`../unfer/unfer_edge/src/main.rs` documents A9: every `/api/cap/*` request is
attributed to `admin::admin_principal()`; the default loopback listener is the
compensating boundary. This is an existing documented limitation, not a newly
discovered hidden authentication mechanism.

`caprpc.rs::resolve_promise` inserts a record for a supplied unused ID without
requiring an owned pending promise. `revoke(id)` takes no caller. Exposing
these operations to distinct remote principals needs authenticated identity,
ownership checks, grant attenuation, pending-promise validation and restart
semantics. D03 and D17 address the boundary before federation exposure.

### R04 — A rejected burn can already have mutated the ledger (P0)

`../unfer/unfer_consensus/src/certs.rs::apply_burn` validates, sums and removes
inputs in one loop. If a later input is invalid, duplicated, or causes overflow,
an earlier input has already been removed and marked spent. `IdempotencyStore`
records a key only after the closure succeeds, so it does not roll this back.

Validate the full operation before mutation. Test full state equality after
every refusal, including duplicate inputs and overflow. Inspect the other
ledger variants for the same pattern without assuming they share this defect.
See D04.

### R05 — Application rejection blocks the committed log suffix (P1)

`node.rs::sync` propagates `apply_transaction`'s error before advancing
`applied_seq`. Signature-valid but semantically invalid operations can enter
the engine, so the same rejected entry can stop every later sync. Replay also
does not call the generic Ed25519 verifier in `apply_transaction`; threshold
certificate verification is an explicit exception.

Separate deterministic domain rejection from local corruption/storage failure.
A committed domain rejection needs a durable, reproducible result and an
advanced applied position; a local failure must halt recovery safely. Check
contiguous sequence numbers and immutable log entries. D05.

### R06 — The TLS harness shares payload state across replicas (P1)

`net.rs::NetworkCluster::build` creates one `SharedLedger`. Each
`LedgerStateMachine` receives its clone; the TLS consensus value is a `u64`
whose payload is found in that shared pending map. The test's application
nodes all use `cluster.engine()` over the same committed bank. This exercises
real TLS/QuePaxa integration and replay, but cannot establish independent
payload replication or data availability on separately deployed hosts.

Missing pending IDs are silently skipped by `SharedLedger::append`.
`AllowAllAvailability` is configured at the recorder. The deployment ID is
hard-coded to 1. Preserve this fast integration harness with accurate naming;
add independently persisted payload availability and replica isolation in
D10–D12.

The locked `rust-quepaxa` 0.1.0 source was also inspected in the local Cargo
registry. Its `NetworkConsensusHandler<V, ...>` is generic over the value, so
the first implementation can replicate bounded full command bytes instead of
introducing a separate content-availability protocol. Its `StateMachine<V>`
contract explicitly requires idempotence by slot and logical value ID because
its runtime store cannot commit atomically with the application's store.

### R07 — Snapshot persistence has atomicity and durability gaps (P1)

`net.rs::SharedLedger` protects bank, pending entries and counters separately.
`append` removes pending entries before extending bank; `persist` reads them
under separate locks. Snapshots can therefore straddle a transition. Calls to
`LedgerFile::save` also share a temporary filename and are not serialized by
one enclosing storage transaction. Memory is changed before persistence
success is known.

`LedgerFile::save` uses write + rename without file/directory synchronization.
The Loro backend's `persist` has the same synchronization limitation. These
operations do not establish the documented power-loss durability guarantee.
The testnet restart test is a graceful shutdown test.

Use one storage transaction/serialized writer for each atomic record and its
durable position. Define process-crash and power-loss promises separately;
test ambiguous errors instead of assuming an error proves no write occurred.
D07 and D13.

### R08 — Deduplication and leases have local preconditions (P1)

`idempotency.rs` keys selected operations by actor and serialized operation
kind, excluding sequence and signature. This can collapse intentional repeated
commands with equal content. `prune_before` forgets keys with no admission
watermark preventing an old operation from being submitted at a new log slot.
Only selected transaction variants use the store. `once` assumes its closure
is atomic, which R04 refutes for burn.

`lease.rs::LeaderLease` is a deterministic round-robin calculation over a
tick and participant count. It gives unique selection only for identical
inputs; it has no agreed epoch, durable lease, receiver-enforced fence or
cross-host tick agreement. `jobs.rs::JobQueue` is local state; `JobClaim` is
Clone. These utilities are useful but do not establish distributed exclusion.
D06, D14 and D15.

### R09 — Submit results have incompatible meanings (P1)

`engine.rs::LocalConsensus::submit` returns a committed one-based log sequence.
`net.rs::NetConsensus::submit` returns `SharedLedger::allocate`'s provisional
zero-based value ID. Both implement the same `Result<u64, Diagnostic>` trait.
`../unfer/unfer_identity/src/lib.rs::DidManager` submits, immediately syncs,
and returns as though the identity was applied; a network proposal need not
be committed at that point. Content publishing uses the same pattern.

Add typed admission/commit/application receipts and query/wait APIs, preserve
legacy local callers through an adapter, and model uncertain outcomes. D05.

### R10 — Compile-time grants do not implement dynamic revocation (P1)

`safestos/cranelift/src/cps.rs::emit_expr` calls `check_call_permission` while
emitting an application, then emits a direct call. The helper gates
`uk_*`, `uz_*`, `rs_*`; other names are admitted. This is narrower than some
older descriptions saying every foreign call is checked. Authorizer and
deployment principal are process-global in `auth.rs` and replaced by
`module.rs::load`/`hot_swap`.

This supports admission checks for known imports. It does not demonstrate
per-invocation policy freshness, multi-module isolation, safe hot-swap of
in-flight authority, or an authenticated remote principal. Review the existing
FFI caller-context checks as a separate enforcement layer; do not infer their
absence from this finding. D08 binds context and adds the dynamic distributed
boundary while retaining static admission.

### R11 — umbrella is an integration seam, not a Rust sandbox (P1)

`umbrella/src/main.rs` runs Cargo and `dlopen(RTLD_GLOBAL)` before installing
the manifest authorizer. Cargo build scripts and library initializers can run
before a later `rs_*` denial. `capstd_mod.rs` explicitly states that trusted
in-process Rust can bypass cap-std through `std` or unsafe code. The umbrella
demo itself does not inject directory/network capabilities.

Use umbrella for audited Rust adapters, host-created capabilities and pinned
artifacts. Untrusted builds and binaries require the existing isolation tier
or a separately specified sandbox. Denial before an Austral call is a narrower
claim than preventing all crate code from executing. D09.

### R12 — Proof coverage is narrower than the distributed goal (P1/P2)

`scripts/au2mm0` proves polynomial identities in its reflected fragment, not
arbitrary total state-machine induction or distributed temporal properties.
`scripts/lean2mm0-stmts` explicitly emits asserted statements: MM0 checks their
encoding, not the truth of the original Lean theorems. The ISA/CLIF and memory
projection work retains explicit gaps. Code fingerprints identify subjects
without proving compiler preservation.

`LiquidWhy3.required()` currently reads `AUSTRAL_LIQUID_REQUIRED`; a manifest
claim alone does not establish that required mode reached the compiler.
`federation.rs` creates a LocalConsensus node and, without its feature, uses
`DefaultHasher` for `artifact_cid`. Stub identities and that fallback are not
suitable distributed trust anchors.

Reuse the tools with precise evidence classes and fail required distributed
profiles closed. D18–D24 extend proof coverage incrementally.

## 4. What Hydro contributes

The reference is the local checkout above, not a claim about an unspecified
upstream release. The following files were inspected directly.

| Mechanism and source | Adaptation here |
|---|---|
| Location, ordering and retry markers: `hydro_lang/src/live_collections/stream/mod.rs`; formal carriers: `hydro_lean/Hydro/Grades.lean` | Describe each existing boundary's ordering, duplicate scope, completeness and authority; require algebraic laws only where a consumer needs them |
| Nondeterminism records and site census: `Hydro/Decisions.lean`, `HydroGen.lean`, `internal/FIDELITY.md` | Registry of timer, retry, delivery, crash, recovery and storage-outcome sites; replayable explicit choices |
| Shared denotation, graph and process step layers: `Hydro/Graph/IR.lean`, `Project/Process.lean`, `Project/Model/DistributedModel.lean` | One explicit transition core run by simulator and production host, with a documented correspondence to the source and runtime |
| Byte codecs and trace transfer: `Hydro/Wire.lean`, `Project/Model/WireTrace.lean` | Versioned bounded codecs; decoding, authenticated attribution and reconnect behavior are part of the argument |
| Contracts and counterexamples: `Examples/Tutorials/Quorum.lean`, `Examples/Paxos/Falsification.lean` | Reproduce failures before fixing/proving; count distinct authenticated members, not packets |
| Axiom and implementation audits: `Hydro/AxCheck.lean`, `Examples/AxCheck.lean`, `internal/GATE.md` | Audit theorem dependency closures, trusted leaves, subject changes and reachable nontrivial executions |
| Conditional progress: `Hydro/Liveness.lean`, `LivenessChain.lean` | Prove safety under arbitrary delay; state fairness, eventual delivery, recovery and quorum assumptions for progress |
| Algebraic merge: `lattices/src/`, existing unfer Loro backend | Apply idempotent/commutative merges to selected metadata, with explicit tombstone/retention rules |

Hydro's own fidelity document records a trusted manual correspondence to Rust,
a guarded Paxos variant with fixes absent from the faithful variant, fixed
deployment membership, and FIFO fail-stop transport; lossy transport and dynamic
membership are outside that model. Its tutorial-step library permits unfinished
proofs while the audited libraries forbid them. Do not import its README's
headline claim without these qualifications. Its pinned Lean/Mathlib is 4.33.1;
australVM's existing exporter shell is pinned to 4.30.0, so wholesale import
would introduce a toolchain migration and substantial proof dependencies.

The useful direction is to adapt these mechanisms to existing unfer protocols.
Rewriting the project as a Hydro application, adopting its Paxos implementation,
or depending on its entire metaprogramming framework is unnecessary for the
first deliverable.

## 5. Recommended delivery boundary

First deliver an authenticated, recoverable request path over the existing
consensus service: a local linear request token, a durable stable request ID,
independently available payload bytes, an ordered accepted/rejected result,
and a receiver-checked authority epoch. Demonstrate one existing certificate
operation and one scheduled-job action through retries and abrupt restart.

Keep floats, GPU calculations and general external effects outside deterministic
consensus transitions. A probabilistic answer can be a hashed, policy-validated
input; it cannot silently become shared nondeterministic control state. Metadata
can use proven merges where appropriate. Ownership, spend, authority revocation
and exclusive jobs retain coordination.

The development plan turns this boundary into small tasks, with generalization
and stronger proofs following measured behavior.
