# Compositional proof-producing compilation in MM0

**Status: proposed, 2026-11-17. PC02 has its foundational word/map subset and
clamp bounds (`mm0/proof_chain/word.mm0`, `docs/proof/SEMANTICS.md`); PC03 has
the edge/composition layer and the conditional→discharged premise step
(`mm0/proof_chain/relation.mm0`, `docs/proof/COMPOSITION.md`); PC06 applies
that composition to G13's memory plan and states its residuals
(`mm0/proof_chain/gpu_plan.mm0`, `docs/proof/GPU_PLAN.md`); PC04 reflects the
real clamp body and its KernelIR into a proved bound
(`mm0/proof_chain/clamp_source.mm0`, `docs/proof/SOURCE_IR.md`); PC07 is the
first whole-chain gate (`mm0/proof_chain/chain.toml`, `docs/proof/CHAIN_GATE.md`);
PC08 captures one real host fragment and states its edge, open, because no
x86_64 model exists (`mm0/proof_chain/native/*.capture`,
`docs/proof/NATIVE_EDGE.md`), and PC09's MLIR stage boundary is declared with its
uncaptured passes counted (`mm0/proof_chain/mlir/stage.toml`,
`docs/proof/MLIR_CHAIN.md`); PC00, PC01 and PC05, and PC10–PC15 are pending.** This is an extension of
[PLAN_gpu_kernels.md](PLAN_gpu_kernels.md) for the owner's requested proof reuse
across Austral, intermediate representations, and CPU/GPU/NPU instructions.
The [design contract](docs/PROOF_CHAIN.md) specifies the semantics, obligation
interfaces, evidence classes and explicit native-code boundaries.

No new proof is supplied by these planning files. Existing W3 approvals,
`mm0/isa.mm0`, CLIF/memory gaps and unrelated in-progress proof work remain as
found. The goal includes final instructions; any earlier cut is an intermediate
milestone, never a replacement definition of native completion.

## 1. Required architecture

1. Application properties are proved at the highest useful shared representation.
2. Each representation has an explicit semantics and observation/resource model.
3. Lowering edges emit per-compilation correspondence/translation certificates,
   using reusable lemmas for supported constructs and passes.
4. Lower proofs apply imported higher theorems and discharge their exact residual
   obligations. Splitting/strengthening a residual interface requires an MM0
   implication proof connecting the interfaces.
5. A proof dependency DAG records exact subjects, theories, premises and proof
   applications. Its metadata is checked against actual MM0 proof dependencies.
6. Each claim reports its deepest checked representation and remaining boundaries.
   GPU virtual ISA and NPU operator IR are useful cuts, not native completion.
7. MM0-required profiles accept only proofs checked by MM0 under their declared
   foundation/assumption policy. Lean/Why3 checks, statement encodings, hashes,
   tests and human approval keep their distinct evidence classes.

Prefer per-compilation translation validation to trying to verify all of
Cranelift/LLVM/Naga/vendor compilers initially. This still requires a defined
source/target semantics, sound proof rules and a decoding/byte connection.
An unsupported instruction/pass must remain an open edge or refuse a strict
profile, not become an unchecked semantic axiom.

## 2. Relation to the GPU implementation tasks

| GPU task | Proof-chain integration |
|---|---|
| G01–G04: checked frontend/KernelIR | PC01/PC02 define the proof interface and semantics before schema freeze; PC04 relates actual source bodies |
| G06/G07: WGSL and validation | PC05 emits/checks semantic correspondence; syntactic Naga validation alone is insufficient |
| G12: first device release | May remain an explicitly execution-tested release; only a separate proof profile can claim composed MM0 evidence |
| G13: memory-plan obligations | Consumes the shared semantics/interface in PC02/PC06; Why3 is a separate evidence route, not an automatic MM0 discharge |
| G14/G15: authorized async host | PC13 supplies resource/event transfer obligations |
| G16/G17: float/composites/iteration | PC12 supplies numeric/quantization/error and progress interfaces |
| G19–G21: MLIR/native GPU backend | PC09/PC10 capture passes, actual emitted subjects and virtual/native boundaries |
| G18/G23: integrations/devices | Reuse the algorithm theorem with separately checked target/runtime instantiations |

CPU proof work also extends the existing C/CPS/CLIF/native routes; those do not
yet share KernelIR. NPU work begins with PC11's named target/operator profile,
not an assumption that a generic GPU map is accepted by every accelerator.

## 3. Executor rules and command profiles

Read `AGENTS.md`, [docs/PROOF_CHAIN.md](docs/PROOF_CHAIN.md), the current card
and the relevant existing theory/scripts. Preserve unrelated changes, especially
`scripts/lean-typing-proof` and current numerical work. All new code/models stay
in australVM unless a later task explicitly identifies a cross-repo patch.
`../hydro` is a read-only architecture reference.

Use one bounded task/subtask per patch. Begin with one property, one kernel and
one edge. Add model interpretation/negative controls before widening coverage.
Do not add an axiom with the same meaning as the preservation theorem being
attempted. A missing proof is a named conditional premise. Do not auto-approve
statements or change existing trust budgets to make a gate pass.

Candidate paths below are **new, proposed interfaces**, not existing tools.
Register emitted theories/certificates with the existing comparator/emitter
registry only after their statements and proof ownership are concrete; do not
invent approval state or alter the current W3 table on this planning turn.

Run compiler commands under the required Nix devshell, as in AGENTS.md. For a
new MM0 artifact, the actual tool invocation shape is:

```sh
mm0-rs compile /tmp/proof-stage/example.mm1 /tmp/proof-stage/example.mmb
mm0-c /tmp/proof-stage/example.mmb < /tmp/proof-stage/example.mm0
```

The `.mmb` output is positional. Use the flake environment that actually supplies
these tools and record their versions. `mm0-c` accepts a proof relative to its
specification/axioms; report that dependency closure. Missing tools/provers are
gaps, not proof success. All `dune` commands run inside
`nix develop "git+file://$PWD#ocaml" -c ...`; Lean/export commands use the existing
version-matched Lean shell. Do not import Hydro's different Lean/Mathlib pin.

Proposed gate commands after PC07/PC14:

```text
scripts/proof-chain check MANIFEST --profile POLICY
scripts/proof-chain report MANIFEST
scripts/proof-chain selftest
```

`check` returns 0 only when the requested property/cut/assumption policy is met;
1 is refusal; 2 is malformed input or unavailable required checking tools,
reported distinctly. `report` may successfully describe an incomplete chain,
but its success cannot satisfy `check`. An optional report never produces a
native-proved flag. Strict deployment profiles also require the existing
statement-approval policy independently of proof elaboration.

## 4. Milestones

| Milestone | Tasks | Deliverable |
|---|---|---|
| C0: interfaces and foundations | PC00–PC03 | Audited semantic subjects and real MM0 composition lemmas |
| C1: first reused proof | PC04–PC07 | One source property reused through a real KernelIR→WGSL edge; exact remaining native gaps |
| C2: backend branches | PC08–PC11 | Target-specific CPU, MLIR/GPU and NPU cuts, with actual instruction/compiler boundaries |
| C3: stronger properties and enforcement | PC12–PC15 | Numerical/resource refinements, strict profiles and scalable checked reuse |

PC00–PC03 are design prerequisites for the GPU proof interface; PC04–PC07 can
follow the relevant G tasks without waiting for physical hardware. A proved
source-to-WGSL chain is a meaningful first result. Final CPU/GPU/NPU instruction
coverage is complete only for the specific backends/claims whose remaining
edges are actually discharged.

## 5. Task cards

### PC00 — Audit the reusable proof inventory and actual compiler graph

**Depends:** none. **Files:** new `docs/proof/INVENTORY.md`, execution log.

Record existing artifacts by subject, theorem statement, theory, assumptions,
evidence class and approval status. Inspect the actual emitted CPU target and
the existing RV64 conversion separately. Inspect Hydro's files named in the
design, including `internal/FIDELITY.md`, rather than importing its README's
broad claims. Record source revisions and which native/device artifacts can
actually be captured on this host.

Do not copy historical counts as new measurements. Reuse current proof/check
commands only on the small relevant artifacts; no broad Lean corpus rebuild.

**Accept:** the inventory separates structural proofs, asserted statements,
conditional obligations, decoded subjects and external boundaries. It explicitly
records `clif.mm0` as shape-only, `store_projection` as open, and Lean statement
translation as statement evidence. No existing approval/proof file is changed.

### PC01 — Define typed subject, claim and residual-obligation interfaces

**Depends:** PC00. **Files:** new `docs/proof/SCHEMA.md`, schema/fixture files,
small isolated `scripts/proof-chain` parser/report skeleton.

Freeze `austral.proof-chain.v1` records from the design. Include canonical
statement/context identity, source/target binding, numeric/memory theory,
parameter/specialization mapping and allowed assumptions. Assign residual
obligation IDs from these semantics, not a human label. Bind imported theorem
names to exact content/context and dependency closure.

Reserve the optional GPU-bundle manifest reference before G02/G07 freeze their
schema. Enforce bounded parsing, no duplicate IDs/keys, no dangling references
and an acyclic proof DAG. Distinguish a missing backend from an implemented but
unproved transformation.

**Accept:** small valid manifests report exact cuts/residuals. Unknown theory,
wrong context, cycles, duplicate obligation, stale target or missing intermediate
subject refuse. A hash-only link cannot report semantic preservation.

### PC02 — Establish the minimal shared MM0 semantics

**Status:** partial. The word/order carrier, map memory and clamp bounds are
implemented in `mm0/proof_chain/word.{mm0,mm1}` with finite controls in
`scripts/pc02-controls`; the source reflection edge remains PC04 work.

**Depends:** PC01; coordinate with G02/G04 for actual KernelIR constructors.
**Files:** new `docs/proof/SEMANTICS.md`, small `mm0/proof_chain/` foundational
theories and their `.mm1` proofs; independent interpretation/witness tests.

Start with the admitted typed scalar/conditional fragment and the map memory
plan. Define program objects, evaluations, u32/Boolean values, resources and
observations. Choose an explicit reviewed foundational logic/order/word theory;
state what is defined and what its foundational axioms mean. Reuse existing
libraries only with a justified interpretation into this semantic domain.
Do not treat the old integer-ring or abstract register `R` sort as u32 by name.

Prove the first source property, the clamp result bound, over all admitted
inputs in this model. Define how the theorem's variables instantiate the actual
source function. Add finite interpretation tests/counterexamples as independent
controls, without counting finite tests as the general proof.

**Accept:** actual MM0 proof terms check against the chosen specification;
foundation axioms and interpretation/trust boundaries are listed. A changed
comparison direction or incompatible numeric model invalidates the claim.
If a necessary library/proof is absent, keep it open rather than adding the
application property as an axiom.

### PC03 — Prove generic transfer, composition and premise substitution

**Status:** partial. Edges, observation, identity, the two-edge composition,
parameter instantiation and the conditional→discharged premise step are in
`mm0/proof_chain/relation.mm0` with proofs in `.mm1`, checked assembled over
`word.mm0` by `scripts/pc03-compose`. The trace is a chain of step lemmas with
successor edge facts as premises; there is no trace induction principle yet.

**Depends:** PC02. **Files:** common MM0 relation/trace theory and proofs;
small proof-composition emitter/fixtures.

Define source/target state/observation relations, target-step simulation and the
safety-property transfer relation precisely. Prove identity, composition,
parameter instantiation and hypothesis discharge under those definitions.
Do not simply declare `refines_trans` as an uninterpreted axiom and call the
compiler proved. Trace inclusion direction must preserve every modeled target
behavior; safety and progress lemmas remain distinct.

Construct a two-edge example that references one imported source-property
theorem. Compose it conditionally with one explicit pending edge premise, then
discharge that exact premise with a checked example proof. The emitter uses
normal MM0 theorem applications/hypotheses, respecting the repository's proof
argument conventions.

**Accept:** both conditional and discharged versions check; the second has a
strictly smaller open-premise set. Replacing an assumption with a weaker or
unrelated proposition, changing its binder or duplicating a fake discharge fails.
There is no new axiom that grants the desired lowering as its own premise-free rule.

### PC04 — Bind one real Austral source property to KernelIR

**Status:** partial. The direct clamp case is reflected from the checked typed
AST and the actual KernelIR (`lib/gpu/Gpu_proof_interface.ml`, `bin/gpu_proof`)
and the committed fragment proves the reflected bound
(`mm0/proof_chain/clamp_source.mm0`, `docs/proof/SOURCE_IR.md`). The
AST/IR→MM0 mapping is a recorded trusted edge; helper reflection and the
reporting cut are not implemented.

**Depends:** PC03 and G04. **Files:** new `lib/gpu/Gpu_proof_interface.*`;
small source/IR reflection emitter; `examples/gpu/` proof fixture.

Use the actual checked typed source and serialized KernelIR, not a retyped
handwritten model of the sample. Emit the source-body reflection and lowering
witness for identity/clamp, including resolved comparisons/variables and helper
bindings supported by that sample. The MM0 statement names both semantic
objects; source parsing/reflection/decoding assumptions are separately recorded
until proved.

Apply the PC02 application theorem and a checked source→IR preservation proof
for this fragment. Extend to one helper only after the direct case checks.
Capture unsupported constructs as explicit uncovered rules.

**Accept:** the real fixture's IR inherits the source bound via theorem reuse;
reversing the actual source condition or mapping a parameter to the wrong IR
value breaks the proof. Merely changing a digest must not make a false semantic
correspondence true. A trusted reflection edge is visible in the reported cut.

### PC05 — Validate the actual KernelIR→WGSL semantic edge

**Depends:** PC04 and G06/G07. **Files:** a restricted WGSL/Naga semantic adapter,
MM0 translation-certificate emitter and tests; no general shader verifier.

Read the actual rendered shader through the parsed representation used by the
bundle checker. For the initial scalar fragment, provide a model of its
operations and a witness mapping KernelIR variables/conditions/calls to actual
shader nodes. Prove the scalar relation and generated-wrapper memory obligations
under the shared map model. Do not infer scalar equivalence from the existing
fixed-pattern memory audit alone.

Record the WGSL parser/model correspondence as a boundary unless a checked
decoding/reflection theorem closes it. Naga validation remains a syntactic/type
check and separate prerequisite. Refuse unsupported expressions/passes instead
of silently abstracting them to the source value.

**Accept:** source, IR and parsed actual WGSL have composed claim evidence.
Swap subtraction operands, change unsigned comparison, alter output indexing
and change buffer roles in scratch copies with recomputed hashes: semantic
checks fail even though byte provenance has been made internally consistent.

### PC06 — Implement exact obligation discharge and reusable proof caching

**Status:** partial. `compose_bound` is the shared algorithm theorem, applied by
the clamp clause and by G13's plan at two launch bounds
(`mm0/proof_chain/gpu_plan.mm0`, `docs/proof/GPU_PLAN.md`);
`scripts/pc06-obligations` checks the assembly, the reuse, the residual table
and forged records. The plan's arithmetic/runtime premises stay residual.
The obligation cache and the alpha-renaming matrix are implemented over that
one assembled theory: `mm0/proof_chain/gpu_plan.closure` records each
statement's canonical digest, its applications and its transitive premise
closure, and is recomputed and refused when stale; instance rows are checked by
a binder-substitution matcher that refuses a renamed-away binder, a dropped
premise and a collision. What remains is reuse *across* theories (the cache is
single-theory) and a discharge that closes a residual, since no residual has an
MM0 route yet.

**Depends:** PC03,PC05. **Files:** obligation linker/checker, tiny cache, negative
fixtures; shared theory imports and bound proof references.

Implement theorem matching with exact types/contexts, binder substitution,
subject and theory identities. Replace a pending obligation only with an actual
MM0 proof application or a checked implication to a new concrete interface.
Recompute the transitive assumption set; disallow silently added trusted leaves.
Do not accept a cited file or encoded theorem statement as a discharge.

Demonstrate one shared algorithm theorem reused by two different valid lowering
artifacts and by the same artifact with a new concrete launch bound. Reuse the
proof only when the statement's instantiated premises still hold. Cached results
are hints; final MM0 replay remains authoritative.

**Accept:** unused higher proof bytes remain stable when a lower obligation is
closed. Wrong target/profile, hidden stronger precondition, stale theorem,
alpha-renaming collision, missing binder and a forged “proved” record fail.
Changing lower code with the same source invalidates the edge certificate.

### PC07 — Deliver the first complete IR-level composition and gate

**Depends:** PC06. **Files:** `scripts/proof-chain check/report/selftest`, one
small frozen proof-chain fixture, source/IR/shader subject manifests and docs.

Replay actual MM0 artifacts and compare the declared proof graph with the
checker-established theorem/dependency closure. The acceptance claim is one
source property through the checked IR/WGSL-model cut, with parser/reflection and
remaining driver/runtime premises listed precisely. Do not call this native
GPU instruction certification.

Separate logical gaps from required approval state and external model
assumptions. Exercise missing tools, changed formulas and incomplete proof files.
Document a bounded repeat command for a cheap executor; do not require the full
Lean corpus or a GPU merely to check these proof terms.

**Accept:** theorem reuse and exact residual closure are demonstrable in a real
small source fixture. Required cut/profile failures are nonzero. Report mode can
show a valid conditional theorem without satisfying a stricter policy.

**Status:** implemented. `mm0/proof_chain/chain.toml` declares the fixture —
assembly order, the source→IR→shader cut (function, proved bound, equality link
and recorded digest per layer), one proof-graph row per theorem, and the entry
theorem with its premises, axiom leaves and residual classes — and
`scripts/proof-chain check` replays the five artifacts with `mm0-rs`/`mm0-c`
before holding every row to what the replay establishes. The graph counts
proof-position references as dependencies, so `transported_bound` is seen to
reuse PC05a's `wgsl_inherits_source_bound` rather than restate it; PC06's cache
uses the same definition (`name@n` applied, bare `name` mentioned) and checks it
against the artifact that exercises it, so the two graphs cannot drift. `check --profile
certified` refuses the one external model assumption (`native_agrees`);
`report` prints the same chain as a conditional theorem without applying the
profile, which is the plan's "report mode" sentence. `selftest` runs ten
controls — a changed statement, an incomplete proof file, a forged proof
record, a deleted cut layer, a graph row that drops a dependency, an undeclared
residual, a disagreeing shared digest, the strict profile's refusal, report
mode's success where that profile fails, and an absent toolchain as a gap.
Nothing here is GPU instruction certification, and no parser/decoder is proved:
the cut's edges are recorded reflections.

### PC08 — Connect one actual CPU instruction fragment

**Depends:** PC07; actual CPU code-capture/build availability from PC00.
**Files:** isolated CPU decoding/semantics adapters, small captured code fixtures,
new MM0 edge certificates; existing bridge capture interface only if required.

Pick the actual emitted host ISA and one bounded scalar fragment. Capture the
real code bytes, ABI, relocations and specialization. Define the decoder→semantic
object relation and prove the selected instructions simulate the corresponding
actual C/CPS/CLIF fragment. Add separate edges where an external C compiler is
used. Existing KernelIR source properties can be reused through a proved common
source relation; do not invent a KernelIR→CPS path that the compiler does not run.

Use a supported reviewed instruction model or record the exact foundational
model assumptions. RV64 model statements cannot certify AArch64 output. Opcode
census and `encdec` laws remain useful coverage checks, not this preservation proof.

**Accept:** one actual native fragment has a conditional or fully discharged
claim with an accurate target cut. Change an instruction operand/immediate or
relocation and the relation fails. An unavailable decoder/model produces an
explicit open edge, not a successful native profile.

**Status:** partial, by the card's own third sentence. Two fixtures are
compiled on the CPS path and captured through the bridge's `cps2cell`:
`native/add.capture` holds the machine code the JIT would run for `add` (22
bytes, 7 instructions, one `Abs8` relocation to `trappingAdd`) and its caller
`main`; `native/arith.capture` holds `sub` (68 bytes, 19 instructions) and `mul`
(71 bytes), whose arithmetic is *inlined* by `trapping_arith_op` — `sub r12,
rsi`, `imul rsi`, `seto r8b` and a trap branch falling back to
`trappingSubtract`/`trappingMultiply` — plus their caller's immediates `9`/`4`/`3`.
`scripts/native-edge` re-decodes each capture with a closed per-fragment x86_64
table (13 and 28 forms; an unknown byte is refused, and a relocation must land on
an immediate of matching width), re-derives each capture byte-identically from
its fixture when a toolchain is present, and holds the generated statements
`native_add.mm0`/`native_arith.mm0` to them. The edges are **open**:
there is no x86_64 instruction semantics in this repository (the converted
models are RV64, which `docs/PROOF_CHAIN.md` records cannot certify other
targets), so the artifact states `native_add_edge` with `-- certificate: [gap]`,
no `axiom` and no `theorem`, and the gate prints the gap rather than a profile.
Ten controls per fragment each refuse: a changed argument immediate, a changed
relocation target, a flipped instruction byte, a truncated body, a relabelled
arch, a hand-edited statement, an edge relabelled a certificate, an added axiom,
a **reviewed x86_64 model appearing** (`MODEL_SOURCES`: `$X86_MODEL` or
`mm0/statements/x86_instructions.mm0`), and a changed *cited* model whose pin is
stale. The model itself is now real, not deferred:
`mm0/proof_chain/native/X86Arith.lean` cites the Intel SDM for `SUB`, `IMUL` and
`SETO` and proves twenty-four theorems on a toy machine state — the inline value
and the clear overflow flag for in-range arguments, the guarded path returning
that value, the out-of-range cases setting OF, the `imul` truncation, the SysV
frame at real stack addresses (`push`/`pop`/`sub rsp`/`mov rbp,rsp`, the
callee-saved `r12`, `call`/`ret` each moving `rsp` one slot), and the edge from
entry to `ret` (`run_fragment_in_range`, `run_fragment_restores_caller_stack`,
`run_fragment_restores_rbp`, `run_fragment_overflows`). The fallback is wired
in `run_fragment_calls_the_trap`: the callee is a theorem parameter, so
`trappingSubtract`'s body is not assumed, but the call's `rdi`/`rsi` arguments
and its `rax` result are proved. `scripts/native-edge check`
runs `lean` on it, pins its sha256 into both stated edges, and refuses a capture
whose ALU form has no semantics in `MODELED`. The `[gap]` is now named: the
arithmetic core, the stack-pointer discipline and the `call`/`ret` wiring are
proved; the stack's byte contents, the ABI's memory model and the trap callee's
body are not. Closing the rest needs those in the same model (and a bitvector
word type); none of that is invented here. The sibling `../Bend` checkout was searched
and is not an ISA model: it emits C/Metal/CUDA, so nothing was converted from it.

### PC09 — Track and certify selected MLIR lowering steps

**Depends:** PC07 and G19/G20. **Files:** MLIR subject/pass capture, restricted
semantic adapters and per-pass witness/checker modules.

Capture real pre/post IR for every selected pass or an explicitly declared
combined pass boundary. Include dialect semantics, legalizations, canonicalization
and target options in the subject identity. Start with one scalar clamp/map
without floating-point transformations, fusion or shared memory.

Reuse the PC03 composition and PC04 algorithm theorem. Prove selected pass-local
relations using small instantiated rules/translation validation. A pass-manager
success or MLIR verifier is not an equivalence proof. An uncaptured optimization
cannot disappear from the chain because the previous and next files have hashes.

**Accept:** actual Austral→KernelIR→MLIR→selected device-IR claim composes, with
all unchecked passes listed. Mutate a comparison predicate or pass option and
stale/invalid evidence is refused.

**Status:** started, with the first pass now **captured**. The pre-MLIR side is
the `clamp` KernelIR PC04 reflects and PC07 composes (`ir_sha256 =
48caf04e…`, predicate `GreaterThan`, `u32-modular-v1`). `G20a` is built:
`lib/gpu/Gpu_mlir.ml` renders that same KernelIR to a `gpu`/`arith`/`memref`/
`scf` module (signless `i32`, unsigned predicates, no wrap flags, explicit
static extent, dominating `ult` guard), `gpu_proof dump-mlir` writes it, and
eleven tests hold the encoding, the named refusals (helper call, non-v1 profile,
non-positive extent, one-armed return) and the mutation requirement. The
capture is byte-pinned: `mm0/proof_chain/mlir/ir/clamp.kernelir` and
`clamp.mlir`, with `kernelir-to-mlir = captured` in
`mm0/proof_chain/mlir/stage.toml`. `G19a` is met on this host: `scripts/gpu/mlir-probe` pins `mlir-opt`/`llvm`
19.1.7 and runs verify → `--gpu-lower-to-nvvm-pipeline=cubin-format=isa
cubin-chip=sm_70` → PTX for both the handwritten smoke fixture and the emitted
`clamp` module, recording versions, target, pipeline, parameter layout and
digests in `mm0/proof_chain/mlir/toolchain.toml` (four controls; a `[gap]` when
no MLIR is installed). So the captured `clamp.mlir` is *verified and lowered* to
`sm_70` PTX (`mm0/proof_chain/mlir/ir/clamp.ptx`, guard visible as
`setp.gt.u32`), not merely captured, and `scripts/mlir-chain check` attests that
the file it records is that same module. `mlir-canonicalize` is captured as a
real pass now: `scripts/mlir-chain capture-canonicalize` runs `mlir-opt
--canonicalize` on the captured module (the `scf.if` becomes `arith.select`)
and pins both sides, which must chain by digest to `kernelir-to-mlir`'s output.
`convert-gpu-to-nvvm` stays `uncaptured` as a *PC09 pass*: the bundled pipeline
is not a pass-local relation and no MLIR semantics is available, so no
preservation is proved and the checker prints both the
capture-is-not-preservation `[gap]` and the uncaptured-pass `[gap]` beside its
`[pass]`. `scripts/mlir-chain` also
re-derives the subject from `clamp_source.mm0`, requires exactly the pinned route
and options, rejects a captured row whose IR is missing or does not hash, and
refuses a captured pass whose pre side is not its predecessor's post; seven
controls refuse (comparison predicate, pass option, dropped pass, forged
capture, stale digest, restated count, an unchained capture).

`G20b` is built: `--target-type=mlir` publishes `kernel.kernelir`,
`kernel.mlir`, `kernel.ptx` and `bundle.json` through the compiler's own
frontend, with the chain `kernelir → mlir → ptx` plus toolchain, target,
pipeline, ABI and the numeric contract. `gpu_proof verify-mlir-bundle` re-hashes
each derivative, requires the emitter's own header digest to match the recorded
IR (a re-hashed forgery does not rescue the pair), and re-derives the ABI from
the PTX. `scripts/mlir-bundle` compiles the fixture through WGSL and MLIR and
requires their KernelIR identity and bytes to agree, publishes the MLIR set
twice for byte identity, and fires each refusal (tampered derivative,
conflicting profile, reduction, non-positive extent, missing `mlir-opt`,
nonexistent override) beside the positive control that a CPU target still
compiles without MLIR. What this buys the proof chain is a *subject*: PC09's
captured module now has a compiler-produced sibling with a checkable provenance
chain, and the compiler's PTX was measured equal to the G19a probe's capture. It
does not buy preservation — no MLIR semantics is declared, so
`mlir-canonicalize` and `convert-gpu-to-nvvm` stay unproved.

### PC10 — Make GPU virtual/native boundaries enforceable

**Depends:** PC09 and G21 for the native-MLIR branch; PC05 suffices for an initial
WGSL-branch inventory. **Files:** GPU subject/capture records, strict target
profiles and smallest available decoder/translation tests.

Track actual backend output: SPIR-V where produced, PTX where produced, and the
native GPU executable only when it can be captured and identified. Driver
specializations/optimizations, device revision, ISA and launch ABI are separate
inputs. Reuse device-IR proofs only after proving an appropriate machine-state,
memory and schedule relation for the supported instruction fragment.

Initially the useful result may be a rigorous PTX/SPIR-V cut with a named
native-lowering gap. To extend it, implement/check a target-specific decoder and
semantic translation certificate for one actual native fragment. Do not create
a universal GPU ISA or use disassembly text alone as a semantics proof.

**Accept:** a PTX-only or WGSL-only certificate cannot satisfy native-required
policy. An inaccessible native blob is reported as uncaptured. Any claimed
native fragment is linked to actual bytes with operand/memory/ABI mutation
controls. Final hardware behavior remains relative to its stated ISA model.

**Status:** implemented for the NVIDIA branch, with the native-lowering edge
still a named gap. `safestos/gpu/src/target.rs` makes the boundary a total order
(`wgsl < spirv < ptx < native`) and a record (tier, digest of the bytes at that
tier, ABI, and for native the PTX subject, device and blob facts); `verify`
refuses a record below the policy, a native record with no bytes (an
*uncaptured* blob), bytes whose digest is not the recorded one, and a container
that no longer identifies as what was recorded. `avm-target capture` links the
published PTX with the driver's own linker (`cuLinkCreate`/`cuLinkAddData`) and
`target::identify` reads the result as an ELF64/`EM_CUDA` container: format,
`e_machine`, the `.text.*` byte count and per-section digests — identification,
not disassembly, and the record carries that limit. `scripts/gpu/boundary`
captures `difference` and `add` at extent 65 (2600-byte container, 384 bytes of
SASS each), records them in `mm0/proof_chain/nvidia/native.toml`, and requires:
a `ptx` claim that names the same bytes is refused `--policy native` (and
satisfies `--policy ptx`); an operand-order edit to `sub.s32 %r6, %r4, %r5`
links to different bytes the record refuses; the same entry at extent 66 (a
different baked memory bound) likewise; a bundle whose recorded `abi.params`
was raised by one descriptor is refused before linking; and an absent blob is
reported *uncaptured* (exit 3), never verified. Measured while building it: a
commutative `add` with its operands swapped links to the *same* cubin (ptxas
canonicalises it), which is why the operand control uses `sub`. `check` holds a
fresh capture to the record field by field, so a driver change is visible drift.
What is not established: the native-lowering relation (PTX → SASS semantics),
any operand/memory/ABI *preservation* — the mutation controls show the record's
subject is these bytes, not that the bytes mean the PTX — and the SPIR-V tier
has a policy slot but no producer. One device and driver are measured
(GTX 1060 Max-Q, `sm_61`, driver API 13000).

### PC11 — Define the first NPU proof branch and its opaque cut

**Depends:** PC01,PC02,PC07. **Files:** `docs/proof/NPU_PROFILE.md`, isolated
operator/quantization/command schemas and proof fixtures; no generic NPU backend.

Choose one actual target/runtime and one supported tensor operation. Establish
whether its compiler IR, command/executable format and instruction semantics are
accessible. Define a tensor/operator extension and its mapping from an admitted
Austral computation; include shape, layout, accumulator and numeric semantics.
Reuse high-level algorithm proofs only via a checked interpretation.

Emit MM0 obligations for the inspectable operator/layout/DMA-command stages.
If a vendor compiler outputs an opaque artifact, name its identity and preserve
an explicit unproved operator→binary/firmware edge. The existing NPU SRAM gate
can contribute its narrow size property but cannot discharge this edge.

**Accept:** an honest target-specific proof inventory and at least one concrete
inspectable claim or conditional interface. A DMA certificate or successful
inference test cannot satisfy native-instruction-required policy. If the target
cannot expose the required semantics, its native proof remains blocked on that
specific external prerequisite while higher claims stay reusable.

### PC12 — Add numeric-domain and approximation transfer

**Depends:** PC03,PC06; G16/G17 before numerical kernel integration.
**Files:** shared numeric interpretation/error theories and MM0 lemmas, domain
profiles and targeted semantic counterexamples.

Define explicit bridges for mathematical integers, modular words, trapping
integers, real models, Float32 and the chosen NPU quantization profile. Separate
algebraic identity from ordered bounds/no-overflow properties. For approximate
lowering use a declared distance/error relation and prove the application
property's robustness under that relation. Compose sensitivity/error bounds,
not an unjustified sum of epsilons.

**Accept:** a simple bounded application property transfers with its exact or
weakened bound. Reassociation, saturation, signedness, bad zero-point and
insufficient margin cases fail or require new premises. An integer theorem
cannot silently become a floating-point result certificate.

### PC13 — Relate runtime resource and scheduling events

**Depends:** PC03,PC05; G14/G15 for actual adapter integration.
**Files:** a small launch/completion event semantics, MM0 resource/simulation
lemmas and adapter trace-validation fixtures.

Use explicit context/generation, buffer regions, access permissions, submit,
completion/visibility and release events. Prove invariants under arbitrary
modeled schedules and distinguish them from progress under named fairness/
completion assumptions. Relate observed adapter events to these states; document
trusted capture hooks and any unobserved driver activity.

**Accept:** the kernel's higher pre/postconditions are reused only with valid
bindings/visibility. Wrong-context receipt, early reuse after timeout, lost
completion and duplicate completion are counterexamples/refusals. Correct value
outputs alone cannot discharge the resource proof.

### PC14 — Enforce complete per-property proof profiles

**Depends:** PC07; each native/numeric/resource profile additionally depends on
its relevant branch tasks. **Files:** manifest/loader policy and `proof-chain`
gates, integration with G12/G13 and existing approval checks.

Profiles select property set, theory/model versions, backend/cut and permitted
foundation/external assumptions. Audit exact transitive theorem dependencies,
open premises, decoded subjects and current byte bindings. Reject stale,
weakened, skipped or asserted-only evidence. Keep optional execution-tested
releases available with their narrower labels.

**Accept:** staged proofs meet their own cuts and fail stricter ones. Required
native, ownership or numeric claims fail if any relevant edge is unresolved.
Approving statements does not promote a conditional theorem or an opaque NPU
binary to full proof. A missing tool cannot produce a proof-pass marker.

### PC15 — Bound proof size and preserve reuse across changes

**Depends:** PC06,PC07,PC14. **Files:** proof cache/certificate packaging and
small scaling controls; isolated tooling adapters.

Package shared theorem libraries once with content-bound imports and exact
assumption closures. Reuse the existing obligation-stream manifest/index design
where appropriate, without changing in-progress emitters. Avoid expanding a
large Lean/Mathlib proof corpus for each kernel or using a JSON cache hit as
proof truth. Bound parser depth, nodes, proof bytes and replay time by profile.

**Accept:** two backend certificates share the same application proof dependency;
a local lowering edit rebuilds its edge and dependent composition only. Cold
MM0 replay still checks the complete required closure. Missing/cyclic imports,
truncated chunks and stale cached dependencies refuse; unchanged lower proofs
are not trusted solely because their filenames stayed constant.

## 6. Completion and handoff

The first executable proof milestone is PC07, not native end-to-end completion.
The eventual claim must name a supported property/backend and the concrete
instruction boundary reached, with its explicit foundations and physical/runtime
assumptions. There is no blanket “all CPU/GPU/NPU properties verified” flag.

Create a small execution ledger in PC00 with status, relevant revisions,
commands, checked theorem IDs, pending obligations and next action. All cards
above start pending. Suggested prompt:

> Implement the next eligible PC task under PLAN_proof_chain.md, coordinating
> with its named GPU task prerequisites. Preserve existing proof work and
> approvals. Demonstrate reuse by actual MM0 theorem application, audit the exact
> remaining premises, and run the task's positive and mutation controls. Keep
> unavailable native/compiler/NPU boundaries explicit. Do not replace a missing
> preservation proof with an asserted axiom, statement export, digest or test.
