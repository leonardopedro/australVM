# Extending AustralVM toward Bend capabilities

**Proposed, 2026-11-19.** Start at L11d.1 below. This plan continues
[Liquid Austral §10](../PLAN_liquid_types.md#10-bend-2-and-the-next-liquid-index-layer)
from the checked split pilot. It develops a practical dependent programming
profile first, then a separately checked profile for the three BendTT type
features. The CPU and accelerator work continues through the existing runtime
and GPU plans.

## 1. Outcomes and baseline

The target is one Austral application that can state and prove a pure
algorithm's laws, run admitted computation on CPU or GPU, and hold explicit
resource and completion tokens at its native boundaries. Richer source types,
backend execution and service progress have separate acceptance milestones.

| Existing work | Reuse it for | Boundary at the start of this plan |
|---|---|---|
| `LiquidCheck`, `LiquidIndexClosure`, `LiquidIndex`, `LiquidSplitPilot` | Typed admission, scoped finite conversion, branch-local facts and erasure | L11a–L11c work for a bounded index language and one audited span split. L11d's held-out proof and L11e's direct structural descent are open. |
| Why3 and `LiquidSubset`/`LiquidMeasure` | Existing VC and measure extraction integration | Why3 results identify the obligation and trust assumptions of that route. MM0 replay needs its own derivation. |
| unfer Logos, `deltanet_unf`, `netcert`, `au2mm0` | Graph normalization, explicit witnesses, proof-producing arithmetic and independent controls | The compiler's DeltaNet check is opt-in, with legacy fallback. The reducer's numeric input rejected the recorded Lean dependent substitution obligations. The checked MM0 arithmetic theories have specific domains. |
| `docs/cic_typing.mm0`, `lean-typing-proof`, the MM0 proof chain | Named typing/conversion premises and proof composition | Typing experiments still carry assumptions. The executable WGSL path and the reflected clamp bound exist. PC05's complete scalar WGSL edge and PC07's bundle composition remain open. |
| CPS/JIT, Rust adapters, `Jobs`, WGSL/wgpu | Existing CPU execution, foreign calls, explicit heap ownership and device completion | The GPU tests record physical execution and a local job lifecycle. G15a hardening, one-process two-adapter placement, and optional MLIR are future tasks. |

[Liquid implementation record](../PLAN_liquid_types.md#10-bend-2-and-the-next-liquid-index-layer),
[conversion probe](lean4mm0.md#138-the-toolchains-build-again-the-certificate-path-stops-at-the-front-end),
[GPU execution record](gpu/EXECUTION.md), [proof-chain tracker](../PLAN_proof_chain.md).

Use BendTT's explicit equality, expected-type-driven matches, affine closure
discipline and structural descent as design references. Its result about the
executable `Book.check` concerns the accepted BendTT book. A new Austral
elaborator, live resource interface and compiler each need a correspondence
argument. Use the published checker as an optional comparison oracle. The
reviewed Bend revision `9c586f094f4fc88bef440dc8b4d77a2f92c257cc` is Apache-2.0.
Create original Austral fixtures. Record the source and license of any later
reuse, including any integration of a separately licensed component.
[BendTT, §§2–8](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/paper/BendTT.pdf),
[executable checker and proofs](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/bend2/bendtt.lean),
[pinned license](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/LICENSE).

### Long-term mathematical target

The long-term target is to cover as much mathematics as practical, especially
engineering problems, through the effective Solovay–Kopperman–Mehler
fragment described in `timepiece/Book`. MM0 is the final verifier. AustralVM
is one authoring and execution interface. An Austral contract, a Logos
controlled sentence, a numerical problem description, a Bend law or a Lean
formalization can supply the same mathematical claim. Develop richer Austral
types where the engineering proofs need them. Native support for the BendTT
trio is independent of entry into the common MM0 proof path.

The Book distinguishes the completed Hilbert/measure model from the effective
language that can ask questions about it. Solovay, Arthan and Harrison provide
decidability for specified pure two-sorted real inner-product/Hilbert-space
languages by reduction to scalar algebra. The Book also establishes
finite-head integral reductions and Gaussian finite-marginal uniqueness in
the analytic model. An effective engineering profile still needs finite
syntax, effective coefficients and a verified decision or certificate
procedure for its actual signature. Even `tensor_language_decidable` in the
Book is an integral identity. Arbitrary head laws and operators retain their
own computability and approximation obligations.
[Decidability and tensor models](../../timepiece/Book/SolovayTensor.lean),
[completion and effective presentation](../../timepiece/Book/PaFreeHilbert.lean),
[Book proof plans](../../timepiece/Book/ProofPlans.lean),
[Solovay–Arthan–Harrison, §6](https://arxiv.org/pdf/0904.3482).

| Boundary | How to use it |
|---|---|
| **Authoring interfaces** | Let each frontend elaborate to an exact, typed claim with named hypotheses and a theory/profile identity. Start from the existing Logos claim language for its admitted equations and the proof-chain subject format for executable artifacts. Add a shared representation only where an additional frontend needs it. |
| **Effective solving** | Start with finite supported algebraic data and selected pure inner-product claims. Use exact Gram constraints, real-algebraic reasoning where needed, and separately specified Gaussian/Hermite moment queries. A solver can supply a proof, a certified refutation or a visible refusal. A bounded search failure leaves the claim open. |
| **Relation to the full theory** | For each shared signature, define the interpretation into the broader analytic or logical theory. Prove the translation of statements and the required satisfaction or proof-transfer direction. A conservativity claim needs the appropriate reverse direction as well. Route goals outside the effective fragment to submitted proof terms or checked imports in a named theory. This route still works when validity in the full language is undecidable. Keep original assumptions visible when changing theories. |
| **Common MM0 replay** | Reuse `mm0-rs`, `mm0-c` and the held-out comparator for exact statements and dependencies. Check the translation to the theorem the user asked for as well as the derivation. Record any trusted import. Keep model truth, a solver verdict, a statement export, and a replayed proof at their actual evidence levels. |

There is a running starting point in unfer. `logos mm0 claims` already produces
and replays arithmetic and selected Hilbert-space equation certificates.
The Kopperman overlay models finite heads. The optional Mehler overlay adds
coordinate and tail-determination laws to bounded proof search. It can close
some coordinate goals that the base overlay leaves open. General vector
equality and the Book's full quantified Gram-matrix route are further tasks.
Keep the overlay corpus as a regression gate when adding an exact solver or
another interface. [Logos MM0 and its overlays](../../unfer/docs/LOGOS_MM0.md),
[existing cross-repository MM0 gate](../scripts/logos2mm0),
[artifact proof contract](PROOF_CHAIN.md).

The first mathematical integration test should state the same overlap query
through a Logos sentence and a typed engineering input. Can two unit vectors
in the reviewed infinite-dimensional real Hilbert model have inner product
`r`? For `r = 3/5`, an
orthonormal pair gives an exact witness with coefficients `3/5` and `4/5`.
For `r = 6/5`, the required Gram matrix has determinant `-11/25`, so the
model refutes the claim. Require each interface to expose its exact statement,
dimension hypotheses, and coefficient domain. Replay the proof or refutation
in MM0, then prove the corresponding result in the Book model through the
shared interpretation. Extend next to one exact finite moment query and one
finite engineering approximation with a certified error bound. Track how
many problems in a representative engineering corpus the effective route
discharges, and what evidence is needed for the remaining claims.

## 2. Execution order

| Milestone | Cards | Deliverable |
|---|---|---|
| Checked split law | L11d.1 → L11d.2 | One reflected source split and an independently replayed conditional MM0 proof |
| Useful dependent programming | L11e → L12.1 → L12.2 → L12.3 | Indexed arguments and results, branch refinement, explicit equality transport and one proof by induction |
| Certificate-producing conversion | L13.1 → L13.2 | One DeltaNet-assisted index equality and one checked scoped substitution |
| BendTT-style expressiveness | L14.1 → L14.2 → L14.3 → L14.4 | An opt-in dependent profile with all three features and a checked live-soundness and erasure boundary |
| Runtime ergonomics | R1 → R2, R3 independently | Explicit shared immutable values, optional automatic release, and checked structured parallel operations |
| Bend interoperability | B1 and B2 independently → B3 | One Bend law checked in MM0 and one Bend component called through the existing host adapter |

L11d.1 is the first patch. The Rust shared-value pilot R1 and the existing
GPU/MLIR work can progress independently. L14.1's external checker probe can
also run while the practical profile is being built. L14.2 starts from the
small elaboration and binder representation delivered by L12. R3 can use the
current split pilot and a scalar kernel. Bend interoperability can start from
the existing Lean translator and host adapter as soon as those toolchains are
available. Later cards reuse earlier proof artifacts after their actual
compiler subjects have been checked.

The L12 profile preserves the bounded first-order TRL as the ordinary path.
New dependent syntax enters behind a profile with its own grammar, bounds,
and diagnostics. Rich terms are elaborated to an explicit checked
representation. Proof values and ghost measurements can erase after checking.
Live ownership stays with Austral's resource checker. Callable proof-only
definitions get their own audit of admissible operations and termination.
An optional tool may supply candidates for expensive conversion or proving.
Qualification for strict deployment comes from checked evidence tied to the
exact implementation and assumptions.

## 3. Finish the existing proof and add the useful dependent features

### L11d.1 — Reflect the actual split and hold out one theorem

**Files.** `lib/liquid/LiquidSplitPilot.ml`, its focused tests and one existing
`024-liquid-split` fixture. Extend the current checker to return structured
evidence for the recognized implementation, including resolved function and
field IDs, scoped binders, measured operands, numeric interpretation and the
span pair it actually constructs. Reuse the existing `LiquidCheck` entry
point. Retain source locations for diagnostics and use canonical binder
identities for proof subjects.

State a reviewed theorem independently of generated output. For one base
allocation and natural element units `off`, `n`, `k`, assume `k ≤ n` and
that the original half-open extent `[off, off+n)` fits the validated domain.
Relate element ranges to byte addresses with checked multiplication by element
width. Successful construction returns `[off, off+k)` and `[off+k, off+n)`.
Their lengths add
to `n`, and no valid element belongs to both. Preserve the relation between
unbounded index arithmetic and checked machine offsets. Separate the layout
and allocation premises supplied by the host.

**Exit.** One admitted source module produces the structured obligations.
`off=10`, `n=8`, `k=3` gives the expected ranges. Endpoint splits `k=0` and
`k=n` produce the appropriate empty half. Reversing a field, using another
base, or retaining a branch fact after its scope ends refuses the same
held-out theorem. The existing CPS erasure fixture still passes.

### L11d.2 — Discharge the split law in the supported proof lanes

**Depends on L11d.1.** Reuse `LiquidConstraints`/`LiquidWhy3` for the scoped
split VCs. For MM0, start with the exact signed/unbounded arithmetic
projection already covered by `scripts/au2mm0` and `scripts/mm0_ring.py`. For
example, under `k ≤ n`, relate natural subtraction to integer `n-k`, then use
the ring proof for `k + (n-k) = n`. Derive the guarded natural and order facts
from a reviewed theory with direct proofs or an explicitly declared residual.
The ring normalizer handles polynomial equality. Replay the conditional
source theorem with its declared premises. A strict deployment profile must
separately discharge each premise required for its actual layout and bindings.

Use the existing proof-chain obligation interface and statement comparator.
Stage new statements through the owner workflow in a scratch directory.
Record whether each claim is Why3-valid, replayed in MM0, externally trusted
or still open. The same source theorem must be usable by two launch plans
without regenerating its proof when only a lower-level binding changes.

**Exit.** `mm0-rs` and `mm0-c` replay the exact held-out derivation with its
assumptions visible. A wrong offset, missing guard, stale subject digest and
extra assumption each fail an appropriate check. A missing prover is visible
in the requested evidence lane. A Why3 verdict carries no MM0 discharge by
itself.

**Landed (L11d.2, 2026-11-20).** The split law is emitted from
`examples/verified-kernel/liquid-split/split.obligations.json`, whose variables
are the law's own (`off`, `n`, `k`, `width`): a launch plan binds them; the
artifact never names a module. `scripts/liquid-split-mm0 emit` writes
`mm0/liquid_split.{mm0,mm1}` (file sha256 `d0d91b2e…` / `4286b6a0…`), reusing
`scripts/au2mm0`'s reflection/decision and `scripts/mm0_ring.py`'s proof chains,
and records `12` certificates, `14` structural steps, `2` grounded coefficient
facts, `3` premises (guard, domain, layout) and `1` residual (half-open-range
disjointness). `check` re-emits byte-identically, replays with `mm0-rs 0.1.0`
and `mm0-c` (both exit 0), and exercises nine drift controls (plus the
artifact's own re-emit/replay): wrong offset, missing guard, extra premise, a
subject digest that moves with the law but not with a binding, two different
launch plans conforming to the one artifact, one plan with a one-element-late
right half refused, and a stale recorded subject digest refused. `check --evidence <dump>`
compares a compiler-produced evidence dump against the artifact's canonical
variables.

Evidence levels, kept separate by design: the `12` ring identities are
**MM0-replayed** (`-- trust-split: structural=14 grounded=2 certificates=12`);
the guard `k ≤ n` and the domain/layout premises are **declared and checked for
presence**, not re-derived inside the ring theory; the order fact that touching
half-open element ranges are disjoint is an **explicit residual**; the
connection from an Austral module's spans to the law is **compiler evidence**,
checked by `check --evidence` rather than folded into the artifact. The Why3
lane emits scoped VCs from `LiquidConstraints` through the opt-in
`AUSTRAL_LIQUID_SPLIT_VERIFY` / `AUSTRAL_LIQUID_SPLIT_DUMP` knobs and records
`lane: why3 …` in the evidence. With why3 1.8.2 and alt-ergo 2.6.3 installed
from this flake (`nix profile install "git+file://$PWD#why3"` and `#alt-ergo`,
then `why3 config detect`), all `8` VCs are **Why3-valid** for the fixture;
the `001-accept-split` evidence says `lane: why3 valid (8 goal(s))`. A missing
engine is still a structured `[gap]` (the suite asserts it with a fake
`WHY3_CLI`), and a Why3 `Proved` line never upgrades the MM0 artifact. Two
latent bugs surfaced once the engine was real and are fixed in `LiquidWhy3`:
`Util.run_command` gives the child only `PATH`, so why3 could not find
`~/.why3.conf` and a prover-lookup failure read as a refusal; and `goals_in`
looked for `"  goal"` after trimming, so a fully proved file reported
`Proved 0`.

The staged statements (`91`, sha256 `65214b34…`) are registered in §W3 as
`[gap]`, awaiting an owner commit under `mm0/approved/`; nothing here approves
them.

Commands: `nix profile install "git+file://$PWD#why3"` / `#alt-ergo` / `#z3`
(the last added as a flake package so the z3-backed golden check is
installable without the devShell), then `why3 config detect`;
`python3 scripts/liquid-split-mm0 selftest` (exit 0),
`python3 scripts/liquid-split-mm0 check --evidence <dump>` (exit 0),
`nix develop "git+file://$PWD#ocaml" -c dune runtest` (18 cases in
`LiquidSplitPilot`, all passing; with why3 configured, the positive lane test
asserts `lane: why3 valid (8 goal(s))` and the fake-`WHY3_CLI` test asserts the
gap).

### L11e — Check direct structural descent against the existing fold

**Depends on the scoped L11d interface.** Follow the existing L11e card.
Use `LiquidIndexClosure` and the typed call graph to recognize direct recursion
on one proper constructor subterm. Compare one list length or tree measure
with the existing structural `fold`. Reuse the measure and closure checker
before broadening it. Check the erased result against the existing CPU path.

**Exit.** The proper-subterm definition and fold agree on representative
inputs, with a stated structural correspondence. Self-calls on the same value,
on a rebuilt value without a decrease proof, and mutual or effectful recursion
refuse. Record the evaluator's fuel and worst observed cost separately from
the proof of termination. Lexicographic descent can follow after this rule is
measured and proved for its stated fragment.

**Landed (L11e, 2026-11-20).** The fragment is opt-in: `pragma Liquid_Descent`
on a function promises one immutable inductive argument consumed as a proper
constructor subterm in every self-call. `lib/liquid/LiquidDescent.ml` reads the
typed tree (following the temporaries typing creates for a `case` scrutinee),
resolves the recursive binder, and refuses a self-call on the original value,
on a rebuilt value, outside such a case, or in a body with an effect
(assignment, loop, mutable borrow). Mutual recursion remains L3's T1: the
carve-out removes only the marked function's *self* edge from a body-local copy
of the call graph. Fixing this exposed a latent bug in `LiquidCheck.totality`,
which returned after the first contracted function and never checked the rest.

`AUSTRAL_LIQUID_DESCENT_EVIDENCE=<path>` records the extracted rank sanity
(`fuel([])=0`, `fuel([1;2;3])=3`, `weight([2;3])=5`), the index evaluator's
`256`-step budget, the module's marked fold, and per-function self-call count
and deepest case nesting. Those are **measurements**; the decreasingness proof
stays in `lib/liquid/subset/liquid_measure.mlw` (`fuel_decreases_on_tail`), and
the checker does not claim termination from syntax alone.

Measured: `test/LiquidDescentTest.ml` (6 cases, all passing) admits the proper
descent with evidence `descent: length arg=tail self-calls=1 case-depth=1
verdict=held`, refuses original/rebuilt/effectful/mutual variants, and checks
that the marker erases to identical CPS bytes. `test-programs/suites/027-liquid-descent`
adds the CPU-path case (`001-agrees-with-fold`, fold and descent print the same
measure) and two refusal fixtures. The recursive-union checker fixture cannot
be emitted as C (its by-value tail is an incomplete C type), so its
proper-subterm cases are checked at `--target-type=tc` and the executable
correspondence uses the Int64 measure the fold combinator already uses.
Lexicographic descent and a range-tree measure are **not** admitted yet.
Rebuilding the compiler for this card moved the W5 code fingerprint in
`mm0/flux1d_certificate.mm0`; re-emitting it required re-pinning the flux cite
in `mm0/isa.mm0`/`mm0/clif.mm0` and, in turn, the isa/clif/store_projection
cite digests. Those are generated header updates, not approvals.

### L12.1 — Generalize the split to explicit indexed families

**Depends on L11d.1.** Improve `LiquidTypes`, `LiquidParse`, `LiquidIndex`
and the existing branch checker. Declare the index sorts of one indexed
family explicitly. Use the same measures and representation relation as the
split pilot to bind its indices to actual typed fields. Avoid a second
normalizer or an additional syntax path for the same first-order index terms.
Use checking against the expected family/constructor type and synthesis for
variables, annotations and calls. A successful constructor match refines the
expected result with branch-specific binders. Scope the initial extension to
one immutable positive family such as a length-indexed list. Keep the split
fixture as the resource-bearing example.

**Exit.** A checked length-preserving map has one type-level length tied to
its constructors and call sites. Refuse wrong element widths, wrong index
arity, using a branch equality outside its scope and shadowing that changes
the resolved binder. Preserve existing source diagnostics and byte erasure
when evidence annotations are added. Where indices affect runtime layout,
check the representation bridge separately.

### L12.2 — Add explicit equality and dependent application

**Depends on L12.1.** Start with a small explicit core over inherited sorted
terms. A dependent function's result type may mention the argument. A
dependent pair's second component may mention the first. Start with closed
definitions and explicit binder annotations, then check one form of each
against its expected type. Use resolved binders and capture-avoiding
substitution. Add `Eq A x y`, reflexivity after successful conversion, and
transport along a checked equality with an explicit motive. Reuse the
`LiquidIndex` comparison for first-order indices. Let unsupported, distinct
normal forms and exhausted conversion carry their existing well-defined
outcomes. A fix for symbolic arithmetic belongs in a proof rule or lemma.

**Exit.** A typed client transports a span or list from `n + 0` to `n` and
uses an independently checked equality for a non-definitional case. Refuse
the wrong equality orientation or motive, capture under a binder, evidence
from a closed branch, and use of a ghost measurement to read a moved buffer.
No proof-only argument changes the runtime calling convention.

### L12.3 — Prove one inductive law at source level

**Depends on L11e and L12.2.** Pick list append length or a pure tree map law.
Check the expected type separately in each constructor branch. Use the
structural self-call as an induction hypothesis and an explicit equality
rewrite like BendTT's addition example. Reuse the MM0 relation and induction
rules of an audited theory where applicable. State additional rules before
using them. Keep qualifier inference separate from checking an explicit proof.

**Exit.** A stated symbolic law has a replayable derivation and the linear and
erasure regressions stay green. A mutated recursive call or modified
implementation must break its held-out proof. Time the old fold path and the
new source proof path on the same members of the corpus.

## 4. Reuse DeltaNet conversion without widening trust

### L13.1 — Replay an admitted graph-assisted arithmetic conversion

**Depends on L11d.1.** Reuse the existing `unfer/logos` reducer and its
canonical walk. Start with one admitted, open sorted term whose ground
arithmetic subexpression can contract, such as `n + (2 + 3)` versus `n + 5`.
Use a typed view of `LiquidIndex` with resolved binders to select the existing
front end or lower directly into the supported CoreIR constructors. A wire
format, if needed across the bridge, carries the context and numeric profile.
Use the existing `netcert` command as a differential probe for the legacy
surface. For the first natural-number fixture, cap `n` at 1,024 and check the
embedding into the reducer's `Int64` arithmetic. The strict adapter accepts
neither a legacy float value fallback nor a bare digest as evidence of equality.

Record a replayable path from each original term to its reduced form under
the same semantics. Recheck the node mapping against the actual graph labels
and ports when an isomorphism is used. For the first case, emit exact
arithmetic and congruence steps into MM0 using the existing rules. Report any
remaining translation/graph relation as a premise. Do not treat polynomial
TED equality as definitional equality or `tcert_witness` as a proof of an
unchecked certificate. A mismatch in signedness, width, `Nat` versus modular
arithmetic, or scope refuses before proof reuse.

**Exit.** The positive fixture agrees with the local index checker on the
supported input, and an independently replayed derivation checks the actual
term pair. Change one binder, literal, wire, or arithmetic profile and check
the intended refusal. Budget exhaustion stays unknown. The existing opt-in
constant check keeps its compatibility behavior.

### L13.2 — Close one real scoped substitution obligation

**Depends on L12.2 and L13.1.** Read the existing `lean-net-cert`/`netcert`
diagnostics and `docs/cic_typing.mm0`. Choose one source dependent application
with an explicit λ or Π and an open argument. Use the existing `CoreIR` binder
nodes if a typed lowering can preserve scope and substitution. The simpler
reference case may perform capture-avoiding substitution in the core and use
the graph to compare only its admitted scalar subterms. Require a checked
bridge lemma for any new graph encoding. Keep general Lean export, types with
unknown primitives, and arbitrary λΠ conversion outside this card.

**Exit.** One real `tdefeq (tsubst …) …`-shaped obligation, or its typed Liquid
equivalent, is discharged by replay under its actual context. Export the
input, checked steps, exact residual premises and proof dependency. A
same-spelling variable from a different context, capture in a negative
fixture, or an incorrect β result fails. Add no second generic reducer.

## 5. Optional direct support for the BendTT trio

Keep this profile independently selectable from ordinary Liquid. The project
can deliver useful L12 features before opening the richer profile to
production modules. Full parity means Austral-authored definitions can
express and check the three features under a stated semantics. Support in an
external proof assistant is useful earlier and has a separate evidence label.

### L14.1 — Build a small cross-check corpus and choose the reused core

Write original fixtures for dependent case/equality transport, a polymorphic
identity at its own type, a negative recursive family `R` with a field
`R → Empty`, an unpack operation that returns the field once, and a
self-application that copies it. Include an erased-only stress case and an
attempted live escape. Pin the kernel and record the source-to-core translation.
Use `bend --verdict` or its exported `.bendtt` book and Lean executable
checker in an optional job. Compare actual accepted and refused terms.

**Exit.** The corpus records the verdicts and their exact kernel version. A
changed export that flips the intended live result gets caught. Decide from
this corpus whether to elaborate the shared proof-only subset to the existing
BendTT checker or extend the L12 core with a mechanically checked model.
Prefer a faithful elaboration to the existing proven core where it covers
the admissible terms. Keep either path off the default build.

### L14.2 — Establish the live/erased and reuse model first

**Depends on L12.2 and L14.1.** Write the declarative rules and an executable
checker or checked elaboration for the admitted profile. Distinguish erased
uses, at-most-once affine uses and duplicable first-order data. Keep Austral's
exact-consumption permission for live resources under `LinearityCheck` and
relate that check to the restricted dependent representation. A resource or
closure's absence from a ghost term cannot by itself prove that the live value
can be copied. Enforce closure capture information, case quantities, no
live elimination from erased proofs, and structural descent on live calls.
Bind legacy and foreign functions to separately admitted checked models.

**Exit.** Confirm that an erased proof cannot be inspected by a live match or
grant a live resource, copies of live closures are refused, and a permitted
data value can be copied. Exercise branch duplication and packs containing a
closure. Show expected runtime erasure and agreement on the small positive
L12 corpus.

### L14.3 — Add the three features as separately checked changes

**Depends on L14.2.** First check explicitly typed impredicative dependent
functions in a declared scope and add self-instantiation. Then introduce the
`Type : Type` rule for that scope and a deterministic budget for conversion
through erased expressions. Finally admit negative recursive datatypes and
the constrained one-use closures that inhabit their fields. Reuse the
existing reference checker where its semantics match. Each change needs a
new positive program and a refusal whose success would break a live rule.
Record the accepted combined semantics even if the initial implementation
uses a small separate checked book as its internal representation.

**Exit.** The suite admits the self-instantiated polymorphic identity, a
type inhabiting its own universe as specified, and `R` with a valid one-use
consumer. It refuses repeated closure use, a live witness of `Empty`, an
unsupported opaque model and a required conversion that runs out of budget.
The ordinary Liquid/v1 kernel build exercises the unchanged profile.

### L14.4 — Prove the checker boundary before making the profile a guarantee

**Depends on L14.3.** Either prove source elaboration preserves typing,
live-use judgments and erasure into the pinned certified core, or mechanize
the analogous facts for Austral's admitted core. Check the executable
acceptance path against that model. Cover substitution, preservation,
progress for the pure live core, live termination and the consistency
statement used to rule out empty-type inhabitants. If conversion is supplied
by a different evaluator, establish its soundness on the terms it accepts.
Make parser, foreign models and backend lowering boundaries inspectable.
Where MM0 is the consumer, emit an actual supported derivation or record the
precise still-trusted import. A Lean prototype, a stored statement, and an
MM0-replayed theorem get distinct evidence labels.

**Exit.** The accepted profile has a mechanized soundness argument covering
its combined rules, and a checked erasure/lowering correspondence for the
executable subset it advertises. The corpus includes a minimal paradox-style
live escape, one capture error and a wrong erased layout check. The retained
proof does not claim host progress or native semantics that are outside its
model. Longer compiler proof chains proceed through PC05–PC15 as applicable.

## 6. Runtime features that can reuse AustralVM's host

### R1 — Implement explicit shared ownership for immutable host data

Use the existing `extern rust`/umbrella manifest path. Start with one flat
immutable byte or word buffer. Keep `Arc` inside Rust, following the existing
GPU host's `Arc` use. Expose an opaque Austral `Linear` handle and private
constructor. A `share` function borrows a live handle and returns one new
owning handle for the same immutable payload. `release` consumes exactly one
handle. Issue distinct host-side ownership handles and check identity and
generation. Remove each handle once at the release linearization point and
let `Arc` reclaim the payload after its last owner drops. Use scoped reads or
checked copying across the boundary. Match the failure API to ownership, so a
refusal that preserves a resource can
return it to the caller. Reuse the existing native symbol and grant machinery.
[Rust `Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html),
[GPU registry precedent](../safestos/gpu/src/service.rs),
[existing adapter plan](../PLAN_umbrella_production.md).

**Exit.** In a small example, two owners read an immutable payload, release in
either order, and reclaim it exactly once after the last release. A held
async worker owns a lease even if the caller stops waiting. Use-after-release,
duplicate `release`, stale generation and manifest refusal have explicit
tests. Keep actual device aliases and arbitrary cyclic data out of this pilot.

### R2 — Add automatic retain/release for one opt-in shared wrapper

**Depends on R1.** Start with function-local held handles, branch joins and
returns. Elaborate each new owning copy into a distinct retain operation and
each cleanup path into a release. A move transfers an existing owner. Check
the expanded code at the normal `LinearityCheck` and typed lowering seams.
Treat a solver-only or ghost use as erased. Require an explicit
cancellation/teardown rule for any async worker that owns a lease. Document
the compiler-generated operations and syntax before allowing nested records,
captures or multi-threaded owners.

**Exit.** The automatically lowered fixture matches the explicit R1 code
through normal, early return and refusal paths, with the same last-owner
counter. Use negative controls that remove a release or retain at a join.
Compare allocation count and retain/release overhead. Extend coverage only
where the comparison supports the convenience. Keep a device lease until
validated completion or teardown authorized by the device contract.
[BendRT memory model](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/paper/BendRT.pdf),
[Austral job ownership contract](gpu/G15_DESIGN.md).

### R3 — Use the existing fork/join candidates and GPU roadmap

Begin with one bounded CPU map over an admitted split, using two independent
jobs through a Rust worker adapter and a sequential oracle. Use the held-out
split law at admission when that evidence is available. Reuse the existing
WGSL pointwise emitter for the same pure kernel, and G22 for a specified
integer workgroup join. Use the existing optional MLIR work for the second
GPU path. Keep language-level fork/join small until one task representation
and its effect, ownership and scheduling semantics have been justified.

**Exit.** Serial and parallel results agree for the same integer inputs.
Measure CPU progress with one held worker, saturation, and disjoint output
paths. Keep GPU hardware outcomes separately scoped. G15a and G23 retain
authority over local multi-GPU placement and completion hardening. A named
NPU operator with explicit layout/DMA premises is a later backend cut.
Cross-host retries and publication follow the existing distributed plan with
fences, durable identities and bounded work. Cover service responsiveness and
hardware overlap with separate tests from the pure proof.
[GPU implementation continuation](gpu/NEXT_STEPS.md),
[distributed security roadmap](../PLAN_distributed_security.md),
[proof-chain tasks](../PLAN_proof_chain.md).

## 7. Bend as a source of proofs and libraries

Integrate Bend through two independent paths. The proof path uses the
Lean-to-MM0 translator already under development in AustralVM. The runtime
path uses the manifest-gated Rust adapter seam that already carries external
components. Both paths can start before AustralVM implements the BendTT trio.

### B1 — Take one Bend law through the existing Lean-to-MM0 path

There are two proof objects to account for. Bend's `law` and matching `def`
give an application proof in Bend. `bend --verdict` checks the elaborated
BendTT book. BendTT's Lean source separately proves checker soundness and
related results such as `adequate`. A theorem about that checker becomes
evidence for an application law by applying it to the concrete accepted book
and term and discharging the stated hypotheses.
[BendTT paper, §§2 and 8](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/paper/BendTT.pdf),
[BendTT checker and theorems](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/bend2/bendtt.lean).

Start with an original closed law for a bounded successor function, for
example `step(4) = 5`. Preserve its `.bendtt` export and exact statement.
Use `lean4export` on a small Lean declaration about that concrete checked
book and, where feasible, its specialization of the relevant BendTT theorem.
Begin with the closed validation and statement lanes. The selected Bend
revision pins Lean 4.34.0. Pair its exporter with that toolchain in an isolated
profile and keep AustralVM's existing Lean pin for its other fixtures.

Use AustralVM's `scripts/lean2mm0-stmts` to record the precise declaration
and dependencies, `scripts/lean2mm0` for supported closed certificate shapes,
and `scripts/lean-typing-proof` for the derivation lane under development.
Use unfer's Nanoda verifier as a separate source-export check where the
export is supported. Keep any remaining conversion or foundation premise in
the evidence. Extend the existing translator only for the first missing
construct that the real fixture identifies. An accepted boolean about a
book can be useful evidence, while the mathematical equality requires its
own checked proof transfer. [Existing Lean-to-MM0 translator and scope](lean4mm0.md),
[unfer's Lean export verification](../../unfer/prob_kernel/src/verify.rs),
[Bend export path and Lean version](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/bend2/safe.ts).

**Exit.** Record the actual law exported by Bend, the Lean/Nanoda result
where available, and the deepest check achieved in MM0. Close this card's
proof milestone only when `mm0-c` replays a derivation of the held-out
application law under its named assumptions. A statement export, checker
exit code, unresolved conversion, or assertion of the desired conclusion
leaves proof transfer open. Changing the law, proof, or book must invalidate
the relevant check.

### B2 — Import one pure Bend component through the current host

Use the existing U10/U11 adapter descriptor, typed scalar lowering and
manifest checks. Compile the same pure CPU successor component with the
pinned Bend toolchain. The reviewed compiler emits a C program with a
`main` and does not advertise separate native compilation. Use one bounded
worker executable behind a Rust provider first, reusing the bounded-child
path where it fits. If a minimal stable C entry point can be established,
add direct loading later through the same adapter contract. Keep the
Bend runtime's internal object layout out of the public ABI.
[Existing adapter execution record](umbrella/EXECUTION.md),
[Bend targets and current compilation limits](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/README.md#limitations).

Start with an explicitly encoded `u32` argument and result under the bound
`0 ≤ n ≤ 1,024`. Check the embedding into Bend's `Nat` computation and the
result width. Then try one immutable buffer transfer. A typed adapter must
spell out whether a resource is borrowed, moved or returned. Bend's affine
rule permits a value to be dropped. Austral's linear handle needs a defined
consumption or return on every outcome, including refusal or worker failure.
Preserve allocator ownership, region lifetime and cleanup on each side.
Resolve actual device completion before releasing any device lease. Manifest
grants cover the exposed operation. Reuse `cap-std` for host-mediated
filesystem and network access where applicable, and record the isolation
boundary selected for the foreign executable or library.

**Exit.** An Austral module calls the Bend provider through the ordinary
host path, and the typed result agrees with a direct call on the same guarded
inputs. A missing grant, invalid range, unexpected result width and failed
worker each produce a distinct refusal. No raw Bend heap pointer crosses the
interface. A GPU integration can follow after the CPU boundary has evidence.

### B3 — Bind the checked law to the implementation the host calls

**Depends on B1 and B2.** Reuse the existing proof-chain subject and
dependency model. Associate the exact Bend law and elaborated term with
the compiled component, its toolchain, the ABI adapter and the numeric
profile. Establish the needed runtime correspondence for the admitted
observable operation. The artifact hash identifies the compiled bytes.
The proof transfer establishes what those bytes are allowed to return.
Treat any still-trusted compiler or foreign model as an explicit boundary.

**Exit.** Load the same certified component through the manifest gate and
require the exact checked law and ABI premises in its strict profile. A law
about `step(4)` certifies that case. Extending the guarantee to other inputs
requires a law over the guarded domain. Changing the artifact or wrapper
invalidates the binding. Keep the component optional and preserve the pinned
Bend license and notices where applicable.
[Proof-chain contract](PROOF_CHAIN.md),
[Bend license at the inspected revision](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/LICENSE).

## 8. Executor handoff

Read root `AGENTS.md` and the instructions in the destination subtree before
editing. Keep each patch to one card or one of its verifier/lowering steps.
Prefer changes in current files and fixture families. Record positive and
refusal cases, exact commands, exit codes, tool versions, profile and the
remaining trust boundary. Cross-repository edits need the applicable sync
scope. Preserve unrelated work and use the existing owner approval flow for
MM0 statements. Run tests against the normal compiler or host entry point.

Existing verification entry points include:

```sh
nix develop "git+file://$PWD#ocaml" -c make all
nix develop "git+file://$PWD#ocaml" -c dune runtest
python3 test-programs/runner.py 024-liquid-split
python3 scripts/lean2mm0 check
python3 scripts/lean2mm0 selftest
```

Use the repo's proving shell for Why3 and its configured MM0 toolchain for
proof replay. A scratch replay is `mm0-rs compile <artifact.mm1> <scratch.mmb>`
followed by `mm0-c <scratch.mmb> < <artifact.mm0>`. Use the existing emitter and
staged comparator once a new approved artifact is needed. Record unavailable
provers, dependency builds and hardware as gaps in their own lanes. Continue
independent patches using the executors and configurations already supported.

> Start with L11d.1. Read the source of `LiquidSplitPilot` and its tests.
> Find one real accepted split and one mutated implementation. Reuse the
> checked constructor and call-site evidence to emit a structured obligation
> for the held-out law. Run the focused test through the ordinary Liquid
> compiler path and record the preconditions and any proposed proof premises.
> Leave L11d.2, GPU lowering, shared memory and the new dependent syntax to
> their own cards.
