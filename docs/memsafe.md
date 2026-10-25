# memsafe — translation validation for the AustralVM compiler, adapted

> **Adapted 2026-10-25 against the tree.** The generic plan this document
> replaced proposed a `Strategy B` for a project that had none of its parts:
> a fresh `austral.mm0` (~450 lines), an OCaml `Mm0Emitter.ml` writing packed
> `.mmb` bytecode directly, and instrumentation of `lib/Linearity.ml`,
> `lib/BorrowCheck.ml` and `lib/Liquid.ml` — three files that do not exist
> with those shapes. Measured against what *is* here, the plan needed three
> corrections:
>
> - **The certificate pipeline already exists.** `docs/lean4mm0.md` landed
>   Phase D: `docs/cic.mm0`, a stdlib-only emitter (`scripts/lean2mm0`),
>   `mm0/{gap,layout}_certificate.{mm0,mm1}`, and `mm0-c` as the frozen root
>   of trust, all gated in `scripts/verify-invariants`. A second emitter
>   writing `.mmb` bytes in OCaml would duplicate the toolchain, not add
>   assurance.
> - **One of the three properties is already discharged, by a different
>   mechanism.** Liquid refinements are Why3 verification conditions
>   (`lib/liquid/`), and the gate now proves every golden under z3. Routing
>   them through MM0 would require an Alethe→MM0 translation and would
>   *weaken* what is currently checked (see §7).
> - **The linearity property had no artifact at all.** `lib/LinearityCheck.ml`
>   is a complete linearity *and* borrow checker, and it threw every decision
>   away. That is the gap this iteration closes: the checker now emits a
>   ledger, and the existing certificate machinery certifies it.
>
> **Improving the existing feature outranks adding a new one** — so the
> deliverable is one new module (`lib/LinearityLedger.ml`), hooks at the
> checker's existing decision points, one new theory, and one new emitter that
> follows `scripts/lean2mm0`'s modes, budget and mutation anchors rather than
> inventing its own conventions.

---

## 1. Generic plan → concrete project

```text
  AustralVM source (.au)
        │
        ▼
  ┌───────────────────────────────────────────────────────────────────────┐
  │ Compiler frontend (OCaml, UNTRUSTED for this argument)                │
  │  Parser → TypingPass → LinearityCheck ──┐                             │
  │                                          │  decisions (EXISTS+NEW)    │
  │                                          ▼                             │
  │                            lib/LinearityLedger.ml  ──► ledger JSON     │
  │                            (`--emit-linearity-ledger`)                │
  └──────────────────────────────────────────┬────────────────────────────┘
                                             │ timelines, one per
                                             │ (linear var, execution path)
                                             ▼
  ┌───────────────────────────────────────────────────────────────────────┐
  │ scripts/ledger2mm0 (NEW; same modes as scripts/lean2mm0)              │
  │   • audit: replay each timeline in a second, independent automaton    │
  │   • emit: one `run_cons` step per event, against docs/memsafe.mm0     │
  │   • budget: 17 certificates, structural=17 grounded=0 — pinned        │
  └──────────────────────────────────────────┬────────────────────────────┘
                                             ▼
                              mm0/linearity_certificate.{mm0,mm1}
                                             │  mm0-rs elaborate
                                             ▼
                                  ┌──────────────────────┐
                                  │   mm0-c (frozen)     │ ──► every
                                  │  ~3k lines C verifier│     transition
                                  └──────────────────────┘     re-checked
```

The compiler stays outside the TCB in exactly the sense the generic plan
wanted: it produces a *witness* (the ledger), a small program (the emitter)
turns the witness into a proof, and a verifier nobody in this repo can edit
informally (`mm0-c`) decides whether the proof stands.

### Mapping table

| Generic plan item | Concrete component | Status |
|---|---|---|
| §1.2 `austral.mm0` "formal specification" (~450 lines) | `docs/memsafe.mm0` (116 lines): the linear-resource automaton + run relation | **Adapted** — states the property that is actually emitted, not a hoped-for memory model |
| §2.1 instrument `lib/Linearity.ml` | `lib/LinearityCheck.ml` hooks → `lib/LinearityLedger.ml` | **Exists + instrumented (this iteration)** |
| §2.2 instrument `lib/BorrowCheck.ml` | same checker: `TBorrow` already tracks `BorrowedRead`/`BorrowedWrite`; borrow start/end/reborrow/drop are ledger events | **Exists + instrumented** — there is no separate borrow pass to hook |
| §2.3 SMT proof logs → MM0 (`lib/SmtToMm0.ml`) | Why3 `.mlw` goldens discharged by z3 (`lib/liquid/golden/`, gate in `scripts/verify-invariants`) | **Deferred, by design** — see §7.1: the mechanism exists, the Alethe→MM0 bridge is a different project |
| §3 DeltaNets equivalence hook | `lib/deltanet_plugin.ml` + `uk_austral_defeq` + `net_isomorphism_step` in `docs/cic.mm0` | **Exists** — `docs/lean4mm0.md` §9, gated |
| §4 `lib/Mm0Emitter.ml` (binary `.mmb` writer) | `scripts/lean2mm0` → `.mm1` → `mm0-rs compile` → `.mmb` | **Exists** — a hand-rolled binary writer would be a second, unreviewed encoder |
| §4 mm0-c root of trust | `mm0-c <stem>.mmb < <stem>.mm0` | **Exists** — same verifier, second subject |
| §5 `--certified` driver flag deleting the binary on failure | `scripts/verify-invariants` + CI `phase-d` job | **Adapted** — the gate refuses a drifted or unproved certificate; it does not gate a *binary*, because the certificate is about the frontend's decisions, not the emitted C |
| §6 9-month, 2–3 engineer roadmap | §8 | **Rescoped** — the machinery existed; this iteration is the delta |

---

## 2. Executed this iteration (2026-10-25)

All of it lands inside existing seams. Nothing was rewritten.

* **`lib/LinearityLedger.ml` / `.mli` (new, ~250 lines).** An event
  vocabulary (`Acquire`, `Consume`, `Reacquire`, `BorrowRead`, `BorrowWrite`,
  `Reborrow`, `Unborrow`, `Drop`), branch paths, resolution of rows into
  per-path timelines, and the JSON export. Recording is *off* by default:
  `LinearLedger.record` returns immediately unless the flag turned it on, so
  ordinary compilation cannot be affected by a defect here.
* **`lib/LinearityCheck.ml` instrumented at its own decision points** —
  `add_entry` (acquire), `consume_once` (the one legal consumption),
  `remove_entry` (drop of a write reference), the two assignment paths
  (reacquire), `TBorrow`'s three branches (borrow start, nested read borrow,
  borrow end), `TIf`/`TCase` arms (branch paths, §3.1), and the end of a body
  (a write reference still owned is discharged by exit). `linearity_check`
  gained `?name`/`?kind`, which label the function in the ledger and change
  nothing about checking.
* **`--emit-linearity-ledger=<path>`** — `CliParser`, `CliEngine`,
  `Compiler.emit_linearity_ledger_path`, merging per module like
  `--emit-derivations` does for plan JSON.
* **`docs/memsafe.mm0` (new, 116 lines).** The automaton (`st_idle`,
  `st_owned`, `st_read`, `st_write` × 11 edges) and the run relation
  `run : L → St → St → Prop`, in hypothesis-binder form so the rules are
  applicable (the lesson `docs/cic.mm0` records).
* **`scripts/ledger2mm0` (new).** `generate` / `check` / `verify` /
  `selftest`, `BUDGETS` pinned per artifact, coverage and transition lines
  emitted into the artifact, and two mutation anchors (§6).
* **`mm0/fixtures/linearity_ledger.aum`**, **`mm0/ledgers/linearity_ledger.json`**
  (pinned, byte-stable across runs — verified by recompiling and diffing), and
  **`mm0/linearity_certificate.{mm0,mm1}`** (217 lines each, 17 certificates).
* **`test/LinearityLedgerTest.ml`** — 9 cases over the path resolution (scope
  inheritance, arm separation, maximality, nesting) and the two wire contracts
  (`event_token` for the emitter, `event_rule` for the theory).
* **Gates.** Eight `memsafe:` items in `scripts/verify-invariants`; a CI step
  in the OCaml `test` job (which is the only job that builds `austral.exe`,
  hence the only place the ledger can be re-derived) and `ledger2mm0 verify`
  in the `phase-d` job (the only job with the mm0 toolchain).

Measured here: `python3 scripts/ledger2mm0 verify` → `mm0-rs compile OK
(linearity_certificate) — 4 sorts, 33 term/def, 30 ax/thm`, `mm0-c OK`, and
`mutated timeline rejected`. `scripts/verify-invariants` → **52 passed, 0
explained gaps, 0 failed**. `dune runtest --force` → 160 cases across 18
suites, all OK.

---

## 3. The ledger

A row is `(variable, type, loop depth, event, branch path)`. Two design
decisions make the trace checkable rather than decorative.

### 3.1 Branch paths: a tree of decisions, flattened per path

The checker walks *both* arms of an `if` from the same table and then demands
the arms agree (`tables_are_consistent`). A flat event stream would therefore
show a variable consumed twice — once per arm — and no automaton could accept
it. So each arm is recorded under a branch path: rows inside the then-arm
carry `[(branch_id, 0)]`, the else-arm `[(branch_id, 1)]`.

Export resolves rows into **one timeline per (variable, maximal path)**: a row
belongs to a timeline when its path is an outer prefix of the timeline's path
(events before the `if` are on both arms' timelines; events inside one arm are
on that arm only). Non-maximal paths are dropped — a variable consumed before
an `if` must not also produce an unbalanced "entry" claim.

The payoff is that branch consistency, which the checker enforces with a table
comparison, becomes a *property of the trace*: each complete path must be a
run ending at `st_idle`. `mm0/fixtures/linearity_ledger.aum`'s
`consume_in_both_arms` produces exactly two certificates, one per arm.

### 3.2 The fixture is the certificate's premise

`mm0/fixtures/linearity_ledger.aum` (11 small functions) exists to make every
declared transition reachable:

| Shape | Events | Edges |
|---|---|---|
| parameter/local consumed by destructure or call | `acq, cons` | `t_acq`, `t_cons` |
| write-borrow block | `acq, bw, unbor, cons` | `t_bw`, `t_unbor_write` |
| write reference read through | `acq, bor, unbor, drop` | `t_bor`, `t_drop` |
| nested read borrow | `acq, bor, bor, unbor, unbor, cons` | `t_bor_nested`, `t_unbor_idle` |
| `&~w` reborrow | `acq, reb, unbor, drop` | `t_reb` |
| consume + reassign in a loop | `acq, cons, reacq, cons` | `t_reacq` |
| `if` consuming in both arms | two timelines | branch paths |

`scripts/verify-invariants` derives the edge list from `docs/memsafe.mm0` and
requires every `axiom t_*` to appear in the artifact's
`-- transitions exercised:` line — a transition added to the theory without a
fixture that reaches it fails the gate instead of shipping untested.

---

## 4. The theory (`docs/memsafe.mm0`)

The generic plan wanted "linear states as multisets" with `state_add` /
`state_remove` axioms. Multiset equality needs induction; MM0 has none, so
such a proof can only be closed by *postulating* the fact it was supposed to
check. The ledger makes the claim decidable instead, and a decidable claim can
be checked step by step:

```text
st_idle --e_acq-->  st_owned --e_bor-->  st_read --e_unbor--> st_owned
                            --e_bw--->   st_write --e_unbor-> st_owned
                            --e_reb-->   st_write
                            --e_cons-->  st_idle          (the one use)
                            --e_drop-->  st_idle          (implicit release)
st_idle --e_reacq-> st_owned
st_read --e_bor-->  st_read           (nested read borrow)
st_owned --e_unbor-> st_owned         (see §5.1)
```

Every certificate is `run <timeline> st_idle st_idle`, derived by
`run_cons` once per event with `run_nil` at the end. There is no grounded
class: `-- trust-split: structural=17 grounded=0`. That is the sharpest
difference from the Lean-side certificates, where 5 of 8 `layout_certificate`
definitions rest on the emitter's own evaluation; here the verifier checks
every step, and the artifact says so.

---

## 5. What the certificate establishes — and what it does not

**Established.** For every linear variable on every execution path of the
fixture, the sequence of decisions the checker made is a legal run of the
automaton: acquired at most once before ownership, used at most once, not used
while the checker holds the value borrowed, and released (consumed or
discharged) by the end. The automaton's *missing* edges are the illegal
programs — a double consumption has no `trans`, so no proof exists. That is
machine-checked by `mm0-c` for each of the 17 timelines, not asserted.

**Not established.** The ledger is produced by the compiler, which is
untrusted: the certificate is about the *trace*, and the trace's fidelity to
the program rests on `LinearityCheck`. What the pipeline buys is that the
checker's *bookkeeping* — per-path balance, the drop rules, the borrow
transitions — is re-derived by an independent implementation
(`scripts/ledger2mm0`'s Python automaton) and then machine-checked. A checker
that accepted an unbalanced path would emit a ledger for which no certificate
exists, and `verify-invariants` fails. A parser or C-lowering bug is entirely
outside this argument (§7.2).

### 5.1 Finding: borrow nesting is not counted

`TBorrow`'s second branch (a read borrow of an already read-borrowed value)
resets the original to `Unconsumed` when the inner body ends, while the outer
borrow is still live — the checker does not count borrow depth. The ledger
therefore emits two `unborrow` events for one `st_read`, and the automaton
needs the idempotent edge `t_unbor_idle` to accept the genuine trace.

The edge enables no new `consume` (consumption is only legal from `st_owned`),
so conservation is unaffected, but borrow exclusivity is exactly as strong as
the checker — no stronger. Recording the finding here rather than hiding it in
an emitted artifact is the point: a certificate that quietly dropped the
second event would have claimed a property the checker does not have.

### 5.2 Finding: linear parameters can be discharged without a scope

`remove_entry` records `Drop` for a write reference leaving a `let` or borrow
scope, but a write-reference *parameter* is never removed — the body just
ends. `TReturn` explicitly tolerates it. Without a hook at the end of the
body, `read_ref(w: &![Handle, R])`'s ledger showed `w` as never released and
the emitter refused to certify it. The hook makes the implicit rule explicit;
it is the same rule `remove_entry` and `TReturn` already encoded.

---

## 6. Gates and commands

```bash
# the artifact half (needs python3 only)
python3 scripts/ledger2mm0 check      # artifacts in sync with ledger + theory
python3 scripts/ledger2mm0 selftest   # the audit refuses an unreleased resource
# the root-of-trust half (needs mm0-rs + mm0-c)
python3 scripts/ledger2mm0 verify     # + mm0-c, + a mutated-timeline rejection
# re-derive the ledger from the fixture (needs the compiler and its .so)
./austral compile mm0/fixtures/linearity_ledger.aum --target-type=c \
    --no-entrypoint --output=/tmp/ll.c \
    --emit-linearity-ledger=mm0/ledgers/linearity_ledger.json
# everything at once
bash scripts/verify-invariants
```

`verify` fails when mm0-rs emits a *warning*: a warning is a proof hole (the
`sorry` path), the same stance as `scripts/lean2mm0`. A missing toolchain is
exit 2 / an explained gap, never a silent pass.

**The mutation anchors.** A certificate pipeline's characteristic failure is a
proof that would verify anything. Two mutations make that failure loud:

| Mutation | Anchor that must trip |
|---|---|
| drop a timeline's last event (resource never released) | the emitter's own audit: `timeline ends in st_owned, not st_idle` — no toolchain needed |
| splice an extra `cons` into a verified timeline, leave the proof | `mm0-rs`: `failed to unify: trans st_idle e_cons ?n =?= trans st_owned e_cons st_idle` |

The second is the one that matters: it runs against a certificate `mm0-rs` has
just been shown to accept, so the rejection cannot be blamed on the input.

Where the mm0 toolchain comes from: `docs/lean4mm0.md` §5 (built from
`digama0/mm0` at commit `0d414c0`; `mm0-rs` elaborates, `mm0-c` is the
frozen verifier). CI's `phase-d` job rebuilds both and runs
`ledger2mm0 verify`.

---

## 7. Deferred, with the prerequisite named

### 7.1 Liquid refinements → MM0 (the generic plan's §2.3)

Currently: TRL contracts become Why3 verification conditions
(`lib/liquid/LiquidConstraints.mlw` → `.mlw`), and `scripts/verify-invariants`
proves every golden under z3, failing on `Invalid` or `Unknown`. That is a
real discharge, checked on every gate run, but the *prover* is inside the
trusted path for it.

Prerequisite for the generic plan's version: an Alethe (or LFSC) proof log
from z3 that survives translation to a first-order MM0 arithmetic theory, plus
a decision about array/extensional reasoning. That is a multi-week theory
project — and until it exists, replacing the z3 gate with a "grounded"
MM0 certificate would *reduce* assurance, which is why it is not done here.
`docs/cic.mm0`'s `teval` grounding is the small, honest version of the same
idea and shows the cost: every grounded fact is emitter-evaluated.

### 7.2 C99 lowering equivalence (the generic plan's Phase 5)

`CRenderer`/`CRepr` have no formal model, no translation map, and no
verified C compiler in the loop, so "the emitted C is the program" remains an
assumption. Prerequisites, in the order that makes each one useful:
a per-construct structural lowering map (the generic plan's "1:1 structural
correspondence"), then a semantics for the fragment, then either CompCert in
the loop or a proof-carrying lowering. Nothing in this iteration pretends
otherwise: `mm0/linearity_certificate.*` is about the frontend's linearity
decisions, and its header says so.

### 7.3 Strengthening borrow-exclusivity (§5.1)

Count borrow depth in `LinearityCheck` (a per-variable depth alongside the
`BorrowedRead`/`BorrowedWrite` state), then delete `t_unbor_idle` and the
`e_unbor` events it produces. The prerequisite is the checker change, and it
should land with a test that consumes a value while an outer read borrow is
live — currently *accepted*, which is the looseness the edge encodes. Doing it
in the ledger alone would have made the ledger disagree with the checker.

### 7.4 Parallel reduction / multicore

Out of scope here, unchanged from `docs/lean4mm0.md` (§5: Phase E follows
`PLAN_parallel_australvm.md` and must keep the certificate outputs
bit-identical).

---

## 8. Resource note

The generic plan estimated ~450 lines of MM0, ~1,800 lines of OCaml emitter,
and 9 months for 2–3 engineers. Measured in this tree: the theory is 116
lines, the ledger is ~275 lines of OCaml (+116 of interface) plus ~30 hook
lines in the checker, the emitter is ~530 lines of stdlib-only Python, and the artifacts are 217
lines each. The reason it is that small is not efficiency — it is that the
certificate was scoped to a property the checker *already computes*, so the
work was instrumentation and proof shape rather than a second semantics of
memory. The two large items the generic plan wanted (Alethe→MM0, a C99
semantics) are still large, and §7 says what each needs first.
