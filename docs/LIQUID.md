# Liquid Austral — TRL and Liquid Typing Rules (stage L0)

**Status**: draft specification, stage **L0** of `PLAN_liquid_types.md`.
Normative for stages L2–L6 (surface syntax, totality gate, constraint
generation, Why3 encoding). Non-normative sections are marked.

**Scope**: LiquidTypes (refinement types with qualifier-based inference) for
the **Total Austral Subset** of the extended Austral used by australVM — the
fragment guaranteed to terminate. Refinements are ghost: checked at compile
time by Why3 (subprocess; the LGPL seam of `../unfer/docs/WHYML_CYCLE.md`),
then erased before monomorphization/CPS lowering. The frozen `uk_*`/`uz_*`
ABI, the `module.toml` grant vocabulary, and the JIT are untouched.

**Normative references**:
- `../unfer/logos/src/austral_codegen/validate.rs` — reference semantics of
  the Total Austral Subset (totality + linearity).
- `../unfer/logos/src/core_ir/types.rs` — `CoreIR` (the subset's IR:
  `Var/Lit/Con/Lam/App/Let/Match/Fold/Prim/Clone/Drop`).
- `lib/Stages.ml` — `Tast` (the AST the liquid pass runs over).
- `PLAN_liquid_types.md` — architecture, stages, decisions.

Key words: **MUST** (soundness-relevant), **MUST NOT**, **SHOULD**, **MAY**.

---

## 1. The Total Austral Subset (normative)

A declaration is in the **total fragment** (admissible to refinement-level
logic) iff all of the following hold. These mirror `validate.rs`, translated
from `CoreIR` to Austral surface/Tast:

- **T1 — No recursion.** The call graph of refinement-level functions
  **MUST NOT** contain a cycle, direct or mutual (cf. `check_cycles`:
  "a function may call another defined function, but never itself,
  directly or transitively").
- **T2 — Iteration is `fold` only.** The only iteration construct is the
  designated `fold` combinator (§4.4) plus structural `match`/`case`.
  `while` and `for` statements (`TWhile`, `TFor`) are **MUST NOT** appear in
  the fragment (cf. ENG_PLAN: "No general recursion. No `while` loops. No
  self-referential `Let`").
- **T3 — Structural patterns only.** `case` arms use constructor patterns
  with variable binders (`Pattern::Tag(tag, binders)` analogue). No guards
  with side effects; no fallthrough.
- **T4 — Purity.** Refinement-level functions **MUST NOT** call foreign
  functions, `@embed`, `@embed`-based wrappers, or any `uk_*` symbol, and
  **MUST NOT** allocate/free or mutate state.
- **T5 — Immutability of refined variables.** A variable carrying a
  refinement (appearing in Γ with a non-`true` refinement) **MUST NOT** be
  the target of `TAssign`/`TAssignVar`/`TInitialAssign` (see §5.8 for
  borrows).

Linearity (each binder used exactly once) is **not** re-checked here: on the
compiler side it is already enforced by `check_module_linearity` over the
typed module; `validate.rs`'s linearity rules (`clone`/`drop` discipline) are
the logos-side analogue. The liquid layer never weakens linearity: it only
adds facts.

Anything outside the fragment is **unchecked**, not rejected — unless the
module manifest sets `[verify] liquid = "required"`, in which case
refinement-level violations (T1–T5) are compile errors.

---

## 2. TRL — the Total Refinement Logic (normative)

TRL is the language refinements are written in. By design it is *exactly* the
total fragment's expression language plus logical connectives: every TRL term
denotes a total, strongly normalizing computation, so the logic is consistent
by construction and SMT encodings cannot diverge on our definitions.

### 2.1 Sorts

| Sort | Meaning | Why3 model |
|------|---------|------------|
| `Int` | mathematical integers (results of machine ints) | `int.Int` |
| `Nat` | `{ν: Int \| 0 ≤ ν}` (subsort of `Int`) | `int.Int` + hypothesis |
| `Bool` | formulas / booleans | `bool` |
| `Real` | reals, for probability values | `real.Real` |
| `Prob` | `{ν: Real \| 0.0 ≤ ν ≤ 1.0}` (subsort of `Real`) | `real.Real` + hypothesis |
| `List[T]` | lists of a sort | Why3 `list` + measures |
| ADT sorts | records/unions of the module | algebraic datatypes |

Austral machine types map to `Int` **with range obligations** (§6):
`Int64 ↦ Int`, `Nat64 ↦ Nat`, etc. The i64-centric CPS convention means most
values are `Int64`; the width only changes the obligation side condition.

### 2.2 Grammar

Lexical: identifiers are Austral identifiers; `ν` (ASCII `result` in pragma
strings) is the refinement variable of the current binding's type; integer
literals **MUST** lie in `[-2^62, 2^62)`.

```
trl-term    ::= int-lit | real-lit | "true" | "false"
              | var                                   -- program vars, params, result
              | trl-term "+" trl-term | trl-term "-" trl-term
              | trl-term "*" trl-term                 -- Int/Nat only; Real only via measures
              | "-" trl-term
              | "if" trl-term "then" trl-term "else" trl-term
              | fun-name "(" [trl-term ("," trl-term)*] ")"   -- measures, total fns

trl-formula ::= trl-term relop trl-term
              | "!" trl-formula
              | trl-formula ("&&" | "||" | "==>") trl-formula
              | "(" trl-formula ")"
              | "true" | "false"

relop       ::= "==" | "!=" | "<" | "<=" | ">" | ">="
var         ::= identifier | "result"
```

There are **no quantifiers** in v1 (`forall`/`exists` are MUST NOT).
`fun-name` **MUST** resolve to a measure (§4.3) or a total-fragment function
(§4.2); the totality gate (stage L3) enforces this.

### 2.3 Well-formedness (WF)

- **WF1** — every free variable of a contract is a parameter of the
  declaration, the `result` variable, or a module-level constant of total
  fragment type.
- **WF2** — sorts check (§2.1); `result` has the declaration's return sort.
- **WF3** — every function symbol applied is total-fragment-admissible (T1–T4
  checked at its definition site).
- **WF4** — formulas in `Liquid_Ensures` are over the post-state value only
  via `result` and the parameters (no hidden state; v1 has no state
  refinements).

---

## 3. Surface syntax (normative; the `pragma` seam)

Contracts ride the existing pragma machinery. Real syntax precedent
(`examples/kernel/UnferKernel.aum`):

```austral
pragma Foreign_Import(External_Name => "uk_version");
function kernelVersion(): Int64 is end;
```

`lib/Lexer.mll` lexes `pragma`; `lib/Parser.mly` builds
`make_pragma name args` (`ConcreteNamedArgs`/`ConcretePositionalArgs`);
pragmas are threaded as `pragma list` through `Cst`/`Stages` to
`LFunction … * pragma list`. New pragma forms (stage L2):

| Pragma | Args | Meaning |
|--------|------|---------|
| `Liquid_Requires` | `Contract => "<trl-formula>"` | precondition over params |
| `Liquid_Ensures` | `Contract => "<trl-formula>"` | postcondition over `result` + params |
| `Liquid_Invariant` | `Contract => "<trl-formula>"` | fold/accumulator invariant (§4.4) |
| `Liquid_Measure` | (none) | mark a total-fragment function as a measure (§4.3) |
| `Liquid_Fold` | (none) | mark the designated `fold` combinator (§4.4) |
| `Liquid_Trusted` | `Contract => "<trl-formula>"` | **assumed**, not checked (§7) |

Rules:

- Pragmas appear **before** the declaration, after its docstring (grammar:
  `docstringopt pragma* interface_decl_inner` / `body_decl_inner`).
- Contracts **SHOULD** live in the interface (`.aui`) — that is the contract
  surface; body pragmas are for body-local annotations (fold invariants).
- Multiple `Liquid_*` pragmas may combine with each other and with
  `Foreign_Import`; `TypingPass`'s pragma match is extended accordingly (today
  it accepts exactly `[ForeignImportPragma s]`, `[ForeignExportPragma _]`, or
  `[]`, and `Errors.fun_invalid_pragmas ()` otherwise).
- DSL strings are parsed by `LiquidParse` with span-accurate errors pointing
  into the pragma string.

Example (interface):

```austral
module UnferKernel is

    """
    Evolve a model. Returns the kernel status code.
    """
    pragma Liquid_Requires(Contract => "0 <= len");
    pragma Liquid_Ensures(Contract => "result >= 0 || result == -code");
    function kernelInit(cfg: Address[Nat8], len: Int64): Int64;

end module;
```

---

## 4. Definitions: contracts, total functions, measures, fold

### 4.1 Function contracts

For `function f(x1: T1, …, xn: Tn): R` with `Liquid_Requires P` and
`Liquid_Ensures Q`:

- the parameter environment at entry is `xi : {ν: Ti | P[xi := ν]}`;
- every `return e` **MUST** check `e : {ν: R | Q[result := ν]}` (§5.10);
- call sites instantiate `P`/`Q` by capture-avoiding substitution of actuals
  for formals (§5.6).

A function with no `Liquid_Ensures` gets `Q = true` (the trivial contract);
under `[verify] liquid = "required"` every refinement-relevant function
**MUST** carry a non-trivial `ensures` (policy choice; configurable).

### 4.2 Total refinement functions

A refinement-level function is any function whose body passes T1–T5. Its
**definition** may be used two ways:

1. *Opinionated*: its contract is used at call sites (§5.6).
2. *Logical*: it appears inside TRL terms (measures, §4.3) — then its
   body is emitted as a Why3 `function` definition (Cycle A, §8.3).

Because T1 forbids call cycles and T2 restricts iteration to `fold`, every
such function denotes a total computation; its Why3 translation is a
**conservative definitional extension** (no well-foundedness proof required
beyond the structural `fold` translation, §8.3).

### 4.3 Measures

A function marked `pragma Liquid_Measure` **MUST**:
- have sort-correct signature ending in `Int`/`Nat`/`Bool`/`Real`/`Prob`;
- be a total refinement function (T1–T5);
- be **purely structural**: defined by `fold` over a list, by record/union
  projection, by primitive operations, or by calls to other measures.

Measures are emitted as Why3 `function`s. At every `case` arm on a
constructor `C`, the VC carries the constructor's **unfolding equations** as
explicit hypotheses (e.g. for `length` on `Cons(h, t)`:
`length(Cons(h,t)) == 1 + length(t)`), so provers never rely on recursive
unfolding heuristics.

Trusted measures (`Liquid_Trusted` with an equation contract, e.g. a buffer
length recovered via `@embed`) are axiomatized instead of defined; the
attestation (stage L8) counts them.

### 4.4 The `fold` combinator

One declaration per module (typically from the total-subset prelude) is
marked `pragma Liquid_Fold`. Its shape follows the `CoreIR::Fold` reduction
order (`Fold(f, init, cons(h,t)) → App(App(f, h), Fold(f, init, t))`):

```austral
pragma Liquid_Fold;
generic [A: Type, E: Type]
function fold(f: (E, A) -> A, init: A, xs: List[E]): A is … end;
```

The step function takes **element first, accumulator second**. Typing rule in
§5.9.

---

## 5. The liquid judgment (normative)

### 5.1 Judgments

- `Γ ⊢ e : {ν: T | Q}` — expression: if `e` evaluates to `v`, then `Q[ν := v]`.
- `Γ ⊢ s ▷ Q` — statement: if `s` completes *normally* (falls through), then
  `Q` holds. (Austral is statement-oriented; `return` exits — §5.10.)
- `Γ ⊢ {ν:T|p} <: {ν:T|q}` — subtyping; emits the VC `Γ ∧ p ⇒ q` (with `ν`
  fresh).
- `Γ ⊢ ok` — pure side-condition (overflow, WF); emits a VC over Γ.

### 5.2 Environments

Γ maps variables to refinement types and accumulates path facts. Because of
linearity, every binder is consumed exactly once — refinements are *ghost
facts* and may persist in Γ after consumption (e.g. `freed(m)` after
`freeModel(m)` in v2; v1 keeps facts about immutable values only, T5).
Weakening holds: facts are never dropped except by the borrow rule (§5.8).

### 5.3 Literals, variables, primitives

```
(LIT)  ─────────────────────────────────────────  Γ ⊢ lit : {ν: T | ν == lit}

(VAR)  x : {ν: T | p} ∈ Γ ──────────────────────  Γ ⊢ x : {ν: T | p}

(PRIM) Γ ⊢ ei : {ν: Ti | pi}   Γ ⊢ range(op(e…)) ok
       ─────────────────────────────────────────
       Γ ⊢ op(e1,…,en) : {ν: T | ν == op(e1,…,en)}
```

`range` obligations are §6. Primitive comparisons yield
`{ν: Bool | ν == (e1 < e2)}`.

### 5.4 Let / destructure

```
(LET)  Γ ⊢ e : {ν: T | Q}    Γ, x: {ν: T | Q} ⊢ s ▷ R
       ─────────────────────────────────────────────────  Γ ⊢ let x: T := e; s ▷ R
```

`TDestructure` binds each pattern variable with the constructor's measure
equations folded into its refinement (§4.3).

### 5.5 Conditionals

```
(IF)   Γ, [[c]] ⊢ s1 ▷ Q      Γ, ![[c]] ⊢ s2 ▷ Q
       ──────────────────────────────────────────  Γ ⊢ if c then s1 else s2 ▷ Q
```

`[[c]]` is the TRL translation of the Boolean expression `c`. The
expression-form `TIfExpression` gets the analogous join:
`{ν: T | (c ⇒ Q1) && (!c ⇒ Q2)}`.

### 5.6 Application (user functions)

For `f` with contract `(P, Qf)`, actuals `a1…an`, and fresh `r`:

```
(APP)  Γ ⊢ P[a̅/x̅] ok                    -- caller proves precondition
       Γ ⊢ ai : {ν: Ti | true}
       ──────────────────────────────────────────────────────
       Γ ⊢ f(a1…an) : {ν: R | Qf[result := ν][a̅/x̅]}
```

The precondition obligation is discharged as `Γ ⇒ P[a̅/x̅]`; the result
refinement substitutes actuals for formals (syntactic; inlined as TRL terms —
termination is guaranteed by T1 at the *definition* level, and substitution
of terms never introduces recursion into the logic).

Method calls (`TMethodCall`/`TVarMethodCall`) follow the instance method's
contract; **v1 SHOULD** treat unrefined typeclass methods as `true`/`true`.

### 5.7 Case / match

```
(CASE)  for each arm C(p̅) ⇒ s:   Γ, ctor-facts(C, p̅), [[p̅]] ⊢ s ▷ Q
        ─────────────────────────────────────────────────────────────  Γ ⊢ case e of … ▷ Q
```

`ctor-facts(C, p̅)` are the measure unfolding equations of `C` (§4.3) plus
the scrutinee equation `e == C(p̅)`. Exhaustiveness is Austral's own
(`TCase` on unions is checked upstream); refinements never weaken it.

### 5.8 Borrows

- Read borrow (`TBorrow`, read mode): refinement facts about the borrowed
  variable persist through the body (v1: values are immutable within).
- Mutable borrow: the borrowed variable's non-`true` refinements are
  **invalidated** inside the body and after (conservative); facts about other
  variables persist. v2 may restore them via frame conditions.

### 5.9 Fold (the invariant rule)

With `fold` per §4.4 and invariant `I(a)` from `Liquid_Invariant` (or
inferred, §9):

```
(FOLD) Γ ⊢ init : {ν: A | I(ν)}
       Γ ⊢ f     : (e: {ν: E | true}, a: {ν: A | I(ν)}) -> {ν: A | I(ν)}
       Γ ⊢ xs    : {ν: List[E] | true}
       ─────────────────────────────────────────────────────────────────
       Γ ⊢ fold(f, init, xs) : {ν: A | I(ν)}
```

Side facts: the step's postcondition **MUST** imply `I` (VC:
`I(a) ⇒ I(f(e, a))`); the result additionally carries measure facts, e.g.
`length(result) == length(xs)` when `A` is a list. Optional length-decrease
refinements (`len` on nested folds) are v2.

**Invariant rule of thumb**: `I` is exactly the qualifier vocabulary
instantiated over the accumulator type — the reason `fold` needs no
termination metric is that the fold itself is structural (T2).

### 5.10 Return; statements without postconditions

```
(RET)  Γ ⊢ e : {ν: R | Q}      (subtyping against the declared ensures)
```

Every exit path **MUST** satisfy the declared `ensures`; `check_ends_in_return`
already guarantees there is no fall-off-end. `TSkip`, `TDiscarding`, `TWhile`,
`TFor`, `TEmbed`, `TDeref`, `TSizeOf`, `TFptrCall`:
- `TSkip`/`TDiscarding e`: `Q = true` (plus e's own obligations).
- `TWhile`/`TFor`: **outside the fragment** (T2) — under `required`, reject;
  otherwise unchecked (no facts in, `true` out).
- `TEmbed`: opaque. Result refinement `true` unless the binding is covered by
  a `Liquid_Trusted` contract.
- `TFptrCall`: like a foreign call without contract: `true`/`true`
  (error under `required`).
- `TCast`: `{ν: T' | range(T', ν)}` (§6), preserving source facts when
  widening is provable.

### 5.11 Foreign calls

For `TForeignFunction` with `Liquid_Requires P` / `Liquid_Ensures Q`:
call sites behave as (APP) with `P`/`Q`. Without contracts: `P = Q = true`;
under `required`, any call to a `uk_*`/`uz_*` symbol **MUST** have a
contract. The contract is the *only* fact about the C ABI — soundness depends
on it matching the kernel (see §11 TCB; contracts for `UnferKernel.aui` are
written against `unfer_ffi` in stage L7).

---

## 6. Overflow discipline (normative)

TRL models machine ints as mathematical ints (Why3 `int`). Every machine-int
operation `e` generates a **range obligation** `Γ ⊢ min(T) ≤ e ≤ max(T) ok`
for its result sort `T`. Concretely, for `Int64`:

- literals **MUST** be in `[-2^62, 2^62)` (the native-OCaml-int extraction
  precedent of `unfer_ocaml.drv`);
- `+`, `-`, `*` each emit `result`-range VCs;
- comparisons and `==` operate on mathematical values (no wraparound
  semantics);
- casts emit the target range (§5.10).

Rationale (plan §1.4): alt-ergo discharges linear integer arithmetic well;
bitvectors are the documented fallback (`bv63` theory) if obligation noise
becomes unmanageable. Reals: `Prob` bounds are hypotheses at ABI boundaries;
`*` on `Real` is restricted to measures (non-linear real arithmetic is
prover-dependent; keep products rare).

---

## 7. Trusted contracts (`Liquid_Trusted`) (normative)

`Liquid_Trusted` **assumes** a contract without checking it. It is the escape
hatch for `@embed` glue and C-ABI facts the fragment cannot express (T4).
Rules:

- A trusted contract **MUST** be flagged in the module's attestation (stage
  L8: count + names in `liquid.ok`), and **SHOULD** carry a comment naming
  its external justification (e.g. the `unfer_ffi` function it mirrors).
- Trusted contracts are **not** transitive soundness: they enter the TCB
  (§11).
- Refinement-level code (T1–T5) **MUST NOT** use `Liquid_Trusted` results in
  measure *definitions* that are emitted as Why3 functions unless the measure
  itself is marked trusted (§4.3).

---

## 8. VC generation and Why3 encoding

### 8.1 One `.mlw` per module

`LiquidConstraints` collects obligations; `LiquidWhy3` emits a single module
`Liquid_<ModuleName>.mlw` containing:

- the TRL prelude (`int.Int`, `real.Real`, `list` theory, `liquid.drv`
  mapping — native ints, sibling of `unfer_ocaml.drv`);
- measure definitions and total-function definitions (Cycle A, §8.3);
- one `goal g_<span-hash>` per obligation, named by the Austral source span
  of the construct that generated it (blame mapping, stage L5).

Sketch:

```why3
module Liquid_DemoHosted
  use int.Int
  use list.List
  use list.Length

  function length_int (l: list int) : int = ...
  goal g_demo_aum_42_7 :
    forall xs: list int. length_int xs >= 0
end
```

### 8.2 Discharge

`why3 prove -P alt-ergo Liquid_<M>.mlw` runs as a **subprocess**
(`WHY3_CLI` override, then PATH — the Cadabra2 engine-missing pattern).
Verdicts map back through `g_<span-hash>` to Austral spans; any `Unknown` or
`Timeout` is a failure (no "probably fine"). Absent engine: structured
warning + skip, or hard error under `liquid = "required"`.

### 8.3 Cycle A — total Austral extends Why3

TRL definitions are emitted as Why3 `function`s. The **only** recursive
definitions generated are the translations of `fold` uses — structural
recursion over the list argument, justified by T2 (the fold terminates). No
other definition is recursive (T1), so the emitted theory is a conservative
defitional extension of the prelude. This is what allows the subset to extend
Why3's environment (measures, lemmas, grant-lattice predicates) and, via
`why3 extract -D liquid.drv`, to extend the checker itself (Cycle B) — see
`PLAN_liquid_types.md` §3.

### 8.4 Golden pinning

Generated `.mlw` files are pinned under `lib/liquid/golden/` and byte-diffed
in CI (the `authorize_gate.mlw` discipline of WHYML_CYCLE). Extraction output
is pinned the same way when Cycle B is exercised.

---

## 9. Inference (non-normative sketch; stage L6)

- **Qualifier vocabulary**: templates over Γ's variables and measures
  (e.g. `0 <= x`, `x <= y`, `result >= x`, `length(xs) == …`), plus user
  qualifiers via `[verify] qualifiers = [...]` in `module.toml`.
- **Algorithm**: generate Horn constraints (L4) and run Houdini-style
  elimination — start with all candidate qualifiers per binder, drop those
  falsified by counterexamples, iterate to a fixed point. No recursion in the
  fragment ⇒ the fixed point is over a finite lattice of qualifier sets and
  converges without widening.
- **Fold invariants** (§5.9) are qualifiers over the accumulator type; the
  `Liquid_Invariant` pragma overrides inference when present.
- **Default measures**: `length` on lists, field projections on records, and
  `Prob` bounds on the kernel's probability surface.

---

## 10. Erasure semantics (normative)

- Refinement content exists only in `pragma` payloads and measure/function
  declarations; nothing refinement-derived reaches `Mtast`, the CPS IR, or
  the JIT.
- Erasure point: after the liquid pass on `Tast`, the existing body-extraction
  and monomorphization pipeline runs unchanged (`pragma list` is already
  ignored by `Compiler_cps.ml`/`CpsGen`).
- **Guarantee (proof-by-test, stage L9)**: `--emit-cps` output is
  byte-identical for a module compiled with and without refinement
  annotations.

---

## 11. Soundness sketch and TCB (non-normative)

Soundness claim: if the liquid pass accepts a module, then at every call the
actual arguments satisfy the callee's `requires`, and every return value
satisfies the declared `ensures`, *relative to* the trusted contracts.

TCB: Why3 + alt-ergo + the Why3 extractor; `LiquidConstraints`/`LiquidWhy3`/
`TotalityCheck` (the VC generator is trusted — mitigated by golden pinning
and differential testing); every `Liquid_Trusted` contract (§7); the C ABI
correspondence of foreign contracts. Not in the TCB: the JIT, the CPS
lowering, `module.rs`/`modhost` (they consume the attestation, not the
proofs). Optional cross-check of hard lemmas via Lean4
(`prob_kernel::verify`, unfer S29) — Cycle-adjacent, later.

---

## 12. Non-goals (v1) / v2 pointers

- Quantifiers in contracts (`forall` over lists) — v2 (measure-based
  workarounds first).
- State refinements and linear ghost state (`alive`/`freed` on `Model`) —
  v2 (§5.8 frame conditions first).
- Dependent index types (`Span[Nat8, n]`, `Vector[T, n]`) — Cycle C, stage
  L9; indices are TRL terms (totality ⇒ typechecking normalizes).
- Bitvector overflow modeling — fallback per §6.
- `while`/`for` invariants — out of scope (the fragment is the point).

---

## 13. Conformance checklist

vs. `validate.rs` (logos):

| validate.rs | This spec |
|-------------|-----------|
| `check_cycles` (no call cycles) | T1, stage L3 `TotalityCheck` |
| `Fold` only iteration; no `while`/self-ref `Let` | T2, §4.4 |
| `Pattern::Tag` structural patterns | T3, §5.7 |
| exactly-once use / `clone` / `drop` | already enforced by `check_module_linearity`; liquid never weakens it |
| errors at the violating node | span-accurate diagnostics via `LiquidParse`/blame mapping |

vs. stages (`PLAN_liquid_types.md`):

| Spec section | Stage |
|--------------|-------|
| this document | L0 |
| §3 surface syntax | L2 |
| §1 totality gate | L3 |
| §5 judgment, §8.1 VC gen | L4 |
| §8.2 discharge, blame | L5 |
| §9 inference | L6 |
| §5.11, §7 foreign contracts | L7 |
| §7 attestation counts | L8 |
| §10 erasure guarantee | L9 |
| §8.3 Cycles A/B | L10 |
