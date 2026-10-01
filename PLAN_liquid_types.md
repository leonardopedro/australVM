# PLAN L — Liquid types for the Total Austral Subset ("Liquid Austral")

Parallel workstream, additive to `PLAN_parallel_australvm.md` (Plan B).
Companion docs: `../unfer/docs/WHYML_CYCLE.md` (the Why3 → compiler-extension
cycle this plan rides on), `../unfer/logos/src/austral_codegen/validate.rs`
(reference semantics of the Total Austral Subset), `../unfer/docs/ENG_PLAN.md`
(CoreIR `Fold`-only totality).

**Status at handover (2026-09-29)** — §8 is the execution checklist for the
next agent (file-level tasks, L1 surveyed to line numbers), §9 is the second
workstream (DeltaNet Engram, from `engram.md` + logos). Done so far: this
plan, `docs/LIQUID.md` (L0 spec), and the `LiquidPragma` carrier in
`lib/Common.ml`. Not done: everything else. Build note: `make bridge` first
(the root `libaustral_cranelift_bridge.so` is stale — AGENTS.md gotcha).

## Goal

Add **LiquidTypes** — refinement types with qualifier-based inference
(Rondon–Kawaguchi–Jhala style, not a full dependently typed calculus) — to the
extended Austral used here, **restricted to the Total Austral Subset**: the
fragment guaranteed to terminate (no direct/mutual recursion, `fold` over
linear lists and structural `match` as the only iteration). Refinements are
**ghost**: discharged at compile time by Why3 (subprocess only — the LGPL seam
of WHYML_CYCLE), then **erased** before monomorphization/CPS lowering, so the
frozen `uk_*`/`uz_*` ABI, the grant vocabulary, and the JIT are untouched.

Three properties make this project the right place for it:

1. **The fragment is total.** No recursion ⇒ no termination metrics, no
   fixpoint widening over recursive refinements; `fold` is structural over
   finite lists, so its invariant is a qualifier instantiation over the
   accumulator. Liquid inference is at its most tractable exactly here.
2. **Why3 is already the extension mechanism** (WHYML_CYCLE): the kernel emits
   `.mlw`, `why3 prove -P alt-ergo` discharges, `why3 extract` produces OCaml
   loaded through `Compiler_plugin`. Liquid verification is a second pass on
   the same seam — same toolchain, same license discipline, same golden-file
   pinning.
3. **The total subset is self-hosting material.** Because the fragment is
   strongly normalizing, its functions are *conservative definitional
   extensions* of any logic: they can extend Why3's theory environment and the
   liquid checker itself (extracted OCaml plugins), and they can serve as the
   index language of a dependent-type layer where typechecking is guaranteed
   to normalize. See §3.

## Why liquid types *here* (concrete payoff)

- **`uk_*` ABI contracts** (`examples/kernel/UnferKernel.aui`): status codes
  `result >= 0 || result = -code`; handle validity (`wrapModel` returns an
  `alive` handle); buffer/length pairs (`Address[Nat8]` + `Int64 len` with
  `len >= 0` and paired `data/len` invariants) — the C ABI's classic
  misuse class, statically excluded at the module boundary.
- **Probability bounds** (the kernel's domain): `uk_event_probability` results
  refined `0.0 <= p <= 1.0`.
- **Grant-set facts**: `import-set ⊆ grants` as a refinement-level predicate
  (the `authorize_gate` property, now reusable inside module proofs).
- **Linear protocols**: Austral's linear types (`type Model: Linear`) already
  give exactly-once consumption; refinements add ordering facts
  (`kernelEvolve` requires `alive(m)`, `freeModel` transitions `alive→freed`).

---

## 1. Design

### 1.1 The refinement language is the Total Austral Subset (TRL)

Define the **Total Refinement Logic** (TRL) as *the* refinement language:

- **Terms**: literals; primitive ops on machine ints; booleans; reals (for
  probability values, modelled as Why3 `real` with `0 ≤ p ≤ 1` at the ABI
  boundary); ADT **measures**; calls to **total refinement functions**.
- **Formulas**: quantifier-free + inductively declared predicates. No user
  quantifiers in v1 (qualifier templates may bind variables).
- **Totality gate**: a refinement function is admitted only if it passes the
  subset checker (stage L3) with `logos::austral_codegen::validate.rs`
  semantics — no call cycles (direct or mutual), `fold`/`match` only.
  Consequence: every TRL term is strongly normalizing, every definition is a
  conservative extension of the logic, and SMT encodings cannot diverge
  because of *our* definitions.

This is what ties LiquidTypes to "the subset guaranteed to terminate": the
subset is simultaneously (a) the programs that may carry refinements, and (b)
the language the refinements are written in.

### 1.2 Erasure principle

Refinement annotations live only in `pragma` strings and measure declarations.
They are checked and erased at the Stages→Mtast boundary. `Compiler_cps.ml`,
`CpsGen`, `safestos/cranelift`, and `module.toml` grants are **byte-identical
with and without refinements** (proof-by-test in L9). Zero runtime cost.

### 1.3 Linearity × refinements

v1: refinements are pure facts about values (arithmetic, measures, booleans).
Linear-state refinements (`alive`/`freed` ghost state on `Model`) are a thin
later add-on: the linear checker (`check_module_linearity`) already enforces
consumption; the refinement layer only adds *order*. Do not re-verify
linearity in the solver.

### 1.4 Machine-int overflow discipline

Two options: (a) SMT bitvectors; (b) Why3 `int` + explicit range obligations
(`-2^62 <= e < 2^62` per checked operation). **v1 = (b)** — alt-ergo is strong
at linear integer arithmetic and (b) matches the `unfer_ocaml.drv` precedent
(Why3 `int` → native OCaml `int`, no Zarith). Option (a) is the documented
fallback if (b)'s obligation noise becomes unmanageable.

---

## 2. Architecture

```
.aui/.aum  (pragma Liquid_* contracts + measure decls)
   │  existing parse/combine/desugar pipeline (pragmas already flow
   │  Cst → Combined → … → Linked → LFunction … * pragma list)
   ▼
TypingPass → Tast.typed_module
   ▼
[L] TotalityCheck   — subset gate for refinement-level code (L3)
   ▼
[L] LiquidPass      — judgment → Horn constraints → module.mlw (L4)
   ▼
why3 prove -P alt-ergo module.mlw     (SUBPROCESS — LGPL seam, L5)
   ▼
verdict: VerdictReject w/ blame spans   |   accept
   ▼
ERASE refinements → monomorphize → CPS IR → JIT (unchanged)
```

### 2.1 Surface syntax — the `pragma` seam

`lib/Lexer.mll` already lexes `pragma`; `lib/Parser.mly` builds
`make_pragma name args`; pragmas ride along in `pragma list` on every
function decl through all stages (`Stages.ml` `LFunction … * pragma list`),
and `TypingPass.ml` currently accepts exactly `ForeignImportPragma` /
`ForeignExportPragma` (anything else is `Errors.fun_invalid_pragmas ()`).
That is the established, upstream-compatible extension seam (real syntax,
cf. `examples/kernel/UnferKernel.aum` — pragmas precede the declaration,
named-argument style):

```austral
pragma Liquid_Requires(Contract => "0 <= len");
pragma Liquid_Ensures(Contract => "result >= 0 || result == -code");
pragma Foreign_Import(External_Name => "uk_init");
function kernelInit(cfg: Address[Nat8], len: Int64): Int64 is end;
```

- Contracts live in the **interface** (`.aui`); bodies inherit them.
- **Measures** are ordinary functions in the total subset, marked
  `pragma Liquid_Measure;`; `pragma Liquid_Invariant(Contract => "...")`
  carries fold invariants, `pragma Liquid_Trusted(Contract => "...")`
  assumptions for `@embed`/C-ABI glue. Full grammar and typing rules:
  `docs/LIQUID.md` (stage L0).
- New pragma variants are added in `Cst.ml`/`Stages.ml` and threaded through
  `TypingPass` (extend the pragma match to allow `Foreign_Import` +
  `Liquid_*` together). The string payload keeps the upstream grammar
  untouched; the DSL inside it is ours and versioned.
- Alternative considered and rejected: refinements inside docstrings (zero
  grammar churn, but no typed tooling, no span-accurate errors).

### 2.2 New code — `lib/liquid/`

Do **not** name anything `Qualifier*`: `lib/Qualifier.ml` is name
qualification. Layout:

| File | Role |
|------|------|
| `LiquidTypes.ml` | refinement AST (terms, formulas, qualifiers, measures) |
| `LiquidParse.ml` | pragma-string DSL → AST, span-accurate errors |
| `TotalityCheck.ml` | subset gate: call-graph cycles, `fold`/`match`-only (L3) |
| `LiquidConstraints.ml` | Tast → liquid judgments → Horn constraints (L4) |
| `LiquidWhy3.ml` | Horn → `.mlw` emitter + `why3 prove` subprocess driver (L5) |
| `LiquidInfer.ml` | qualifier fixed point / Houdini elimination (L6) |
| `lib/liquid/golden/` | pinned `.mlw` + extraction diffs (the `authorize_gate` discipline) |
| `lib/liquid/liquid.drv` | extraction driver (native ints, sibling of `unfer_ocaml.drv`) |

Hook point: `Compiler.compile_mod`, after `check_module_linearity typed`,
before `extract_bodies`/`monomorphize` (see §5 for the pre/post-mono decision).

### 2.3 Why3 seam (reuse the WhyML-cycle machinery)

- One `.mlw` per module; goldens pinned under `lib/liquid/golden/`.
- `why3 prove -P alt-ergo` as **subprocess** (`WHY3_CLI` override, then PATH)
  — the Cadabra2/engine-missing pattern: absent engine ⇒ skip with a
  structured warning, or hard error when `module.toml` says
  `liquid = "required"`.
- No Why3 library linkage in any Apache binary (LGPL discipline of
  WHYML_CYCLE §"License"); extracted OCaml is the user's own code.
- Failure reporting: `.mlw` goals carry the Austral span in comments;
  `LiquidWhy3` maps `why3 prove` failures back to source spans and produces a
  `VerdictReject`-style diagnostic.

### 2.4 The plugin seam

`Compiler_plugin` / `Vm_plugin` (register pass → `run_on_typed` →
`VerdictOk | VerdictReject`) are *specified* in WHYML_CYCLE §3–4 but are
**not yet present in this tree**. Two options:

- **(a), preferred**: land the seam as stage L1 and make the liquid pass its
  first consumer. This also un-blocks the WhyML cycle's authorization gate.
- (b) fallback: inline the pass in `compile_mod` and refactor onto the seam
  later.

### 2.5 Manifest + runtime integration (additive only)

- `module.toml` gains a `[verify]` section:
  `liquid = "off" | "optional" | "required"`, optional
  `qualifiers = [...]`. Additive to the frozen vocabulary (Plan B rules).
- `modhost`: a load-time attestation gate. A module with
  `liquid = "required"` carries a `liquid.ok` sidecar (hash of sources +
  generated `.mlw` + prover name/version + verdict). Tampering or stale
  attestations ⇒ refuse to host; hot-swap re-verifies if sources changed.
  Same posture as UK-4001: the compiler check becomes part of the kernel's
  capability story.
- First showcase: `examples/kernel/UnferKernel.aui` gets status-code and
  buffer/length contracts; `examples/modules/demo_hosted` gets one refined
  entrypoint plus a deliberately failing sibling.

---

## 3. The self-extension cycles (the point of the total subset)

**Cycle A — total Austral extends Why3.** `LiquidWhy3.emit_theory` translates
*total* Austral functions into Why3 `function`/`predicate` definitions in a
generated theory (measures, arithmetic lemmas, grant-lattice predicates in the
spirit of `authorize_gate`). Because the subset has no recursion and only
structural `fold`, the translation is a **conservative definitional
extension** — no well-foundedness proofs required, so the translation is
trustworthy by construction (its correctness obligation reduces to
type-preserving syntactic mapping, covered by golden diffs). The liquid VCs of
stage L4 are then discharged *in the extended theory*.

**Cycle B — total Austral extends the liquid checker itself.** Verified code
written in the subset — qualifier libraries, measure tables, VC simplifier
rules, abstract domains — can be extracted to OCaml
(`why3 extract -D liquid.drv`, pinned-extraction discipline) and loaded
through `Compiler_plugin.register` as a plugin module. The checker is extended
by code from the very fragment it checks, and that code carries its own liquid
contracts (checked before extraction).

**Cycle C — dependent types of the total fragment.** Index-carrying types
(`Span[Nat8, n]`, `Vector[T, n]`, sized lists via measures) where indices are
*TRL terms*. Totality guarantees type-level normalization: index equality is
decidable and elaboration terminates. Refinements and indices share one logic
and one solver; indices are a syntactic discipline over the same TRL, not a
second system. This is the "LiquidTypes/Dependent Types of AustralVM itself
(in the subset guaranteed to terminate)" story: the type-level language cannot
diverge because the fragment cannot.

Bootstrap (optional): `TotalityCheck` itself is small enough to carry liquid
contracts and be checked by the pipeline (dogfooding).

---

## 4. Stages

Sizes: S ≤ 1 day, M ≤ 3 days, L > 3 days. Stages ordered small → large; do not
skip ahead. Acceptance commands run from the repo root.

| Stage | Summary | Size |
|-------|---------|------|
| L0 | TRL spec + pinned Why3/alt-ergo toolchain | S |
| L1 | Plugin seam (`Compiler_plugin`/`Vm_plugin`) + no-op liquid pass | S |
| L2 | `Liquid_*` pragma syntax + DSL parser (per `docs/LIQUID.md` §3) | S |
| L3 | Totality gate (`TotalityCheck.ml`) | M |
| L4 | Constraint generation + golden `.mlw` | M |
| L5 | Why3 subprocess driver + blame mapping | M |
| L6 | Qualifier inference (Houdini/Horn fixed point) | L |
| L7 | `uk_*` contract library + example modules | M |
| L8 | `[verify]` manifest + `modhost` attestation gate | M |
| L9 | Dependent index layer (Cycle C) + erasure test | L |
| L10 | Cycles A/B: theory emission + extracted checker plugin | L |

### L0 — Spec + toolchain (S)
Write `docs/LIQUID.md`: TRL grammar, the liquid judgment and its typing rules
(let, if, match, application, `fold`, foreign-call-with-contract), erasure
semantics, overflow discipline (§1.4). Add `why3` + `alt-ergo` to the nix dev
shell (the flake already provisions cadabra2/GHC — same pattern).
**Acceptance**: `nix develop -c why3 prove -P alt-ergo <hello-goal.mlw>` exits 0;
`docs/LIQUID.md` reviewed against `validate.rs` semantics.

### L1 — Plugin seam (S)
Implement `lib/Compiler_plugin.ml(mli)` and `lib/Vm_plugin.ml(mli)` exactly as
specified in WHYML_CYCLE §3–4 (`register`, `run_on_typed`, `VerdictReject`
aborts compilation; `CliEngine` routes through the registry). Register a
no-op `liquid` pass from `empty_compiler`.
**Acceptance**: `dune build lib/ bin/ test/` green; a test pass that rejects a
`pragma Liquid_Ensures(Contract => "false")` decl proves the verdict path
end-to-end.

### L2 — Surface syntax (S)
Add the `Liquid_*` pragma variants to `Cst.ml`/`Stages.ml` per
`docs/LIQUID.md` §3; extend the `TypingPass` pragma match (allow
`Foreign_Import` + `Liquid_*` combinations); implement `LiquidParse` (the
TRL string DSL, `docs/LIQUID.md` §2) with source-span errors.
**Acceptance**: new suite `test-programs/suites/020-liquid-syntax/` passes;
malformed DSL reports the pragma's span, not a parse crash.

### L3 — Totality gate (M)
`TotalityCheck.ml`: build the refinement-level call graph (reuse the
`TailCallAnalysis` traversal style), reject direct/mutual cycles naming the
cycle in the diagnostic, allow `fold`/`match` only — the
`validate.rs` semantics, ported to the OCaml side.
**Acceptance**: `dune runtest` includes cycle-rejection and fold-acceptance
cases; a mutual-recursion fixture errors with both function names.

### L4 — Constraint generation (M)
`LiquidConstraints.ml`: implement the liquid judgment over `Stages.Tast`
(pre-monomorphization; see §5) for the total fragment; emit Horn constraints
as one `.mlw` per module (environment as hypotheses, one goal per obligation).
Pin goldens under `lib/liquid/golden/`.
**Acceptance**: golden `.mlw` diffs stable across runs; `demo_hosted`'s module
generates a well-formed `.mlw` (checked by `why3 prove` when present, by
golden diff when not).

### L5 — Why3 driver (M)
`LiquidWhy3.ml`: emit, run `why3 prove -P alt-ergo` (subprocess, `WHY3_CLI`
override), parse verdicts, map failures to Austral spans, produce
`VerdictReject` diagnostics; `liquid.drv` for the extraction path; skip with
structured warning when the engine is absent (hard error under
`liquid = "required"`).
**Acceptance**: a failing obligation blames the exact annotation line; with
Why3 absent the suite skips cleanly (`dune runtest` green either way).

### L6 — Inference (L)
`LiquidInfer.ml`: qualifier vocabulary (templates over the typing
environment), Houdini-style elimination over candidate qualifiers, Horn
solving via L5; default measures for stdlib ADTs; `fold` invariants as
qualifier instantiations over the accumulator type.
**Acceptance**: a list-sum example infers `result >= 0`-style bounds with **no
annotations**; an under-specified fold fails with a named missing invariant.

### L7 — Contract library + examples (M)
Contracts for `examples/kernel/UnferKernel.aui` and
`examples/zenodo/ZenodoStore.aui` (status codes, buffer/len pairs, probability
bounds, `Model` liveness v1); one refined entrypoint in
`examples/modules/demo_hosted` + a negative sibling.
**Acceptance**: `LD_LIBRARY_PATH=. dune runtest` + the demo `run_demo.sh`
paths show a contract violation rejected **before** JIT compilation.

### L8 — Manifest + attestation (M)
`[verify]` section in `module.toml` (additive); `modhost` load-time gate with
the `liquid.ok` sidecar (source hash + `.mlw` hash + prover version +
verdict); hot-swap re-verifies changed sources.
**Acceptance**: `cargo test` in `safestos/cranelift` covers: tampered sidecar
refused, stale attestation refused on swap, `off` modules unaffected.

### L9 — Dependent index layer (L)
Index-carrying types over TRL terms (`Span[Nat8, n]`, `Vector[T, n]`);
elaboration + erasure; interaction with monomorphization.
**Acceptance**: sized-buffer misuse is rejected at compile time;
`--emit-cps` output is **byte-identical** with and without refinements
(erasure proof-by-test).

### L10 — Cycles A/B (L, optional)
`emit_theory` (total Austral → Why3 theory) feeding L4 goldens; one extracted
OCaml plugin (e.g. a qualifier library or measure table) written in the total
subset, liquid-checked, extracted, loaded through `Compiler_plugin`.
**Acceptance**: an L4 golden proves *in the extended theory*; the extracted
plugin measurably changes inference results and is covered by a test.

---

## 5. Decisions & risks

| # | Decision | Recommendation | Risk if wrong |
|---|----------|----------------|---------------|
| 1 | Check pre- or post-monomorphization | Pre-mono for contracts that don't mention type params; post-mono instantiation check for refined generics | Refined generics re-checked at every instantiation (cost) |
| 2 | Solver: Why3+alt-ergo vs direct Z3 | Why3+alt-ergo (toolchain & license seam consistency); `LiquidWhy3` hides the backend | Bitvector-heavy obligations may need (§1.4a) fallback |
| 3 | Probability as `real` | Why3 `real` + `0≤p≤1` refinements at ABI boundaries | Mixed int/real obligations; alt-ergo nlin can be slow — keep products rare |
| 4 | Plugin seam first vs inline | Seam first (L1) — un-blocks WHYML_CYCLE too | If the seam design changes, L1 rework |
| 5 | `while` loops in full Austral | Outside the subset: unchecked unless `liquid = "required"` (then rejected as non-total) | User surprise — document prominently in `docs/LIQUID.md` |

**Frozen contract respect** (Plan B rules): no signature changes to
`uk_*`/`uz_*`; `module.toml` vocabulary is additive-only (`[verify]`); new
UK-#### codes only additively (via `[SYNC]`). Erasure keeps `CpsGen` output and
the JIT untouched.

**TCB**: Why3 + alt-ergo + the Why3 extractor, `LiquidWhy3`/`LiquidConstraints`/
`TotalityCheck`. Mitigations: pinned goldens (byte-diff of `.mlw` and
extraction), differential testing (inference vs. checked contracts), and —
optional — Lean4 cross-check of the hard lemmas through unfer's
`prob_kernel::verify` (S29 `lean4export` path).

## 6. Testing strategy

- **ounit2**: `test/LiquidTest.ml` — DSL parse, totality gate, constraint-gen
  goldens (no engine needed).
- **e2e**: `test-programs/suites/020-liquid/` positive + expected-failure
  programs through `test-programs/runner.py`.
- **Golden discipline**: `.mlw` and extraction diffs pinned under
  `lib/liquid/golden/` (the `authorize_gate` pattern).
- **CI**: default job runs engine-absent (skip pattern, like cadabra2 tests);
  one optional job with `why3`+`alt-ergo` from the nix shell proving the
  goldens.

## 7. `[SYNC]` steps (cross-repo, do only these)

1. unfer `docs/WHYML_CYCLE.md` "Extensions": note the liquid template family
   (Horn-VC emission) as a `WhymlOp` variant, if kernel-side emission is
   adopted over compiler-side emission.
2. `../unfer/docs/MODULES.md` §6: note the `liquid.ok` attestation alongside
   the manifest-grant gate.
3. `unfer_protocol::codes`: new UK-#### codes for liquid engine-missing /
   attestation-refused (additive; regenerate `EXPECTED_SYMBOLS.txt` only if a
   new `uk_*` is added — expected: none).

---

## 8. Handover — detailed task breakdown (stages L1–L10)

> For the next agent: this is the execution checklist. Stage intent is §4;
> typing rules and syntax are normative in `docs/LIQUID.md`. Tick boxes as
> you go and keep this section current.

### Current state (2026-09-29, master)

- [x] `PLAN_liquid_types.md` + `docs/LIQUID.md` (L0 spec) written.
- [x] `lib/Common.ml`: `pragma` extended with `LiquidPragma of string * string`
      (kind, contract string). Landed at L0, consumed from L1 on.
- [x] Everything else in **L1** (§8 below) — landed and verified 2026-10-01.

**L1 corrected the "not done" column above.** When L1 was picked up, the
plugin-seam steps were *already committed* and predate this plan:
`d87049ed` (2026-08-23, `Vm_plugin.boot`, Why3 gate + `deltanet_unf` passes)
and `b6c41c84` (2026-08-27, NPU DMA gate). So L1.6/L1.7/L1.9/L1.10/L1.11
were pre-existing, not new work. What L1 actually added: the `Liquid_*`
pragma recognition, pragma threading to the typed AST, a **second pass kind**
(`typed_pass`) beside the 3-arg `gate_pass`, the third plugin-gate site on
the hot-swap path, and the liquid acceptance tests. See the L1 section for
the two places the plan's text was wrong about the existing code.

Build note: the toolchain is the repo's own `flake.nix` devShell
(`nix develop`) — no opam needed. It resolves OCaml 5.4.1 + dune 3.23.1 and
all six OCaml libraries; `make bridge` must run first because the checked-in
`libaustral_cranelift_bridge.so` goes stale. Verification loop:
`make bridge && dune build lib/ bin/ test/ && LD_LIBRARY_PATH=$HOME/.local/lib
dune runtest && LD_LIBRARY_PATH=$HOME/.local/lib python3
test-programs/runner.py` (the `LD_LIBRARY_PATH` is needed for both the
runtest and the runner: the runner shells out to `./austral`, which dlopens
the bridge).

Build/preconditions: `make bridge` (or `run-tests.sh`, which does it) before
`dune build lib/ bin/ test/` — the checked-in `libaustral_cranelift_bridge.so`
is a deliberate release artifact that goes stale whenever the bridge changes
(AGENTS.md gotcha). Verification loop:
`dune build lib/ bin/ test/ && LD_LIBRARY_PATH=. dune runtest && python3
 test-programs/runner.py`.

### L1 — plugin seam (current task; surveyed to file level)

> Note: minimal `Liquid_*` **pragma recognition** is pulled up from L2 into
> L1 because the L1 acceptance criterion (a test pass rejecting
> `pragma Liquid_Ensures(Contract => "false")`) cannot be met without it.
> L2 keeps the TRL DSL *parser* and the interface/body merge rules.
> Each checkbox below ≙ one step of the in-flight todo list when work paused.

- [x] **`lib/CstUtil.ml`** — extend `make_pragma` (~line 99): before the
      trailing `Errors.unknown_pragma s`, add branches for `Liquid_Requires`,
      `Liquid_Ensures`, `Liquid_Invariant`, `Liquid_Trusted` — argument shape
      `ConcreteNamedArgs [(a, CStringConstant (_, c))]` with `a = Contract`,
      → `LiquidPragma (kind, c)` — and `Liquid_Measure`, `Liquid_Fold` —
      `ConcretePositionalArgs []` → `LiquidPragma (kind, "")`. Suggested
      helper `make_liquid_pragma` with `kind = String.sub s 7 (…)`, invalid
      shape → the local `module Errors` (line 13) `pragma_argument_error`.
- [x] **`lib/Stages.ml`** — `Tast`: append `* pragma list` to `TFunction`
      (8→9 components) and `TForeignFunction` (7→8). Pragmas already flow
      through `Combined.CFunction` / `Linked.LFunction`; this completes the
      threading to the typed level (required by the acceptance test and by
      L4 contract lookup).
- [x] **Arity fixes** — the only match sites: `lib/BodyExtractionPass.ml:16`,
      `lib/LinearityCheck.ml:766`, `lib/Monomorphize.ml:456`,
      `lib/Monomorphize.ml:469` (one extra `_` each; Monomorphize drops the
      pragmas = the §10 erasure point).
- [x] **`lib/TypingPass.ml`** (~lines 612–635) — replace the exact-list
      pragma match with partitioning: `List.find_map` for
      `ForeignImportPragma`/`ForeignExportPragma`; count-validate (≤1 each,
      never both, no `UnsafeModulePragma` at function level, no unknown
      combinations → `Errors.fun_invalid_pragmas ()`); construct
      `TForeignFunction (…, s, doc, pragmas)` / `TFunction (…, body', doc,
      pragmas)`. `Foreign_Import` + `Liquid_*` **must coexist** (L7 puts
      contracts on foreign decls).
- [x] **`lib/ExtractionPass.ml`** (~lines 386–393) — same partitioning for
      `external_name`/`export_name`: today the exact match `[ForeignImportPragma
      s]` means any additional pragma silently disables foreign-import
      detection.
- [x] **`lib/Compiler_plugin.mli` + `.ml`**: *pre-existing since `d87049ed`;
        L1 extended it rather than creating it.* Note the plan's 2-argument
        `gate_pass` was **not** adopted: the committed `register` is
        3-argument (it also takes `constants`, which the deltanet UNF gate
        needs) and is used by `Why3_plugin`, `Deltanet_plugin` and
        `Npu_dma_plugin`. Narrowing it would have broken all three, so
        `typed_pass` was added *alongside* it instead.
      - `type verdict = VerdictOk | VerdictReject of string`
      - `type gate_pass = module_name:string -> foreign_externals:string list
        -> constants:(Identifier.identifier * Stages.Tast.texpr) list ->
        verdict` — the committed WHYML_CYCLE §3 signature, kept.
      - `type typed_pass = Stages.Tast.typed_module -> verdict` — documented
        extension: the acceptance test and the liquid pass must inspect decl
        pragmas, which the gate signature cannot see. Register both kinds in
        one ordered registry; `run_on_typed` runs all, first
        `VerdictReject` wins.
      - `register : name:string -> gate_pass -> unit`,
        `register_typed : name:string -> typed_pass -> unit` (replace-by-name
        ⇒ idempotent), `run_on_typed : Tast.typed_module -> verdict`
        (extracts module name via `Identifier.mod_name_string`, foreign
        externals from `TForeignFunction` decls), plus `unregister`, `names`,
        `reset` for tests.
      - Dependencies: `Stages` + `Identifier` only — keep in `austral_core`,
        never reference `Compiler` (cycle risk).
- [x] **`lib/Vm_plugin.mli` + `.ml`** — *pre-existing since `d87049ed`;
        L1 only added the `liquid` tenant to `boot`.* WHYML_CYCLE
      §4 verbatim: `type compiler_service = { name : string; compile :
      Compiler.module_source list -> Compiler.compiler }`,
      `register_compiler` (replace-by-name), `run_compiler` (boot, then
      dispatch through the `austral-builtin` entry so it is swappable),
      `list_compilers`, `boot` (idempotent: register `austral-builtin` =
      `fun mods -> Compiler.compile_multiple Compiler.empty_compiler mods`
      and install the no-op liquid pass
      `Compiler_plugin.register_typed ~name:"liquid" (fun _ -> VerdictOk)`).
- [x] **`lib/Compiler.ml`** — local `module Errors` with `plugin_rejected`
      (`austral_raise DeclarationError [Text "Module rejected by a compiler
      plugin: "; Text msg]`; `open Error` is already present, `Text` is
      `ErrorText.Text`). Helper `check_plugin_verdict : typed_module -> unit`
      → calls `Compiler_plugin.run_on_typed`. Insert `let () =
      check_plugin_verdict typed in` immediately after `check_module_linearity
      typed` at **three** sites: `compile_mod` (~line 121) and both compile
      loops inside `cps_jit_swap_modules` (~lines 215, 237) — the hot-swap
      path must re-verify.
- [x] **`lib/Cli.ml`** — `main'` calls `Vm_plugin.boot ()` before `exec cmd`
      ("the application boots by loading its plugins").
- [x] **`lib/CliEngine.ml`** — route all three `compile_multiple
      empty_compiler mods` sites (`exec_target` TypeCheck branch,
      `exec_compile_to_bin`, `exec_compile_to_c`) through
      `Vm_plugin.run_compiler mods`.
- [x] **`lib/dune`** — module lists are explicit: add `Compiler_plugin` to
      `austral_core`'s `(modules …)`, `Vm_plugin` to `austral_lib`'s.
- [x] **`test/PluginTest.ml`** (new) + `test/dune` stanza
      `(tests (names PluginTest) (libraries austral_lib ounit2))`:
      1. *(acceptance)* typed pass rejecting `TFunction` decls carrying
         `LiquidPragma ("Ensures", "false")`; compile a body-only module via
         `Vm_plugin.run_compiler`; expect `Austral_error` whose
         `render_error_to_plain` contains the rejection text; then
         `unregister` and recompile → success. Fixture (real syntax —
         pragmas *precede* the decl, body terminator `end module body.`):
         ```austral
         module body PluginTest is
             pragma Liquid_Ensures(Contract => "false");
             function f(): Int64 is
                 return 0;
             end;
         end module body.
         ```
      2. gate pass observes `module_name = "PluginTest"` and the
         `foreign_externals` list (add a `pragma Unsafe_Module;` at file top
         + one `pragma Foreign_Import(External_Name => "uk_version");`
         decl to see a foreign external).
      3. registry semantics: replace-by-name, `unregister`, `names`, `reset`.
      4. `Vm_plugin`: double `boot ()` idempotent; wrapping
         `austral-builtin` in a counting delegate proves `run_compiler`
         routes through the registry; `list_compilers` lists it.
      Always `Compiler_plugin.reset`/`unregister` in teardown — registries
      are global and ounit runs in one process.
- [x] **Acceptance**: `dune build lib/ bin/ test/` green (after
      `make bridge`); `LD_LIBRARY_PATH=. dune runtest` green;
      `python3 test-programs/runner.py` unaffected.

### L2 — surface syntax (`docs/LIQUID.md` §3)

- [x] `LiquidParse` (in `lib/liquid/`): TRL string DSL → refinement AST
      (`LiquidTypes`), span-accurate errors *into* the pragma string
      (grammar: `docs/LIQUID.md` §2.2–2.3).
- [x] Interface/body contract merge rules (`Combined`/`SmallCombined`):
      interface is the contract surface; a body contract must not weaken the
      interface one — fail on mismatch (decision recorded in `docs/LIQUID.md`
      §3 once implemented).
- [x] `test-programs/suites/020-liquid-syntax/` — parse + negative cases.

### L3 — totality gate

- [ ] `TotalityCheck.ml`: refinement-level call graph (reuse the
      `TailCallAnalysis`/`TailCallUtil` traversal style), reject
      direct/mutual cycles *naming the cycle path* (mirror
      `../unfer/logos/src/austral_codegen/validate.rs::check_cycles`),
      `fold`/`match` only; enforce T1–T5 of `docs/LIQUID.md` §1.
- [ ] Tests: mutual-recursion fixture errors with both function names;
      fold-only recursion accepted; `while`/`for` → error under
      `required`, untracked otherwise.

### L4 — constraint generation

- [ ] `LiquidConstraints.ml`: liquid judgments (`docs/LIQUID.md` §5) over
      `Stages.Tast` → Horn constraints → one `.mlw` per module, goals named
      `g_<span-hash>`; measure unfolding equations as explicit hypotheses
      (`docs/LIQUID.md` §4.3).
- [ ] `lib/liquid/golden/` pinned `.mlw` (the `authorize_gate` discipline).
- [ ] Pre- vs post-monomorphization checking per §5 decision row 1.

### L5 — Why3 driver

- [ ] `LiquidWhy3.ml` + `lib/liquid/liquid.drv`: emit; `why3 prove -P
      alt-ergo` subprocess (`WHY3_CLI` override then PATH); verdict→span
      mapping; engine-missing = structured skip, hard error under
      `liquid = "required"`; `Unknown`/`Timeout` = failure.
- [ ] Tests: failing obligation blames the exact annotation line; suite
      green with and without the engine.

### L6 — inference

- [ ] `LiquidInfer.ml`: qualifier templates over Γ, Houdini-style
      elimination, Horn fixed point (finite lattice — no widening, the
      fragment is recursion-free); fold invariants as accumulator
      qualifiers; default measures (`length`, record projections, `Prob`
      bounds).

### L7 — contract library

- [ ] `examples/kernel/UnferKernel.aui` contracts against `unfer_ffi`
      semantics (status `result >= 0 || result == -code`, buffer/len pairs,
      probability bounds); `examples/zenodo/ZenodoStore.aui` similarly.
- [ ] Refined entrypoint in `examples/modules/demo_hosted` + a deliberately
      failing sibling for the negative test.

### L8 — manifest + attestation

- [ ] `[verify]` in `module.toml` (`liquid = "off"|"optional"|"required"`,
      `qualifiers = [...]`); `liquid.ok` sidecar (source hash + `.mlw` hash
      + prover name/version + verdict + trusted-contract count);
      `modhost` load gate + hot-swap re-verify on changed sources.
- [ ] Tests in `safestos/cranelift/tests/` (tampered/stale sidecar refused;
      `off` modules unaffected).

### L9 — dependent index layer

- [ ] Index-carrying types over TRL terms (`Span[Nat8, n]`, `Vector[T, n]`)
      + erasure; `--emit-cps` byte-identity test with/without refinements.

### L10 — cycles A/B

- [ ] `emit_theory`: total Austral → Why3 `function`/`predicate` theory fed
      into L4 goldens (only `fold` translates recursively, structurally —
      conservative by T1/T2).
- [ ] One extracted OCaml plugin (qualifier library or measure table) written
      in the total subset, liquid-checked, extracted via `liquid.drv`,
      loaded through `Compiler_plugin.register_typed`.

### Tenants of the same seam (do not collide)

`LOGOS.md` already specifies `australVM/lib/deltanet_plugin.ml` (registered in
`Vm_plugin.boot` beside the Why3 gate) and WHYML_CYCLE specifies
`lib/why3_plugin/`. All are pass plugins on the L1 seam — keep pass names
distinct (`liquid`, `why3_gate`, `deltanet`) and registration inside
`Vm_plugin.boot` idempotent.

---

## 9. Workstream E — DeltaNet Engram (handover task breakdown)

> **Goal**: an *engram* for LLM input processing (decoder side), analogous to
> the one in `engram.md` (DeepSeek, arXiv:2601.07372 — conditional memory:
> hashed suffix N-gram keys → O(1) static-embedding lookup → context-aware
> gating → residual fusion), but keyed by **logos**' DeltaNet pipeline
> (`../unfer/logos` + its australVM integration points): every text fragment
> is compiled to an interaction net, reduced to its **unique normal form**,
> and addressed by content — normal forms, not surface N-grams.

### E0 — design summary (to pin in `docs/ENGRAM.md`, stage E1)

**E0.1 The lookup module is the paper's; only the key changes.** Keep Engram
§2.3–2.5 as-is: context-aware gating (Eq. 3–5: RMSNorm'd dot-product gate
α_t ∈ (0,1), depthwise causal conv kernel 4, residual add), multi-branch
parameter sharing (§2.4), deterministic-ID prefetch + multi-level cache
(§2.5). Replace §2.2 (tokenizer compression + multi-head XOR hashing) with
canonical normal-form keys. The paper's ablation (§6.2) says gating and
multi-branch matter — do not touch them in v1.

**E0.2 Keys — "structural N-grams".** For decode position `t` and
granularity `g`:

| g | segment | analogue |
|---|---------|----------|
| `window` | token window of order n = 2..N (paper baseline) | `E_{n,k}` |
| `subderiv` | every CCG sub-derivation → its own CoreIR → net → UNF | compositional n-gram |
| `sentence` | whole fragment → reduced net → UNF | whole-pattern |

`EngramKey = { unf_hash: [u8;32], ted_hash: [u8;32], granularity, depth,
l1_weight: Option<f64> }`. Lookup `E_g[unf_hash]` is O(1) and *deterministic*
— the paper's prefetch story gets stronger: keys are computable at corpus
ingest time, before any decode step. `window` keys remain as (a) the ablation
axis and (b) the fallback for input outside the CNL fragment.

**E0.3 "Unique normal form up to accounted isomorphisms" — exactly what is
already quotiented by logos:**
1. **reduction-order independence** — confluence of net reduction,
   machine-checked in `unfer/logos`' `lean/Confluence.lean` (diamond,
   Church–Rosser, UNF uniqueness; exported via lean4export and re-checked in
   nanoda), and corroborated at runtime by the double-reduction
   self-check (`verified` in the `uk_logos_compile` report);
2. **node-index / α-isomorphism** — `deltanet::unf::canonical_serialize`
   walks the rooted port structure and never emits node indices, so
   isomorphic wirings serialize to identical bytes;
3. **algebraic isomorphism** — `deltanet::ted` canonicalizes the Int64
   fragment to sorted polynomials over ℤ/2⁶⁴ (commutativity, distribution,
   like terms), so `ted_hash` merges e.g. `x + 1` with `1 + x`;
4. **not accounted (out of scope)**: graph isomorphism of *unreduced* nets —
   keys are always computed on reduced normal forms.

**E0.4 What DeltaNet keys buy (hypotheses, to be measured in E3/E4):**
- *semantic dedup*: surface variants share one table slot → more coverage per
  table-parameter at iso-budget; expected to move the paper's U-curve
  optimum (§3.1: ρ ≈ 75–80% of sparse budget to MoE) — measure, don't assume;
- *compositional generalization*: `subderiv` keys fire across paraphrases;
- *verified memory*: entries are content-addressed and confluence-checked;
- *probabilistic engrams*: logos' `l1` world-splitting attaches weights
  ("probably P") to keys — the paper has no analogue;
- *denotational nuance* (document in ENGRAM.md): keys identify *normal
  forms*, so `John adds two three` and `Bob adds three two` both reduce to
  `5` and collide at `sentence`/`subderiv` granularity. That is the intended
  denotational quotient; use `window` keys or derivational metadata when
  surface identity must be preserved.

**E0.5 Where the code lives** (parallel-execution rules apply):
- `../unfer/logos/src/engram/` (new module) — key derivation, segmentation,
  table, ingest. **Cross-repo: unfer/Plan-A owned, or explicit `[SYNC]`**.
- `./` (australVM) — runtime integration only: `uk_engram_*` symbols +
  `safestos/cranelift/src/lib.rs` `UNFER_SYMBOLS` + `symbol_sync` test +
  Austral bindings in `examples/kernel/`; optionally the `deltanet_plugin`
  pass on the L1 seam (§8).
- Decoder-side reference module: port the key function into the open-source
  `github.com/deepseek-ai/Engram` harness (external repo; pin a rev).

### Stages

| Stage | Summary | Size | Repo |
|-------|---------|------|------|
| E1 | `docs/ENGRAM.md` spec (pin E0) + golden key corpus | S | this + [SYNC] |
| E2 | `logos::engram` key derivation + segmentation | M | unfer [SYNC] |
| E3 | Content-addressed engram table + corpus ingest + stats | M | unfer [SYNC] |
| E4 | Decoder reference module + N-gram-vs-UNF ablation | L | external harness |
| E5 | L1 probabilistic engrams | S | unfer [SYNC] |
| E6 | Kernel/runtime integration (`uk_engram_*`, examples) | M | this repo |
| E7 | Prefetch/offload + cache tiers (paper §2.5) | L | unfer + this |
| E8 | `deltanet_plugin` pass on the L1 seam (optional) | M | this repo |

- [ ] **E1 (S)** — `docs/ENGRAM.md`: pin E0.1–E0.4; define the `EngramKey`
      byte-layout (versioned, `unf_hash` first = stable addressing), the
      granularity lattice, the fallback rule (fragment unparseable by
      `harper_gate`/CCG → `window` key; keep the parse-rate metric), and the
      ingest/lookup API. Golden corpus: ≥30 groups pinned as
      `corpus/engram_keys.tsv` — paraphrase/algebraic variants expected to
      collide (per E0.4's nuance), near-misses expected **not** to collide.
- [ ] **E2 (M)** [`../unfer/logos`, `[SYNC]`] — `src/engram/{mod,key,
      segment}.rs`: `engram_key(fragment) -> EngramKey` =
      `harper_gate::lint` → `ccg::parse_sentence` →
      `core_ir::compile_to_core_ir` → `core_ir::linearity::insert_linearity`
      → `deltanet::compile_to_net` → `reducer::reduce` (≤1M iters; stuck →
      fallback key) → `unf::unf_hash` + TED canonicalization + `ted_hash`
      (mirror `logos::translate` steps 3–5 and `uk_logos_compile`'s report
      fields). Segmentation: sliding windows n = 2..N (N = 3 default, paper
      §4.1), CCG sub-derivations (walk `ccg::DerivationTree`), whole
      sentence. Property tests: determinism (intensional equivalence,
      double-run identical), collision-on-equivalence, discrimination,
      α-renaming invariance.
- [ ] **E3 (M)** [`[SYNC]`] — `src/engram/table.rs`: `EngramTable` =
      per-granularity `HashMap<EngramKey, Embedding>` (drop multi-head
      hashing; keep prime-sized bucketing only if the memory layout needs
      it); corpus ingest (segments → keys → store); stats: dedup ratio per
      granularity, TED-merge rate (algebraic collisions), lookup
      p50/p99. Acceptance: ingest the L0 `corpus/` plus a larger raw corpus;
      dedup > 0 on paraphrase sets; O(1) lookup benchmark recorded.
- [ ] **E4 (L)** [external] — decoder module: port the key function into the
      `deepseek-ai/Engram` reference implementation (pin rev); train small
      with (a) surface N-gram keys vs (b) UNF keys at iso-table-budget and
      iso-FLOPs. Metrics: val loss, long-context RULER subset,
      table-coverage curves; re-run the sparsity-allocation sweep (paper
      §3.1) to see where ρ* moves under semantic dedup. Acceptance:
      reproducible run log + ablation table appended to `docs/ENGRAM.md`.
- [ ] **E5 (S)** [`[SYNC]`] — probabilistic engrams: `l1::split_l1` worlds →
      weighted key sets, aggregated with `l1::aggregate_results`
      (sum-preserving). Acceptance: `probably John loves Mary` → weighted key
      set summing to 1.0; lookup returns the distribution.
- [ ] **E6 (M)** [this repo + `[SYNC]`] — `uk_engram_lookup` /
      `uk_engram_store` over the unfer C ABI, following the S29 registration
      checklist (unfer: `unfer_protocol/src/symbols.rs`,
      `scripts/gen_symbol_artifacts`, `EXPECTED_SYMBOLS.txt`, generated
      header; then here: `UNFER_SYMBOLS` table + `symbol_sync` test +
      `GrantSet.kernel`) — additive-only per the frozen-contract rules.
      Austral bindings `examples/kernel/EngramKernel.aui/.aum` mirroring
      `UnferKernel.aui`'s buffer protocol (`Address[Nat8]` + `Int64` len);
      demo module storing + re-looking-up a sentence engram through the JIT.
      These contracts are among the first L7 liquid contracts.
- [ ] **E7 (L)** — prefetch/offload: ingest-time key computation →
      host/SSD cache tiers (paper §2.5); optional VM tier
      (`cloud_hypervisor_vm/`). Acceptance: offload overhead target < 3%
      (the paper's number) on a synthetic 100M-entry table; prefetch hit rate
      reported.
- [x] **E8 (M, optional)** [this repo] — `lib/deltanet_plugin.ml` registered
      in `Vm_plugin.boot` beside the `liquid`/`why3_gate` passes (L1 seam,
      §8): serialize top-level constant expressions to the subset, call
      `uk_austral_unf` through the rust bridge, reject the module when the
      kernel's UNF disagrees with the compiler's evaluation (LOGOS.md's
      "unique normal form as independent arbiter"); compiled module
      constants then carry engram keys for free. Acceptance: mismatching
      constant → compile rejection; pass name `deltanet` distinct from the
      other tenants.
      **Already landed** — `lib/deltanet_plugin.ml` + `test/DeltanetPluginTest.ml`
      are tracked and committed since `d87049ed` (2026-08-23), i.e. *before*
      this plan was written (2026-09-29). The `vm_test` suite reports
      `boot ok (passes: why3_gate,deltanet_unf,npu_dma_gate)` and
      `DeltanetPluginTest` passes. Only the plan's tick was missing.

### Open questions (record decisions in `docs/ENGRAM.md`)

1. Reduction cost at decode time vs the paper's strict O(1) budget —
   mitigation: fragment→key memo cache (UNF is deterministic); measure the
   hot `window` path separately in E3.
2. CNL coverage: L0 grammar + 46-entry lexicon is small; fallback rate and
   its decay as the lexicon grows should be tracked alongside dedup.
3. F64 vs Int64 hash distinctly (`unf` byte-tags them) — keep; document.
4. Conv semantics (Eq. 5, dilation δ = max N-gram order) need a granularity
   analogue once `subderiv`/`sentence` keys replace ordered N-grams; zero-init
   conv keeps the identity at start (paper §4.1) — preserve that.
