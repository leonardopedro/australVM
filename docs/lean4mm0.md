# Lean 4 → DeltaNets → MM0 — Adapted Plan

> **Adapted 2026-10-23; re-grounded 2026-10-25** against the tree. The
> original generic plan proposed *building* four modules (a Nanoda↔AustralVM
> flat-AST C ABI, an Austral-language DeltaNets engine, a `cic.mm0`, and an
> mm0-rs synthesizer). Three of the four already existed here in
> better-integrated form, and Phases C and D have since landed — so this plan
> **reweights toward improving the existing implementation**, per the standing
> rule:
>
> **Improving an existing feature outranks adding a new one. There is
> exactly one DeltaNets implementation in this project
> (`unfer/logos/src/deltanet`), driven from AustralVM through
> `lib/deltanet_plugin.ml` and `prob_kernel` — it is improved here, never
> duplicated.**
>
> **State at 2026-10-25:** Phases A–D landed and green. Measured here:
> `cargo test -p logos --lib` clean (303 passed, 1 ignored, including the five
> reducer tests §9.1 adds), `scripts/lean2mm0 check` byte-identical, `selftest`
> refusing both tampered anchors, and `mm0-rs` + `mm0-c` accepting both
> certificates — the `phase-d` CI job runs the same three. §9 is what changed
> in this iteration: the reducer correctness pass, the trust-split budget, the
> coverage label, and the emitter self-test — all *inside* the existing
> pipeline, no new module.
>
> Numbers deliberately absent: `prob_kernel`'s test count is not quoted here
> because this host cannot rebuild that crate (its `gemm-f16` dependency
> assembles `+fullfp16` instructions and the host is `aarch64` without the
> feature), so any figure would be second-hand. The kernel path this document
> is about — `logos::deltanet` — is built and tested here.

---

## 1. Architecture: generic plan → concrete project

```text
  [ Lean 4 ]
      │  lean4export 3.1.0 (official leanprover/lean4export, NDJSON)
      ▼
  ┌────────────────────────────────────────────────────────────────────────┐
  │ 1. Nanoda verification (EXISTS — unfer/prob_kernel/src/verify.rs, S29) │
  │    • `verify_export` type-checks the NDJSON payload with nanoda_lib    │
  │      (linked in-process — no flat-AST C ABI needed; the generic        │
  │      plan's Module 1 FFI was superseded by this direct linkage).       │
  │    • Verdict: `ProofReport.verified`; hard-fail mode UK-4801.          │
  │    • Pinned fixtures: confluence.ndjson, layout_bijective.ndjson, …    │
  └───────────────────────────────────┬────────────────────────────────────┘
                                      │ conversion goals (defeq t₁ t₂)
                                      ▼
  ┌────────────────────────────────────────────────────────────────────────┐
  │ 2. DeltaNets normalization & canonicalization (EXISTS, IMPROVED —     │
  │    unfer/logos/src/deltanet, Rust, inside the unfer kernel)            │
  │    • `compile_to_net` → `reducer::reduce` (≤1M iters, free-list slot  │
  │      recycling, native Prim evaluation = the "graph-to-native trap"). │
  │    • Canonicalization: root-directed canonical serialization          │
  │      (`unf::canonical_serialize`) → `unf_hash` (SHA-256) — the        │
  │      content-addressed UNF identity. Algebraic layer: `ted` over      │
  │      ℤ/2⁶⁴ for the Int64 fragment.                                    │
  │    • **NEW this iteration**: `unf::isomorphism_witness` — the         │
  │      conversion certificate π : V(G₁) → V(G₂) the generic plan's      │
  │      Module 2.3 promised, derived from the *same* canonical walk      │
  │      (no second traversal scheme, no new hash — docs/ENGRAM.md        │
  │      discipline) and re-checkable via `IsomorphismWitness::verifies`. │
  └───────────────────────────────────┬────────────────────────────────────┘
                                      │ equal UNF digests + witness π
                                      ▼
  ┌────────────────────────────────────────────────────────────────────────┐
  │ 3. Conversion-check seam (EXISTS — this iteration)                     │
  │    • `prob_kernel::logos::austral_defeq(a, b)` — in-process kernel    │
  │      API: reduce both sides, report `equivalent`, both UNF and TED    │
  │      digests, and the witness.                                         │
  │    • `uk_austral_defeq` — the C ABI symbol (S29-registered: registry  │
  │      row, EXPECTED_SYMBOLS.txt, generated header, UNFER_SYMBOLS,      │
  │      ProverRunner preset, UK-4805 failure code, `austral_defeq` event).│
  │    • `logos iso <a> <b> [--json]` — the same check as a CLI          │
  │      subprocess (exit 0 equivalent / 2 not equivalent / 1 failed),    │
  │      the shape `formalize`/`Deltanet_plugin`-style callers parse.     │
  │    • `Deltanet_plugin` now asks the *conversion* question: the       │
  │      initializer and the compiler's evaluated literal must reduce to  │
  │      the same net, certificate included (`austral_defeq_translate`    │
  │      bridge). Equal nets ⇒ equal values, so this subsumes the legacy  │
  │      float-comparison gate (kept as fallback; it was lossy above 2^53 │
  │      and never equal for NaN).                                         │
  │    • `deltanet::unf` remains the sole hash authority; nothing here    │
  │      introduces a second net representation.                           │
  └───────────────────────────────────┬────────────────────────────────────┘
                                      │ Phase D: .mmb emission (toolchain
                                      │ provisioned, see §5) →
                                      ▼
  ┌────────────────────────────────────────────────────────────────────────┐
  │ 4. MM0 target theory (SPEC EXISTS — docs/cic.mm0, adapted)             │
  │    • Rewritten from generic CIC to the *actual* agent signature:      │
  │      tapp/tabs/tcon/tfold/tdup/tera/tprim/lits/tentity + teq + the     │
  │      certificate rule `net_isomorphism_step`.                          │
  │    • Emission (mm0-rs) and the mm0-c root of trust: Phase D, §5.       │
  └────────────────────────────────────────────────────────────────────────┘
```

### Mapping table

| Generic plan module | Concrete component | Status |
|---|---|---|
| §1 Nanoda flat-AST C ABI (`bridge.h`) | `prob_kernel::verify::verify_export` (nanoda_lib linked in-process, S29) | **Exists** — FFI superseded; no new ABI surface needed |
| §2 Austral-language `DeltaNet.Core/Parallel/Canonicalize` | `unfer/logos/src/deltanet` (Rust: `compiler`, `reducer`, `readback`, `symbolic`, `ted`, `unf`) | **Exists** — improved, not duplicated |
| §2.1 linear node pool / slot recycling | `Net::free_list` + `freed` forwarding stubs in `deltanet::types` | Exists (single-threaded) |
| §2.2 multicore work-stealing CAS loop | — | Deferred → `PLAN_parallel_australvm.md` (§5); correctness pass landed, §9 |
| §2.3 WL canonicalization + isomorphism witness π | `unf::canonical_serialize` (root-directed DFS, backrefs for cycles) + **`unf::isomorphism_witness`** | **Exists + improved (this iteration)** |
| §1/§4 conversion-check C ABI `reduce_and_check_isomorphism` | **`prob_kernel::logos::austral_defeq` + `uk_austral_defeq` + `logos iso`** | **Exists + S29-registered (this iteration)** |
| §3 `cic.mm0` (generic CIC) | `docs/cic.mm0` (agent-signature theory + `net_isomorphism_step`) | **Spec adapted (this iteration)** |
| §4 mm0-rs `.mmb` synthesizer | `scripts/lean2mm0` → `mm0/*.mm1` → `mm0-rs compile` | **Exists** — Phase D, §7 |
| §4 mm0-c root of trust | `mm0-c <stem>.mmb < <stem>.mm0` | **Exists** — Phase D, §7, gated in CI |
| AustralVM compiler-side gate | `lib/deltanet_plugin.ml` (`deltanet_unf` pass, via `CamlCompiler_rust_bridge.austral_unf_json` + `austral_defeq_json`) + `lib/formalize_plugin.ml` (`logos unf` subprocess) | Exists — conversion-gated (this iteration) |

The Austral→DeltaNets compiler cycle (S36/S37 in `unfer/docs/LOGOS.md`):
`Deltanet_plugin` serializes every top-level constant to the Austral subset,
calls the kernel's `uk_austral_unf` through the cranelift bridge, and rejects
the module when the kernel's UNF value disagrees with the compiler's own
evaluation. The kernel's unique normal form is the independent arbiter —
exactly the "call the kernel from the compiler" direction this plan needs for
conversion checking, so Phase C only has to *extend* that seam with the
witness, not build a new one.

---

## 2. Design rules this adaptation follows

1. **Improve, don't duplicate.** The witness (Module 2.3's missing piece) is
   ~200 lines *inside the existing `unf.rs`*, derived from the canonical
   serializer's own visit order. A second traversal scheme would be a second
   thing to trust.
2. **No new hash, no second net representation** (`docs/ENGRAM.md`
   discipline). The witness adds bytes to no digest; `unf_hash` and
   `ted_hash` values are unchanged, so every pinned fixture
   (`prob_kernel/tests/fixtures/*.ndjson`, engram keys, `uk_austral_unf`
   reports) stays valid.
3. **Hash stability is a contract.** `unf.rs` tags are load-bearing: the
   five legacy tag bytes and the `prim_tag` discriminant mapping must not
   change; the witness rides on the same serialization.
4. **One dispatch seam.** Source-shape dispatch (module body / function /
   statement list / expression) was duplicated in `prob_kernel` and now
   lives once in `logos::translate::translate_austral_any`, shared by the
   kernel op, the CLI `iso` command, and any future `uk_*` symbol.
5. **Certificates are re-checkable.** A witness is only as good as the
   traversal that produced it, so `IsomorphismWitness::verifies`
   re-walks both nets, requires the pairing to be exactly the canonical zip,
   and checks kind- and wiring-preservation under π. Emitters must call it.
6. **New `uk_*` symbols stay behind the S29 checklist** (EXPECTED_SYMBOLS
   txt, generated C header, australVM `UNFER_SYMBOLS`, `GrantSet.kernel`).
   Applied in full for `uk_austral_defeq` (Phase C, executed — see §3).

---

## 3. Executed this iteration (2026-10-23)

All items below are landed and tested.

* **Isomorphism witness** — `unfer/logos/src/deltanet/unf.rs`:
  `IsomorphismWitness` + `isomorphism_witness(g1, g2)` + `canonical_walk` +
  `verifies`. Handles freed-node forwarding stubs, sharing, cycles
  (back-references), unwired aux ports; refuses (`None`) on unequal
  serializations; errors propagate the depth guard. Tests: allocation-order
  permutation, tamper rejection, sharing, cycles, end-to-end over reduced
  translations.
* **Reduced nets carried on the translation** —
  `logos::translate::UnfTranslation.net`, so conversion checks reuse the
  nets the pipeline already reduced instead of re-compiling.
* **Shared ingestion dispatch** — `logos::translate::translate_austral_any`;
  `prob_kernel::logos` now delegates to it (dedup, same semantics).
* **Conversion-check kernel API** — `prob_kernel::logos::austral_defeq` +
  `AustralDefeqReport` (equivalence verdict, both UNF digests, both TED
  digests, witness). Tests pin the net-vs-algebraic distinction:
  `(x + x)` vs `(2 * x)` are *not* net-equal but share a `ted_hash`.
* **CLI conversion check** — `logos iso <a> <b> [--json]` with the
  `unf`-style exit-code contract (0/2/1) and tab-separated or JSON output.
* **Adapted target theory** — `australVM/docs/cic.mm0`: MM0 spec over the
  real agent signature, with `valid_iso` + `net_isomorphism_step` as the
  certificate rule. **Validated 2026-10-23 with the real toolchain**:
  `mm0-rs compile` elaborates it and `mm0-c` verifies the emitted `.mmb`
  back against it (both exit 0) — two syntax fixes the validator forced
  (`provable sort Prop`, a declared `infixr` implication, and a
  `delimiter` declaration for parentheses in math strings).
* **This document** — the plan rewritten against the concrete project.

### Executed this iteration (2026-10-23, continuation — Phase C + toolchain)

* **mm0 toolchain provisioned** — `mm0-rs` (Rust elaborator/compiler) and
  `mm0-c` (the C root-of-trust verifier) built from
  `github.com/digama0/mm0` and installed to `~/.local/bin` on this host;
  system-wide: `sudo install -m755 mm0-rs mm0-c /usr/local/bin/`. Requires
  rustc + gcc only — no sudo needed to build.
* **`cic.mm0` validated end-to-end** — `mm0-rs compile cic.mm0 -> cic.mmb`
  (3 sorts, 20 terms, 11 axioms, exit 0), then `mm0-c cic.mmb < cic.mm0`
  (exit 0): the emitted binary checks back against the source with the
  frozen verifier. The validator forced three fixes (declared `infixr`
  implication, `provable sort Prop`, a `delimiter` declaration for parens).
* **`uk_austral_defeq` registered (S29, complete)** — one registry row
  (`Observe`-kind) in `unfer_protocol::symbols`; `scripts/gen_symbol_artifacts`
  regenerated `EXPECTED_SYMBOLS.txt` (92 uk_) and `unfer_kernel.h`; new
  `KernelEvent::AustralDefeq` with its `handles.rs` matcher; UK-4805
  (`AUSTRAL_DEFEQ_FAILED`, consts + names table); `GrantSet` ProverRunner
  preset extended; australVM `UNFER_SYMBOLS` row; FFI tests for the
  witness-carrying report, the not-equivalent-yet-success verdict, the
  UK-4805 path and result-channel freshness (S46).
* **`Deltanet_plugin` conversion gate** — the pass now asks the *conversion*
  question: initializer `src` vs the compiler's evaluated literal as source
  (`uk_austral_defeq` via the new `austral_defeq_translate` bridge),
  certificate included. Equal nets ⇒ equal values, so the witness check
  subsumes the legacy float comparison — which was lossy for Int64
  magnitudes above 2^53 and never equal for NaN. The legacy comparison is
  kept as the fallback (bridge without the defeq surface, malformed report),
  and the "error must not pass silently" doctrine carries over. The bridge
  UNF/defeq now share one kernel model (`deltanet_bridge_model`).

Verification: `cargo test -p logos` (298 lib tests + integration suites,
all green), `cargo test -p prob_kernel --lib logos`, `cargo test
-p unfer_protocol`, `cargo test -p unfer_ffi --test ffi austral_defeq`,
australVM `make bridge` + `make test` (incl. `DeltanetPluginTest`) —
commands re-runnable; counts recorded by the runner, not trusted
from prose.

---

## 4. What the generic plan got right (kept)

* **Unique normal forms as identity.** Confluence is the backbone: formal
  in `unfer/logos/lean/Confluence.lean` (kernel-computed `rfl`, exported via
  lean4export 3.1.0, re-verified in nanoda) and cheap-checked at runtime by
  the fixed-point self-check in `translate_coreir`.
* **Proof-irrelevance analogue.** `ProofReport.verified` (nanoda) and the
  UNF digest (DeltaNets) are the same pattern: external engine → boolean +
  content-addressed certificate.
* **The mm0-c root of trust.** A small, frozen, auditable verifier outside
  the TCB of Lean/nanoda/AustralVM remains the end goal of Phase D.
* **Decoupling.** The reduction engine can be optimized (Phase E) without
  touching proof soundness, because certificates are *checked*
  (`verifies`), not trusted.

## 5. Deferred phases (not executed — prerequisites named)

* **Phase C — kernel symbol for the conversion check.** ✅ **Done
  (2026-10-23, iteration 2)** — `uk_austral_defeq` is S29-registered end to
  end and `Deltanet_plugin` gates on conversion with the witness; see §3.
* **Phase D — `.mmb` emission + mm0-c.** ✅ **Done (2026-10-23, iteration
  3)** — see §7: `australVM/scripts/lean2mm0` transpiles the pinned
  nanoda-verified lean4export of `timepiece/GapCertificate.lean` into
  `mm0/gap_certificate.{mm0,mm1}` over `docs/cic.mm0`, mm0-rs elaborates
  the proofs, and mm0-c re-verifies the `.mmb` (both exit 0); the round trip
  is gated in `scripts/verify-invariants`. (The earlier Phase-D note —
  toolchain provisioned, `cic.mm0` round trip green, theorem side
  remaining — is superseded by §7.)
* **Phase E — parallel reduction.** Lock-free multicore active-pair
  reduction stays *out* of this plan; when pursued it follows
  `PLAN_parallel_australvm.md` and must keep `unf_hash`/witness outputs
  bit-identical to the single-threaded reducer (same canonical walk, same
  digests) — that identity is the regression test.

---

## 7. Executed this iteration (2026-10-23, continuation — Phase D)

Phase D's remaining piece — the *theorem* side — is landed:

* **`cic.mm0` rule form fixed for applicability.** The structural rules
  stated their premises `->`-inside-formula; MM0 has no internal modus
  ponens, so such an axiom only proves an *implication term* is derivable
  and can never be *applied*. Restated with `>` hypothesis binders
  (`teq_symm`, `teq_trans`, `teq_app`, `teq_subst`, `tsubst_beta`,
  `fold_nil`, `fold_cons`, `net_isomorphism_step`), which is the form the
  generated proofs consume. `cic.mm0` revalidates (mm0-rs + mm0-c, exit 0).
* **The emitter — `australVM/scripts/lean2mm0`** (stdlib-only Python,
  ~1k lines). Input: the pinned lean4export NDJSON of
  `../timepiece/GapCertificate.lean` (sha256 in the artifact header), the
  same fixture nanoda re-verifies in unfer's prob_kernel suite. Output:
  `mm0/gap_certificate.{mm0,mm1}` — the two streams line-identical except
  `pub` and the proof tail (self-checked), with `docs/cic.mm0` embedded
  verbatim so either file stands alone for audit.
  * **Encoding**: each Lean def's unfolded body becomes a `Term` in the
    agent signature — `tint`/`ttrue`/`tfalse` literals, `tprim <prim_tag>`
    at the *exact* `unf.rs` byte tags, `tentity` for symbol-level
    applications (Nat.div → `idiv`). Lean-level recursion (pow10's brecOn)
    is *evaluated*, not encoded — the signature has no recursion agent,
    exactly what cic.mm0 prescribes for `teval`.
  * **Proofs**: congruence lifting (`teq_app`) to ground redexes, then
    `prim_eval` + a grounded `teval` bridge axiom per (op, a, b), or a
    direct grounding axiom for bool-valued prims / `not` / `idiv`
    (outside `prim_eval`'s tint codomain). Every grounding axiom is a
    trivially checkable decimal fact — the auditing stance.
  * **Self-verification before emitting**: every redex is evaluated by an
    independent evaluator pass and must equal the encoded literal.
* **Root of trust green**: `mm0-rs compile gap_certificate.mm1` (3 sorts,
  46 term/def, 39 ax/thm, no warnings — a warning is a proof hole here)
  and `mm0-c gap_certificate.mmb < gap_certificate.mm0` both exit 0. Nine
  certificates: read_gap/read_width, gap/width aggregates, lower/upper,
  lower_pos, upper_gt_lower, strong_coupling_consistency.
* **Modes**: `generate` (regenerate), `check` (drift gate, no toolchain
  needed), `verify` (check + mm0-rs + mm0-c; toolchain absence is a
  distinct exit 2, never a silent pass).
* **Gated**: `scripts/verify-invariants` Phase-D section runs the drift
  check always (python3) and the full verify when mm0-rs/mm0-c are on
  PATH, else records an explained gap — mirroring the why3 gates' stance.
  23 passed / 2 explained gaps (why3 absent) / 0 failed.
* **Second source — `layout_certificate` (follow-up iteration)**: the same
  emitter now drives a second pinned nanoda-verified export,
  `../timepiece/Layout.lean` → `mm0/layout_certificate.{mm0,mm1}` — eight
  rfl certificates over shapes gap never exercised: a source-defined
  inductive (`OuterState`), list constructors/recursors (`List.rec`/
  `casesOn` machinery), `Nat.mod`, `HMod`, and Bool chains over `&&`/`||`/`!`.
  The evaluator grew a generic ADT layer (constructor/recursor dispatch
  registered from the export's own `inductive` blocks) and demand-driven
  IHs — thunks forced only where the kernel would reduce them, which turns
  the 32×32-grid `eraseDup` scan from quadratic to linear (2.3 s, not 20+
  min). mm0-rs + mm0-c accept it (3 sorts, 34 term/def, 24 ax/thm).
* **Trust split — pinned per def, not prose**: the emitter tags every
  `<x>_checked` in the artifact itself and counts the classes in the file
  header, so the trust story cannot rot away from the code. The split:
  * `layout_certificate`: **3 structural / 5 grounded**. Structural:
    `conflict_pair`, `image_too_small`, `grid_image_small` — scalar prim
    spines (and/or/not, eq, lt) whose reduction mm0-c checks step by step
    via congruence, `prim_eval` bridges and grounded instances. Grounded:
    `bijective`, `lookup_inverse`, `insert_existing`, `insert_new`,
    `row_even` — list computations (`nodup`/`all`/`map` over
    `brecOn` machinery) with no agent in the signature, evaluated to
    `ttrue`; their anchor is nanoda's kernel-verified rfl plus the
    emitter's two independent paths (evaluator vs net reduction), and
    mm0-c verifies only the resulting tautology.
  * `gap_certificate`: **9 structural / 0 grounded** — the aggregates keep
    their full net spine even where subterms (pow10) were evaluated.
  * Structural ≠ fully checked by mm0-c alone: literal leaves inside a
    structural spine (`rowImage.length → 16`) are emitter-evaluated too —
    covered by the dual-path cross-check (evaluator value vs net reduction)
    that generation runs before anything is emitted.
  * **Why the grounded class cannot simply be encoded structurally.** The
    signature has `tnil`/`tcons`/`tfold`/`tabs`/`tapp`/`tsubst`, and `tfold`
    does reduce (`fold_nil`, `fold_cons`). What it cannot do is *compute*: MM0
    has no beta rule for `tapp (tabs b) a` — the theory gives
    `teq (tapp (tabs b) a) (tsubst b a)` with `tsubst` **opaque**, so a fold
    whose step function is a Lean lambda reaches `tsubst b a` and stops. A
    `row_even`-shaped def is `List.all` over a 32-element row: its normal form
    is a tower of `tsubst` applications, not `ttrue`. Encoding it structurally
    would produce a goal no axiom can close, so the emitter evaluates it and
    says so. Making those five structural would require a substitution
    calculus in the theory (a `tsubst` congruence with per-constructor rules) —
    a different, much larger theory, not an emitter fix.
* **CI**: `.github/workflows/ci.yml` has a `phase-d` job that rebuilds the
  frozen mm0 toolchain from digama0/mm0 at the host's pinned commit and
  runs `scripts/lean2mm0 verify` — both certificates re-checked by mm0-c on
  every push.

---

---

## 9. Executed this iteration (2026-10-25)

All inside the existing pipeline: no new module, no second reducer, no new
hash. Every item below is a property the document previously *stated* and now
*checks*.

### 9.1 Reducer correctness — the Phase-E prerequisite

The single-threaded reducer had four defects that surfaced as invalid reads
(`no port at node 3 slot 2`) or silently wrong readbacks, all in
`unfer/logos`:

* `deltanet/types.rs` — `alloc_node` recycled freed ids. A freed node is a
  *forwarding stub* that live ports legitimately point into, so recycling
  rewired walks to an unrelated node. Ids are no longer recycled (the free list
  is gone); `wire`/`get_aux`/`resolve_port` are bounds-guarded, as is the
  freed-node branch of `readback`/`symbolic`.
* `deltanet/reducer.rs` — `commute_dup_through` created one fresh Dup per aux
  port but wired each original port twice and left one of the Dup's outputs
  dangling. Both outputs now terminate on the copies' principals.
* `core_ir/linearity.rs` — the duplication insertion (ΔI/ΔA) wrapped the body
  in extra `Lam`s and left uses pointing at the same wire; multi-use binders
  now keep their name and get a `rename_uses` pass, and `insert_clone_chain`'s
  terminal Dup erases its surplus output instead of leaving an unconnected
  port.

Tests added in `linearity.rs`: `dup_in_lam_reduces_to_a_pair`,
`…_under_application`, `dup_and_let_alias_share_a_normal_form` (UNF-hash
equality), `drop_in_lam_inserts_an_eraser_and_reduces`,
`triple_use_fans_out_through_a_dup_chain`. This is what Phase E (parallel
reduction) must keep bit-identical, so it had to be right first.

### 9.2 Trust split is a budget, not a paragraph

`scripts/lean2mm0` now pins, per certificate, how many definitions are
structurally checked by `mm0-c` and how many are emitter-evaluated, and
**fails generation** when the grounded set widens or the certificate set
changes:

```
gap_certificate      structural=9 grounded=0 certificates=9
layout_certificate   structural=3 grounded=5 certificates=8
```

The reasoning: a regression inside the encoder (an unfold that stops firing, a
prim table entry that stops matching) would quietly move a definition from
*checked* to *evaluated* — the proof would still verify, because a grounded
definition's proof is the tautology `teq ttrue ttrue`. The artifact's header
already counted the split in prose; a prose count cannot fail. The count is now
in `BUDGETS` in the emitter, is emitted as a machine-readable
`-- trust-split: structural=N grounded=M certificates=K` line, and is gated by
`scripts/verify-invariants`.

Widening the budget is still allowed — deliberately, by editing the table in
the same commit that widens it, so the diff shows a human decided. That is the
same posture as the pinned lean4export fixtures.

### 9.3 Both trust anchors are tested by mutation

The document's load-bearing claim is the *dual-path cross-check*: the emitter
re-derives each checked def twice (Lean-side evaluation, and the net encoding's
reduction) and refuses to emit when they disagree. Until now nothing tested
that the refusal happens — a cross-check that never fires is
indistinguishable from one that cannot.

`scripts/lean2mm0 selftest` mutates one value on each path and requires the
emitter to raise:

| Mutation | Anchor it must trip |
|---|---|
| evaluator returns a flipped boolean | `Lean-side evaluation is … not \`true\`` — the kernel-rfl anchor |
| encoder's node value is flipped after encoding | `net reduction is … not \`true\`` — the evaluator-vs-net anchor |

Both run against the real pinned exports, so the mutation is applied to a
computation the emitter has already been shown to accept. Unmutated generation
is asserted clean in the same run. Gated by `scripts/verify-invariants` and by
the `phase-d` CI job (no mm0 toolchain needed: the mutation is caught before
any `.mmb` is produced).

### 9.4 Theory substrate is labelled, not implied

`docs/cic.mm0` declares more of the agent signature than the two certificates
exercise. Rather than describe that in prose (which goes stale the moment the
encoder changes), each artifact now carries the answer it was generated from:

```
-- theory coverage: used=tapp,tint,ttrue,tfalse,tprim,tentity,teval
--   unused=tvar,tabs,tf64,tcon,tnil,tcons,tfold,tdup,tera,tsubst
```

(`gap_certificate`; `layout_certificate` differs only in that it never emits
`tfalse`.) The line is scanned from the *generated* declarations only — the
`.mm0` embeds `cic.mm0` verbatim for audit, so scanning the whole file would
mark everything used — and it is part of the drift gate, so a change in what
the encoder emits shows up as a diff.

What the unused set means, precisely:

- `tcon`/`tnil`/`tcons`/`tfold` are the constructor spine and the structural
  fold. The encoder *can* emit them (`const_node` on a surviving constructor
  application), but every list-shaped computation in these two sources is
  grounded, for the reason §7 gives: without a `tsubst` calculus a fold over a
  lambda normalizes to a tower of opaque `tsubst` applications, never to
  `ttrue`. So the spine support is exercised by construction, not by these
  certificates.
- `tabs`/`tvar` are the binder and the De Bruijn index: the two sources have
  no open terms (every certificate is a closed `Bool` computation), and
  `pow10`'s `brecOn` machinery is evaluated rather than encoded.
- `tf64` is the F64 literal. Neither source computes on floats.
- `tdup`/`tera` are the duplication and erasure agents. Ground `Bool`
  computations never need them; they appear in the *net* side of the pipeline
  (`deltanet::reducer`), which is not what this theory certifies.
- `tsubst` is the opaque substitution term of `tsubst_beta`. It is declared
  because the beta rule needs it; it is unused because nothing here reasons
  about a beta redex it cannot evaluate.

A later reader can therefore tell a checked rule from a reserved one without
trusting a paragraph: the paragraph is generated from the same pass. The drift
gate keeps it honest in one direction (artifact vs regeneration);
`scripts/verify-invariants` keeps it honest in the other, by requiring every
`used=` name to be a `term` the theory declares — a name invented by the
encoder would otherwise produce a coverage line that reads complete and a
certificate `mm0-rs` rejects at the next gate.

---

## 10. Resource note (revised)

The generic plan estimated ~2,500 LOC for a new DeltaNets engine, ~1,200 for
Nanoda instrumentation and ~1,800 for an mm0 synthesizer against a 2–3
engineer team over 10 months. Measured in this tree: the engine is ~2.5k lines
of Rust that already existed, `scripts/lean2mm0` is ~1.5k lines of stdlib-only
Python, `docs/cic.mm0` is 122 lines, and this iteration's delta is the
reducer-fix set (§9.1) plus ~120 lines of emitter/gate code (§9.2–9.3). The
remaining genuinely-new work is the substitution calculus §7 names — sized in
weeks, and independently verifiable at each step.

---

## 11. Wave 3 — the statement of record over set.mm (2026-10-29)

§7's certificates prove closed Boolean reductions over an *embedded*
`cic.mm0`. This layer carries the **statement itself** — the quasi-Riemann
hypothesis at 7/8 — into the real Metamath corpus, importing set.mm instead
of re-axiomatizing it:

* **Artifact.** `mm0/quasi_rh_7_8.{mm0,mm1}` (51 lines) is generated from
  the same pinned export as `quasi_rh_certificate` (the `STATEMENTS` table
  in `scripts/lean2mm0`). The whole statement is one axiom:

  ```
  import "metamath/set.mm0";   (the .mm1 imports "metamath/set.mmu")
  delimiter $ ( $ $ ) $;
  axiom riemannZeta_ne_zero_of_seven_eighths_lt_re (s: class):
    $ wcel s cc $ > $ wbr (co c7 cdiv c8) (cfv s cre) clt $ > $ wne (cfv s czeta) cc0 $;
  ```

  which reads `s ∈ CC → ( 7 / 8 ) < ( Re ` s ) → ( ζ ` s ) ≠ 0` — exactly
  the source-of-record statement in `timepiece/QuasiRiemannCertificate.lean`.
  The two files are line-identical except the import line (self-checked):
  mm0-rs reads imported *proofs* from an imported `.mm1`/`.mmu`, never from
  a spec, hence `.mm0` imports `set.mm0` and `.mm1` imports `set.mmu`.

* **Argument order — the trap in this layer.** MM0 parameters are in
  set.mm's `$f` declaration order, not the surface-text order of the
  Metamath math string: `wbr(A,B,R)` is `A R B`, `co(a,b,F)` is `( a F b )`,
  `cfv(point,F)` is `( F ` point )`. Grounded by reading `term` lines and
  real theorems (`reval`, `df_zeta`) in `set.mm0`; the artifact header
  records the resulting mapping table.

* **Three anchors, all mutation-tested by `selftest`:**
  1. *shape* — the export's value must parse as
     `∀ s : Complex, boundScaled < complexReScaled s → riemannZeta s ≠ complexZero`
     with every atom resolved by name (Complex / LT.lt / Ne / …), so a
     fixture whose statement changed is refused, not reinterpreted;
  2. *bound* — `boundScaled`/`denom`/`boundMantissa`/`boundExp` are
     *evaluated* from the pinned export (8750/10000/8750/4) and tied to
     seven-eighths by cross-multiplication (`8·8750 = 7·10000`) before
     `( 7 / 8 )` is emitted — the numeral is the Lean def's value, not a
     comment. A flipped numerator must be refused;
  3. *label contract* — the 13 set.mm labels the axiom may name, with the
     sorts set.mm declares them at, are pinned in `SETMM_LABELS` and
     grounded against `metamath/set.mm0` by `verify` (existence + sort).

* **Verify chain.** Same three gates as the certificate path, but the
  streams are written under a tempdir whose `metamath/` symlink lets
  `import` resolve: `mm0-rs compile` elaborates the `.mm1` (pulling in
  `set.mmu`, whose proofs it checks), `mm0-rs join` inlines `set.mm0` into
  one spec stream, and `mm0-c` checks the `.mmb` against the joined spec —
  so the root of trust re-verifies all 50,030 imported set.mm statements
  alongside the new axiom (50,031 after ours). Full `verify` (check +
  selftest + four artifacts) runs in ~16 s.

* **Provisioning (untracked toolchain state).** `mm0/metamath/{set.mm,
  set.mm0, set.mmu}` is not committed — `set.mmu` is 548 MB. Provision with:

  ```
  mm0-from-mm from-mm mm0/metamath/set.mm -o mm0/metamath/set.mm0 mm0/metamath/set.mmu
  sed -i 's/^(pub //' mm0/metamath/set.mmu    # mm0-rs's reader takes no `pub ` prefix
  ```

  (`from-mm`'s spec output is deterministic — regeneration is
  byte-identical — and `set.mm0` is ~21 MB, should committing it ever be
  wanted.) When the theory is absent, both gates print a `[gap]` *with
  these commands* and still verify the certificate stems — a gap, never a
  silent pass, the same idiom as the missing-toolchain gaps in
  `scripts/verify-invariants`.

* **Gates.** `scripts/verify-invariants` Phase D now honours the
  `-- theory source:` line an imported-theory artifact carries: every
  `used=` label must be a `term` declared in that file (gap when it is not
  provisioned). The artifact's `-- trust-split: structural=0 grounded=0
  statements=1` is pinned by `BUDGETS` like every other artifact: the
  statement's *truth* is asserted (proof of record: the Mathlib
  DirichletL development cited in the header), and what mm0-c checks is
  everything else — well-formedness against set.mm's grammar, and via the
  import, the whole imported corpus.
