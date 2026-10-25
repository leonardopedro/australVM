# Whole-Terrain Verification in australVM — Adapted Plan

> **Adapted 2026-10-25.** The generic proposal this file used to hold planned
> five *new* components (~8,000 LOC: an Austral-language DeltaNets engine, an
> Austral Egison matcher compiler, liquid-type enhancements, an `austral.mm0`,
> a synthesizer). Four of the five already exist in this tree in a different —
> and better integrated — form, so this document is now a **map of the real
> pipeline plus the gaps that are actually open**, under the standing rule:
>
> **Improving an existing feature outranks adding a new one.** Where the
> generic plan says "build X", this document says where X already lives, and
> what is genuinely missing inside it.

Companion documents: `docs/lean4mm0.md` (the Lean → DeltaNets → MM0 half of the
same trust story), `docs/LIQUID.md` (normative spec of the refinement layer),
`PLAN_liquid_types.md` (its staging and decisions), `../unfer/docs/LOGOS.md`
(the DeltaNets side, S37).

---

## 0. What the generic plan assumed, and what is here instead

| Generic claim | Reality in this tree | Where |
|---|---|---|
| "DeltaNets implemented **in AustralVM itself**, `src/deltanet/`" | A Rust implementation inside the unfer kernel, driven from australVM through the C ABI. There is exactly one implementation, and duplicating it in Austral is explicitly banned | `../unfer/logos/src/deltanet/` (~2.5k LOC), `lib/deltanet_plugin.ml`, `lib/CamlCompiler_rust_bridge.ml` |
| "BLAKE3 Merkle/WL sum-of-hashes isomorphism witness" | A root-directed canonical serialization plus SHA-256 (`unf_hash`), and a *checked* permutation witness (`IsomorphismWitness::verifies`). No second hash was introduced — the witness rides on the canonical walk | `../unfer/logos/src/deltanet/unf.rs`, `docs/lean4mm0.md` §2 |
| "`austral.mm0` axiomatizing linear kinds, liquid types, graph isomorphism" | `docs/cic.mm0`: the agent signature actually used by the reducer, judgmental equality, and `net_isomorphism_step`. Linearity and liquid obligations are *not* certified in MM0 — they are OCaml passes with no proof trace to emit | `docs/cic.mm0`, `mm0/*.mm0`, `scripts/lean2mm0` |
| "Egison matchers compiled to decision trees by the AustralVM compiler" | Egison exists as a **Haskell tidepool module** (`sweet-egison` quasiquoters). On the Austral side, `case` is exhaustiveness-checked at `TypingPass` and lowered to CPS blocks; `typed_when` carries **no guard field** at all | `examples/modules/fock_match/`, `lib/TypingPass.ml`, `lib/Stages.ml` |
| "Liquid types erase, Z3/CVC5 do the SMT" | Liquid types exist (stages L0–L10) and erase at the `Tast` → `Mtast` boundary. The solver seam is **Why3 + alt-ergo as a subprocess**, never a linked solver | `lib/liquid/`, `lib/Why3_plugin.ml` |
| "Bare-metal C99/Cranelift, zero-GC" | Exists: CPS → Cranelift JIT (`safestos/cranelift`) and C99 emission (`compile_c_cell.c`) | `safestos/`, `lib/CliEngine.ml` |

The generic plan's headline — *the language of verification is the systems
language itself* — is what this tree already does differently and better: the
refinement layer is a pass over the compiler's own typed AST, so erasure is
structural (§10 of `docs/LIQUID.md`) rather than a separate extraction target.

---

## 1. The pipeline as it actually runs

```text
  .aui/.aum
      │  Lexer/Parser → Combined → Linked
      ▼
  TypingPass .............. types, linearity, exhaustiveness
      │                    (`Errors.case_non_exhaustive`)
      ▼
  Compiler_plugin.run_on_typed ....  ordered tenants, first reject wins
      │   liquid            LiquidCheck    parse/WF1/T1,T2,T4,T5 → VCs
      │   liquid_measure    LiquidSubset   fold ranking via extracted measure
      │   why3_gate         Why3_plugin    S36 authorization gate
      │   deltanet_unf      Deltanet_plugin  UNF / conversion gate (S37)
      │   npu_dma_gate      Npu_dma_plugin
      │   logos_cnl         Formalize_plugin  CNL/formalization gate
      ▼
  ERASURE (pragmas dropped) → Monomorphize → CPS IR
      │
      ├─ Cranelift JIT (safestos/cranelift)   ← hosted modules, `modhost`
      └─ C99 emission (compile_c_cell.c)
```

Independent arbiters, all *outside* the compiler's TCB and all checked rather
than trusted:

| Arbiter | Question | Certificate |
|---|---|---|
| Why3 + alt-ergo | do the refinement VCs hold? | prover verdict per goal; `liquid.ok` sidecar at L8 |
| unfer `logos::deltanet` | are two terms the same after reduction? | `unf_hash` (SHA-256) + permutation witness π |
| `mm0-rs` / `mm0-c` | do the Lean theorems hold? | `.mmb`, re-verified against `docs/cic.mm0` |

---

## 2. Component map and the gaps that are open

### 2.1 Egison pattern matching — no Austral port (deliberate)

The generic plan's lowering `[#(x), x]` → borrow + equality guard presupposes
guards. This compiler has none: `Stages.Tast.typed_when` is
`TypedWhen of identifier * typed_binding list * tstmt` — a constructor tag, its
binders, and a body. There is no place to put a boolean guard, so the lowering
has no target and porting the Haskell matcher compiler would produce a second,
unused pattern engine.

What exists and is worth keeping:

- `examples/modules/fock_match/` — Multiset Wick contractions, normal-ordering,
  twin primes by breadth-first search, poker hands, unordered pairs, all as
  `sweet-egison` quasiquoters compiled into a tidepool module. This is where
  non-free matching lives, and it is reachable from the JIT path.
- The Austral side's half of the generic plan's promise — *deterministic
  decision trees instead of a backtracking stream* — is already what `case`
  compilation does, and `TypingPass` rejects non-exhaustive matches before any
  lowering.

**Gap recorded, not built.** Adding guards to `case` is a language change
(parser, `Stages`, `TypingPass`, `LinearityCheck`, CPS lowering); it is not a
compiler pass. It is listed in §4, not executed here.

### 2.2 Liquid types — the frozen-layer gaps

Stages L0–L10 landed (`PLAN_liquid_types.md` §8). The generic plan's headline
item, "eliminate the framing problem with linear types", is **already true by
construction**: the refinement layer never models the heap. `LiquidInfer.Γ`
holds facts about immutable values only, a mutable borrow invalidates them, and
nothing emits a heap-frame axiom — there is no `x.f := 5` in the refinement
logic to frame. That is a design win to state, not a feature to build.

What was actually missing — and what this iteration executes:

1. **§5.6 application preconditions.** `LiquidConstraints` emitted only the
   `§5.10 RETURN` obligation. A caller could pass `n` to a callee whose
   `Requires` says `n >= 0` and every emitted goal still held, because the
   callee's contract entered the `.mlw` as an *axiom* and nothing checked the
   call site. `docs/LIQUID.md` §11 claimed exactly this property, so the claim
   was ahead of the code.
   → **Executed:** one `g_<hash>` goal per call to a contracted callee inside a
   refinement-level body, body = the callee's `Requires` with actuals
   substituted (TRL substitution, no alpha-renaming needed: v1 has no binders),
   assumption = the caller's own `Requires`.

2. **T5 — refined variables are immutable.** Listed as "L4 owns it" in
   `TotalityCheck`'s header since L3, never implemented. The real state is
   layered, and the layering is what the work turned up:

   - The **first** line already exists, in `TypingPass`: assigning to a
     function parameter is `TypeErrors.cannot_assign_to_parameter`
     ("Cannot assign a value to a function parameter"), and a *mutable* borrow
     of one is `TypeErrors.cannot_borrow_param_mutably`. Since contracts may only
     name parameters, `result`, and module constants (WF1), no source program
     can reach the gate with a refined binding already overwritten.
   - The **second** line was missing. `TotalityCheck` now states T5 itself —
     an assignment (`TAssignVar`, `TAssign`, `TInitialAssign`) or a write
     borrow (§5.8) to a contract-named parameter is rejected — so the rule is
     checked where the fragment is defined rather than only where the binder
     is typed, and a loosening upstream would surface as a T5 diagnostic
     instead of a silently unsound `Requires ⇒ Ensures` goal.

   `result` is exempt by construction. Because no source can reach the branch,
   the gate is tested by calling it on a hand-built `Tast` (`LiquidVCTest`),
   which is the only honest way to exercise it.

3. **T3 — structural patterns only.** Listed as deferred in `TotalityCheck`'s
   header on the grounds that it "needs `case` arm guards, which `typed_when`
   does not expose". That is circular: the AST has no guards because the
   language has none, and `TypingPass` rejects any non-constructor pattern
   before `Tast` exists. T3 therefore holds **by construction at the gate's
   seam**.
   → **Executed as a correction**, not as code: the header now says so, and
   `docs/LIQUID.md` §13 records where each T-rule is enforced.

4. **§5.9 fold invariants were dead surface.** `pragma Liquid_Invariant(...)`
   parsed, type-checked and then vanished: `LiquidConstraints` produced goals
   for `KEnsures` only, and `to_mlw` printed `KFold`/`KMeasure` as comments.
   A contract that is silently ignored is worse than one that is rejected.
   → **Executed:** the invariant becomes the *result half* of §5.9 — a
   `§5.9 FOLD` goal `Γ ∧ Requires ⇒ I(result)` — so it is an obligation the
   prover must discharge, not an assumption and not a comment.

   The step half revealed a *different* gap than the stage notes claimed. The
   notes said the §4.4 combinator needed to exist in `Tast`; it did exist —
   `pragma Liquid_Fold` parses, merges across interface/body, and reaches
   `Tast` — but the measure pass ignored it and recognised folds by *name*
   (`is_fold_name`: "fold" at the head of a name). So the marker was recorded
   and then not used, which is the same class of defect as an ignored contract.
   → **Executed (C7):** `LiquidSubset.is_fold_marked` reads the marker through
   the same `LiquidTypes.contract_kind_of_string` table the VC generator uses,
   so a marked declaration is a fold whatever it is called — verified
   end-to-end (a `pragma Liquid_Fold` on a function named `reduceAll` is
   ranked).

   The closing claim about the *step* half — "this compiler has no function
   types, so no term denotes the passed step" — was wrong, and the deferred
   row that repeated it is retired with it: `Fn[E, A, A]` **is** a named type
   (`TypeParser` → `FnPtr`) and a named function in argument position denotes a
   function value (`TFunVar`).
   → **Executed:** the two remaining §5.9 premises are now emitted at the call
   site of the §4.4 combinator — `§5.9 FOLD (init half)` for
   `Γ ∧ Requires ⇒ I(init)` and `§5.9 FOLD (step half)` for
   `Γ ∧ Requires ⇒ I(a) ⇒ I(f(e, a))`, with `__elem`/`__acc` bound by the goal
   — and the shape was shown to survive the whole pipeline
   (`020-liquid-syntax/015-fold-premises` compiles and runs). This also
   surfaced a latent emitter defect: a theory function with two parameters was
   emitted `function f (a: int, b: int)`, which Why3 rejects at the comma —
   every golden carried one-parameter functions only, so the gate had never
   seen it. Parameters are now curried groups, `(a: int) (b: int)`, and the
   new golden `vc-fold-premises.mlw` is under the same `why3 prove
   --type-only` gate as the others.

5. **The `.mlw` did not survive the target engine's type checker.** TRL and
   Why3 spell the same connectives differently (`==` / `!=` / `&&` / `||` /
   `!` versus `=` / `<>` / `/\` / `\/` / `not`), and the emitter wrote contract
   text verbatim. Why3 *parses* `==` (as an infix symbol) and then rejects it as
   `unbound function or predicate symbol '(==)'`; that exits 1, which
   `LiquidWhy3` reads as "the prover refused". `UnferKernel.aui` — the L7
   contract library — uses `result == 0` in one declaration, so the failure was
   reachable, not hypothetical. It was invisible because no golden used any of
   those operators, and because `verify-invariants`' golden gate ran
   `why3 prove -P alt-ergo`, which without a prover exits before it ever reaches
   the type checker.
   → **Executed:** one translation point (`LiquidConstraints.why3_ops`) applied
   to every goal body, assumption and theory axiom; a pinned golden
   (`lib/liquid/golden/vc-operators.mlw`) whose contract uses all five operators
   and which `LiquidVCTest` compares byte-for-byte; and a `why3 prove
   --type-only` gate over every golden, which needs no prover (verified to fail
   on `==` and pass on the translated file).

### 2.3 DeltaNets — one implementation, improved in place

The engine is `../unfer/logos/src/deltanet/` (Rust, in the kernel), reached
through `uk_austral_unf` / `uk_austral_defeq` and the `deltanet_unf` compiler
pass. The generic plan's "write it in AustralVM" is refused: a second
implementation would be a second thing to trust, and the kernel needs the
reducer in-process anyway.

Landed in the same iteration as this document (recorded here because it is the
prerequisite the generic plan's Stage 2 named "in-place recycling"):

- `Net::alloc_node` no longer recycles freed ids — a freed node is a forwarding
  stub that live ports legitimately point *into*, so recycling silently rewired
  walks to an unrelated node.
- `commute_dup_through` rewires one fresh Dup per aux port and terminates both
  of the Dup's outputs on the copies' principals; the previous form wired each
  original port twice and left one output dangling.
- `core_ir/linearity.rs` (the ΔI/ΔA insertion): multi-use binders keep their
  name and get a `rename_uses` pass, and the terminal Dup of a clone chain is
  erased instead of left with an unconnected port — the cause of
  `no port at node 3 slot 2`.

The π certificate the generic plan asks for exists and is *checked*:
`unf::isomorphism_witness` + `IsomorphismWitness::verifies`, exposed as
`logos iso <a> <b> [--json]` (tab-separated `g1>g2` pairs) and carried on
`AustralDefeqReport`. There is no Merkle-sum heuristic anywhere, on purpose.

### 2.4 MM0 root of trust — `cic.mm0`, not `austral.mm0`

`docs/lean4mm0.md` §3/§7 executed the theorem side: `scripts/lean2mm0`
transpiles pinned nanoda-verified lean4export fixtures into
`mm0/{gap,layout}_certificate.{mm0,mm1}` over `docs/cic.mm0`, `mm0-rs`
elaborates, `mm0-c` re-verifies, CI has a `phase-d` job.

The generic plan's `austral.mm0` (linear kinds, liquid refinements) is **not**
built, and the reason is concrete rather than a deferral of convenience: MM0 is
a first-order logic over a declared term signature with no beta reduction
(`tsubst` is opaque, §7 of `lean4mm0.md`). A linearity or refinement judgment
lives in OCaml passes whose obligation is a *program* property, not an
equational one; certifying it would mean emitting the pass's own derivation
(perp maps, qualifier elimination trace) as MM0 theorems — a different project
than adding axioms to a theory.

What is executed here is the integrity half (`lean4mm0.md` §9): the certificate
family now pins its **trust split** (how many definitions are structurally
checked by `mm0-c` versus emitter-evaluated), **self-tests both anchors** by
mutation, and **states the theory coverage** it was generated from, so the claim
"the emitter cross-checks evaluation against the net reduction" is a test
rather than a paragraph, and a reserved rule (`tcon`, `tfold`, `tdup`, …) cannot
be mistaken for a checked one. All three lines are gated:
`scripts/verify-invariants` checks the pinned split counts, runs the mutation
self-test, and requires every rule an artifact claims to use to be a `term`
`docs/cic.mm0` actually declares.

### 2.5 Bare-metal codegen — exists; the doc's claim was too strong

Cranelift JIT (hosted modules: load-once/call-many/hot-swap through `modhost`)
and C99 emission both exist, and refinement content provably does not reach
them (`LiquidErasureTest`: byte-identical `--emit-cps` output with and without
annotations, 284 bytes each).

Two generic claims are **not** true here and are corrected rather than kept:

- *"Array bounds checks are eliminated whenever the liquid layer verified index
  safety."* There is no bounds-check elimination: the dependent index layer (L9)
  gives index-carrying types and erasure, not codegen elision.
- *"Destructors are inserted at the exact lexical point where ownership ends."*
  Linearity is checked (exactly-once use unless `clone`/`destroy`), and cleanup
  follows the CPS lowering's own discipline; there is no separate RAII insertion
  pass to point at.

---

## 3. Executed this iteration (2026-10-25)

Ordered as implemented; acceptance commands run from `australVM/`.

| # | Item | Files | Acceptance |
|---|------|-------|------------|
| C1 | §5.6 call-site preconditions: one goal per call to a contracted callee, its `Requires` substituted with the actuals; `LiquidInfer.term_of` gains `TCast` (desugaring wraps every actual in one, so without it no call-site obligation could ever be stated) | `lib/liquid/LiquidConstraints.ml`, `lib/liquid/LiquidTypes.ml` (`subst_formula`), `lib/liquid/LiquidInfer.ml` | `test/LiquidVCTest.ml`, suite `014-application-and-invariant`, golden `vc-callsite-and-invariant.mlw` |
| C2 | T5 stated by the gate (assignment **and** write-borrow of a contract-named parameter); the upstream `TypingPass` rejections recorded as the first line | `lib/liquid/TotalityCheck.ml`, `lib/liquid/LiquidCheck.ml` | `LiquidVCTest` (hand-built `Tast`; no source can reach the branch — see §2.2) |
| C3 | §5.9 result half: `Liquid_Invariant` emits a `§5.9 FOLD` obligation instead of vanishing | `lib/liquid/LiquidConstraints.ml` | `LiquidVCTest`, suite `014-application-and-invariant` |
| C4 | T3 corrected: enforced by construction at `Tast` (no guards exist); §13 conformance row added | `lib/liquid/TotalityCheck.ml` header, `docs/LIQUID.md` §13 | `scripts/verify-invariants` unchanged gates stay green |
| C5 | TRL → Why3 operator translation, pinned by a golden, + a `--type-only` gate over every golden | `lib/liquid/LiquidConstraints.ml`, `lib/liquid/golden/vc-operators.mlw`, `scripts/verify-invariants` | `LiquidVCTest:operators_are_translated` (byte-compares the golden); gate fails on `==`, passes translated |
| C6 | §11 soundness claim narrowed to what the VC set actually establishes | `docs/LIQUID.md` §11 | review; the claim now matches C1–C7 |
| C7 | `pragma Liquid_Fold` actually designates a fold (the measure pass honours the marker, not just the name); header corrected — the pass ranks, it never rejects | `lib/liquid/LiquidSubset.ml` | `LiquidSubsetTest:the_marker_pragma_designates_a_fold_regardless_of_name`; a `reduceAll` marked by pragma is ranked |
| C8 | §5.6 call sites with arithmetic actuals: `term_of` renders `pervasive::trapping{Add,Subtract,Multiply}` (both `TFuncall` and `TMethodCall` — desugaring picks one per shape) and resolves the `TLetTmp` temporary the desugarer hoists the argument into | `lib/liquid/LiquidInfer.ml`, `lib/liquid/LiquidConstraints.ml` | `LiquidVCTest:arithmetic_actuals_are_stated` + golden `vc-arithmetic.mlw`: `atLeast(n - 1)` now yields `forall n: int. (n >= 1) -> n - 1 >= 0` |
| C9 | **§6 range obligations** (the sort environment they needed) + **record projections at call sites**: `env_of_function`/`int_bounds_of_ty`/`range_facts` give each variable its declared type and each integer parameter its bounds as an antecedent; `__slot_X(r)` renders a projection — resolving the desugarer's read-borrow rename back to the parameter — and is declared as an uninterpreted symbol so Why3 can parse the file; multi-argument application renders curried (`why3_of_formula`) because `f(a, b)` is a tuple application to Why3 | `lib/liquid/LiquidInfer.ml`, `lib/liquid/LiquidTypes.ml`, `lib/liquid/LiquidConstraints.ml` | `LiquidVCTest:arithmetic_actuals_are_stated` (asserts the §6 goal and its `min ≤ n ≤ max` antecedent), `LiquidVCTest:projection_actuals_are_stated`, goldens `vc-arithmetic.mlw` + `vc-projection.mlw`, `why3 prove --type-only` over both |
| C10 | **§5.9 init and step premises** (the rule's two undischarged halves): at a call of the `pragma Liquid_Fold` combinator the emitter substitutes the actual seed for `result` in the parsed invariant (`init half`) and binds `__elem`/`__acc` for the step side condition (`step half`), applying the step actual by the name it resolved to — including through a `TCast` and through a local binding (`let s := step; fold2(s, …)`; `calls_in_stmt` now carries a source `let`'s `TInitialAssign` into the call-site env, scoped to the `TLet`) — with the call index in the goal names so two fold calls in one function cannot collide — plus the latent emitter fix that made a **two-parameter** theory function parseable by Why3 (curried parameter groups) | `lib/liquid/LiquidConstraints.ml` (`find_fold_combinator`, `fold_goals`, `calls_in_stmt`, `to_mlw`) | `LiquidVCTest:fold_premises_are_stated` (one step premise per fold call: exactly two) + golden `vc-fold-premises.mlw` (byte-compared, `why3 prove --type-only`, and `why3 prove -P z3`: all 6 goals **Valid**), `runner.py` `020-liquid-syntax/015-fold-premises` (compiles, runs, byte-identical stdout) |
| C11 | **Return-family goals carry the body equation** (`Requires ∧ (result = e) ⇒ Q`): as emitted before, `forall …, __result. Requires ⇒ Q` demanded that *every* value satisfy the postcondition — unprovable by construction for any non-trivial contract, a false rejection waiting for `liquid = "required"`. The goal now states §5.10's subtyping judgment over the returned expression, falling back to the un-linked shape when the body does not structurally render (branching body, unrenderable actual) | `lib/liquid/LiquidConstraints.ml` (`return_expr`, `make_return_goal`) | golden diffs: every §5.10/§5.9-result line in `constraint-generation`, `vc-callsite-and-invariant`, `vc-arithmetic`, `vc-operators`, `vc-fold-premises` gained the equation; `why3 prove -P z3` now reports **Valid** for the `atLeast`/`step`/`fold2` goals it could never discharge before |

Verification loop (the repo's own, per `PLAN_liquid_types.md` §8):
`make bridge && dune build lib/ bin/ test/ && dune runtest &&
python3 test-programs/runner.py && scripts/verify-invariants`. Run on
2026-10-25 with Why3 on `PATH`: `dune runtest` green (9 cases in
`LiquidVCTest`), `runner.py` "All tests passed" (including suite
`020-liquid-syntax/014`), `verify-invariants` 40 passed / 0 gaps / 0 failed —
which includes `why3 prove --type-only` over all three goldens and the
extraction-match check for `LiquidMeasure`.

Re-run on 2026-10-25 after C10 (the §5.9 premises): `dune runtest` green
(11 cases in `LiquidVCTest`), `runner.py` "All tests passed" (including
`020-liquid-syntax/015-fold-premises`, which takes the fold shape through the
C backend and runs it byte-identical), `verify-invariants` 42 passed / 0 gaps
/ 0 failed — the two new checks being the type-only and parse gates over
`vc-fold-premises.mlw` — `scripts/check-links` OK (235 documents).

Re-run again after C10/C11's local-binding, goal-index and body-equation
follow-ups: `dune runtest` green (11 cases in `LiquidVCTest`), and the
prover actually armed this time — z3 5.1.0 fetched via `nix build
nixpkgs#z3`, registered with `why3 config detect` (alt-ergo still fails to
build on this host, see §2.2's environment notes):
`why3 prove -P z3 lib/liquid/golden/vc-fold-premises.mlw` → **exit 0, all
6 goals Valid** (result/init/step × both call sites); `constraint-generation`
also fully Valid now, where the pre-equation shape could prove none of its
goals. The other goldens' remaining non-Valid goals are honest: unrenderable
bodies fall back to the un-linked shape (documented in §5.9's status), and
`vc-operators`/`vc-projection` carry fixture contracts their own bodies
discharge only under assumptions the caller never states.

**Environment notes, recorded because they cost time.**

- `nix develop` on this host still fails while building `alt-ergo` (its
  assemble step: `ocamlopt` shells out to `as`, which reports an I/O error —
  a plain `as` invocation and a trivial derivation both build fine, so it is
  this package's build, not the sandbox or the disk). Because the devShell is
  all-or-nothing, that failure used to take the *engine* down with it too.
  **Fix:** `flake.nix` now exposes `packages.why3` and `packages.alt-ergo`
  separately, so `nix build .#why3` / `nix profile install .#why3` installs
  why3 1.8.2 without touching alt-ergo, and
  `nix profile install .#alt-ergo` is the (still-failing) prover. `why3` is on
  `PATH` from the profile, so `scripts/verify-invariants` closes its two Why3
  gaps with no hand-assembled store paths.
- The OCaml builds above still run with the devShell's store paths assembled
  by hand — OCaml 5.4.1 + dune 3.23.1 + menhir from `/nix/store`, with
  `OCAMLPATH` and `CAML_LD_LIBRARY_PATH` pointing at each `site-lib` and
  `site-lib/stublibs` — because `nix develop` cannot complete while alt-ergo
  fails.
- `unfer`'s `prob_kernel` could not be *compiled* on this ARM host at all
  until now: `candle-core` pulls `gemm` unconditionally and `gemm-common`'s
  hand-written fp16 kernels need `+fullfp16`, which nothing enabled. It builds
  and passes (`cargo test -p prob_kernel --lib` → 94 passed) with the
  aarch64-gated flags in `../unfer/.cargo/config.toml`; that section is
  `cfg(target_arch = "aarch64")`, so Intel/AMD builds are byte-for-byte the
  same as before, and gemm's own runtime `feature_detected!("fp16")` gate
  still keeps the fp16 kernels off CPUs that lack them.

## 4. Deferred, with the prerequisite named

| Item | Why not now | Prerequisite |
|---|---|---|
| §6 **cast** range obligations (the narrowing case) | every argument already carries a `TCast`, so emitting one VC per cast would emit one per call argument; the useful case needs the operand type compared against the target | keep both `Type.ty`s through `term_of` (the cast node has the target; the operand's node carries its own) and emit only when they differ |
| TRL **projection syntax** (`r.X` inside a contract) | the goal side is done (C9): `__slot_X(r)` is emitted and declared | extend `LiquidParse`'s term grammar with `expr.ident`, so a `Requires` can state what the goal asks the prover to prove |
| Comparisons in **term** position (`f(a < b)`) | would need a Why3 `predicate` symbol, i.e. a second sort through the emitter, the theory section and the binder; the old rendering (`@cmp`) did not parse at all | a bool-sorted symbol table next to the int one in `to_mlw` |
| direct SMT-LIB2 / Z3 / CVC5 output | Why3 is the established seam (same licence discipline, same golden pinning); a second backend is a second TCB for no new guarantee | a decision to replace Why3, not to add beside it |
| linear ghost state (`alive`/`freed`) | v2 by `docs/LIQUID.md` §5.8/§12; needs the frame rule the linear fragment makes unnecessary for values | §5.8 frame conditions first |
| Egison guards in `case` | language change (parser → CPS), not a pass; the Haskell path covers the use case today | a decision to make `case` guarded |
| parallel DeltaNets reduction | `PLAN_parallel_australvm.md`; must keep `unf_hash` and the witness bit-identical to the single-threaded reducer | the correctness pass this iteration, then a sharded active-pair queue |
| MM0 certificates for linearity/refinements | no derivation is emitted by the OCaml passes | a proof-trace mode for `LinearityCheck`/`LiquidConstraints` |

## 5. The Haskell comparison, measured rather than asserted

The generic appendix argued that Egison-in-Haskell is slow and that AustralVM
would be faster. Grounded version:

- The comparison that can be made **inside this repository** is
  `examples/modules/fock_match/` (sweet-egison, shallow embedding, ~40× faster
  than miniEgison per the stage B9b notes) against the JIT path that hosts it.
  No benchmark in this tree compares an Austral implementation against it,
  because no Austral implementation exists — so no speed claim is made here.
- The lazy-equivalence question (shared higher-order terms reduced once) is
  answered by the kernel's DeltaNets engine, not by the Austral codegen. That
  engine is Lévy-optimal in the same sense the generic document describes, and
  its normal forms are what `uk_austral_defeq` compares.
- What *is* measurable and already measured: refinement content costs nothing at
  runtime (byte-identical CPS, 284 bytes), and the MM0 re-verification of nine
  gap and eight layout certificates runs in seconds (`scripts/lean2mm0 verify`).

## 6. Maintenance budget, revised

Actual sizes in this tree (`wc -l`), against the generic plan's ~8,000-line
estimate for greenfield work:

| Area | Lines | Note |
|---|---|---|
| `lib/liquid/*.ml` (spec, parser, VCs, inference, driver, gates) | ~3.2k | generic estimate 2.2k for "enhancements" only |
| extracted measure (`LiquidMeasure.ml`) + Why3 source | 10 + 91 | Cycle B: the checker extends itself |
| DeltaNets engine (Rust, unfer) | ~2.5k | generic estimate 2.5k for "in AustralVM" |
| `lib/deltanet_plugin.ml` + `formalize_plugin.ml` + bridges | ~700 | the compiler-side gates |
| `scripts/lean2mm0` + `docs/cic.mm0` | ~1.7k | the MM0 emitter and theory |
| Egison (Haskell module + scripts) | ~0.3k | replaces a ~1.8k "matcher compiler" |

The stack stays maintainable for the reasons the generic plan named — no
external runtime, a frozen `mm0-c` root of trust, one language for compiler and
target — plus the one this adaptation adds: **each arbiter is a different
implementation of a different question**, so a bug in one is not hidden by the
other, and every one of them is checked rather than believed.
