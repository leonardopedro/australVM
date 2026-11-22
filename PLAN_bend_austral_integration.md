# Bend and AustralVM as frontends for one compiler and proof toolchain

**Proposed, 2026-11-20.** The integration is complete for a feature when both
languages can express it, both feed the same implementation of that feature,
and a proof about the shared representation transfers to either program
through checked source and lowering edges. Each language keeps its source
syntax and typing rules. Publish the supported features and the deepest proof
cut per target so authors can tell where switching frontends preserves a guarantee.

Start with two versions of the same unsigned 32-bit clamp. Compile both to the
existing KernelIR, run them through the same CPU reference evaluator and WGSL
backend, then reuse one MM0 bound theorem. SafestOS is the common runtime and
CPS/Cranelift is the primary native CPU backend for both languages. SafestOS
also hosts the shared WGSL and MLIR accelerator paths. BendRT supplies design
ideas and benchmark baselines. The shared execution milestone requires that
both frontends' artifacts to run with BendRT absent. The plan extends
[the SafestOS parallelism plan](docs/SAFESTOS_PARALLEL_PLAN.md),
[the proof-chain plan](PLAN_proof_chain.md),
[the GPU work](docs/gpu/NEXT_STEPS.md), and [the Liquid/Bend work](docs/LIQUID_BEND_PLAN.md).

## What is already verified

| Path | Existing evidence | Integration boundary |
| --- | --- | --- |
| Bend `law`/`def` → TypeScript elaboration → `.bendtt` → Lean `Book.check` | Lean proves soundness and consistency of the executable checker, plus progress, termination and equality adequacy under the published hypotheses. Users write program laws and their proofs in Bend. | `bend2/safe.ts` elaboration and the Lean text parser are outside the checker theorems. Native compilation in `bend2/comp.ts` and the BendRT scheduler and device runtime need their own semantics and correspondence. [BendTT §8](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/paper/BendTT.pdf), [BendRT §12](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/paper/BendRT.pdf) |
| Austral frontend → Liquid obligations → Why3 | Typed admission, selected refinement VCs, limited totality and index conversion, and audited split machinery. | A proved Why3 VC is evidence under that prover route. Shared strict MM0 requires a derivation of the identical obligation in the approved theory. Compiler pipeline correctness requires additional edges. [Liquid work](PLAN_liquid_types.md), [Liquid/Bend continuation](docs/LIQUID_BEND_PLAN.md) |
| Austral frontend → KernelIR → WGSL/wgpu | Compiled pointwise and bounded-fold examples have run on physical adapters. MM0 has modeled word/map semantics, transfer/composition, a reflected source/IR clamp bound with a recorded trusted reflection, and selected memory-plan reasoning. | PC05 and PC07 still owe the actual WGSL semantic edge and its composition. Naga and execution tests check different properties from semantic preservation. [Execution record](docs/gpu/EXECUTION.md), [PC04 reflection](docs/proof/SOURCE_IR.md), [proof tracker](PLAN_proof_chain.md) |
| Austral → CPS → CLIF → CPU | JIT execution, opcode census and encoding coverage, selected step lemmas, imported RV64 model statements, and explicit projection work for memory. | Coverage and toy proofs leave compiler layout, CLIF/native semantics and actual machine bytes as separate proof work. KernelIR has a CPU reference evaluator today. A common native KernelIR lane is new. [Kernel verification plan](PLAN_sota_kernel_verification.md), [proof tracker](PLAN_proof_chain.md) |
| Lean → `lean4export` → AustralVM translation → MM0 | The repo records statement encodings, selected closed certificates and generated typing derivations with explicit premises. Unfer can also check supported Lean exports with Nanoda. | Lean export checking, MM0 statement translation, a typing derivation, and MM0 replay of the original mathematical claim are distinct milestones. Full replay of BendTT's Lean metatheory is open. [Translator lessons and limits](docs/lean4mm0.md), [unfer verifier](../unfer/prob_kernel/src/verify.rs) |

## Shared tooling and proof contract

```text
Bend source    → Bend checked book and live-term projection ┐
                                                            ├─→ typed shared subject
Austral source → Austral typed body and Liquid obligations ─┘    (KernelIR first)
                                                                     │
        Bend laws / Liquid contracts / Lean / Logos                   ├─→ CPS/Cranelift CPU and accelerator lowering
                           │                                         ├─→ SafestOS ABI, scheduler and job service
                           └─→ named claims and proof producers ──────┴─→ one MM0 proof graph and verifier
```

- **Shared subject.** Extend PC01 and the existing `KernelIR` serialization.
  Pin the canonical semantic bytes, typed inputs and outputs, numeric profile,
  any observable traps or effects, and the memory contract when applicable.
  Put source spellings, positions, entry names, frontend versions and AST or
  book identities in the source binding. Two bindings can refer to the same
  semantic subject. Shared compilation and theory reuse follow semantic identity.
  An existing bundle may still contain frontend-specific provenance.
- **Source and target edges.** A source edge connects the actual checked live
  body to the common subject. Source acceptance is a prerequisite. Each later
  edge describes which observations or properties it preserves and the premises
  it needs. Bind it to the actual emitted bytes and their decoded form. Use
  PC03's MM0 composition to apply the higher theorem and discharge only the
  residuals for this edge, such as the wrapper's tail guard or CPU operand width.
- **Proof import.** A Bend proof term, its Lean metatheory, and an Austral VC
  may justify the same canonical theorem. Each import identifies the exact
  statement, theory and transitive assumptions. MM0 may replay a translated
  derivation or reconstruct the same theorem using the existing MM0 rules.
  Reconstruction earns credit for that theorem. Keep the proof's origin accurate.
  An encoded Lean statement, a boolean checker verdict, and an unchecked
  Why3 success are visible evidence with different guarantees. Keep unresolved
  source, conversion or foreign premises visible in a conditional report.
- **Common proof gate.** Extend the PC07/PC14 proof-chain verifier and existing
  W3 statement comparator. Require `mm0-c` success under the profile's declared
  foundations, exact dependency closure, and any owner-approved statements the
  profile requires. Report separate results for execution, semantic evidence,
  assumptions and the deepest representation reached. Cache a shared theorem by
  its semantic subject, statement, theory, assumptions and checker version.
  Cache each lowering edge by its own input/output, checker and toolchain pins.
  Revalidate a changed source binding without rebuilding the shared theorem.

For example, establish `clamp(x) ≤ 100` for every unsigned 32-bit `x` once in
MM0. The two source edges connect their checked bodies to one KernelIR clamp.
The WGSL edge then justifies applying the theorem to each valid output element
under the array and launch premises. A CPU edge justifies applying it to the
decoded CPU program under the CPU ABI. A hash identifies what those edges cover.
The edge proofs supply the semantic argument.

## Implementation cards

The IDs below track integration deliverables. The PC, G, L and B cards remain
the places to implement their existing compiler, solver and runtime work. Each
card below should land as small patches with its acceptance command and a
negative control. Use the first actual failing obligation to scope a patch.

### BI00 — Pin the inputs and reproduce one example from each frontend

Record the Bend commit, the Bend and Austral toolchains, exporter versions, and
the exact current PC04 assumptions. Start with Bend
`0e2f38f759cc830849e489307f3d256a7d148288`. Read both repositories' `AGENTS.md`.
Create an original Bend U32 clamp with branches that match `Pointwise:clamp`.
Run it through Bend's ordinary checker and native or JS runner. Freeze a small
oracle covering `0`, `99`, `100`, `101`, `2^31` and `2^32−1`. Drive the existing
Austral clamp through its normal WGSL bundle command and `avm-gpu reference`.
Keep B1's closed successor law as a separate Lean-import probe. Run each Bend
Lean/export command with the matching pin from Bend's `safe.ts` (4.34.0 at this
revision) and use AustralVM's existing 4.30.0 shell for its own artifacts.

**Exit.** Checked-in fixtures and replay instructions reproduce both frontends'
results and identify the evidence at every cut. Reverse the comparison in one
fixture and require the oracle to catch it. [Austral command and fixture](examples/gpu/pointwise/README.md),
[Bend export/toolchain](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/bend2/safe.ts),
[B1](docs/LIQUID_BEND_PLAN.md#b1--take-one-bend-law-through-the-existing-lean-to-mm0-path).

The Bend-side commands, with `BEND_FIXTURE` pointing to the new source and
`REPLAY_DIR` to its scratch output directory, are:

```sh
bun ../bend/bend2/main.ts "$BEND_FIXTURE" --verdict
bun ../bend/bend2/main.ts "$BEND_FIXTURE" -o "$REPLAY_DIR/clamp.bendtt"
bun ../bend/bend2/main.ts "$BEND_FIXTURE"
```

### BI01 — Make the existing KernelIR consumable by both frontends

**Depends on BI00.** Keep one semantic schema. Reuse `lib/gpu/Gpu_ir.ml` and
the strict parser, evaluator and validator under `safestos/gpu`. Expose a small
versioned consumer for admitted KernelIR plus existing memory-plan inputs.
Extract the existing bundle construction behind it where needed. Preserve the
ordinary Austral `--target-type=wgsl` path and its v1 bundle semantics. Keep
the source binding around the canonical IR so language names and source spans
can differ while the backend cache and proofs use the same key. The shared
consumer checks types, operation identity, literal ranges, argument order and
shape before code generation.

**Exit.** The existing clamp still passes `scripts/check-gpu source`. Refeeding
its admitted IR yields the same `ir_sha256`, effective memory plan and shader.
An unknown profile, out-of-range literal and mismatched parameter type refuse
before emission. Land this extraction independently of the Bend adapter.

### BI02 — Add the checked Bend export and connect the two paths

**Depends on BI01.** Use the checked Bend `Book` from the ordinary load/check
path. Export only the chosen definition's admitted live body, signature and
referenced definitions through a small adapter outside the protected
`bend2/bend.ts`. Keep `safe.ts`'s concrete `.bendtt` output and verdict as
separate evidence about the proof terms. Record the relation between the typed
Bend body and the projected book. If either view replaces an executable body
with an opaque declaration, keep that definition outside this profile. Start
with U32 literals, scalar arguments, unsigned comparison and branch returns.
Lower the actual selected body to the common IR. Preserve operation IDs,
module resolution, branch order and the U32 width. Bind the existing Austral
`Compiler.prepare_module`/`Gpu_subset` output through the same consumer.
Defer unsupported live constructs with named source-level diagnostics.

**Exit.** The two clamp sources produce identical semantic IR and the same
generated WGSL through BI01, with distinct provenance. Run both with `avm-gpu
reference` and `avm-gpu validate`. Add a pair for asymmetric unsigned compare
or modular subtraction when that operation is admitted. Swapped operands,
signed comparison, a different literal, a foreign stub and an attempted
proof-to-runtime leak must either produce a different semantic subject or
fail admission, as appropriate to the mutation. This closes the first
shared-generation milestone.

### BI03 — Certify both source edges and one shared theorem

**Depends on BI02, PC01–PC04.** Extend the PC04 reflection with a checked
translation for the admitted AST forms. Define evaluation for each restricted
source representation and prove the reusable case rules for literals,
arguments, comparisons and branch selection. Generate a derivation from the
actual typed tree on each build. Retain an explicit input boundary for each
frontend and close it before claiming a guarantee from source text. A first
implementation can start at the independently validated typed export and report
that cut. To reach source text, validate or verify the source-to-export relation
for the admitted grammar, including entry resolution and elaboration, using the
actual source and checked core. Apply the existing word theorem to both edges.

**Exit.** The verifier checks two source-specific edge proofs that refer to
one byte-identical shared MM0 theorem and statement. Against the held-out
bound, an altered predicate, high-bit signedness mutation, swapped branches
or independently rehashed changed export fails proof admission. Report both
source boundaries with their actual evidence. Keep PC04's existing trusted
reflection assumption visible until the new certificate discharges it.

### BI04 — Connect the shared Lean and Why3 proof inputs

**Starts after BI00. Integrates with BI03.** Execute existing B1 first. Export
the exact Lean declaration specialized to the accepted Bend book and preserve
its root and dependencies. Check the supported export with Nanoda where
available. Use the existing statement, closed-certificate and derivation tools
at their documented scopes. Minimize the first actual unsupported Lean rule
and give it positive and negative fixtures before extending
`scripts/lean-typing-proof` or its certificate path. For the clamp, require
that a Bend law's specialization and the held-out theorem have the same
translated proposition, including U32 semantics and its quantification. On
the Austral side emit the corresponding Liquid/Why3 VC, match its premise set
to the shared claim, and replay that derivation in MM0 or reconstruct it from
the same word rules. Preserve the useful Why3 feedback in the compiler. Reuse
`au2mm0` for admitted integer-polynomial obligations and the L13 Logos/DeltaNet
path for supported conversion witnesses, with exact word semantics handled by
the existing word theory or an explicit bridge.

**Exit.** `mm0-c` replays the held-out closed B1 claim and the selected
shared claim at the stated theory. At least one proof authored in Bend and one
Austral contract reach the shared verifier with the declared proof origin and
assumptions. If replay initially needs a missing rule, retain a conditional
record and keep the proof milestone open. Negative fixtures change a Lean
dependency, drop a Why3 premise or weaken an equality while preserving checker
acceptance of the surrounding file. Each relevant strict check still fails.

### BI05 — Finish one shared GPU proof and one native CPU proof

**Depends on BI03.** Land the GPU and CPU patches independently. For GPU,
finish PC05/PC06/PC07 over the actual WGSL/Naga representation: prove the
scalar relation, wrapper indexing and guarded writes against the memory plan.
Record the driver/device boundary after WGSL. For CPU, implement one KernelIR
lowering into the existing CPS/Cranelift toolchain for scalar entrypoints. Reuse
its checked arithmetic operations where their semantics match the IR. Keep
the initial entry free of foreign calls and reuse the existing memory plan for
future array kernels. PC08 must identify and decode actual native bytes on one
pinned target and certify the admitted operators, ABI and control flow. Run
both sources under the ordinary SafestOS module host and keep the reference
interpreter as the oracle. This establishes the CPU backend for the
[parallel execution tasks](docs/SAFESTOS_PARALLEL_PLAN.md#implementation-cards).

**Exit.** Both frontends use each backend and reference the same corresponding
lowering proofs. Both receive the WGSL-level pointwise bound with the exact
launch premises. Both receive a CPU-native bound for the pinned CPU subset
once the bytes are certified. Use a valid, rehashed changed shader and an
altered CPU condition that still decodes to exercise semantic refusal. Record a
physical GPU run as execution evidence and report native GPU separately.

### BI06 — Give both languages the same resource and library boundary

**Depends on BI02, BI05's CPU path and the relevant U10/U11 admission work.**
Extend the existing manifest-gated scalar ABI and its generated wrappers.
Compile the first shared Bend library component through CPS/Cranelift and
load it through SafestOS. Use the same ABI from Bend to a shared host provider
or Austral component. B2's bounded Bend worker is an optional compatibility
lane for libraries whose live code is outside the shared compiler profile.
Start with guarded scalars, then an immutable byte buffer with a checked
layout and lifetime.
Record call effects via the shared host. Use `cap-std` for applicable
host-mediated capabilities and explicitly record worker isolation and OS limits.
Keep internal heap layouts behind the wrappers. Distinguish affine values which
can be dropped from linear handles which every branch must consume or return.
Permit forgetting an owned resource only through a defined discharge operation.
Treat read-only sharing with the explicit retain/release protocol from R1 before
considering R2's automatic wrapper. Use B3 to attach a proved law to the actual
component and ABI. Treat the existing Bend native backend as a named premise
until covered by its own correspondence evidence or replaced for that component
by the shared lowering.

**Exit.** Each frontend calls a shared provider through the same contract and
can call an admitted component authored in the other language via SafestOS
and Cranelift, with BendRT absent during execution. Tests cover
missing grants, out-of-range words, double consumption, live-borrow escape,
bad provider bytes and worker failure. Trace cleanup and any result publication.
[Existing host and adapter evidence](docs/umbrella/EXECUTION.md),
[B2/B3 and shared ownership](docs/LIQUID_BEND_PLAN.md#6-runtime-features-that-can-reuse-australvms-host).

### BI07 — Add a common dependent proof profile, then widen it

**Depends on BI03/BI04 and the L11–L13 prerequisites per feature.** Introduce
each common feature using an admitted program from each language. Begin with
the existing Liquid split invariant, bounded indices, branch-local equality
and explicit equality transport. Reuse the L11/L12 admission and erasure work
and BendTT's checked terms and definitions. Add structural recursion and one
indexed family after the preceding equality and scope certificates replay.
Model erased indices/proofs and live data with explicit quantity rules. Test
that erased data cannot decide a live branch except through an admitted
computational witness and that a live affine closure cannot be duplicated.
Keep automated Liquid checks on this supported fragment for quick feedback.

For the opt-in BendTT profile, follow L14 and extend BI04 against the selected
BendTT Lean theorems in dependency order. First list their actual assumptions.
Replay the subset's typing/substitution rules, then live/erasure preservation,
then the used instances of `sound`, `halts`, `consistent` and `adequate` as
needed. Reuse a modeled BendTT checker through tooling if its checked interface
and translation suffice. Extend the native Austral surface only for proven
use cases. Treat `Type : Type`, impredicative functions and negative recursive
datatypes as separate increments with independent positive and paradox
refusals. Carry proof-only forms through the existing erasure model so the
same lowered executable can benefit from their proofs. Close the applicable
metatheory debt before advertising full native parity for these features.

**Exit.** Both frontends prove the same split and indexed structural law and
consume one proof of the shared claim. Mis-scoped substitution, duplicating
a live closure and dropping a required linear handle have specific refusals.
The feature matrix lists each of the three BendTT forms and its actual proof
and execution support in each frontend, including tooling-only support.
[L11–L14](docs/LIQUID_BEND_PLAN.md#2-execution-order).

### BI08 — Share accelerator targets and bounded non-blocking execution

**Depends on BI05 and BI06.** Bring up the SafestOS CPU worker path and shared
CPS/Cranelift fork/join in SP00–SP02. Add the G19–G21 MLIR accelerator branch
over KernelIR for both frontends. Reuse G22's reduction model for CPU and GPU.
Unify submission and completion at the SafestOS host using the existing strict
GPU service's job lifecycle. SP03–SP05 cover CPU grain size and queue policies,
Cranelift continuation optimization, and batched device work. Use bounded
queues, explicit placement, cancellation, failure reporting and exactly one
result publication per job identity. Start with deterministic fake devices
under G23/PC13, then exercise CPU, two physical GPUs and one supported NPU
target as independent milestones. Reuse unfer's
federation transport for a remote worker only after duplicate, late and failed
completion tests pass locally. Apply the existing distributed plan's durable
retry identity to remote requests and make result publication idempotent on
duplicate delivery. Keep lease return tied to observable completion even when
cancellation races it. State liveness separately, with scheduler fairness and
device/network completion assumptions where needed.

Use PC09–PC12 to record each target's actual proof cut. A PTX certificate
covers virtual instructions. Native GPU completion needs a checked relation
to the actual machine instructions. For an NPU choose one documented operator
and layout, prove transfers and the numeric contract, and mark opaque vendor
bytes at the boundary until a suitable instruction model and decoder exist.
Extend exact U32 first, then add Float32 or quantization under declared
rounding, overflow and error bounds.

**Exit.** Both clients submit the same compiled job graph to the same service
and obtain the same result and resource/completion trace under the admitted
observation model. A mixed-frontend batch uses bounded asynchronous submission
while completion is delayed or reordered. Tests exercise queue saturation,
timeouts, duplicated completions, device loss and failed remote delivery. Each
device release records backend identity, verified cut and physical test result.
[GPU next steps](docs/gpu/NEXT_STEPS.md), [PC09–PC13](PLAN_proof_chain.md#pc09--track-and-certify-selected-mlir-lowering-steps),
[SafestOS and Cranelift parallelism tasks](docs/SAFESTOS_PARALLEL_PLAN.md),
[distributed runtime plan](PLAN_distributed_security.md), [unfer federation](../unfer/docs/FEDERATION.md).

### BI09 — Use the same proof service for the engineering mathematics profile

**Depends on BI04.** Establish one canonical, bounded theory profile for
effective queries drawn from the Solovay–Kopperman–Mehler work in
`timepiece/Book`. Start with exact rational Gram feasibility for two unit
vectors. Have Bend and Austral describe the same query for overlap `3/5`.
Use the same decision procedure and MM0 witness checker after elaboration;
include a refutation for `6/5`. Reuse unfer's Logos logic where supported and
preserve a Lean route for richer theorems. Add one checked interpretation of the
finite condition into the Book's Hilbert-space setting, with its dimension and
coefficient hypotheses. Give the effective fragment and full theory explicit
signatures and a checked interpretation, then extend predicates and quantifiers
only when their decision procedure and witness checking are supplied.

**Exit.** Bend, Austral, and an independent Lean or Logos input can request
the same canonical theorem and reuse the same checked certificate. Altering
the coefficients or using an operator outside the declared fragment changes the
claim or receives a named refusal. A cross-theory transfer cites its actual
interpretation theorem. [Book's decidability scope](../timepiece/Book/SolovayTensor.lean),
[unfer Logos](../unfer/logos/README.md).

### BI10 — Release one supported toolchain for both frontends

**Depends on BI03–BI09 for the capabilities advertised.** Maintain one adapter
protocol with version compatibility tests in both repositories. Initially keep
the shared compiler, proof verifier and host in their current homes and let
either installed frontend invoke the pinned release. Package a common tool
only after both adapters exercise it. Keep Bend-native features with no shared
coverage available through Bend's current tools and expose their status in the
capability manifest. A shared-profile request reports an unsupported feature
before execution. Document both source examples, their shared artifacts,
supported targets and trust assumptions in the same release record. Align
upgrades of the shared semantic schema and verifier through paired fixtures.
Record cold and cached compile/check time, peak memory and certificate bytes
for the paired fixture. Apply PC15's bounded replay and reuse checks so a
second frontend does not regenerate a shared theorem and a changed backend
rechecks its own edge without invalidating the high-level theorem.

The inspected Bend revision has an Apache-2.0 root license. Record each reused
component's license and notices with the pin. Keep any separately AGPL-licensed
Bend/HVM component identified with its own license and attribution. The first
adapter can use external processes and original fixtures without importing
Bend runtime code into AustralVM. [License at the inspected Bend pin](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/LICENSE).

**Exit.** A locked CI job builds both fixtures, runs them under SafestOS with
no BendRT installed in the execution environment, and replays the shared
proofs. Exercise the common
Cranelift fork/join under SP02/SP04 before advertising parallel support.
The CI job exercises a source change with fixed semantics,
a changed assumption, a changed backend, and a stale statement approval. Use
CPU-only fixtures on normal changes and named jobs for hardware and large Lean
exports. A release only calls a target end-to-end verified when source parsing
through final instructions and all dependencies required for the advertised
property satisfy its strict profile. Remote and asynchronous properties have
their own event-level profile.

## Executor order and checks

| Milestone | Start and acceptance |
| --- | --- |
| Shared generation | BI00 → BI01 → BI02. Two source files produce the same semantic clamp IR and one backend's generated program. |
| Shared proof | BI03 and BI04. The source-specific proof edges meet at one MM0 theorem. BI04 can start with B1 while the source adapters are built. |
| SafestOS and Cranelift | BI05 CPU path → SP00–SP02. Both sources execute under SafestOS, then compile and run the same bounded fork/join. SP03 measures optimization and SP04 composes the proofs. |
| Verified targets | BI05 → BI08 and SP05. Shared WGSL and pinned CPU-native cuts first, then device-specific instruction and host event cuts. |
| Shared libraries and richer claims | BI06 and BI07 use the available compiler and proof boundaries. BI09 can start in parallel once BI04 has the proof interface. |
| Integrated release | BI10 for each capability as it matures. A frontend switch preserves the tools, theorem and guarantees for the declared profile. |

**First patch for an executor.** Do BI00 only. Add the original Bend fixture,
the paired replay command, and a short result record linked to the existing
Austral clamp. Pin source hashes, tool versions and the bad comparison.
Record any missing runtime or tool as a named gap. Hand BI01 the exact current
KernelIR, memory-plan and PC04 evidence identifiers. Keep later cards out of
this patch.

Run Dune only in `nix develop "git+file://$PWD#ocaml"`. Use the prover shell for
Why3 and the version-matched Lean/export environments for Lean. For the current
GPU path use `cargo test --locked --manifest-path safestos/gpu/Cargo.toml` and
the `scripts/check-gpu source` profile. For a generated MM0 artifact compile
with `mm0-rs compile ARTIFACT.mm1 ARTIFACT.mmb`, then run `mm0-c ARTIFACT.mmb <
ARTIFACT.mm0`. Use the W3 staging flow for newly held-out statements and leave
approval to its owner. Reuse PC07's planned `scripts/proof-chain check` for the
integrated gate once available. Keep the actual commands, versions, output
identities, checked premises and mutation results with each completed card.
