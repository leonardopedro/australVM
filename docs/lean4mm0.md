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

* **Negative anchor — a corrupted `set.mmu` proof must be refused.**
  Acceptance alone is satisfied by a chain that never reads the imported
  proofs at all, so `verify` then streams a copy of the real 548 MB corpus
  with one line flipped — `idi`'s proof (set.mm's *first* theorem, so the
  refusal costs ~1 s, not a re-read) becomes `ax_1`, which proves something
  else — and requires `mm0-rs compile` to fail **on that line**
  (`set.mmu:8:7: error: unknown subproof`, rc≠0). Three ways to fail, all
  loud: the compile accepting it (the proofs are not being checked), the
  error not naming the corruption (refused for the wrong reason), or the
  sed not matching (the corpus moved — the anchor re-pins itself instead
  of corrupting nothing).

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

* **CI provisions it (phase-d), so the gap cannot hide there.** The job
  fetches `metamath/set.mm` from a pinned rev and sha256-checks it against
  the host's byte-identical copy, restores the generated `set.mm0`/`set.mmu`
  from `actions/cache` (keyed on that rev + the from-mm compat patch +
  `flake.lock`), and on a miss runs the cold path: `nix build
  .#mm0-from-mm` (the flake's pinned GHC closure) → `from-mm` → the `pub `
  strip. Two guards keep the wiring honest: a step asserts the theory files
  exist *before* verify, and the verify step greps its own log for
  `label contract grounded` and `negative anchor OK` — both print only on
  the provisioned branch, so an un-provisioned run fails CI instead of
  passing with an explained gap.

* **Gates.** `scripts/verify-invariants` Phase D now honours the
  `-- theory source:` line an imported-theory artifact carries: every
  `used=` label must be a `term` declared in that file (gap when it is not
  provisioned). The artifact's `-- trust-split: structural=0 grounded=0
  statements=1` is pinned by `BUDGETS` like every other artifact: the
  statement's *truth* is asserted (proof of record: the Mathlib
  DirichletL development cited in the header), and what mm0-c checks is
  everything else — well-formedness against set.mm's grammar, and via the
  import, the whole imported corpus.

## 12. The corpus pipeline — any Lean module → MM0 statements (2026-11-04)

§5–§11 converted *chosen* theorems into MM0 certificates (`mm0/*_certificate.mm0`,
proofs included). This section is the other half: turning **any** Lean module's
declarations into MM0 *statements*, mechanically, and verifying that the encoding
is well-formed — the substrate on which later certificates are stated.

```
Lean module ──lean4export──▶ NDJSON ──lean2mm0-stmts──▶ <Mod>.{mm0,mm1} ──mm0-rs──▶ .mmb ──mm0-c──▶ exit 0
             (narrowed)      (cic.mm0 embedded)         (elaborates)                  (re-checks)
```

One command does all four steps for each module of a project:

```sh
scripts/lean-corpus-mm0                 # every top-level *.lean of $LEAN_PROJECT
scripts/lean-corpus-mm0 Layout Journal  # or only the named modules
LEAN_PROJECT=../timepiece scripts/lean-corpus-mm0
```

Artifacts land in `mm0/statements/<Mod>.mm0` (+`.mm1`); the NDJSON exports in
`mm0/exports/` and the compiled `.mmb` are regenerable and git-ignored.

Only statements up to 5 MB are tracked, and the pipeline enforces that itself:
`scripts/corpus-size-gate`, called by `lean-corpus-mm0` as soon as `mm0-c` has
accepted a module, leaves an oversized statement on disk (it is still a verified
artefact) and adds its path to the managed block of `.gitignore`, so no later
`git add -A` can stage it. A single Mathlib-importing chapter reaches 900 MB
(`BookProof.Ns.ChapterNavierStokesDiffHashimoto.mm0`), far past GitHub's 100 MB
per-file limit — which is why the limit is applied at the point of writing
rather than by auditing the index. `mm0/statements/LARGE.tsv` records size and
SHA-256 of everything kept out, so a regenerated file can be checked against the
published run; a rerun that shrinks a statement under the limit removes its
entry again, and `scripts/corpus-size-gate --audit` re-derives the whole block
from what is on disk (for statements produced before the gate existed). Each
module is reported as converted+verified, skipped (exports nothing of its own,
or is not a lake target) or failed (build/export/emit/elaborate/mm0-c), and the
exit status is non-zero if anything failed — no silent passes.

### What "converted to mm0" does and does not mean

The statement of each declaration is `axiom decl_<name>: $ teq <type-code> ttrue $;`
with the type encoded in `docs/cic.mm0`'s term language (`tapp`/`tabs`/`tentity`
/`tint`/`tvar`). mm0-rs elaborates the encoding against the embedded theory and
mm0-c re-checks the compiled file, so the *shape* of every declaration is
machine-checked against the same grammar the certificates use. The *truth* of
the statement is not: that is the Lean kernel's verdict on the export (re-checked
by nanoda in `../unfer/prob_kernel`). A theorem the corpus wants **proved** in MM0
still needs a certificate from `scripts/lean2mm0` — statements are the interface,
certificates are the proofs.

### Two traps this pipeline exists to encode

* **`lean4export Mod -- roots` exports the roots *plus their dependency
  closure*.** The `--` argument list narrows the ROOTS, not the payload: for a
  Mathlib-importing module the closure is ~50 000 declarations / ~300-390 MB
  (measured on `BookProof.ChapterA4`: 24 own declarations → 54 053 exported,
  369 MiB; a *single* Mathlib lemma alone → 6 760 declarations / 29 MiB). That
  is not exporter waste — nanoda re-checks that closure, so it has to be there.
  `lean2mm0-stmts` therefore emits statements for the roots only (`--only`), and
  `MAX_DECLS` bounds *those*; a module that declares nothing of its own still
  cannot produce a degenerate file. Modules with no Mathlib imports export
  exactly their own declarations (~800 KB), so nothing changes for them.
* **mm0-rs has no numeral literals.** `(tentity 0)` is a parse error
  (`expected '$'` at the `(`): `0` is not a token of any declared notation. Each
  value used is therefore declared once, ahead of the statements —

  ```
  term n0: nat; prefix n0: $0$ prec max;
  ```

  — exactly as `mm0/gap_certificate.mm0` already did for its own numerals. The
  generator emits this block automatically; forgetting it makes every statement
  in the file unparseable from the first payload onwards.

### Status on this host (timepiece, Lean 4.28.0 + Mathlib)

`GapCertificate` (36 declarations → 792 lines) and `Layout` (89 → 1 528 lines)
convert and pass mm0-c, as do `QuasiRiemannCertificate`, `extract_sketch_info`,
and — through the nanoda leg with the budget lifted — Mathlib-importing chapters:
`BookProof.ChapterA4` (24 → 484 lines, closure re-checked in ~400 s) and
`BookProof.ChapterAbelianDiagonal` (11 → 339 lines, inside the default budget). Modules whose `.lean` file is a script rather than a
lake target (`minitest`, `axiom_gate`) and umbrella modules that only re-export
sub-libraries (`PnpProof`, `BayesianTransformer`, `Book`) are reported as
skipped, which is the honest status: there is nothing of their own to encode.
The nested sub-libraries (`Book.*`, `BayesianTransformer.*`, ~14 600 files)
convert with the same command by module name but are not part of the default
corpus — one module per invocation, as their builds are the expensive part.

### The nanoda leg (added 2026-11-04)

mm0-c checks that the *encoding* is well-formed against the theory; it says
nothing about whether the export was a real derivation. That is nanoda's job, and
the corpus driver now runs it **between the export and the emission**:

```
lean4export ──▶ NDJSON ──▶ nanoda (unfer/prob_kernel) ──▶ lean2mm0-stmts ──▶ mm0-rs ──▶ mm0-c
```

```sh
# one-time build of the checker (same code path as the pinned-fixture tests)
(cd ../unfer && cargo build -p prob_kernel --example verify_export)
```

`prob_kernel/examples/verify_export.rs` exposes `verify_export()` as a command —
before it, the independent kernel was only reachable from `#[test]`s hard-wired
to the vendored fixtures, so a fresh export could not be re-checked without
overwriting a fixture. Exit 0 = nanoda accepted every declaration; 1 = rejected
(failing declaration printed); 2 = usage/IO error. The driver treats a rejection
as a failure of that module (`<mod> (nanoda)`) and writes no statements.

**Size cap, not a kernel verdict.** `LeanVerifySpec::max_export_bytes` defaults to
16 MiB because the *service* accepts payloads over the wire; a local audit of a
module that quotes large syntax trees easily exceeds that (`extract_sketch_info`
exports 35 MB). The CLI therefore defaults to 256 MiB and takes an explicit
`[max-bytes]` argument. Hitting the cap is a verification *gap* to report, never
"unproved".

**The driver's own budget (`NANODA_MAX_BYTES`, default 256 MiB).** It exists
because a Mathlib-importing module's export is far past that, and the cost is
real: on this host (14 GB RAM) `BookProof.ChapterA4`'s 369 MiB export took
**399–443 s and ~840 MiB peak RSS** to re-check (the larger figure is the run
that also writes the trusted-name manifest), and an earlier attempt against
several such exports **triggered the OOM killer**. So over-budget exports are
reported as `GAP` — statements are still emitted and mm0-c still checks the
encoding, but the summary says `NOT kernel-backed` and the module is *not*
counted as converted+verified. Measured, not guessed: the nanoda leg of a
single module peaks under 1 GB, so `NANODA_MAX_BYTES=0` (unbounded) is safe for a
serial run on a machine with ≥3 GB free; leave the default in place when
batching, where several kinds of heavy process overlap.

**What that 1 GB figure does *not* cover (`CORPUS_STATEMENT_SAFETY`, default
64 MiB).** The export check is not the memory peak. Sampling the pipeline on
2026-11-07 (30 GB host) gave `mm0-rs` **10 GB** for a 50 MB statement and the
emitter **6.6 GB** for a 454 MB export, i.e. mm0-rs needs roughly two hundred
heap bytes per statement byte. Two chapters crossed that line and took the whole
run with them, twice: `BookProof.Qg.ChapterQgAssembledFlatTransfer` (446 MB
export) and `BookProof.Ns.ChapterNavierStokesEsaConsolidation` (472 MB export →
**968 MB statement**) both exhausted the host, and the kill left a sparse
multi-hundred-megabyte `.mm0` behind to be cleaned up by hand. The driver now
measures the emitted statement and, past the ceiling, records the module as
`skipped` and stops before `mm0-rs`: such statements are two orders of magnitude
past the 5 MB publication limit, so the mm0-c verdict they would earn could not
be published either. Raise the ceiling only on a host with the memory to match.

### Checking the shared closure once (added 2026-11-04)

The closure cannot be dropped from a module's export: nanoda type-checks each
declaration against the others, so the dependencies must be present in the file.
Nor can two exports be concatenated into a shared base — the NDJSON ids are
file-local (`{"in":1,…}`, `"pre":0`), so a second file's ids, level ids and
expression references would all have to be renumbered consistently. What *can* be
dropped is the **re-check**: `verify_export --trusted NAMES` parses the whole
export, leaves every declaration in the environment (their types are needed), and
type-checks only the names the manifest does not already cover.

```sh
scripts/lean-corpus-trust BookProof.ChapterA4             # once: full check → manifest
NANODA_TRUSTED=mm0/exports/trusted.names \
  scripts/lean-corpus-mm0 BookProof.ChapterA5             # delta only
scripts/lean-corpus-trust --update BookProof.ChapterA5    # widen the base
```

Measured on this host (14 GB RAM), same binary, same inputs:

| run | export | decls checked | wall | peak RSS |
|---|---|---|---|---|
| full check (writes the manifest) | `ChapterA4`, 369.7 MiB | 53 954 | 443 s | 837 MiB |
| trusted, same file | `ChapterA4`, 369.7 MiB | 0 | 35 s | 736 MiB |
| full check | `ChapterA4b`, 318.5 MiB | 51 028 | 331 s | 717 MiB |
| trusted | `ChapterA4b`, 318.5 MiB | 5 470 | 103 s | 659 MiB |

The closure re-check was 92 % of the wall time on a chapter the manifest already
covered; 35 s — parse and load — is the floor trust cannot remove. `ChapterA4b`
still pays 103 s because 5 470 of its declarations were not in a manifest built
from `ChapterA4` alone, so `--update` after a few chapters converges it toward the
floor.

Trust is **by name**, so the manifest is only as good as the run behind it:
`mm0/exports/trusted.names.meta` records which export it came from (SHA-256, size,
checker, elapsed). Rebuild it when the Lean toolchain, Mathlib, or the project's
imports change — otherwise a declaration whose statement was edited since the
manifest was written would be skipped. The pipeline regenerates it from a full
verdict on an export of this environment rather than accepting one from anywhere
else.

### Memory: one chapter is 6–8 GiB, and `lake build` multiplies it

The OOM events that killed sweeps had a specific cause, not a general "Lean is
hungry". Per step of one Mathlib-importing chapter
(`BookProof.ChapterEsaFarisLavineIndex`, measured with a process-tree sampler on
a 14 GiB host):

| step | peak RSS | wall |
|---|---|---|
| elaborate the module (`lake env lean -o`) | 6 335 MiB | 29 s |
| declaration walk (`lake env lean --run lean-module-decls.lean`) | 8 375 MiB | 5 s |
| export (`lake env lean4export`) | ~7 053 MiB | 17 s |
| nanoda, 370 MiB export, trusted base | 736 MiB | 35 s |

Every step is fine on its own — the sweep runs them one module at a time. What
was not fine is that the driver used to start with `lake build <module>`, which
Lake resolves against the whole `BookProof` **library**: it plans all 8 144
artifacts (a five-declaration chapter included) and compiles stale ones with one
job per core. One stale file is a 6.3 GiB `lean`; a stale cluster is twelve of
them at once, and the kernel OOM killer then picks whatever it likes — often the
sweep itself, which is how a run can die mid-module without writing a state row.

The driver no longer calls `lake build` at all: it checks whether the module's
own olean (`.lake/build/lib/lean/<Mod>.olean`) is newer than its source and,
when it is not, elaborates that single file with `lean -o` into `mm0/olean/`.
When an import has no olean it says so (`imports unbuilt`) instead of building
the project behind your back. To build the project itself, run `lake build` once
by hand — on an idle machine, because that fan-out wants far more memory than
this box has.

`scripts/lean-corpus-sweep` adds the guard: while a module runs it samples
`MemAvailable` once a second and, below `SWEEP_MEM_FLOOR_MIB` (default 1024 MiB),
kills that module's own process group and records it as `failed` with the reason —
instead of the kernel choosing a victim. It also records each module's peak RSS
in the state file, since the peak, not the average, decides whether the next
module fits: `ok  36s  peak  8375 MiB  17 decls → 435 lines, mm0-c OK`. After
three memory stops in a row it stops, telling you to free memory and re-run.

### Sweeping the corpus

`scripts/lean-corpus-sweep` converts a whole directory, one module per driver
invocation, and remembers what it did:

```sh
scripts/lean-corpus-sweep                  # every *.lean under timepiece/BookProof
scripts/lean-corpus-sweep BookProof/Bell   # or one subdirectory
SWEEP_RETRY=1 scripts/lean-corpus-sweep    # re-attempt whatever is not `ok`
```

State is one tab-separated line per module in `mm0/sweep-state.tsv`
(`status · module · seconds · detail · oversize`, the detail carrying the
module's peak RSS), with a per-module transcript in
`mm0/sweep-logs/<module>.log` and an append-only `mm0/sweep-logs/sweep.log` that
a `tail -f` can follow. A module that already has a line is never attempted again,
so an interrupted sweep resumes where it stopped and loses at most the module that
was in flight. Statuses: `ok` (nanoda verified, mm0-c re-checked), `gap`
(converted and mm0-c verified, statements *not* kernel-backed), `timeout`
(`SWEEP_TIMEOUT`, default 3600 s per module), `failed` (build/export/nanoda/
mm0-rs/mm0-c), `empty` (no declarations of its own). The exit status is non-zero
while anything in the state file is not `ok`.

`oversize` is `-` when a module's statements all fit, and otherwise
`<n> files, <mb> MB`: those artefacts are on disk and hashed in
`mm0/statements/LARGE.tsv` but kept out of git (`scripts/corpus-size-gate`), and
the sweep's summary names the modules. It is not a status — the conversion and
the `mm0-c` re-check did pass, so the row stays `ok`/`gap` — and it exists
because the only other trace is one warning line inside that module's log.

The sweep defaults to `NANODA_MAX_BYTES=0` and picks up
`mm0/exports/trusted.names` automatically, which is what makes an unbudgeted run
affordable: chapters whose closure the manifest covers cost about 40 s each
end-to-end including `lake build` (`BookProof.Bell.ChapterBell`: 15 declarations →
293 lines in 39 s; `BookProof.Bell.ChapterTsirelson`: 34 → 580 lines in 28 s),
versus ~6 min when every chapter re-checks 54 000 shared Mathlib declarations.

### Files that are not lake targets

Not every `*.lean` file is a `[[lean_lib]]` root: `QuasiRiemannCertificate` is a
module of the `RiemannProof` library, `axiom_gate` is a script. `lake build <mod>`
answers `unknown target` for those, so the driver falls back to elaborating the
file directly into `mm0/olean/<mod>.olean` and prepending that directory to
`LEAN_PATH` for the export. Three outcomes result, and they are reported
separately: *converted*, *skipped* (does not elaborate as a standalone file, or
exports no declarations of its own), *failed* (build error, nanoda rejection,
mm0-rs error, mm0-c refusal).

### A third trap: sanitised names collide

MM0 identifiers are `[A-Za-z0-9_]`, so a Lean name is sanitised (`get_elem?` →
`get_elem_`). The map is **lossy**: `get_elem?`, `get_elem'` and `get_elem_` all
land on the same identifier, and a module that declares several variants then
emits the same axiom name twice — mm0-rs rejects the whole file with
`theorem 'decl_X' redeclared`, which reads like a theory problem but is purely a
naming bug. `fresh_name` therefore indexes the *whole* declaration set first: a
base nobody else claims keeps the readable identifier (no churn in existing
artifacts — `Layout` regenerates byte-identically), while every member of an
ambiguous base gets the first 8 hex digits of `SHA-256(original name)`. The
result is injective AND independent of declaration order, so regeneration of the
same export is byte-identical. Found on `timepiece/extract_sketch_info.lean`
(31 declarations, 15 359 emitted lines at the time — now 31 → 368, because the
closure is no longer restated; see the `--only` note above).

### A fourth trap: `pre` is a name id, not a prefix length

`Name.str` / `Name.num` records carry `{"pre": <id>, "str": <component>}`, and
`pre` is the id of the **parent Name** (`Export.lean` writes `("pre", ← dumpName n)`).
Reading it as a character count still produces plausible strings, because the
parent's own text is already interned under that id — but the components end up
**flattened**: `BookProof.conjugateₗᵢ` becomes `BookProofconjugateₗᵢ`, and the
symbol tables in every artifact carried that damage unnoticed (it is a comment
line, and mm0-c does not care what a symbol is *called*). What broke it was
`--only`: a root list written as real Lean names matched *nothing* against the
export, and the driver reported "none of the 24 requested declarations appear"
while the names were plainly in the file. Components are now re-joined with `.`,
so symbols read like Lean names again and root filtering works. Regenerating the
four pre-existing artifacts changed their symbol tables and axiom identifiers
(`decl_OfNatofNat` → `decl_OfNat_ofNat`) but not their statement structure; all
four were re-emitted and re-checked by mm0-rs and mm0-c afterwards.

### Why the corpus sweep was getting OOM-killed

The first full sweep died at module 146 of 883, with no state row for it. The
journal had the reason:

    app-ghostty-surface-transient-….scope: The kernel OOM killer killed some
    processes in this unit.
    … Consumed 6min 3.640s CPU time …, 11.2G memory peak, 12.7G memory swap peak.

Three separate things had to be untangled.

**1. `lake build <module>` is not a per-module operation.** Lake resolves a
module target against the whole library — `BookProof.ChapterErrorNorms`, five
declarations, plans 8,144 jobs — and compiles whatever is stale with one job per
core. Every one of those is a full Mathlib environment, so a stale cluster turns
into a dozen concurrent environments. `scripts/lean-corpus-mm0` now checks
whether the module's own olean is current and, if it is not, elaborates that one
file with `lean -o` into `mm0/exports/olean`. It never asks Lake for a target set.

**2. The first per-module numbers were inflated fourfold.** Summing `ps -o rss`
over the process tree counts the Mathlib oleans once per process that maps them:
`lake env`, `lean` and `lean4export` all map the same pages. The same run reads
8,203 MiB summed and 2.1 GiB as charged by the kernel
(`BookProof.Bell.ChapterBell`; `ChapterTsirelson` 2.4 GiB). The state file and the
progress line now report `RssAnon + RssShmem`, which counts a shared page once.

**3. Neither memory ceiling that looks obvious works on this host.**

| Attempt | What happens |
|---|---|
| `ulimit -v` | Lean mmaps its oleans, so the cap surfaces as a corrupt import: `failed to read file '…/Mathlib/…/AlgebraicClosure.olean'` |
| `systemd-run --user --scope -p MemoryMax=800M` | The property is accepted and reads back (`memory.max` = 838860800), and a 1.5 GiB allocation still succeeds — the global OOM killer fires instead. The memory controller is not delegated far enough down the user tree to be enforced |
| `lean -M/--memory` | Works, and fails with `lean::memory_exception` — but `lean4export` has its own CLI and ignores it |

So the sweep bounds the *blast radius* rather than the peak: it runs one module
at a time, waits for headroom before starting one, samples `MemAvailable` every
second while it runs, and kills that module's own process group (job control
gives it one) when the machine runs dry. The module is recorded as `failed` with
the reason, and three such stops in a row end the sweep instead of burning
through the rest of the corpus.

| Variable | Default | Meaning |
|---|---|---|
| `SWEEP_MEM_NEED_MIB` | 4000 | wait for this much available before starting a module |
| `SWEEP_MEM_WAIT` | 1200 s | how long to wait for it before starting anyway |
| `SWEEP_MEM_FLOOR_MIB` | 1024 | kill the module's process group below this |

Running the sweep beside a `lake build`, or two sweeps at once, still runs out of
memory: the guard protects the machine from one module's growth, not from two
mechanisms each asking for a few gigabytes.

**Carrying on when the host cannot fit a module.** A module that trips the floor
is recorded as `deferred`, not `failed`: that is a statement about the host, and
one chapter needing 10 GiB must not end a corpus run (it did, at module 466 of
884). The sweep continues with the rest of the corpus and reports how many it
deferred. `SWEEP_RETRY_ROUNDS` (default 1) then re-attempts the `failed`, `gap`
and `timeout` rows at the end of the same invocation, so a transient cause —
imports not yet built, a timeout, a busy machine — is retried without a second
command; `empty` and `deferred` are left alone unless `SWEEP_RETRY_DEFERRED=1`.
The state file keeps exactly one row per module, so a resumed or re-attempted run
replaces that module's verdict instead of counting it twice.

## 13. The first proof, not just the statement (2026-11-08)

Everything above this section translates *statements*. A statement of record is
an axiom with a Lean-shaped name; nothing in the corpus says why it is true, and
§9.2 measured the consequence (0 of 593 artefacts certificate-backed). This
section is the first artefact that closes that gap: one real `BookProof`
declaration whose proof term is *typed* in MM0 and judged by both checkers.

**The theory.** `docs/cic_typing.mm0` extends `docs/cic.mm0` with the judgment
`tty (ctx: Term) (term: Term) (type: Term)` and its rules — `ty_var0`, `ty_varS`
(weakening), `ty_pi`, `ty_abs`, `ty_app`, plus `tdefeq` with `refl`/`symm`/
`trans` and the conversion rule `ty_deq`. Size: **5,661 B / 102 lines**, 26
term/def, 21 axiom/theorem; `mm0-c` accepts it on its own (exit 0). It is the
"Π/λ/App typing + conversion" theory the translator needs, separate from
`cic.mm0` so the statement-of-record theory stays frozen.

**The artefact.** `scripts/lean-typing-proof` (prototype) reads the same
`lean4export` NDJSON the statement translator reads — it *imports* `lean2mm0`
rather than copying the parser — and emits `mm0/proto/_simp_1_1_typing.{mm0,mm1}`
for `BookProof.ChapterNoBestPrior.distinct_priors_each_preferred._simp_1_1`:

| measurement | value |
| --- | --- |
| Π binders in the declaration | 2 |
| derived type vs declared type | equal, syntactically |
| rule applications in the proof | 3 (2 × `ty_abs`, 1 axiom leaf) |
| statement text | 2,594 B (1,433 B excluding the generator header) |
| proof term text | 23 B — `ty_abs (ty_abs ax_body)` |
| verified binary (`.mmb`) | 7,240 B |
| assumptions (body typing) | 1 |
| wall clock | 1.8 s |

`mm0-rs compile` reports no `sorry` and `mm0-c` exits 0 on the concatenation
(`cic.mm0` + extension + statement), which is the same two-checker standard the
statement corpus is held to.

**What a full translator still has to discharge.** The proof above is honest
about its one leaf: the body's typing is assumed, and each `tapp` in a real
proof needs a `tdefeq (tsubst B x) r` premise that only a definitional-equality
checker can produce — `tdefeq_iso` in the extension is where the deltanet enters
(`unfer/logos/src/mm0/dk.rs` refuses to fake it, which is why the existing
emitter declines these theorems by name). So the remaining work is not syntax:
it is one β/conversion obligation per application node.

**Four traps worth not rediscovering.** (1) A proof-carrying theorem must be
declared `pub`: a private theorem's proof commands are dropped from the `.mmb`,
and `mm0-c` then reports `expecting a theorem` at a declaration whose command
stream has already reached `End`. (2) `mm0-c` re-checks the binary against the
*statement* file (`X.mm0`), not the `X.mm1` the binary was compiled from; handed
the `.mm1` it fails at the `=` after the statement. (3) Numerals are not
accepted inside `$…$` — every universe/level/symbol id becomes a declared
constant. (4) Rule parameters are solved by unification (`'(addcom)` style), so
term arguments are *not* written into the proof; writing them is a syntax error.

### 13.1 The body's derivation, not an assumption (2026-11-08)

The artifact above assumes the body's typing: one axiom closes the statement, and
that axiom is exactly what "translate the proof" means to remove. `--spine` on
the same declaration derives it instead, and the difference is the translator's
worklist in numbers:

| | body assumed | `--spine` derivation |
| --- | --- | --- |
| rules applied | 3 | 22 (`ty_app` 10, `ty_var` 5, `ty_abs` 3, `ty_deq` 3, const-type 5, type-as-term 1) |
| assumptions | 1 | 19 (5 constant types, 10 substitution, 3 conversion, 1 type-as-term) |
| statement | 2,594 B | 9,641 B |
| proof term | 23 B | 414 B (18 nested rule applications) |
| verified binary | 7,240 B | 11,160 B |

Both modes derive the *declared* type — the spine mode checks
`derived == declared` on the encoded forms, and `mm0-rs` reports no `sorry` with
`mm0-c` at exit 0. The 5 constant types are Mathlib's declarations (an axiom per
constant is the honest form: a rule would hide the count). The interesting
entries are the other 14:

* **10 substitution obligations.** `ty_app` concludes at `r` from the premise
  `tdefeq (tsubst b x) r`. `tsubst` is a term constructor, not a rewrite MM0 can
  perform, so every application (β-redex) is one premise. These are the deltanet's
  workload: discharging one means emitting a `valid_iso` witness for the two
  concrete net encodings (`tdefeq_iso`), and *that* checker — deltanet normal
  forms for cic-encoded terms — is the piece of the full translator that does not
  exist yet. Everything upstream of it (export parsing, encoding, the derivation
  itself) now runs.
* **3 conversion obligations** from `ty_deq`, where the inferred type is
  β-equivalent but not syntactically equal to the expected one.
* **1 type-as-term.** A Π or a sort passed as an implicit argument needs
  `ty_pi`'s universe, and universes are not stratified in this fragment; it is
  assumed and counted rather than papered over.

Two encoding facts this cost, worth stating because they are silent when wrong:
the derivation must name variables the way the theory does — `tvar (tsucc tzero)`,
not a flat index constant, or `ty_varS` cannot unify — and substituting under a
binder must lift the argument and lower the indices above it, or `ty_app` fails
with `t1 != tsucc` deep inside a nested application.

There is also a cheaper shape available, recorded rather than taken: concluding
each `ty_app` at `r := (tsubst b x)` makes every substitution premise
`tdefeq_refl`, at the price of one *deferred* substitution nest, so 10 obligations
become 1 large one at the boundary. That trades many small deltanet checks for a
single big one, and is the natural knob once the checker exists.

### 13.2 One chapter, measured end to end (2026-11-08)

`scripts/lean-proof-scan` runs the emitter per declaration, assembles each
artifact against `docs/cic.mm0` + `docs/cic_typing.mm0`, and asks both checkers
whether the derivation closes — so what it reports is *verified* closures, not
emitted files. On `BookProof.ChapterNoBestPrior` (the 6 declarations the module
itself contributes; the export's other 10,593 belong to its dependencies):

| declaration | kind | rules | assumptions | verdict |
| --- | --- | --- | --- | --- |
| `distinct_priors_each_preferred` | thm | 27 | 19 | verified |
| `distinct_priors_each_preferred._simp_1_1` | thm | 27 | 19 | verified |
| `distinct_priors_each_preferred._simp_1_2` | thm | 78 | 50 | verified |
| `expectedUtility` | def | 48 | 32 | verified |
| `not_uniformly_better` | thm | 7 | 1 | verified (body assumed) |
| `eq_of_expectedUtility_ge_all` | thm | — | — | **not verified** |

5 of 6, 187 rule applications, 121 assumptions, 14 s total. Four of the closures
are structural (the derivation follows the proof term); `not_uniformly_better` is
not: its type's tail is a reducible abbreviation, so the body's applications have
no Π to decompose, and the whole body is one counted assumption. That distinction
is visible in the artifact rather than buried in it — the assumption is named
`ax_leaf_0` and the rule list says `assumed (unfolded Π)`.

The one failure is a 3.7 MB derivation (2,672 assumptions, 1,430 applications)
whose elaboration `mm0-rs` rejects with a term/type form disagreement
(`tapp != tabs`, `tvar != tpi`) inside a nested conversion. It is a real open
bug in the emitter, not a theory gap: the same shapes verify in the other five.
It is recorded here rather than hidden by lowering the scope.

Two rule gaps the scan closed on the way, both of which had been *silent*
guesses before they were classes: a λ whose expected type is a Π only after
unfolding now gets its domain from the export, infers its body, and pays one
conversion (it used to raise); a Π/sort used as a term is one counted axiom (it
used to be an unstated shortcut). A `natVal` node is an opaque constant per
numeral, and `Nat.rec`/`Eq.rec`/constructor types are read from the export's
inductive records (`recs`/`ctors`) instead of being reported missing.

### 13.3 Deferred substitution: measured, and not taken

`--defer-subst` concludes each *tail* application at `(tsubst b x)` itself, so
the `ty_app` premise becomes `tdefeq_refl` and the redex is left for one
normalisation obligation at the boundary. Measured against the eager shape on
the same chapter:

| declaration | eager | deferred |
| --- | --- | --- |
| `distinct_priors_each_preferred` | 27 rules, 19 assumptions | 28, 19 |
| `…._simp_1_1` | 27, 19 | 28, 19 |
| `…._simp_1_2` | 78, 50 | 79, 50 |
| `expectedUtility` | 48, 32 | 49, 32 |
| `not_uniformly_better` | 7, 1 | 7, 1 |

One extra rule and **the same number of assumptions** per declaration. The
reason is structural: only a tail application can be deferred (a deferred type
has no Π structure, so anything applied to it must be converted back first), and
a proof term's spine has exactly one tail application — so deferral swaps one
substitution obligation for one normalisation obligation, and the normalisation
one is the *bigger* of the two. The eager shape is therefore kept as the
default; the flag stays because the trade flips if a whole spine ever sits in
tail position (a term whose head is applied once, deeply), which is a shape to
look for rather than assume.

### 13.4 The certificate interface (blocked on the toolchain here)

Discharging a substitution obligation means convincing something *outside* MM0:
`tcert (b a r w)` and `tcert_witness` now give that certificate a form in
`docs/cic_typing.mm0` — the two terms and the kernel's isomorphism-witness
digest — and `scripts/lean-typing-proof` writes every obligation structurally to
`<stem>_obligations.jsonl` (10 lines / 8,813 B for `_simp_1_1`), which is the
input a checker consumes. `safestos/cranelift/src/bin/netcert.rs` is the
front-end: two fragments in, the kernel's own `AustralDefeqReport` out
(`uk_austral_defeq`), exit 0/1/2 for equivalent / not / could-not-run.

It could not be *run* in this environment, so no obligation has been discharged
yet and none is claimed:

* `nix develop` (the OCaml shell, and the only place `cargo` was on `PATH`)
  fails to build — `alt-ergo-2.6.3.tbz` is unavailable (`curl: 22 … 504`), twice,
  so `dune` cannot be invoked;
* `cargo` outside that shell cannot resolve the bridge crate at all:
  `arctic = { path = "../../../dynamic-arctic" }` and that directory does not
  exist in this checkout (`.cargo` and a 401-crate registry cache are present,
  so this is a missing sibling repo, not a registry problem);
* there is no prebuilt `safestos/cranelift/target/` and no
  `libaustral_cranelift_bridge.so`, so the check cannot be reached by linking a
  prebuilt bridge either.

The next step on this path is environmental: restore `dynamic-arctic` (or build
with that feature's dependency removed) and the alt-ergo mirror, then run
`netcert` on the dumped pairs and teach the emitter to prefer a certified
`tdefeq_iso` over the counted axiom.
