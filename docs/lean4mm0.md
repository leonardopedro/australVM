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

### 13.5 A second chapter, and what the remaining failure class is

`BookProof.BalabanRG` (scan of the first 12 declarations in the namespace, via
`BookProof.BalabanRG.BlockOperator.ndjson` — a module's declarations land in the
shared namespace, not under the file's name, so the prefix is the namespace):

| | before | after |
| --- | --- | --- |
| verified closures | 4/12 | **8/12** |
| failures on "expression form outside the fragment" | 7 | 0 |
| failures on elaboration (`sorry`) | 1 | 4 |

The seven closed at once when `infer` gained a λ case: the export carries each
λ's domain, so a λ in an *inference* position is typed exactly like one under an
unfolded Π (`lam_type`) instead of raising. The four that remain all fail the
same way, and it is now one class rather than a mystery:

```
terms do not match: tapp != tabs
terms do not match: tapp != tentity
```

MM0 unifies **types syntactically**, so a type written `(tapp (tabs …) v)` (the
encoding of a `let`) and the same type written in its unfolded form are different
terms to the elaborator — even though the theory has `tdefeq` precisely to bridge
them. The emitter does not yet know where to insert that step, so a derivation
that is correct about *what* converts still loses the elaboration. This is the
same signature as §13.2's `eq_of_expectedUtility_ge_all` (`tapp != tabs`,
`tvar != tpi`), now reproduced on four independent declarations, so the next fix
is a single one: canonicalise the encoding (β-reduce `letE` when encoding) and/or
insert a `ty_deq` at each place a type form changes shape across a binder.

A related diagnosis worth keeping: the two attempts at that fix which *looked*
right both made artifacts weaker or broke them, and the measurements said so —
lifting context entries (the context is spine-relative: `ty_abs` forces `tcons`'s
head to be the Π's domain, so an entry is used exactly as declared, and shifting
it broke `_simp_1_1`, which had verified), and treating a scope mismatch as an
assumption (it cost two structural steps on `_simp_1_1`, so a conversion that the
elaborator accepts is better than a leaf assumption that hides it). Both were
reverted. `max_free` stays as an origin check inside `lam_type`.

### 13.6 The value is a term in the context, not a λ spine (model fix)

Everything before this section treated a declaration's value as abstracting over
the type's Π spine, so the artifact was always the *closed* judgment
`tty tnil value type`, with one `ty_abs` per binder. That is wrong in general:
Lean exports `theorem foo (x : α) : P x := proof` with `proof` referring to `x`
and to the section hypotheses by **fvar**, not by a λ. A value that happens to
carry the λs (`_simp_1_1` does) can be closed; one that does not can only be
stated as the open judgment `tty ctx value body_type`. Emitting the closed form
anyway is not a rule gap, it is a misreading of the export — and it showed up as
`KeyError: 'lam'` where the peel ran out of λs.

So the shape is now measured per declaration and the statement follows it:

| chapter (scan) | closures | rules | assumptions |
| --- | --- | --- | --- |
| `BookProof.ChapterNoBestPrior` (6 decls) | 5/6 | 182 | 121 |
| `BookProof.BalabanRG` (first 12) | 8/12 | 17,068 | 4,317 |
| `BookProof.OSReconstruction` (first 8) | 8/8 | 166 | 106 |

The OSReconstruction closures are mostly *shallow* — several are a single
assumed leaf, because a definition whose body is a projection (`toFun`) or whose
application goes through an unfolding has no rule here. That is worth stating
plainly: "verified" means the artifact passes both checkers, not that the
derivation is deep. The `rules` column is the honest measure of depth, and it is
the one to watch as the fragment grows.

The remaining failure class is unchanged and is one fix, not many: MM0 unifies
types syntactically, so a `let`-encoded type and its unfolded form are different
terms, and the emitter does not yet insert the `ty_deq` step where a type changes
shape (four cases in BalabanRG, one in ChapterNoBestPrior, both with the same
`tapp != tabs` signature).

### 13.7 What "insert a `ty_deq`" cannot fix (measured)

The plan for the residual failures was to insert a conversion step where a type
changes shape. That plan is wrong, and the measurement is worth recording so it
is not tried a fourth time.

`ty_deq` is already applied everywhere a term's type and the expected type
differ: 2,072 uses in `eq_of_expectedUtility_ge_all`, 4,292 `ty_app` steps. The
failures are elsewhere. MM0 reports them **on the context**, not on a term:

```
tty (tcons (tvar 6) (tcons (tvar 5) ...))
 =?=
tty (tcons (tpi ... ) (tcons ...))
terms do not match: tvar != tpi
```

The `tcons` head is the Π's domain, and `ty_var0`/`ty_varS` read entries
verbatim, so the theory has **no rule that converts a context entry into the
type it denotes**. A conversion on the term (`ty_deq p ax`) does not help: the
proof of the *function* is fine, the context the rule is handed is not. Closing
this class needs a theory change (context conversion, or a congruence rule on
`tcons` plus a way to justify it) — a design decision, not an emitter patch.

Three consequences, each measured:

1. **Universe levels were the unattributed cause of a large share.** `Sort` is
   encoded as an opaque symbol per level, assigned by first sight, so
   `tsort t_lev0` and `tsort t_lev9` are never the same term and the theory has
   no level algebra. 1,468 conversions in `eq_of_expectedUtility_ge_all` are of
   this kind. They are now their own counted class (`ax_lev_*`,
   "universe level"), because collapsing them silently would be asserting
   `Type : Type`. Closures did not change (5/6) — the artifact now names the gap
   instead of hiding it inside `ax_deq`.
2. **Resolving references to the Π they denote never fires.** A `bvar` type
   was resolved through the context and a `const` through its declared type,
   with a conversion step inserted when either yielded a Π. Across 26
   declarations in three chapters (`ChapterNoBestPrior`, `BalabanRG`,
   `OSReconstruction`) the branch fired zero times, because the resolved value
   is a *type* while the mismatch is on the context head as above. It was
   removed rather than left in place as a plausible-looking path.
3. **Deepening the shallow closures is a rule gap, not emitter work.**
   `OSReconstruction` stays 8/8 with 166 rules and 106 assumptions. Its shallow
   cases are `proj` (no `ty_proj` in the theory, and the export carries no
   per-node structure type to state one) and applications that need the type of
   an unfolded definition. Both need new rules; the emitter cannot derive them.

So the honest state of the translator: it emits a derivation for every
declaration it accepts, the derivations compile, and the two gaps it cannot
cross — context conversion and projections — are stated in the theory's own
vocabulary rather than papered over.

### 13.8 The toolchains build again; the certificate path stops at the front-end

For a stretch this repo could not be built here: `nix develop` died fetching
`alt-ergo-2.6.3.tbz` (a mirror 504) and `cargo` outside it could not resolve the
bridge crate. Both are gone, and the steps that fixed them are worth recording
because none of them is discoverable from the build error:

- `nix develop` fetches cleanly again (dune 3.23.1, cargo 1.97.1). Nothing in
  the tree changed — it was the mirror.
- The bridge crate path-depends on `arctic`, which lives in `../dynamic-arctic`
  and was absent. The fix is the one CI already performs
  (`.github/workflows/build-and-test.yml`): `git clone --depth 1 --branch main
  https://github.com/leonardopedro/dynamic-arctic.git ../dynamic-arctic`.
- `lib/BuiltInModules.ml` is **generated** by the `Makefile` rule
  `lib/BuiltInModules.ml: lib/builtin/*.aui …  → python3 concat_builtins.py`.
  A bare `dune build lib/` therefore fails with *"Some modules don't have an
  implementation"* on `BuiltInModules`, which reads like a code defect and is
  not one: the Makefile prereq is what creates the module. Use `make bridge` (or
  `make all`), which builds it, the Rust library, the `.so` and the `austral`
  binary in one step. Adding `modules_without_implementation BuiltInModules` to
  `lib/dune` "fixes" the bare-dune error and then breaks `make bridge`, which
  reports the opposite complaint once the generated file exists.

With both halves building, `netcert` (the CLI over `uk_austral_defeq`) runs and
answers well on the kernel's own language:

| pair | verdict | report |
| --- | --- | --- |
| `(x + (2 + 3))` vs `(x + 5)` | equivalent | witness `[[0,0],[1,1],[5,2]]`, equal `unf_hash` |
| `(x + 1)` vs `(x + 2)` | not equivalent | exit 1, `witness: null` |
| `(tsubst (tsort t_lev1) …)` vs `(tsort t_lev1)` | refused | UK-4805, exit 2 |

The third row is the whole story of §13.2's certificate interface, now measured.
The kernel's conversion front-end parses **AustralVM CoreIR programs** — numeric
expressions, let-bound `Int64` arithmetic — lowered to interaction nets by
`deltanet_unf`. It has no λ, no Π and no substitution node, so it cannot be
handed a CIC term at all; every refused side is one of those. `scripts/
lean-net-cert` drives this per dumped obligation and reports the verdicts
verbatim: on `_simp_1_1`'s ten substitution obligations, **0/10 decided, 10
refused**. The bridge is real and the certificate is real; what is missing is the
*encoding* between the two languages — either λ in the deltanet front-end, or a
term-to-program encoding whose normal forms agree with β. Until one exists, the
`tdefeq` premises in these derivations stay assumptions, and no artifact claims
otherwise.

### 13.9 Projections derive; context conversion is stated but unused

Two rules the fragment lacked are now in `docs/cic_typing.mm0` (theory: 34
term/defs, 28 ax/thm, elaborates clean).

**Projections.** `proj` nodes carry their structure's *type name* (`{"idx",
"struct", "typeName"}`), so the field type is recoverable from the inductive's
constructor type — `Π params. Π fields. S params`, peel `idx` binders, read the
domain. The rule is `ty_proj`: type the structure, name the fact `tfield A i F`,
conclude `tty c (tproj s i) F`. The `tfield` instance is emitted once per
(structure type, index) *from the declaration*, so it records a structure's field
layout rather than assuming a use site's typing.

**Correction (the numbers first reported here were wrong).** The rule as
originally written could never fire: `Export.decls` stores an inductive's record
*itself*, not a wrapper around it, so the lookup `decl[1].get("inductive")` was
always `None` and every projection fell back to the counted leaf. The "171 rules
(was 166)" below was therefore not about projections at all. Two further defects
kept them dead once the lookup was fixed, and both are now fixed and measured:

- the field type of a *parameterised* structure (`Setoid p`, `PProd p q`) needs
  the arguments of the structure type substituted for the constructor's parameter
  binders, innermost first (`subst0`), read off the applied type's spine;
- the emitter's `Enc` had no case for a string literal (`strVal`), which aborted
  the declarations that use one (`AEMeasurable`).

Measured after the fixes (`--spine`, both checkers): `BookProof.OSReconstruction`
**8/8 closures, 1390 rules, 811 assumptions**; `BookProof.Textbook.ChapterF8`
**8/8, 1023 rules**; `BookProof.ChapterNoBestPrior` **6/6, 967 rules**;
`BookProof.Sirk.ChapterKrylovShiftSpan` **6/6, 102 rules**. A single parameterised
projection now derives and checks end to end — `Setoid.r` emits
`tfield (Setoid α) 0 (α → α → Prop)` from the constructor type and its artifact is
accepted by `mm0-rs` and `mm0-c`.

**Context conversion.** `tctx_deq` with `tctx_deq_nil`/`tctx_deq_cons` and
`ty_ctx` now state that entrywise-convertible contexts are interchangeable —
the precondition for the `tvar != tpi` class, which is otherwise unstatable.
It is deliberately **not wired** into the emitter: the emitter builds exactly one
context per proof, so a `ty_ctx` wrapper would relate nothing. The mismatch MM0
reports is between the context a *rule* instantiates and the one the proof was
built in, and §13.12 is the tool that now attributes it.

**On the liquid-types route** (asked as an alternative): `PLAN_liquid_types.md`
says the status is "Done so far: this plan, `docs/LIQUID.md` (L0 spec), and the
`LiquidPragma` carrier in `lib/Common.ml`. Not done: everything else." — so
Lean4 → Austral-with-liquid-types → MM0 would first require building L0
(refinement inference, Why3 discharge, erasure before CPS lowering). It is a
superset of this translator's work rather than a shorter path to the same
obligations, and it would not touch the netcert front-end language gap in §13.8.

### 13.10 The term-to-CoreIR encoding: the front-end has no application node

**Superseded by §13.13.** The conclusion below — that beta is inexpressible — was
reached with the *statement-list* dialect, whose parser has no definitions at all.
The module dialect does have application (`CoreIR::App` plus `link_functions`),
and once its linking was made capture-avoiding and beta-reducing, `netcert`
decides a real beta obligation. The negative result stands only for the dialect the
emitter writes, which is why the encoding below is still circular there.

The plan for §13.8's gap was to encode a CIC term as a CoreIR program so the
deltanet could reduce a substitution for real. The front-end does accept more
than arithmetic — a function declaration translates:

```
netcert 'function f(x: Int64): Int64 is return (x + 1); end;' <same>
  → equivalent, sym_expr (Add64 x 1), witness [[0,0],[1,1],[2,2]]
```

but it has **no application**: a call in the same fragment is not inlined, it is
dropped, and the body comes back instead of the reduct.

```
netcert 'function f(x: Int64): Int64 is return (x + 1); end; return f(3);' 'return (3 + 1);'
  → sym_expr_a (Add64 x 1)   (the `return f(3)` vanished), sym_expr_b 4
  → equivalent: false
```

So the encoding cannot express β at all: `(λx.b) x` has no image under a language
whose only binding form is a named, non-applicable definition. A Gödel-style
structural encoding would be injective and decidable — but it would decide
*syntactic* equality, and the obligations were produced by a substitution pass,
so certifying them that way is circular (it would "prove" `tsubst b x = r` by
checking `r = r`). The honest options remain: λ (or an application node) in the
deltanet front-end, or an encoder whose nets implement substitution — neither
exists, so `netcert` still decides 0 of these obligations and the artifacts keep
the premises as assumptions.

### 13.11 Closure rate against size

Scanned with an empty namespace prefix (`--limit` per chapter), which is what
reaches the declarations this module owns when the namespace differs from the
file stem:

| chapter | closures | rules | assumptions |
| --- | --- | --- | --- |
| `BookProof.Sirk.ChapterKrylovShiftSpan` | 6/6 | 95 | 50 |
| `BookProof.Textbook.ChapterF8` | 1/4 | 349 | 155 |
| `BookProof.ChapterNoBestPrior` | 5/6 | 182 | 121 |
| `BookProof.OSReconstruction` (first 8) | 8/8 | 171 | 106 |
| `BookProof.BalabanRG` (first 12) | 8/12 | 17,068 | 4,317 |

The rate tracks **proof size, not chapter**: small chapters close completely,
`ChapterF8`'s single closing declaration is a 349-rule derivation with 155
assumptions, and the failures concentrate in the largest terms — where one
context-class mismatch (§13.7) is enough to sink the whole declaration, since a
single unproven premise leaves the theorem `sorry`. A closure rate quoted without
the rules column would therefore be misleading at both ends.

### 13.12 A miniature elaborator that attributes the failure

MM0 checks a derivation by unification, so a failure reports the *first* place two
terms differ — with `file:line:col` into a single-line megabyte artifact, no rule
name and no premise. `tvar != tpi` was exactly that: a true message that says
nothing about where the derivation went wrong. `scripts/lean-typing-check` is the
elaborator that attributes it.

It reads the rules **from `docs/cic_typing.mm0` itself** (so theory and checker
cannot drift), parses a proof term out of an emitted artifact, and walks it
bottom-up: unify each rule's conclusion with the goal, then check every premise
against its subproof — MM0 does the same work, but here a failure is *reported*.

```
lean-typing-check <artifact.mm1> [--theory <file>] [--first]
```

Two pieces make it an elaborator rather than a pretty-printer. Rule variables are
alpha-renamed per application (`?7_a`), because two instances of `ty_abs` in one
derivation both use `?a` while the substitution is shared across the whole
theorem. And the substitution *must* be shared: `ty_deq`'s first premise
`tty c t a` has a variable the conclusion never mentions, so its value can only be
read off the subproof (`ty_varS`'s `b` is the same shape) — unification is
therefore symmetric over unknowns on both sides.

A premise discharged by a local `axiom` (a leaf the emitter assumed, a constant's
type, a conversion, a field fact) is reported as **assumed** and counted: the
checker proves the derivation sound *given* those, it does not invent them.

Attribution, on a deliberately broken derivation (leaf used at the wrong context
head — the shape of the `tvar != tpi` class):

```
FAIL  thm_bad
      at      thm_bad
      differs /tty[1]/tcons[1]:tentity
      reason  ax_leaf_0 proves a different statement
      class   tentity != tpi
```

That is the attribution MM0 does not give: the path into the two terms, and the
first pair of symbols that disagree. On real artifacts it re-derives what both
checkers accept (`thm_add_le_`: 9 rule steps, 2 assumed premises, exit 0) and
exits 1 with the same shape of report when one does not.

It also found that the artifacts for primed declarations (`AbsoluteValue.add_le'`)
never reached their proof at all: MM0's lexer rejects a trailing prime, so
`pub theorem thm_add_le':` was a *parse* error and the run was being counted as a
failed derivation. The emitter now sanitises the theorem name.

### 13.13 A real beta obligation, decided

The application node already existed in the conversion front-end (`CoreIR::App`,
`link_functions`); what did not work was the linking. Two defects in
`unfer/logos/src/translate.rs`:

- **capture.** Every function's parameters were used verbatim, so inlining
  `f(x) = x + 1` into `main(x) = f(x)` captured the caller's `x`. Functions are
  now alpha-renamed to unique parameter names before linking, and substitution is
  capture-avoiding (it renames a binder when the argument has that variable free).
- **no beta under a binder.** `rewrite_calls` left the redex for the net reducer,
  which does not reduce under a λ, so the call survived to readback as a lambda
  applied inside its body (the *printed* form of that readback was also wrong, for
  the printer reason given below). The linker now beta-reduces the application
  itself (`subst` + `beta_apply`).

With that, `netcert` decides a genuine beta obligation — the whole point of the
exercise, since a substitution obligation is exactly `(λx.b) a ≡ b[a:=x]`:

```
# main(x) is return f(x);  where  f(x) is return (x + 1);
netcert <that module> <module with main(x) is return (x + 1);>
  → equivalent: true, witness present
netcert <that module> <module with main(x) is return (x + 2);>
  → equivalent: false
```

That decision first rested on a rendering bug, now fixed. Aux slot 1 of an `Abs`
agent *is* its bound variable (`compile_to_net` binds the parameter to
`Port::new(node, 1)`), but the symbolic readback only special-cased the body on
slot 2, so an occurrence of the variable was read as the abstraction itself and
then tripped the cycle guard: `main(x) is return (x + 1)` printed
`(lam x0 (Add64 (lam x0 <cycle>) 1))`. Reading slot 1 as a variable makes the
printed term the term, so the same comparison now reports
`(lam x0 (Add64 x0 1))` on both sides — the verdict was unaffected (it uses
`unf_hash` plus `isomorphism_witness`, not the string), but the string was, and
is no longer, a term the reader has to discount.

`cargo test -p logos --lib` passes (342 tests, including three added for
capture-free linking and for a bound variable reading back as one); the bridge and
its `.so` were rebuilt with `make bridge`, and `dune build test/` passes against it.
Both fixes are committed in the `unfer` repository (`77f2a15`, `fd3c933`).

### 13.14 The sweep: every chapter export, and what it found

A chapter's numbers say nothing about the corpus, and picking the chapters by
hand says less. `scripts/lean-typing-sweep` runs the whole pipeline over every
export: emit (spine derivation), verify with `mm0-rs compile` **and** `mm0-c`,
then re-derive each artifact rule by rule with `scripts/lean-typing-check`.

Two methodological points, because the naive form of this measurement lies:

- **sample across the scope, not the head.** An export lists its dependencies
  first and alphabetically, so `scope[:n]` samples `AbsoluteValue.abs` and
  `Acc.inv` in every chapter. `lean-proof-scan --stride` takes a sample spread
  over the whole scope instead.
- **count declarations, not chapter rows.** The same library declaration arrives
  through dozens of exports. The sweep de-duplicates by name, so the population
  is distinct proofs; without that, a corpus of 26 chapters reported "104
  declarations" that were 9.

Measured over `mm0/exports` (329 exports; the 194 at or below 200 MB covered, the
135 larger ones skipped as out of budget, 3 declarations each):

```
declarations seen  : 582 (176 distinct; 122 theorems and 44 definitions among
                     those MM0 accepted)
closed by mm0      : 166/176 distinct, 49515 rules applied, 16402 assumptions
re-derived         : 165 of those, 0 disagreements with MM0
non-closures       : 7 × emit KeyError: 'type',  3 × emit to_tree: unhandled proj
```

**No disagreement is the result that matters**: wherever MM0 accepted a
derivation, the independent elaborator re-derived it from the same rules. The 10
non-closures were two emitter gaps, and both were real:

- `letE` was expanded to `(λx. b) v` with a synthetic λ that had **no domain**,
  so when the body's type was needed `lam_type` indexed a key the export does
  carry (`letE` has `type`,`value`,`body`) but the synthetic node did not. 7
  declarations died on that `KeyError`; the fix passes the declared type through.
  `lift`, `subst0`, `max_free` and `to_tree` also had no `letE` branch at all,
  which surfaced one layer lower once the domain was supplied.
- `to_tree` had no `proj` branch (the encoder `enc` did, the tree builder did
  not), so a projection in a leaf position raised `unhandled expression
  ['ie','proj']`. 3 declarations.

With both fixed, all 10 emit, elaborate under `mm0-rs`, are accepted by `mm0-c`,
and re-derive — so the corrected census is **176/176 closed and 176/176
re-derived**. The TSV (`chapter, decl, kind, status, rules, assumptions,
selfcheck, detail`) is written by the sweep; the run above took 62 minutes.

The 135 skipped exports are the >200 MB ones; each needs a full export parse, so
covering them is a question of budget rather than of the pipeline, and the
remaining risk is the same two classes, not a new one.

### 13.15 What that sample was actually sampling

The 176 declarations above were not the chapters' own proofs. A chapter export
carries its whole dependency closure — `ChapterSmDiracYukawa` lists 22 079
declarations for 71 of its own — so a stride over the export is a stride over
Mathlib. Measured against each chapter's roots list (the `mm0/statements/<stem>.roots`
files that record which declarations a module itself defines, generated by
`scripts/lean-module-decls.lean`, not guessed from the namespace):

```
rows 582 · distinct 176 · rows naming a chapter-own declaration: 0
```

Zero. Every one of the 176 was a shared dependency, and the ratio explains why
sampling could not have found otherwise: the dependency closure outnumbers the
own declarations by roughly 600:1 in the worst chapter.

Aiming the sample is therefore a change to the sampler, not to the pipeline:

- `scripts/lean-proof-scan --only=FILE` restricts the scope to listed names
  before the stride and limit are applied, so the sample cannot fall outside the
  set;
- `scripts/lean-typing-sweep --own` passes those names (the chapter's roots) and
  skips exports that carry no roots file, instead of silently sampling their
  closure. `--roots-dir` adds further places to look for a roots file, which is
  what the 55 deferred exports need: their statement sets were never converted,
  so they have no `mm0/statements` roots, and they are exactly the >200 MB
  exports. Regenerating those 55 with the same Lean generator took 8 s each
  (`~/.local/state/australVM-lean/mk-roots.sh`) and covered 329/329 exports;
- every row now carries a provenance column (`own` / `dep` / `unknown`) and the
  run prints the split, so a later run cannot report a dependency sample as if
  it were the corpus.

Two sizes had to be bounded, both measured rather than assumed: the emitter's
proof term for a single declaration reached **157 MB** (the checker spent
minutes and 7.7 GB on it, so artifacts over `--check-max-mb` are reported as
over the cap, not as agreement), and `mm0-rs compile` on a 10⁵-rule term ran
**>12 minutes at 13 GB** without finishing (now bounded by `MM0_RS_TIMEOUT`,
240 s in the corpus run, and reported as `mm0-rs: over 240 s`). Neither is a
disagreement; both are limits of the checkers on proof terms that big, and the
sweep reports them as their own classes.

### 13.16 The whole corpus, own declarations only

The sweep that §13.15 argued for was then run to completion — all **329
exports**, no export skipped for size (`--max-mb 10000`), no export without
roots (`coverage gap: 0`):

```bash
SWEEP_SCAN_TIMEOUT=1200 python3 scripts/lean-typing-sweep \
  mm0/exports ~/.local/state/australVM-lean/sweep-own.tsv \
  --limit 2 --max-mb 10000 --own --resume \
  --roots-dir ~/.local/state/australVM-lean/roots
```

```
exports            : 329 covered, 0 skipped (larger than 10000 MB)
sample             : the chapter's own declarations (roots), 0 export(s) had none recorded
declarations seen  : 493 (489 distinct)
provenance         : 489 the chapter's own declaration(s), 0 from its dependency closure
closed by mm0      : 448/489 distinct
rules / assumptions: 7856144 / 1719842
re-derived         : 404 (0 disagreed, 42 over the 32 MB self-check cap, 6 not yet re-checked, 0 timed out)
rules shared with MM0: 1472874/1472874 rule application(s) in the proof terms the check re-derived
non-closures by stage:
     20  emit
     20  mm0-rs
      1  mm0-c
```

The `emit` rows here are the merged outcome of this sweep and the retry of §13.17
(34 of them closed, 13 moved on to the checkers); §13.17 also says why the
`mm0-rs` count grew from 8 to 27 while the `emit` count fell by 57, and §13.17
plus §13.18 account for the 12 further rows the larger-budget passes closed (five
of the eight 88–558 MB artifacts, six `mm0-rs`-bound rows, and one row that the
final whole-corpus retry of §13.17 settled).

So the dependency problem of §13.15 is gone (0 of 489 declarations came from a
closure), and on what is left the pipeline closes **448 of 489** declaration
names and re-derives **404 rows (400 distinct names: a name rooted in two
chapters is re-checked once per artifact)**, with **every one of the 1 472 874
rule
applications in those terms present in MM0** — still 100%, and still with **no
`disagreed` row**. The shared-rule total is only 12.5% above the
176-declaration sample's 1 309 544 despite covering 2.8× the declaration names
and 2.3× the re-derivations (404 against 176): the chapter-own terms average
3 646 rule applications against the sampled dependency terms' 7 441, i.e.
roughly half the size. The 493 rows exceed the 489
distinct names because four declarations are rooted in two chapters each; the
closure and re-derivation counts are per name, the rule totals per row.

This is the census of *that* pass, at a 32 MB self-check cap and a 10 GiB
`mm0-rs` cap; §13.18–§13.20 lift both and end with all 452 closed rows
re-derived and 8 750 509/8 750 509 applications shared (§13.20).

**Does the failure class change with export size?** Yes, and in one direction
only. Splitting the 493 rows at 200 MB (194 exports below, 135 above):

| outcome | ≤200 MB (357 rows) | >200 MB (136 rows) |
| --- | ---: | ---: |
| verified, term re-derived | 299 | 105 |
| closed, term over the self-check cap, not yet re-checked | 36 | 12 |
| `emit`: over 10 GiB (memory bound) | 5 | 12 |
| `emit`: over 900 s (time bound) | 3 | 0 |
| `mm0-rs`: over 10 GiB (memory bound) | 14 | 5 |
| `mm0-rs` artifact, itself over the budget | 0 | 1 |
| `mm0-c` artifact (100 MB) | 0 | 1 |

The `emit`/`mm0-rs` split in this table is the one these budgets measured; §13.21
re-asks both questions at the host's ceiling and moves one row from each side.

The `forallE`/`ie` class of the first pass is gone from this table entirely: all
four declarations close (8–76 s each) once `infer` has a sort branch, and their
`ie` nodes are Π types rather than an inference gap of the encoding.

**The remaining classes are not size-driven the way the old ones were.** Below
200 MB the open rows are `emit` time (3), `emit` memory (5) and `mm0-rs` memory
(14): the elaborator, not the emitter. Above 200 MB the emitter's **remaining**
memory bound is the largest class — 12 of 136 rows (9%, against 53 of 136 = 39%
before the two fixes of §13.17) — followed by `mm0-rs` memory (5) and the two
checker artifacts.

And the class is size-driven after all, just at a much bigger constant: the rows
`mm0-rs` bounds carry artifacts of **300, 338, 345, 349, 360, 380, 382, 386, 474,
506, 552, 557, 558, 586, 607 and 1 024 MB**, plus three that failed before a size
was recorded. The class begins at 300 MB and no row in it is smaller, so the
elaborator's bound is a threshold on length, not on shape, and the constant that
sets it is what §13.19 measures.

So the fixes did change the failure
profile in the direction §13.15 predicted, but they did not remove the >200 MB
class: what they removed is the *overhead* half of it, and what is left is
derivation size on both sides of the pipeline.

**The 15 exports that first reported nothing.** In the first pass these showed
`0/0 closed` — not because they had no own declarations, but because the sweep
only recognised the scanner's failure lines (`✗ name: reason`) and silently
dropped its cap lines (`⏱ name: over 240 s`). Every declaration in those 15
exports had hit a bound, so no row was written and the export looked empty. The
sweep now matches `⏱` as a `CAPPED` row, and the 15 were retried at
`--limit 2` and `SWEEP_SCAN_TIMEOUT=1200`:

```
30 declarations · 6 verified · 2 closed over the self-check cap
13 emit: over 240 s · 2 emit: over 8 GiB · 7 mm0-rs artifact (88–425 MB)
```

In the first pass all 30 of those declarations ended on the emitter cap, so
nothing was recorded at all; at the larger budget 15 still hit that cap, while
the other 15 get a proof term out and then meet the next bound — 7 of them at
`mm0-rs`, which writes an 88–425 MB artifact. That class is counted separately
from disagreement: at the sweep's 240 s budget the checker has not returned a
verdict either way, so there is nothing to agree or disagree with — and §13.17
measures what happens when it is given the time.

### 13.17 Where the 8 GiB went, and what the `mm0-rs` artifacts were

Three follow-ups to §13.16: the 65 declarations whose emission exceeded 8 GiB,
the 4 that died on `['forallE','ie']`, and the 8 artifacts `mm0-rs` never
returned a verdict on.

**The emitter's memory had two causes, and neither was the derivation.**

1. *An eager parse.* `Export.__init__` decoded every NDJSON record into a dict
   before the first declaration was emitted — 3.79 GB for a 248 MB export, held
   for the whole run. `LazyExprs` now keeps a dense `array('q')` of record
   offsets and decodes on demand (`CACHE_MAX` 200 000 records), and the id of an
   expression record is found by `rfind` + `IE_RE` because `"ie"` is last on most
   records but first on `sort` and `lam`.
2. *Unmemoised DAG rebuilds.* `Deriver.lift`/`subst0` rebuilt a shared subterm
   once per occurrence, so one spine derivation synthesised **7.9 M nodes**
   (6.7 GB peak) on a term whose DAG is orders of magnitude smaller. Both are
   now memoised on `(eid, delta, cutoff)` and `(eid, arg, depth)` with
   `MEMO_MAX = 1_000_000` entries, and `Enc.enc` is memoised by eid under a
   64 MB budget — a root string too big to cache stops caching instead of
   pinning the table.

Measured on `BookProof.Textbook.ChapterH1.duhamel_phiOp1`: derivation peak
573 MB, artifact write 679 MB, 8.0 s end to end, and the `.mm0`, `.mm1` and the
obligations file are byte-identical (`cmp`) to the copy the instrumented build
wrote before the sampler was removed.

**What the retry converted.** The 78 rows that were `emit`-bound or
`['forallE','ie']` were re-run at the same 8 GiB / 240 s budgets:

| first-pass class | closed | still memory-bound | still time-bound | reached the checkers |
| --- | ---: | ---: | ---: | ---: |
| `emit`: over 8 GiB (65) | 34 | 18 | 0 | 13 |
| `emit`: over 240 s (13) | 0 | 0 | 7 | 6 |
| `emit`: `['forallE','ie']` (4) | 4 | 0 | 0 | 0 |

so `emit` fell from 82 rows to 25, closed names from 398 to **436**, re-derived
from 371 to **404**, and the emitted rule total from 4 153 375 to **5 616 666**
(assumptions 900 051 → 1 216 576, shared rules 1 368 541 → 1 472 874, still 100%
and still zero disagreements). The 19 rows that now reach the checkers instead of
failing in the emitter are why `mm0-rs` grew from 8 rows to 27: **the memory fix
moved the boundary downstream rather than removing it.** What is left of the
>200 MB class is real derivation size — 18 declarations still exceed 8 GiB, and
the largest of them (a 1.36 M-rule term) builds ~1.4 GB of obligations alone.

**The `forallE`/`ie` gap.** `infer()` had no Π/sort branch, so an `ie` node in a
type position raised `unhandled expression`; the encoding was fine and the
inference was missing. It now raises `Unfoldable(eid)`, which `check`'s
`assume_leaf` records as a *counted* assumption, so the declaration closes with
its assumptions itemised instead of failing outright. All four close and
re-derive: `ChapterB4.pure_state_satisfies_P2` (71 716 rules / 14 026
assumptions), `YangMillsFormClosureND.magCutoffOp_eq_sum_cre` (148 808 / 31 837),
`YangMillsFormClosureND.occProj_comp_magCutoffOp_comp_occProj` (137 463 / 30 009)
and `BrstLongitudinalExact.brstCharge` (38 066 / 8 105).

**The 8 `mm0-rs` artifacts were not malformed: the checker ran out of budget.**
Re-run with `SCAN_MM0_RS_TIMEOUT` raised from the sweep's 240 s:

| artifact | rules | `mm0-rs` | `mm0-c` | verdict |
| --- | ---: | ---: | ---: | --- |
| 34.9 MB | 71 716 | 17.4 s | <1 s | accepted |
| 52 MB | 148 808 | 31 s | <1 s | accepted |
| 88 MB (`WeylHamiltonian.selfAdjoint_sq_isPositive`) | 37 773 | 513 s (with emit) | in that 513 s | closed, 8 742 assumptions |
| 102 MB (`ChapterSirkTruncation.adjoint_reconstruction_eq`) | 37 441 | 828 s (with emit) | in that 828 s | closed, 9 168 assumptions |
| 102 MB, checked by hand | 37 441 | 819 s | 0.82 s / 113 MB | `9195 ax/thm`, no `sorry` |
| 135 MB (`ChapterStoneResolvent.UnboundedSelfAdjoint.approxU_cauchy`) | 50 799 | 343 s (with emit) | in that 343 s | closed, 13 328 assumptions |
| 137 MB (`FriedrichsFormGap.formSpace_norm_bound`) | 101 026 | 293 s (with emit) | in that 293 s | closed |

Four of the eight closed **through the pipeline** (`lean-proof-scan` +
`mm0-rs` + `mm0-c`) at the larger budget, so the class is closed rather than
merely out of time — and not monotonically in size: the 137 MB artifact needed
293 s and the 135 MB one 343 s where the 102 MB one needed 828 s, because what
`mm0-rs` pays for is the length of the longest proof term, not the bytes of the
file. Nothing in those runs is an error: the 102 MB artifact
elaborates in **66 s**
with `-n` (proof checking off) and takes 819 s with it, so ~92% of the cost is
`mm0-rs` checking the proof — and `mm0-c`, the kernel that actually has to
agree, re-verifies the compiled `.mmb` in under a second. The cost tracks *term*
size rather than file size: that artifact is 18 822 lines averaging 5 709
characters (longest 2.57 M) where a 49 MB artifact averages 809, i.e. one huge
proof term instead of many small ones. mm0-rs has no progress or verbosity flag,
so a row that is still elaborating prints nothing at all — which is why the
first pass could not tell the two classes apart from its own output.

Both budgets are therefore budgets, not verdicts, and both are now overridable
(`SCAN_MM0_RS_TIMEOUT`, `SCAN_MM0_RS_MEM_GB`) so a row can be re-measured rather
than only classified; the sweep also stopped overwriting the checker's reason
with the artifact size, which is what made the original 8 rows unclassifiable:

```bash
# one declaration, generous checker budget
SCAN_MM0_RS_TIMEOUT=2700 SCAN_MM0_RS_MEM_GB=12 \
python3 scripts/lean-proof-scan mm0/exports/<chapter>.ndjson "" <work> \
  --only=<one-name-per-line>
```

The retry now re-measures the whole open set in one pass, and it converted one
more row (a 500 KB artifact that the earlier budget had cut off at the wrong
stage), so what remains open is 20 `emit` (17 memory, 3 time), 20 `mm0-rs` (all
of them memory) and 1 `mm0-c` — 41 of 489 declarations, all of them bounded
rather than wrong.

**Five of the eight artifacts closed, and size is not what decides it.** Re-run
through the pipeline itself at a 2700 s `mm0-rs` budget: 88 MB in 513 s, 102 MB
in 828 s, 135 MB in 343 s, 137 MB in 293 s, 240 MB in 1 839 s. The three that
did not (360, 425, 558 MB) died on `mm0-rs`'s **memory**, not its clock, each
within 341–431 s of starting — `over 12 GiB`. So the class splits: a 137 MB
artifact with 101 026 rule applications needs 293 s, while a 102 MB one needs
828 s, because what `mm0-rs` pays for is not the bytes of the file. Six further
`mm0-rs`-bound rows closed in the same pass, the largest a 666 943-rule term
(`QuantumGravity3DGauge.ccr_poly`, 467 s) — which is the useful part of the
answer: **the largest derivations are not the hardest ones to check.**

The whole-corpus retry measured the remaining 41 rows once more at the widest
budgets this machine can hold (900 s / 10 GiB for `emit`, 1 200 s / 10 GiB for
`mm0-rs`): nothing else converted, and every row's `detail` now names the bound
it hit, so "still open" is a measurement rather than an unfinished run.

The self-check had a third bound of its own, and it was a bug rather than a
cap — see §13.18.

### 13.18 The self-check's bound was a bug, and the retry passes are one flag

**The checker, not the cap.** `--check-max-mb 32` was read as a conservative
budget for an expensive check; it was hiding a defect. `scripts/lean-typing-check`
died with `MemoryError` on the 88 MB term at a 9 GiB cap after 147 s, and the
whole of the resident set was in one function:

```python
def instantiate(t, subst):          # before
    if isinstance(t, str):
        return subst.get(t, t)
    return (t[0], *(instantiate(a, subst) for a in t[1:]))
```

A term here is a **DAG** — the same context subtree sits at many positions — and
that rebuild turns every sharing level back into a tree, so the cost is
exponential in the sharing depth rather than linear in the term. `check` calls
it once per proof node (`goal = instantiate(goal, self.subst)` and once per
premise), which is 37 773 times on that artifact.

Three guards, and they are each measurable:

1. A term that mentions no variable at all cannot be changed by any
   substitution, so it is returned unchanged. That answer is cached by identity
   (`_VARLESS`, bounded at 4 096 entries — *unbounded*, the cache retained every
   goal the derivation built and cost 6 GiB in 108 s, more than the rebuild it
   replaced).
2. A term whose only variables are free in the current substitution is returned
   unchanged too (`_bound_by`), which is what stops the rebuild from
   materialising terms it would then throw away.
3. What does get rebuilt is memoised on `id`, so a shared subtree is built once
   and the result is shared — the rebuild is linear in the DAG again.

Measured: the same 88 MB artifact now runs in **1.2–1.4 GB** instead of dying at
9 GiB, and the verdict is unchanged. Semantics were A/B'd against the committed
checker on a 1 MB artifact (1 940 rules, 4 777 proof nodes): old and new print
the same three lines, `4777/4777 rule application(s) shared with MM0 (100%)`,
`1 re-derived, 0 broken`.

**One flag for both retry passes.** §13.17 and the classification above were run
by two ad-hoc scripts (`retry-emit.py`, `classify8.sh`) that each re-implemented
the sweep's loop — which is why one of them wrote its verdict into the wrong
column and could not be folded back without hand-editing. Both are now
`lean-typing-sweep --retry`:

```bash
S=~/.local/state/australVM-lean
scripts/lean-typing-sweep mm0/exports $S/sweep-own.merged.tsv \
  --retry $S/sweep-own.merged.tsv --stages emit,mm0-rs \
  --emit-timeout 900 --emit-mem-gb 10 \
  --mm0rs-timeout 1200 --mm0rs-mem-gb 10 \
  --scan-timeout 5400 --check-max-mb 200 --check-timeout 900
```

`--retry` re-asks each open row's question — was the declaration too large, or
only over the bound the first pass gave it? — aims the scan at exactly those
names, and rewrites the TSV in place, so the TSV *is* the state and re-running
the same command resumes. `--stages` selects which half to re-measure (`emit`,
`mm0-rs`, `mm0-c`, or `selfcheck` for rows that closed but were never
re-derived). Every budget is a flag, and whatever a row hits is written into its
`detail` with the budget in brackets, so "still open" names the bound instead of
suggesting malformed output. `SCAN_EMIT_TIMEOUT` / `SCAN_EMIT_MEM_GB` joined the
`mm0-rs` overrides in `scripts/lean-proof-scan`, so the emitter's bounds are
overridable the same way.

**Three rows that were neither bound.** Re-running the corpus surfaced three
`emit` rows whose `detail` read `OSError: [Errno 122] Disk quota exceeded`. That
is not a budget: `/tmp` on this host is a 16 GB tmpfs, the emitter keeps its own
`TemporaryDirectory` there, and a 541 MB proof term plus the elaborator's copy
fills it — so a disk full surfaced as a pipeline failure under a memory budget.
The retry pass now points its children's `TMPDIR` at `--work-dir`'s disk, and
re-running those three with that scratch changed all three verdicts: they emit
(607, 552 and 1 024 MB) and then hit `mm0-rs`'s memory bound like the rest. It is
worth keeping in mind as a general lesson about this corpus — *the failure text
names the bound only if the harness is told where to put its files*.

**What the self-check costs, per rule.** The remaining `over cap` rows are the
ones whose artifacts were too big to try, so the question is what a check of one
costs and therefore what cap is a budget rather than a guess. Two artifacts,
timed end to end with `/usr/bin/time -v`, both re-deriving at 100% shared:

| artifact | applications shared | wall | peak RSS | per application |
| --- | ---: | ---: | ---: | ---: |
| 82 MB (`physForm_evol_self_le`) | 173 945 | 322.7 s | 1.34 GiB | 1.86 ms / 7.7 KB |
| 137 MB (`formSpace_norm_bound`) | 281 805 | 442.0 s | 2.50 GiB | 1.57 ms / 8.9 KB |

So the cost is linear in **rule applications**, about 1.7 ms and 8 KB each, and
only incidentally in bytes: 82 and 137 MB differ by 1.7× in size and only 1.4×
in time while differing by 1.6× in applications.

The tempting next step — predict each row's check from the `rules` column the
sweep already records — does not work, and the measurements say so directly. A
row's `rules` and the applications its check replays are *different counts and not
proportional*: `physForm_evol_self_le` records 259 533 rules and replays 173 945
applications (0.67×), `formSpace_norm_bound` records 101 026 and replays 281 805
(2.8×). Predictions made from the TSV therefore say nothing about the check —
the column measures emission, the checker measures replay — and the budget has to
come from a measurement of the artifact, not from a column of the table.

The per-application rate also moves with the term: `brst_leakage_bound` replays
803 650 applications in ~2 760 s, 3.4 ms each, twice the table's average. So a
fixed time cap can only ever report "over the cap" for the slow end of the
corpus; it cannot distinguish "nearly done" from "a quarter done".

### 13.19 `mm0-rs`'s memory, measured per node

**The `--mm0rs-mem-gb` cap bounds address space, not RSS.** `cap_mem()` applies
`RLIMIT_AS`, and a growing arena's address space runs ahead of its resident
pages, so a row can sit on the boundary and land on either side of the same cap.
`brst_leakage_bound` is that row: 240.7 MB, 55 532 axioms, and byte-identical
(`md5 519c6f32…`) across every attempt, so nothing about the input differs. Run
without a cap it closes in 1 839 s end to end; run under `--mm0rs-mem-gb 10` the
same bytes stop at `over 10 GiB` after peaking around 9.7 GiB of RSS. "97% of the
cap" is therefore not "nearly passing" — what the row needs is a cap above its
*arena*, and any comparison of rows by cap has to be read as a VM limit. This is
what made the 448 → 447 movement above look like a measurement when it was an
artefact of where the limit was drawn.

Raised to 12 GiB the same row closes and re-derives (803 650/803 650 applications
shared, ~2 760 s), which pins the error to the cap rather than to the corpus: the
10 GiB setting had classified as open four rows — `canH_coe_velH`, `ccr_poly`,
`brst_leakage_bound`, `cauchySeq_of_dense_of_isometry` — whose RSS peaks sit
between 9.7 and 11 GiB. Their reported outcome was therefore a property of the
harness, and the census left by a 10 GiB cap (444 names) is not the census of the
corpus (448).

The class left over from §13.17 is now the largest one, and it was not obvious
what it is a function of. Two measurements settle it, both with
`/usr/bin/time -v` on artifacts the pipeline had already emitted:

| artifact | nodes (`(`) | peak RSS | `mm0-rs compile` |
| --- | ---: | ---: | ---: |
| 1.07 MB (`multiShiftSeq_succ`) | 111 985 | 60.1 MiB | 0.27 s |
| 82 MB (`selfAdjoint_sq_isPositive`) | 9 490 023 | 3.49 GiB | 505 s |

That is one line, not two: **RSS ≈ 19 MB + 385 bytes per node**, and **≈ 53 µs
per node**. The fit is what predicts the failures. The smallest artifact the
10 GiB cap rejects is 315 MB (`trial_cubic_quartic_bounded_below_general`), which
carries 24 796 581 opens: 385 B × 24.8 M ≈ 9.0 GiB of RSS and more than 10 GiB of
address space, so the cap fires after ~400 s — which is exactly when and how it
died. Nothing under 300 MB is in the class because 300 MB is where the line
crosses 10 GiB; the 1 024 MB artifact (`essentiallySelfAdjointOn_add_bounded`)
would need ≈ 30 GiB.

So the bound is the elaborator's constant per node, not the shape of the term,
and the correction matters: §13.17's paragraph guessed "shape, not length" from
artifact sizes that a truncated `report.py` class key had mangled into
`artifact 10`/`artifact 30`. What the sizes actually are is in that table now.

**Duplicate axioms are not the lever.** The 315 MB artifact holds 187 632 `axiom`
statements for only 2 858 distinct bodies — one body is repeated 33 801 times —
which looks like an obvious 98.5% saving. Hashing the bodies shows it is not:
those repeats are the *small* substitution axioms, so deduplicating them removes
14.7% of the text (296.6 MB of axiom text, 253.0 MB distinct) and leaves the node
count where it is. The bytes — and the memory — are in the handful of genuinely
large terms, one of them a single 21.5 MB axiom.

What would move this class is therefore the elaborator itself: 385 B/node is
Rust AST overhead that a shared/DAG representation would cut, and `mm0-rs` reads
the same text as a tree. Until then the honest statement for the translator's
own correctness is the one §13.16 makes: these rows' *emitted* terms are checked
by `mm0-c` (that is what "closed" means), and what `mm0-rs` adds is elaboration,
which is where the 20 rows stop.

**The three `emit: over 900 s` rows are the same knee, later.** The question was
whether the rows that outlast the emitter's 900 s cap are nearly done or are
growing without bound, so one of them was re-run with that cap alone lifted
(5 400 s) and the memory cap left at 10 GiB. `selfAdjointExtension_eq_adjoint`
kept emitting for **1 158.9 s** — only 29% past the cap, which is why "nearly
done" was the tempting reading — and then printed `emit ✗ … over 10 GiB`. It is
not a slow row; it is a row whose term is still growing when the memory bound
stops it, and the time cap happened to fire first only because 900 s is below
where 10 GiB is reached. The remaining two rows of that class are unmeasured at this point:
their probe was lost mid-run (next paragraph), and they are listed as open with
their bound rather than classified — §13.21 measures both.

**`RLIMIT_AS` is per child, not per box.** The re-derivation pass and that probe
ran concurrently, each with every child capped at 10 GiB — a cap that says
nothing about the *sum*, on a 30 GB host already running a `lean` elaboration at
3.1 GiB, a rustc LTO build and a 4.4 GB `/tmp` tmpfs. At 09:32:29 the kernel
reported a **global** OOM (`constraint=CONSTRAINT_NONE`) and killed one of our
`mm0-rs` children — 9.38 GiB resident, i.e. its own cap, `oom_score_adj 200` —
and both driver jobs died with it; the unrelated processes survived. No state
was lost, because the TSV is the resume state (493 rows intact, 421 re-derived,
sharing still 2 939 445/2 939 445), and the pass resumed from its 31 open rows.
The harness fix is procedural rather than a flag: **one sweep at a time**, with
the budgets left where the census above was measured. Lowering them is not the
safe default it looks like — re-running the same pass with the caps at 8 GiB
flipped `brst_leakage_bound` (240.7 MB, closes uncapped in 1 839 s) from `closed`
to `mm0-rs: over 8 GiB`, which moved 234 390 rules out of the closed column and
the closed count from 448 to 447 without the machine or the term changing at
all. A budget is part of the measurement, so tightening it to save memory buys a
*different* result rather than a cheaper one. A per-child limit is a bound on
one process's damage, never on the pipeline's — the same lesson §13.18's
`TMPDIR` finding made for disk.

### 13.20 The corpus re-derives in full: 452 rows, every rule shared

§13.19 ended on four rows that had closed at a larger budget and failed at 10
GiB. The open question was whether the 10 GiB census was the corpus's or the
cap's. It was the cap's. Those four rows, re-run alone at
`--mm0rs-mem-gb 12` with everything else unchanged (their own 4-row TSV, so
only they were visited), all emit, compile and **re-derive**:

| row | rules | applications shared | check |
| --- | ---: | ---: | --- |
| `canH_coe_velH` | 341 811 | 228 909 / 228 909 | re-derived |
| `ccr_poly` | 666 943 | 644 813 / 644 813 | re-derived |
| `brst_leakage_bound` | 234 390 | 803 650 / 803 650 | re-derived, ~2 760 s |
| `cauchySeq_of_dense_of_isometry` | 122 304 | 483 167 / 483 167 | re-derived |

Their `mm0-rs` peak RSS was observed between 9.7 and 11.9 GiB, so the 10 GiB
`RLIMIT_AS` cut them off mid-arena: the earlier classification
(`mm0-rs: over 10 GiB`, four rows and 2 160 539 applications) described the
harness, not the terms. Splice those four verdicts back into the 493-row TSV and the
census is complete — every row that closes is now re-derived:

```
rows 493 · distinct 489 · closed names 448
rules 7856144 assumptions 1719842
selfcheck {'re-derived': 452}
shared 8750509 / 8750509
non-closure: {'emit': 20, 'mm0-rs': 20, 'mm0-c': 1}
```

So **448 of 489** declaration names close in MM0 and **all 452 closed rows
re-derive**, sharing **8 750 509 of 8 750 509 rule applications (100%)** — 1.11×
applications per emitted rule, and no row whose proof term disagrees with its
MM0 proof. The 41 remaining rows are all bounded: 20 `emit` (17 memory, 3 time),
20 `mm0-rs` memory and 1 `mm0-c`, each recorded with the bound it hit.

Two corrections came out of this pass, both about counting:

- **`rules` and *rule applications* are different columns and not
  proportional.** A row's `rules` is what the emitter produced; the number the
  checker replays (and the number `shared`/`nodes` hold) is the applications in
  the proof term. They differ by 0.67× in one direction and 2.8× in the other,
  so the "budget from the TSV" prediction §13.18 drew from them was invalid and
  a row it called a 1 134 s check (`ccr_poly`) really needs about a factor more.
  The per-application cost, measured, is 1.6–3.4 ms.
- **A 452-row re-derivation count against 448 names is not a contradiction.**
  A name rooted in two chapters is emitted into two artifacts and re-checked
  once per artifact, so rows exceed names by exactly those duplicates — the same
  reason §13.16 reported 404 rows for 400 names.

Nothing in the corpus is now left in the state §13.16's census described (closed
but never re-derived). What remains open is not a question about the types: it
is the 41 rows whose term does not fit the machine's budgets, and every one of
them says which budget in its `detail`. That count is final, but the split
between stages is not: §13.21 re-ran the 20 `mm0-rs` rows and the 3 time-bounded
`emit` rows, and one row from each side changed class. The current split is the
one in §13.21.

### 13.21 Both remaining budgets, probed to the host's ceiling (2026-11-11)

§13.20 left exactly two questions about the 41 bounded rows: do the 20 `mm0-rs`
rows close at a larger cap, and do the 3 `emit: over 900 s` rows finish given
more clock? Each was asked in its own TSV — 20 rows across 16 exports, and the 3
rows across their 3 exports — so that no other row was revisited and only one
sweep ran at a time (§13.19).

**The `mm0-rs` answer is 0 of 20, and 12 GiB is as far as this host can ask.**
Re-run at `--mm0rs-mem-gb 12` with the emit budget left at 1 800 s / 10 GiB:

| verdict at 12 GiB | rows |
| --- | ---: |
| still `over 12 GiB` | 19 |
| `over 3600 s` (clock, not memory) | 1 |
| closed and re-derived | **0** |

12 GiB is not an arbitrary stopping point: `MemAvailable` was ~13 GB for the
whole run and `/proc/meminfo` reported `SwapFree: 196 kB`, so a larger
`RLIMIT_AS` would have had the kernel's global OOM killer (§13.19) decide the
outcome instead of the budget. The children grew into it exactly as §13.19's fit
predicts — the largest were observed between **10.6 and 12.3 GiB RSS over 13 to
32 min**, all of them ending well inside their 3 600 s clock. The cap, not the
clock, is what stops these rows.

Nor is the class a little above it. Spending the fit on the cap: 12 GiB of arena
is ~33 M nodes, which is 290–420 MB of artifact text at the 8.6–12.7 B/node the
§13.19 artifacts measure. The class's census sizes are **300–1 024 MB**, so the
cap sits at the class's floor; only its very smallest member could fit, and the
1 024 MB row would need ≈ 30 GiB. Per row, what each one needs is therefore "an
`mm0-rs` arena above 12 GiB", up to ~30 GiB at the top, and the latter is not a
budget this box can offer at all:

| declaration (chapter) | artifact | at 12 GiB |
| --- | ---: | --- |
| `sirk_error_bound_of_crouzeix` (ChapterH2) | 345 MB | over 12 GiB |
| `norm_physEvol_sub_sq_le` (SemigroupContinuity) | 506 MB | over 12 GiB |
| `tendsto_physEvolCompletion_right` (SemigroupContinuity) | 607 MB | over 12 GiB |
| `tendsto_physEvolCompletionRightFun` (TransferSemigroupRight) | 586 MB | over 12 GiB |
| `whiteningEquiv_left_inverse` (ChapterSirkWhitening) | 558 MB | over 12 GiB |
| `hashimoto_multishift_selects_esa` (ChapterEsaClosure) | — | over 12 GiB |
| `compress_isSelfAdjoint` (ChapterCrouzeixSelfAdjoint) | 474 MB | over 12 GiB |
| `Df_lyapCut` (ChapterKvnKatoEsa) | 386 MB | over 12 GiB |
| `apply_sum_of_diagonal` (ChapterHermiteRelativeBound) | — | over 12 GiB |
| `isPosCol_shiftCol_opCol_of_form_gap` (ChapterYangMillsFockGapChain) | 382 MB | over 12 GiB |
| `apply_sum_of_shift` (ChapterHermiteLadderShift) | — | over 12 GiB |
| `commForm_square_comparison` (ChapterFarisLavineDenseCore) | 557 MB | over 12 GiB |
| `symmetricOn_square_comparison` (ChapterFarisLavineDenseCore) | 552 MB | over 12 GiB |
| `deficiencyTrivialAt_of_dense` (ChapterKatoRellichDeficiency) | 360 MB | over 12 GiB |
| `essentiallySelfAdjointOn_add_bounded` (ChapterKatoRellichDeficiency) | 1 024 MB | over 12 GiB |
| `fock_gap_of_bounded_interaction` (ChapterFockInteractionStability) | 380 MB | over 12 GiB |
| `reduceGenerator_isHermitian` (ChapterH7) | 349 MB | over 12 GiB |
| `trial_cubic_quartic_bounded_below_general` (ChapterFockCubicQuarticStability) | 300 MB | **over 3600 s** |
| `compress_sub_const` (ChapterSirkCrouzeixNumericalRange) | 338 MB | over 12 GiB |
| `sirk_error_bound_numericalRange_decay` (ChapterSirkCrouzeixNumericalRange) | 425 MB | over 12 GiB |

Seventeen of the twenty carry a census artifact size; the other three recorded
their bound without one. The single row that swapped memory for time is the
smallest artifact in the class, which is the fit working in the other direction:
at ~24.8 M opens it is the one member whose arena can sit near 10–12 GiB, so it
is slow rather than oversized: 3 600 s was not enough to elaborate it at 12 GiB.
Its verdict is a clock bound reached inside a memory budget that the rest of the
class blows through, so it is the one row here a larger clock alone could move.

**The `emit` answer splits two ways, and only one of the clock's three tries paid.**
The same three rows at `--emit-timeout 5400`, everything else at the budgets
above, then the still-open one again at `--emit-timeout 21600`:

| declaration | at 900 s | at 5 400 s | at 21 600 s | what it needs |
| --- | --- | --- | --- | --- |
| `selfAdjointExtension_eq_adjoint` | over 900 s | emit `over 10 GiB` | — | an emitter arena above 10 GiB — it reaches the bound at ~1 159 s and would keep growing |
| `comm_map_id` | over 900 s | **emitted**; then `mm0-rs` `over 12 GiB` | — | an elaborator arena above 12 GiB; the clock was the only thing blocking the emit |
| `adjoint_aeval` | over 900 s | still emitting at 5 400 s, ~4.2 GB RSS | emit `over 10 GiB` at 8 889 s | an emitter arena above 10 GiB; the clock was never the binding constraint |

`adjoint_aeval` was the one row that looked open-ended: at 5 400 s it was still
emitting with RSS flat at ~4.2 GB, which is what a slow-but-finite elaboration
looks like. The 6-hour clock says otherwise. It does not finish, and it does not
reach the clock either — between 5 400 s and 8 889 s (2 h 28 min of emitting) its
arena grows past 10 GiB and the emitter dies on the bound. So the flat RSS was a
plateau, not a ceiling: the node count keeps climbing and the flat allocation is
just the reused working set.

**That closes the class.** No row is left open in the emit stage because of the
clock: two of the three are the emitter's 10 GiB arena, and the third converted
to a 12 GiB elaborator arena and got further than either. What remains on time is
the single `mm0-rs` row §13.22 already identified — `trial_cubic_…` at
`over 3600 s` — and that is the smallest term in the memory class, i.e. it is
slow *because* it is the one member that does fit its arena, so it is the same
memory story read backwards. Raising clocks converts nothing further; 39 of the
41 open rows are arena size (the 41st is the lone `mm0-c` artifact), which is
what §13.22 sizes.

**One harness bug came out of this, and it cost three hours before it was seen.**
A check verdict was allowed to land on the first row whose declaration matched,
whether or not that row had closed, so a row that failed `mm0-rs` could read
`over 12 GiB; check: over 3600 s` — two stages, one of which never ran on that
artifact. It does now: `scripts/lean-typing-sweep` mutates a check verdict only
on a closed row and skips the stage otherwise, and a killed checker is recorded
as `timeout` instead of aborting the sweep. The price of the fix is that a row
which fails the elaborator no longer re-records its artifact size — which is why
three rows above have no size.

Splicing both probes back in moves one row from each side and no row out of the
open set:

```
rows 493 · distinct 489 · closed names 448
rules 7856144 assumptions 1719842
selfcheck {'re-derived': 452}
shared 8750509 / 8750509
non-closure: {'emit': 19, 'mm0-rs': 21, 'mm0-c': 1}

452  verified
 20  mm0-rs: over 12 GiB
 19  emit: over 10 GiB
  1  mm0-rs: over 3600 s
  1  mm0-c artifact
```

`selfAdjointExtension_eq_adjoint` moves from `emit` time to `emit` memory (it was
counted as time only because 900 s fired before 10 GiB did), `comm_map_id` moves
from `emit` to `mm0-rs`, and `adjoint_aeval` moves from a 5 400 s clock to the
same 10 GiB emit arena at 8 889 s. The totals that matter are unchanged: 448 of 489
names close, all 452 closed rows re-derive, 8 750 509 of 8 750 509 applications
are shared, and every one of the 41 open rows names the budget it hit.

The census and both probes' TSVs and logs are in
[`data/lean4mm0/`](data/lean4mm0/README.md), so these numbers can be re-read
without re-running a sweep.

### 13.22 What the arena would have to be, per node

§13.21 says the 21 memory-bound rows need "an arena above 12 GiB". That is a
true but useless answer, because 12 GiB is where this box stops and the rows are
not all equally far past it. The actionable question is how far `mm0-rs`'s
cost per node has to fall for them to fit the machine it is already on. Both
halves of it are measurable from artifacts that already exist.

**Node counts, counted rather than inferred.** Seventeen of the twenty-one
artifacts survived under `classify-work/` and `emit-long/`; counting `(` in each
gives the nodes `mm0-rs` elaborates, and the census's size-based guesses were far
too low — a 764 MB artifact holds **91.0 M nodes**, not the ~24 M its bytes
suggested at the 315 MB point. Four artifacts were cleaned up and are marked
estimated: their sizes (506, 586, 607, 1 024 MB) times the class's median
9.36 B/node.

At §13.19's measured 385 B/node, fitting 12 GiB is a per-node budget:

| declaration | nodes | arena at 385 B/node | fits 12 GiB at | shrink |
| --- | ---: | ---: | ---: | ---: |
| `essentiallySelfAdjointOn_add_bounded` (est.) | 114 760 260 | 41.1 GiB | 112 B | **3.43×** |
| `apply_sum_of_diagonal` | 91 051 484 | 32.6 GiB | 142 B | 2.72× |
| `apply_sum_of_shift` | 86 331 453 | 31.0 GiB | 149 B | 2.58× |
| `hashimoto_multishift_selects_esa` | 85 486 260 | 30.7 GiB | 151 B | 2.55× |
| `comm_map_id` | 82 212 229 | 29.5 GiB | 157 B | 2.46× |
| `tendsto_physEvolCompletion_right` (est.) | 68 026 833 | 24.4 GiB | 189 B | 2.03× |
| `tendsto_physEvolCompletionRightFun` (est.) | 65 673 351 | 23.5 GiB | 196 B | 1.96× |
| `whiteningEquiv_left_inverse` | 64 500 413 | 23.1 GiB | 200 B | 1.93× |
| `commForm_square_comparison` | 62 171 099 | 22.3 GiB | 207 B | 1.86× |
| `symmetricOn_square_comparison` | 61 930 195 | 22.2 GiB | 208 B | 1.85× |
| `norm_physEvol_sub_sq_le` (est.) | 56 707 706 | 20.3 GiB | 227 B | 1.69× |
| `sirk_error_bound_numericalRange_decay` | 52 722 249 | 18.9 GiB | 244 B | 1.58× |
| `compress_isSelfAdjoint` | 51 274 982 | 18.4 GiB | 251 B | 1.53× |
| `isPosCol_shiftCol_opCol_of_form_gap` | 42 143 625 | 15.1 GiB | 306 B | 1.26× |
| `deficiencyTrivialAt_of_dense` | 41 434 610 | 14.9 GiB | 311 B | 1.24× |
| `sirk_error_bound_of_crouzeix` | 40 248 496 | 14.4 GiB | 320 B | 1.20× |
| `compress_sub_const` | 37 646 935 | 13.5 GiB | 342 B | 1.12× |
| `reduceGenerator_isHermitian` | 37 597 740 | 13.5 GiB | 343 B | 1.12× |
| `Df_lyapCut` | 35 568 621 | 12.8 GiB | 362 B | 1.06× |
| `fock_gap_of_bounded_interaction` | 33 887 707 | 12.2 GiB | 380 B | 1.01× |
| `trial_cubic_quartic_bounded_below_general` | 24 796 432 | 8.9 GiB | 520 B | — fits |

So the harness needs **1.0–3.4× less per node**, and the distribution is not
flat: six of the twenty-one rows would fit at half the current cost (≤ 192 B),
while the four biggest need 2.5–3.4×.

Two of the probe's own verdicts are checked by this table, which is why it is
worth writing down. `trial_cubic_quartic_bounded_below_general` is the one row
§13.21 found clock-bound rather than memory-bound, and at 24.8 M nodes it is the
one row whose arena fits 12 GiB (8.9 GiB) — the table says it should fail on
time, and it did. `fock_gap_of_bounded_interaction` (12.2 GiB) and `Df_lyapCut`
(12.8 GiB) are the two rows the same table puts nearest the cap from above, and
both are recorded `over 12 GiB`. Neither was tuned to agree: the fit comes from
§13.19's two spot measurements and the nodes were counted from the artifacts
that failed.

**A shared AST would be far more than enough — and the text-level repetition is
not where it comes from.** Hash-consing sampled statements (every 17th `axiom`)
from each artifact gives the number of *distinct* subtrees:

| ratio distinct/total nodes | artifacts |
| --- | --- |
| 0.1–0.3% | `whiteningEquiv_left_inverse`, `hashimoto_multishift_selects_esa`, `apply_sum_of_diagonal`, `apply_sum_of_shift`, `commForm_square_comparison`, `symmetricOn_square_comparison`, `deficiencyTrivialAt_of_dense`, `fock_gap_of_bounded_interaction`, `reduceGenerator_isHermitian`, `sirk_error_bound_numericalRange_decay` |
| 0.3–0.6% | `sirk_error_bound_of_crouzeix`, `compress_isSelfAdjoint`, `Df_lyapCut`, `isPosCol_shiftCol_opCol_of_form_gap`, `trial_cubic_quartic_bounded_below_general`, `compress_sub_const` |

A hash-consed term keeps **1 node in 170–1 000** of what the current
representation allocates. At 385 B per *distinct* node the largest **measured**
artifact (`apply_sum_of_diagonal`, 91.0 M nodes at 0.2%) would hold ~182 000
nodes, ~70 MB — a hundredth of the 12 GiB it needs now. The shrink required is
3.4×, so interning does not merely cover it; it makes the question disappear,
and the top rows stop needing a bigger box at all.

That is worth separating from §13.19's dedup finding, which went the other way.
Deduplicating *statements* saves 14.7% of the text, and counting nodes per
statement confirms it: only **0.1–2.4%** of an artifact's nodes sit in repeated
statements (99.7% of `sirk_error_bound_of_crouzeix`'s 40.2 M nodes are in lines
that occur once). The saving is not in the statements; it is in the subterm
tree, where the same `tapp`/`tabs` spine appears again and again inside
different axioms. A fix aimed at repeated axiom *text* would therefore miss it,
and §13.19's conclusion that "what would move this class is the elaborator" is
now quantified: **385 B/node → ≤ 112 B/node**, or share the subtrees and get
~0.4–2 B/node, which is the same sentence as "stop allocating a tree per
statement".

### 13.23 What a shared arena actually costs, measured in mm0-rs

§13.22's "~0.4–2 B/node" was arithmetic: the measured distinct ratio times the
measured 385 B/node. This is the other half — a prototype that builds the two
representations and counts the bytes they hold, using mm0-rs's own parser
(`mm1_parser`) on the artifacts that failed. It lives at
[`data/lean4mm0/arena_proto.rs`](data/lean4mm0/arena_proto.rs), runs as
`cargo run --release --bin arena_proto -- <file.mm1> [stride]` in an mm0-rs
checkout, and its transcripts are in
[`data/lean4mm0/arena-probe.txt`](data/lean4mm0/arena-probe.txt).

A node here is a node of the **math expression**, which is where the text's
parentheses actually live. The lisp parser does not expand `$ … $`: a `Formula`
is a span, and the tree that grows during elaboration is the `QExpr` tree the
math parser builds per statement. So the prototype rebuilds those trees from the
same spans, in two shapes:

- **tree**: `span` + kind + `Box<[child]>` per node — the shape `QExpr` has
  (`IdentApp`/`App` carry a `Span`, a `TermId` and a boxed child slice);
- **arena**: a hash-consed node store keyed on kind plus child ids, with the
  children in one shared pool.

Both are measured by a counting global allocator (bytes requested, not
`size_of` arithmetic), on the two largest artifacts that survived on disk:

| artifact | text | formulas | tree nodes measured | tree B/node | distinct | arena B/distinct | arena B/node |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `apply_sum_of_diagonal` | 800.9 MB | 71 649 | 98 326 003 (half, stride 2) | 40 | 15 523 (0.02%) | 69 | 0.011 |
| `physForm_evol_self_le` | 86.1 MB | 59 135 | 14 758 422 (all) | 40 | 6 627 (0.04%) | 60 | 0.027 |

Two things follow, and the second one corrects §13.22.

**The shared arena is cheap by four orders of magnitude.** 0.011–0.027 B per
node occurrence, against 385 B — because the distinct ratio on math nodes is
0.02–0.04%, not the 0.1–0.6% the statement-level sample gave. The whole
`apply_sum_of_diagonal` formula set interns to 15 523 distinct nodes, 1.08 MB
for half the file. Interning is not a compromise here; it is free.

**The tree is not the 385 B.** At 40 B per tree node, and ~2.2 tree nodes per
parenthesis (each `(` opens one group, and every atom is a node too), the
QExpr-shaped representation is ~88 B per parenthesised node — about a quarter of
what the elaborator actually spends. The full `apply_sum_of_diagonal` set is
~197 M tree nodes ≈ 7.9 GB, which is a large share of a 12 GiB budget but not
all of it, and mm0-rs elaborates statement by statement, so it does not hold all
of them at once: its largest single formula is 2.8 M nodes, 112 MB. So sharing
the formula tree removes a real quarter and leaves the other three quarters
unexplained. §13.22's "385 B/node → ≤ 112 B/node" stands as the requirement; the
answer to *where the 112 B would come from* is now partly measured and partly
open.

The prototype's own bugs are worth one line, because both would have produced a
wrong conclusion quietly: counting only `SExpr` nodes misses every formula (the
lisp parse of the 800 MB artifact is 2.8 M nodes against 87.6 M parens inside
`$ … $`), and a tokenizer that advances one byte per node instead of one atom
inflated the tree by 6.4× — 40 B/node looks the same either way, but the *node
count* and therefore every total did not.

### 13.24 The mm0-rs work, ordered by how many rows it unblocks

§13.22 sized the requirement and §13.23 measured two of its parts. What is left
is a work list, and the ordering that matters is not "biggest win" but "how many
of the 20 memory-bound rows stop failing". Two of the 41 open rows do not appear
here at all: `trial_cubic_quartic_bounded_below_general` is clock-bound at
`mm0-rs` (§13.22), and the 19 `emit` rows are the emitter's arena (§13.21), a
different program.

The ladder below is §13.22's table inverted into a work list: at a per-node
cost of B bytes, the rows that fit are the ones whose need is at least B. The
counts are cumulative, and the last row is the whole class.

| # | change | cost reached | rows unblocked | nearest row it adds |
| ---: | --- | ---: | ---: | --- |
| 1 | Per-phase heap accounting in the elaborator (`--features memory` exists; nothing reports per phase) | — | 0 | §13.23: the formula tree is ~88 B of the 385 B/node, so 3/4 of the cost has no measurement yet |
| 2 | Hash-cons the formula tree (`QExpr`) | −88 | 0 (adds headroom) | §13.23: 0.011–0.027 B/occurrence at 0.02–0.04% distinct |
| 3 | Drop one word per node (385 → 380 B) | 380 | 1 | `fock_gap_of_bounded_interaction` |
| 4 | → 362 B | 362 | 2 | `Df_lyapCut` |
| 5 | → 343 B | 343 | 3 | `reduceGenerator_isHermitian` |
| 6 | → 342 B | 342 | 4 | `compress_sub_const` |
| 7 | → 320 B | 320 | 5 | `sirk_error_bound_of_crouzeix` |
| 8 | → 311 B | 311 | 6 | `deficiencyTrivialAt_of_dense` |
| 9 | → 306 B | 306 | 7 | `isPosCol_shiftCol_opCol_of_form_gap` |
| 10 | → 251 B | 251 | 8 | `compress_isSelfAdjoint` |
| 11 | → 244 B | 244 | 9 | `sirk_error_bound_numericalRange_decay` |
| 12 | → 227 B | 227 | 10 | `norm_physEvol_sub_sq_le` |
| 13 | → 208 B | 208 | 11 | `symmetricOn_square_comparison` |
| 14 | → 200 B | 200 | 13 | `whiteningEquiv_left_inverse` |
| 15 | → 189 B | 189 | 15 | `tendsto_physEvolCompletion_right` |
| 16 | → 157 B | 157 | 16 | `comm_map_id` |
| 17 | → 149 B | 149 | 18 | `apply_sum_of_shift` |
| 18 | → 142 B | 142 | 19 | `apply_sum_of_diagonal` |
| 19 | → 112 B (3.43×) | 112 | 20 | `essentiallySelfAdjointOn_add_bounded` |

The shape of that table is the finding: shaving buys rows one at a time all the
way down — 1.3% of the cost buys the first row, 3.43× is needed for the last —
and the only step that skips the queue is sharing, because sharing is the one
change whose cost per node is already measured at four orders below the budget.
So the ordering is not "shave bytes", it is:

1. **Measure the split** (§13.23 leaves 3/4 of 385 B unaccounted). Any of the
   later steps taken without it is a guess, and §13.19 already paid for one
   guessed cause (the 10 GiB cap looked like the corpus's limit for two rounds).
2. **Intern the formula tree.** Measured, cheap, and it must happen whatever the
   split says, because 88 B/node of it is text-shaped by construction.
3. **Only then** decide between sharing the elaborated structures and shaving
   the per-node layout. The ladder says shaving alone needs 3.43× to clear all
   20 rows and 1.01× to clear one; sharing is the only step that plausibly
   clears them all at once.

The emitter's side of this list is smaller and better understood: §13.21's trace
is measuring what grows there now, and the answer decides whether the 19 `emit`
rows are a memory fix or a representation fix.

### 13.25 The emitter's 10 GiB is accumulated obligation trees, built three times over

§13.24 ended on the question the 19 `emit` rows turn on: is the emitter's 10 GiB
an accumulation of nodes or a representation cost? `adjoint_aeval` was traced
from inside the emitter — a sampler thread appends a TSV row every ~5 s from the
emitter's own state, alongside `tracemalloc`'s totals, `mallinfo2`'s live/free
split and a node count walked over the pending obligation trees. Four readings
were wrong at the start, and each was retired by measuring rather than arguing.

**`exprs` is not a node count.** The column that looked like growth
(1 925 961 → 1 982 337) is the export's expression *index*, plus nodes
synthesised into `extra`; it is flat from t=80 s on. The export is indexed once
and never re-materialised.

**The proof string is not the consumer.** Proofs are assembled as nested
f-strings, so an O(n²) text build was the first hypothesis. Measured `proofb` —
the largest returned proof string — is **1.17 MB at 1.55 GB RSS**. Dead.

**The representation is not the consumer either.** The same loader and `Enc` on
the same declaration with no derivation at all reach **193 MB in 2.0 s**: 80 MB
for the index, 16.7 MB for the value's encoding, 73.1 MB for the encoder memo's
1 724 entries (`enc-probe.py`). Everything above 193 MB is the derivation.

**The terminal outcome is unchanged, and it is the clock that was never
binding.** The 10 GiB run ends at **8 111.9 s of emit** with no output, i.e.
`over 10 GiB` well inside the 21 600 s the row was given (§13.21).

**What the derivation holds.** `obnodes` counts the dicts in
`Deriver.oblig_tree`, the payload the obligation JSONL is written from at the
end: 544 438 at t=80 s, 1.26 M at t=419 s, 1.38 M at t=2 576 s — and flat for
the next 1 282 s. `to_tree` has no cache: it rebuilds `{"bvar":…}`,
`{"const":…}` and `{"app":[…]}` per obligation and the trees stay resident
until the file is written.

The split, one run at a 4 GiB cap (`uord` is glibc's `uordblks`, `free` its
`fordblks`, `mmap` `hblkhd`):

| t (s) | rss | peak | traced | uord | free | mmap | obnodes | obligations | msubst |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10.2 | 211 | 265 | 32 | 133 | 47 | 30 | 0 | 0 | 0 |
| 80.8 | 561 | 562 | 216 | 342 | 4 | 64 | 544 438 | 5 607 | 41 880 |
| 419.1 | 1 103 | 1 111 | 522 | 678 | 9 | 97 | 1 255 850 | 11 018 | 51 891 |
| 846.3 | 1 205 | 1 251 | 553 | 711 | 9 | 164 | 1 280 946 | 11 306 | 55 346 |
| 1 219.8 | 1 397 | 2 084 | 635 | 797 | 391 | 164 | 1 308 146 | 11 597 | 59 602 |
| 2 576.1 | 1 546 | 2 084 | 779 | 950 | 378 | 164 | 1 378 422 | 12 478 | 63 926 |
| 3 160.4 | 1 546 | 2 084 | 779 | 950 | 380 | 164 | 1 378 422 | 12 478 | 63 926 |
| 3 858.3 | 1 548 | 2 084 | 779 | 950 | 381 | 164 | 1 378 422 | 12 478 | 63 926 |

That table measures *composition*, not level, and it has to be read that way:
`tracemalloc` plus a walk of 1.4 M dicts every sample slows the emitter by
roughly an order of magnitude, so this run reaches 12 478 obligations at
t=2 576 s where the terminal run reached the same count at t≈60 s. Two things in
it still hold independent of the slowdown. The ratio `uord`/traced/RSS is stable,
so the shares below are usable. And `free` reaches **381 MB**: a quarter of this
run's RSS is arena the allocator holds but no longer uses, interleaved with live
blocks so `malloc_trim` cannot return it — a real but secondary term, not the
one that reaches the cap. (The last three rows are flat only because they all
fall inside one obligation: the obligation, lift and msubst counters are
identical across them.)

**The terminal run's own curve is the level measurement.** Its 795 samples,
sampled every 10 s with no `tracemalloc` in the process:

| t (s) | rss | obligations | msubst | kB of RSS per obligation |
| ---: | ---: | ---: | ---: | ---: |
| 502 | 1 374 | 24 996 | 90 161 | 55.0 |
| 1 004 | 1 434 | 39 911 | 92 147 | 35.9 |
| 1 506 | 1 750 | 55 068 | 97 098 | 31.8 |
| 2 008 | 1 841 | 70 962 | 99 099 | 25.9 |
| 2 500 | 2 090 | 85 849 | 105 468 | 24.3 |
| 3 505 | 2 278 | 117 327 | 110 284 | 19.4 |
| 4 509 | 2 634 | 152 465 | 119 519 | 17.3 |
| 5 004 | 3 285 | 170 484 | 181 665 | 19.3 |
| 6 006 | 4 304 | 204 511 | 229 828 | 21.0 |
| 7 002 | 5 268 | 237 168 | 252 698 | 22.2 |
| 7 503 | 5 710 | 252 556 | 289 621 | 22.6 |
| 8 020 | **10 013** | 409 586 | 470 584 | 24.4 |

Two regimes, and the ratio between them is the finding. RSS sits **flat for
minutes** while obligations keep arriving — 6 333→6 334 MB from t=7 824 to
7 944, over 3 612 new obligations — and then steps. The step that ends the run
is +3 679 MB in 60 s (t=7 960.7→8 019.6) while obligations go 265 948 → 409 586:
**143 638 obligations at 25.6 kB each**. Cumulative RSS per obligation is
19–24 kB from t=2 008 to the cap. Flat stretches are arena being reused; the
steps are new pages for retirement-age payload. Either way the level is set by
how many obligations have been accumulated, not by how deep the derivation is.

**Where the live heap goes** — snapshot at t=1 099.3 s (RSS 1 383 MB, 620 MB
traced), lines resolved against the revision that produced it:

| line | allocation | MB | objects | B/object |
| ---: | --- | ---: | ---: | ---: |
| 750 | `{"app": [fn, arg]}` in `to_tree` | 165.1 | 2 579 573 | 64 |
| 740 | `{"const": name}` in `to_tree` | 76.6 | 832 162 | 92 |
| 929 | `(tcons …)` type encoding | 70.1 | 189 | 371 kB |
| 1041 | axiom text | 69.5 | 188 | 370 kB |
| 1118 | per-obligation text | 61.6 | 573 | 107 kB |
| 738 | `{"bvar": n}` in `to_tree` | 42.9 | 466 603 | 92 |
| 119 | `(tapp …)` | 34.7 | 305 | 114 kB |
| 121 | `(tpi …)` | 25.4 | 830 | 31 kB |
| 862 | `subst0` app node | 15.5 | 168 814 | 92 |
| 1126 | `(tsubst …)` | 13.8 | 573 | 24 kB |

Structural copies are **284.6 MB in 3 878 338 dicts, 73.4 B per node**; the same
population as text is the ~270 MB of the four string lines, at 24–371 kB per
obligation rather than per node. So the emitter is not node-expensive: at 73 B
a retained node it is already **5.2× below `mm0-rs`'s 385 B/node** (§13.22). It
is *redundant*, in two measured ways. There are 3.0 structural dicts in the heap
for every one node `obnodes` finds in `oblig_tree` (3.88 M vs 1.31 M), so each
obligation's tree is materialised about three times — once for the text, once
into the stored tree, and once more in a representation that does not survive
the obligation's turn. And the population is
large per obligation: 3 878 338 dicts over 11 597 obligations is **334 nodes and
24.5 kB of tree per obligation** — which is the same 24.4 kB per obligation the
terminal run's cap divides out to (§13.21: 10 GiB at 8 111.9 s). Two independent
measurements, one at 1.4 GB and one at 10 GiB, give the same per-obligation
cost. **The emitter's 10 GiB *is* the accumulated obligation trees.**

That answers §13.24's closing question. The 19 `emit` rows are **not** a node
representation problem and not a proof-string problem; they are an accumulation
problem with a measurable redundancy factor, and the fix order follows from the
table rather than from taste:

1. **Cache `to_tree`.** Measured 3× redundancy over 284.6 MB at the 1.4 GB
   point; a cache keyed on the expression id would make it 1×.
2. **Stream the obligation JSONL.** `oblig_tree` exists only to be written
   once at the end, so writing each obligation as it is created removes the
   retained trees entirely instead of shrinking them.
3. **Only then** the arena and the allocator: 381 MB of `free` says the process
   is also holding pages it cannot return, but trimming cannot compact around
   live blocks, so this is the last lever and the smallest.

Steps 1 and 2 are independent: (1) divides the 24.5 kB per obligation by the
measured redundancy factor, (2) removes it from the resident set altogether by
writing each obligation out and dropping it. Neither needs a larger arena.
Both are implemented and measured in §13.26 (7.8× less RSS per obligation,
peak 2 084 → 1 164 MB).

The trace, both snapshots and `enc-probe.py` with its transcript are in
[`data/lean4mm0/`](data/lean4mm0/README.md).

### 13.26 Streaming the obligations is worth 7×, and the axiom text is next

Steps 1 and 2 of §13.25 are implemented in `scripts/lean-typing-proof` and
measured here.

*Streaming.* `Deriver` takes an `oblig_sink`; `emit_obligation()` writes
`json.dumps(obj, sort_keys=True)` and the object is never referenced again, so
`oblig_tree` holds nothing. `main()` opens the `.jsonl` before the derivation
(the target is known by then: the file is `{target}_typing_obligations.jsonl`)
and closes it after `check` returns. The old end-of-run `write_text("\n".join(…))`
also had to hold the whole serialised file as one string; that peak is gone too.

*Memo.* `to_tree` takes a `memo` keyed on the stripped expression id, and the
three sides of one obligation (`b`, `x`, `r`) share one memo. It is the caller's
memo, not the deriver's: a deriver-wide cache would trade duplicate dicts for an
equally large retained one.

**The output is byte-identical.** On
`_private.Init.Prelude.0.Nat.div_rec_lemma.match_1_1` (ChapterA3e, 20
substitution obligations) the old and new versions produce the same `.mm0`
(47 306 B), the same `.mm1` (48 359 B) and the same `.jsonl` (37 340 B), byte for
byte. The streamed 930-record file from the run below parses with zero bad lines.

**Measured on `adjoint_aeval`**, same command, 4 GiB address-space cap, 2 400 s
budget, trace on, no `tracemalloc`; both runs are directly comparable because
the derivation is deterministic (`msubst` reaches 90 161 at the same point in
both):

| obligations | before, RSS | after, RSS | saved |
| ---: | ---: | ---: | ---: |
| 24 867 | 1 374 | 895 | 479 |
| 49 805 | 1 459 | 928 | 531 |
| 85 034 | 1 949 | 1 055 | 894 |
| 100 025 | 2 110 | 1 104 | 1 006 |
| 115 842 | 2 277 | 1 145 | 1 132 |

Peak RSS 2 084 → **1 164 MB**. The marginal cost over the last third of each run
is the number that matters, because it is what a longer budget multiplies:
**24.8 kB per obligation before, 3.2 kB after** — 7.8× less.

**Two things the change did not do.**

First, the run ends at its 2 400 s budget with **210 MB of obligations on disk**
and `obnodes` at 0 for every row. That is the whole point of streaming: the
10 GiB run produced no artifact at all, and the same kill now leaves 930
complete records. The trade is that a budget-killed run leaves a *partial*
`.jsonl`, so a consumer must treat a truncated last line as "discard this file"
— `lean-net-cert` reads it line by line and stops there.

Second, the remaining growth has a different owner. At 116 874 obligations the
largest single accumulator is **axiom text at 444 MB, 39% of RSS** — the
`ax_subst_N` lines themselves, ~12 kB each early on, ~3 kB by the end. Those
bytes are in the `.mm0`/`.mm1` artifacts by design, so they can only be removed
by writing the artifacts incrementally too, which is the same streaming move one
level up. The obligation trees, by contrast, are now bounded by what actually
got written: 4 641 584 printed nodes over 930 records, 45.4 B of JSON per node
against the 73.4 B each node cost as a dict — streaming halves the byte cost of
the same structure on top of not retaining it.

**What this does not establish.** The 1 132 MB saving at 115 842 obligations is
measured as a total; splitting it into "no longer retained" and "no longer
rebuilt" is not. The retained share is *bounded* by the tree population —
4.64 M nodes at 73.4 B is 341 MB at this point — and the rest is build churn and
the arena it left behind, which is the same mechanism §13.25's `free` column
showed. Nor is the end of the run seen: extrapolating the last-third rate,
409 586 obligations would land near 2.1 GB, under the 4 GiB cap and far under
the 10 GiB that stopped the earlier run — so on this declaration the binding
constraint would move from memory back to the clock. That is arithmetic on a
measured rate, not a run that reached 21 600 s.

The two traces are
[`emit-trace-aeval-terminal.tsv`](data/lean4mm0/emit-trace-aeval-terminal.tsv)
(the 10 GiB run, before) and
[`emit-trace-aeval-stream.tsv`](data/lean4mm0/emit-trace-aeval-stream.tsv)
(after).

### 13.27 `adjoint_aeval` finishes: 1 519 MB at the point that used to stop it

§13.26 left one accumulator standing: the axiom text, 444 MB and 39% of RSS at
116 874 obligations, which is in the artifact by design. The artifact is where
it now goes, as it is built.

`Deriver.add_axiom()` writes each assumption to a spool file
(`{stem}_axioms.tmp`) instead of a list. The header that precedes the
assumptions is only known once the derivation returns (it counts the rules), so
the two artifacts are assembled afterwards — prefix, then the spool copied
through `shutil.copyfileobj`, then the derivation — and the spool is unlinked.
The bytes are again identical, not merely equivalent: on the ChapterA3e sample
`.mm0` (47 306 B), `.mm1` (48 359 B) and `.jsonl` (37 340 B) all match the
previous revision exactly.

Rules are counted rather than retained: `self.rules` was one pointer per applied
rule — **1 845 001** of them on this declaration — for a header summary that
needs the count and the order of first appearance.

**The full 21 600 s budget**, same command and declaration as §13.25's terminal
run, 10 GiB address-space cap, trace on:

| obligations | before, RSS | after, RSS |
| ---: | ---: | ---: |
| 99 879 | 2 110 | 582 |
| 200 176 | 4 233 | 775 |
| 314 803 | 6 850 | 1 016 |
| 409 586 | 10 013 | 1 519 |

Where the old run stopped: the before column ends at t=8 019.6 s with 10 013 MB
at 409 586 obligations, killed by the cap. **This run ends at t=5 752.4 s because
the declaration finished**, with peak RSS 1 662 MB — of 21 600 s available, so
the clock had 74% left. What it wrote: a `.mm0` of 1 297 596 934 B (1.21 GiB), a
`.mm1` of 1 351 567 314 B, and 2 139 obligation records totalling
1 290 539 598 B — about 3.8 GiB that used to be resident, or would have been
resident had the run lived that long. 410 137 assumptions (548 constant types,
2 139 substitution obligations) and 1 845 001 rule applications.

**Growth is no longer the story; the two remaining humps are.** From 39 322
obligations at t=626 s (539 MB) to 409 589 at t=5 747 s (1 519 MB) the RSS rises
~2.4 kB per obligation, and the shape is not a staircase: 557 MB at t=1 242 s and
582 MB at t=1 890 s are within noise of each other, because the text leaves the
process as it is made. The one real step is at the end — 920 MB at t=5 676 s to
1 519 MB 72 s later — where the proof string itself is built and wrapped
(`proofb` 53 970 316 B) and both artifacts are copied out.

**The other 18 `emit` rows.** Each was re-run on its own under the budget its
census row records (900 s and 10 GiB for seventeen of them, 5 400 s for
`EsaClosure`), one at a time, artifacts deleted after each row was measured:

| declaration | budget | elapsed | peak | outcome |
| --- | ---: | ---: | ---: | --- |
| `FockPairPerturbation.abs_re_inner_pairVec_le` | 900 s | 189 s | 1 159 MB | emitted 1.04 GiB |
| `EsaClosure.selfAdjointExtension_eq_adjoint` | 5 400 s | 1 158 s | 3 154 MB | emitted 2.73 GiB |
| `SmFarisLavine.CoreData.esa_core` | 900 s | 558 s | 3 105 MB | emitted 2.50 GiB |
| `FullQuadratic.fqOp.eq_1` | 900 s | 56 s | 832 MB | emitted 0.87 GiB |
| `QgOuterFock.qgSectorHam.eq_1` | 900 s | 98 s | 1 319 MB | emitted 1.19 GiB |
| `YmOuterFockFL.abs_levi_le_one` | 900 s | 61 s | 1 506 MB | emitted 0.76 GiB |
| `SmComparisonEsa.equiv_pgLp` | 900 s | 301 s | 3 513 MB | emitted 4.06 GiB |
| `YangMillsHermite.CoreRep._sizeOf_1` | 900 s | 900 s | 8 793 MB | timeout |
| `QgAssembledFlatTransfer.deficiencyTrivialAt_total_of_flat` | 900 s | 853 s | 9 993 MB | `MemoryError` |
| `YmAbelianFock.coreRep_op_add` | 900 s | 901 s | 8 665 MB | timeout |
| `NsOuterFock.nsOuterFock_farisLavine_timeIndependent_singleTime` | 900 s | 167 s | 9 962 MB | `MemoryError` |
| `SirkPerSystem.diagKR_sirk_crouzeix_domain` | 900 s | 900 s | 3 376 MB | timeout |
| `SmHamiltonian.smField.eq_1` | 900 s | 96 s | 1 240 MB | emitted 1.28 GiB |
| `SirkTruncation.transfer_defect_le_of_leakage` | 900 s | 329 s | 2 294 MB | emitted 1.93 GiB |
| `KatoRellich.dense_range_add_relBounded` | 900 s | 901 s | 3 721 MB | timeout |
| `FockFieldPerturbation.abs_re_inner_fieldVec_le` | 900 s | 182 s | 862 MB | emitted 0.83 GiB |
| `ChapterH7.compress_isSelfAdjoint` | 900 s | 802 s | 1 499 MB | emitted 1.10 GiB |
| `FockCubicQuarticStability.cubic_form_eq` | 900 s | 435 s | 1 355 MB | emitted 1.38 GiB |

Fourteen of nineteen emit, thirteen of them inside their old 900 s clock
(`equiv_pgLp` produces 4.06 GiB in 301 s). The bound moved, it did not vanish:

- **Four rows now spend the whole budget under the cap and do not finish.**
  Their peak RSS is 3.4–8.8 GB, so the 10 GiB cap was masking unfinished
  derivations rather than a leak; at 900 s the clock, not memory, ends them.
  Whether they need 2 000 s or 20 000 s is not measured here.
- **Two rows still die at ~10 GiB, but somewhere new.** Both exit with
  `MemoryError` from `emit_obligation`, one at t=167 s and one at t=853 s, in the
  f-string that builds the assumption line and in the `json.dumps` of the
  obligation record. That is one obligation's tree plus its serialization — the
  retained state is gone, and what is left is the size of a single obligation.
  Streaming the record per node instead of building one string would be the next
  thing to try; it is not measured.

**What is not established.** All nineteen are `emit` measurements. None of them
has been through `mm0-rs`, `mm0-c` or the self-check — the census rows stopped at
`emit` and still do — so "the artifact is judged" is open for every one of them,
and a 4.06 GiB `.mm0` is a far larger input to those stages than anything they
have been given. A row that emits is not a closed row.

The full-budget trace is
[`emit-trace-aeval-full.tsv`](data/lean4mm0/emit-trace-aeval-full.tsv), the run's
own report is
[`emit-run-aeval-full.log`](data/lean4mm0/emit-run-aeval-full.log), and the row
table is [`reemit.tsv`](data/lean4mm0/reemit.tsv).

### 13.28 Three copies out of the assumption path; the wall moves into the encoder

§13.27 ended with two rows dying inside `emit_obligation` and described the next
step: stream each obligation's record per node instead of building one JSON
string. That is done, and this section reports what it bought and what it did
not.

**Three copies removed, all of them in the per-obligation path.**

- The obligation record is written **node by node**. `Deriver.emit_obligation`
  used to hand a dict tree to `json.dumps`, which built the whole record as one
  string beside the tree it encoded; now `tree_json` walks the expression graph
  and writes each node as it reaches it — `{"bvar": 3}`, `{"const": ["x", []]}`,
  `{"app": [` … `]}` — and a module-level `_json_write_str` escapes long strings
  in 64 KiB chunks (a chunk edge is moved off a high surrogate, so no pair is
  split), so the two term fields never exist as one string either. Key order and
  separators are `sort_keys=True` and the defaults: `b, id, kind, lhs, r, rhs,
  x`, `", "` and `": "`.
- The assumption line is written in **pieces**. On a β-reduction the line is
  `tdefeq (tsubst {b} {x}) {r}` — three term texts, and joining them into a
  fourth string was the copy that failed at 10 GiB. `add_axiom(*chunks)` writes
  the pieces to the spool in order and then a newline; the retain path still
  joins once, because the in-process callers have the memory.
- The reduct is encoded **once**. The axiom line and the `rhs` field of the
  obligation both need `enc.enc(strip(res))`; the call site computed it twice
  before, holding two multi-GB copies.

**The artifacts are byte-identical, not merely equivalent.** On the ChapterA3e
sample (`.mm0` 47 306 B, `.mm1` 48 359 B, `.jsonl` 37 340 B) and on a
932 584 058 B `FullQuadratic.fqOp.eq_1` artifact (63 MB of obligations), every
file `cmp`s equal to the previous revision's.

**The two `MemoryError` rows, re-run at 1 200 s and 10 GiB.** One still dies, one
does not:

| declaration | before | after |
| --- | --- | --- |
| `NsOuterFock.…singleTime` | `MemoryError` at 167 s, 9 962 MB, in the assumption f-string | `MemoryError` at 277 s, 9 988 MB, in **`Enc._enc`** |
| `QgAssembledFlatTransfer.deficiencyTrivialAt_total_of_flat` | `MemoryError` at 853 s, 9 993 MB, in `json.dumps` of the record | **timeout at 1 201 s**, 9 789 MB — no `MemoryError` |

So the record streaming and the axiom chunking did what they were for: the row
that died in serialization now runs out its clock instead. The other row's wall
is not in the obligation path at all — it is the **term text**: `Enc._enc` builds
`(tapp …)` by recursing into `self.enc(fn)` and `self.enc(arg)` and formats the
results into a new string, so one term is a string the size of its whole tree,
and this declaration's is larger than the memory left after everything else.
That is the next wall, and it is a different change (encoding into a spool,
not into a string); it is not attempted here.

**Where that row's bytes are, measured rather than inferred.** The same row was
re-run with `LTP_TRACE_SNAP=120` (a `tracemalloc` snapshot at t≥120 s, 5-deep
tracebacks). At t=120.7 s RSS is 6 859 MB, peak 7 698 MB, and traced Python
memory is 1 018.8 MB — so four fifths of it is *not* Python objects. Of the
traced part, **1 067.8 MB sits in 20 allocations on one line**, `Enc._enc`'s
`f"(tapp {self.enc(fn)} {self.enc(arg)})"` (line 120): twenty live term strings
of roughly 53 MB each. The export index that dominates the rest is
**8 078 904 expressions** — this chapter's export is simply huge, and the lazy
index is built before the derivation starts. So the row is over the cap for two
independent reasons: an 8 M-expression index (~5.8 GB, not in the traced part)
and a handful of 53 MB term strings on top of it. A streamed encoder would
remove the second, not the first. The snapshot is
[`emit-snapshot-ns-tm120.txt`](data/lean4mm0/emit-snapshot-ns-tm120.txt) (the
`tracemalloc` run itself was cut short by the harness after 787 s, so the row's
outcome is the one in the table above, measured without `tracemalloc`).

**The four 900 s timeouts, given 5 400 s.** Three of them do not need a bigger
clock at all — they need less memory, and the old 900 s timeout was reporting a
clock bound for a memory wall:

| declaration | new clock | outcome | peak |
| --- | ---: | --- | ---: |
| `YangMillsHermite.CoreRep._sizeOf_1` | 5 400 s | `MemoryError` at t=3 618 s | 9 837 MB |
| `YmAbelianFock.coreRep_op_add` | 5 400 s | `MemoryError` at t=24 447 s | 10 111 MB |
| `SirkPerSystem.diagKR_sirk_crouzeix_domain` | 5 400 s | `MemoryError` at t=3 277 s | 9 989 MB |
| `KatoRellich.dense_range_add_relBounded` | 5 400 s | **still unfinished at t=5 401 s** | 7 845 MB |

`dense_range_add_relBounded` is the one genuinely clock-bound row: RSS climbs
steadily (4 668 MB at t=2 000 s with 229 051 obligations, 6 134 MB at 438 541,
7 768 MB at 522 000 at t=5 005 s) and it was killed with memory still 2.4 GB
under the cap. How much time it needs is not measured; a 5 400 s clock is not
enough.

**One caveat on `coreRep_op_add`, reported as measured.** Its row asked for a
5 400 s clock and the row ran 24 447 s before the allocator gave out, so the
clock did not bound it. `timeout` does kill this script when tried on its own
(rc=124 on a 1 s clock applied to a 72 s row), and nothing in the emitter
installs a signal handler, so the cause is in the harness and not in what is
being measured; the row's *outcome* — 2 222 090 obligations, `MemoryError` at
10 111 MB — stands either way.

**`mm0-rs` on the 1.21 GiB artifact: it does not compile within 10 GiB.** Two
runs, because the first was the wrong command:

- `mm0-rs compile adjoint_aeval_typing.mm1` **as emitted** fails in 9 s with
  `error: sort not found` on `term t_lev0: nat;` (line 10). That is not a defect
  of the artifact: the emitter writes a *fragment*, and `scripts/lean-proof-scan`
  prepends the two theories (`docs/cic.mm0`, `docs/cic_typing.mm0`, minus their
  `import` lines) before any stage sees it. Fed the bare file, `mm0-rs` has no
  sorts at all and rejects the first declaration.
- With the theory prepended (1 351 582 571 B, 14 199 B of theory), `mm0-rs`
  starts elaborating and **aborts after ~190 s** with `memory allocation of 112
  bytes failed` (SIGABRT), peak RSS 10 457 MB, no `.mmb` written. So the answer to
  "where does it stop" is: inside `elab`, at the 10 GiB cap, before the proof is
  checked. Whether a larger cap would finish is not measured.

**What is not established.** The 19 `emit` rows are still emit-only measurements.
`mm0-rs` has now been given one artifact — the largest `emit` row there is, 410 137
assumptions — and it does not fit in 10 GiB, so nothing in that table has been
*judged* by a proof checker. The four clock rows are still four, and one of them
(`dense_range_add_relBounded`) has no measured finishing time. The `nsOuterFock`
row needs the term encoder to stream, which is not done.

The measurements of this round are
[`round3.tsv`](data/lean4mm0/round3.tsv).

### 13.29 The encoder streams its term text; the copies were per level, not per term

§13.28 left one row unfinished in `Enc._enc` and named the line: the
`f"(tapp {self.enc(fn)} {self.enc(arg)})"` f-string. That f-string builds a
*finished string at every level of the spine*, so while the outermost term is
being built each level's string is still alive — on
`nsOuterFock…singleTime` the `tracemalloc` snapshot showed twenty of them, about
53 MB each (§13.28). The fix is not to spool the term but to never build the
intermediate strings: `_enc(eid, out)` now **appends tokens** to one list in emit
order, and `enc(eid)` joins that list once at the top. The token sequence is
unchanged, so the bytes are unchanged by construction.

One thing had to be kept, and the first attempt did not: `enc` used to memoise
*every* child encoding within a 64 MB budget, which is what keeps shared subtrees
from being re-encoded. Appending to a list dropped that, and the effect of
dropping it is not subtle — the row that takes 200 s ran 12 minutes without
finishing before the run was killed. So `_enc` now memoises the **small**
encodings (≤ 64 tokens, joined and cached, which covers the shared types and
short spines the budget was actually holding) and deliberately leaves multi-
megabyte terms streamed, which is the point of the change.

**Byte identity, re-measured, not assumed:**

| artifact | bytes | `cmp` |
| --- | ---: | --- |
| `ChapterSirkMultiShift.multiShiftSeq_succ` `.mm0` / `.mm1` / `.jsonl` | 1 012 315 / 1 074 760 / 893 654 | equal |
| `FockPairPerturbation.abs_re_inner_pairVec_le` `.mm0` / `.mm1` / `.jsonl` | 1 041 771 375 / 1 075 343 397 / 1 012 915 095 | equal |

The 1.04 GiB row is the same one §13.27 timed at 189 s, so it doubles as the
speed check: **200.4 s / 1 646 732 kB peak before, 187.6 s / 1 599 224 kB after**
(both runs on this host with the six-hour `dense_range` run of §13.30 in flight,
so the two numbers share their contention).

**What this does not fix.** §13.28 measured where the `nsOuterFock` row's bytes
actually are: about 1.1 GB in those live term strings, and ~5.8 GB in the export
index of 8 078 904 expressions, which is built before the derivation starts.
Removing the strings removes the smaller half. Whether the row now fits under
10 GiB is a measurement this change has made possible, not one it has produced;
it is recorded as §13.30 when the run comes back.

### 13.30 The statement was 3.9 GB before the derivation began

§13.29 ended with the encoder streaming and the `nsOuterFock` row untested. It
was tested, and it failed sooner: `MemoryError` at **t=30.5 s, 8 510 MB**, in
`enc`'s `"".join(parts)`. The instrumented run names the number that decided
everything:

```
ENC-MEMORY eid=8130165 parts=67574067 peak_mb=9391
```

Sixty-seven and a half million tokens for *one* term. Appending tokens removed
the per-level strings but made each token its own Python string, ~57 B of header
and pointer apiece: **~3.8 GB of objects for a few hundred MB of text**. So the
encoder change of §13.29 is necessary and not sufficient, and the fix is at the
level below it: `EncBuf` now coalesces tokens into 64 KiB chunks, and `_enc`
re-encodes a subtree into a buffer of its own when it is memoised instead of
splicing tokens out of the parent's half-built chunk list.

That reached t=50 s. `LTP_STATS` then answered where the rest of the baseline
was, and the answer is not the derivation at all:

```
[stats] export indexed: peak 215 MB
[stats] type encoded (1943238865 B): peak 3986 MB
[stats] value encoded (1943238432 B): peak 5840 MB
```

**The type text is 1.94 GB and the value text is 1.94 GB**, both encoded before
the derivation starts and both then retained, because the statement line `theorem …: $ tty tnil <value> <type> $` is built with an
f-string, and that one f-string was kept twice, once in the `.mm0` tail and once
in the `.mm1` tail. Three copies of 1.94 GB, held from startup: that is the
whole 10 GiB budget, and the derivation's own growth is what actually killed
each run.

The fix is the spool the row needed all along, applied where the bytes are:

- `Enc.spool(eid, path)` writes an encoding straight to a file, so the type and
  value texts exist on disk and never in memory.
- `Enc.write(eid, sink)` and `Enc.head(eid, n)` let a line that is made of
  several term texts be written in pieces — the largest substitution axiom on
  this row is **2 244 970 499 B**, three term texts joined, and all three were
  strings before this.
- The statement is written as pieces (head, term, space, type, `$`) into both
  artifacts instead of being a string that both tails embed.
- A closed value is stated by its own text, so the stripped body is no longer
  encoded when it is not the statement's term: 740 MB of encoding that no line
  of this row ever read.

**Text equality had to survive the streaming.** The substitution memo was keyed
by `(b_text, x_text)`. Keying it by `(b_eid, x_eid)` is cheaper and *wrong*: two
distinct ids can encode to the same text, and the difference showed up as
143 substitution axioms against 141 for the same declaration. The key is now the
sha256 of each streamed text (`Enc.digest`), which has the equality the strings
had and holds 32 B per expression.

**Byte identity, `cmp` against the committed emitter, on the row that exercises
substitution axioms and both artifacts:**

| artifact | bytes | `cmp` |
| --- | ---: | --- |
| `ChapterSirkMultiShift.multiShiftSeq_succ` `.mm0` / `.mm1` / `.jsonl` | 1 012 315 / 1 074 760 / 893 654 | equal |

**The row, measured at every stage** (`LTP_TRACE`, 10 GiB cap, 1 200 s clock,
one run at a time; [`round4.tsv`](data/lean4mm0/round4.tsv)):

| stage | outcome | at | peak |
| --- | --- | ---: | ---: |
| §13.29 encoder (token list) | `MemoryError` in `join`, 67.5 M tokens | 31 s | 8 798 MB |
| + chunked buffer | `MemoryError` writing an axiom; 3.9 GB of statements resident | 50 s | 9 545 MB |
| + statement spooled | `MemoryError`; largest axiom line 2.24 GB as strings | 970 s | 9 749 MB |
| + substitution lines streamed | **runs out its clock**, 9 262 axioms · 44 816 rules, 2 189 MB resident | 1 201 s | **3 985 MB** |

The row now sits at 2.2 GB for the whole derivation, with the 10 GiB cap never
approached, and it does not finish within 1 200 s — which is a different problem
from the one it had, and the one a longer clock tests.

**The 2-hour run found the last joined string.** It died at t=1 673.7 s with
3 722 MB resident and a `MemoryError` in `add_axiom`, not in the encoder: six
axiom sites (`ax_field`, `ax_const` for a constant and for a literal,
`ax_leaf`, `ax_type`, and `ax_deq`) still built their line by putting two full
term texts into one f-string. At that point the two texts were large enough that
the single allocation for their join, with the copies the f-string makes, was
about 4.5 GB — the cap was 7 GB away and the allocation still did not fit. All
six now hand `add_axiom` pieces, so no axiom line on this row is a string:
the largest is **8 254 713 455 B** and it streams.

Byte identity was re-checked after this change on both rows already in the table
and on a third that exercises the converted sites heavily —
`FockPairPerturbation.abs_re_inner_pairVec_le`, `.mm0` 1 041 771 375 B /
`.mm1` 1 075 343 397 B / `.jsonl` 1 012 915 095 B — every file `cmp`-equal to
the previous emitter's output.

**With pieces everywhere, the row runs out a 7 200 s clock instead of dying:**
2 082 MB resident at the end (peak 3 985 MB, which is the startup of the export
index, not the derivation), **125 408 rules · 24 388 assumptions · 24 035
obligations**, 8.1 GB of artifacts on disk. It is not close to finishing at two
hours: past t≈3 170 s the derivation is deep in a subtree where it adds a
handful of rules a minute, so the honest statement is that this row's *memory*
problem is solved (2.1 GB under a 10 GiB cap, from a row that used to die in the
first 31 s) and its *time* problem is now visible for the first time, at a scale
the two-hour clock does not bound.

**`dense_range_add_relBounded` does not need a clock, it needs memory.** §13.28
left it unfinished at its 5 400 s clock (7 845 MB, 522 000 obligations), which
left open whether it was slow or merely outlived its clock. With a 21 600 s
clock it dies anyway — `MemoryError` at **t=5 916.1 s, 10 107 MB**, in
`add_axiom`, with 2 966 164 expressions and **609 298 obligations** written. Six
times the clock changed the outcome not at all: that row's cost is per-obligation
retention, and it reaches the cap before any clock in that range would stop it.

### 13.31 `dense_range_add_relBounded` finishes in 307 s: the context was rebuilt at every leaf

The last paragraph of §13.30 is wrong, and the trace rows that contradict it were
already in the file. At the 10 GiB cap `dense` held **177 MB** in the deriver's
own containers (`accb`), its trimmed RSS was within 17 MB of its RSS, and
`axmax` had not moved for hours: nothing was retained. The row was *slow*, and
the 21 600 s clock was measuring the wrong thing about it.

The cost is one function. Every assumption line that mentions the context —
`ax_const`, `ax_type`, `ax_deq`, `ax_leaf`, the substitution lines — calls
`ctx_str(env)`, which writes one `tcons` around each binder, so a context of *n*
entries is O(n²) characters. A typing derivation stands in a context and walks
every leaf beneath it, so each leaf asks for the same context again; on
`dense`'s sixteen binders the run makes **648 307 lookups for 163 distinct
contexts**, 648 144 of them hits, 48 entries hashed per lookup. The text is now
memoised by the environment itself — a tuple of expression ids, which is already
immutable and already shared — and the memo returns the very string the rebuild
produced, so no output byte changes.

| | before (§13.30) | after |
| --- | --- | --- |
| outcome | `MemoryError` | **finishes** |
| wall clock | 5 916.1 s to the cap | **307.6 s** |
| peak RSS | 10 107 MB (cap) | **2 007 MB** (cap 8 GiB) |
| obligations | 609 298 written | 619 976 written |
| rules · assumptions | — (killed at 2 966 164 expressions) | 2 708 694 · 624 425 |

The artifacts are the ones that row always wanted: `.mm0` 13 396 035 927 B,
`.mm1` 13 623 937 462 B, `.jsonl` 7 546 051 022 B. The last obligation line is
`ax_subst_9696` (538 kB, the exact seven keys `sort_keys` orders), which matches
the 9 697 substitution axioms in the report — the streamed file is complete, not
truncated at the clock.

**Byte identity, on the largest row both emitters finish.** The memo returns the
string the rebuild produced, so the artifacts of a memoised run must be
the committed emitter's byte for byte, and on
`FockPairPerturbation.abs_re_inner_pairVec_le` they are:

| artifact | bytes | `cmp` |
| --- | ---: | --- |
| `.mm0` | 1 041 771 375 | equal |
| `.mm1` | 1 075 343 397 | equal |
| `.jsonl` | 1 012 915 095 | equal |

Same row, same machine, same hour: **73 s with the memo against 258 s
without** — 3.5× on a term of a gigabyte, with the `.mm0` unchanged.
`multiShiftSeq_succ` (§13.30) stays `cmp`-equal as well.

**What the memo costs where it does not pay.** On `nsOuterFock…singleTime` the
contexts are long and the reuse is small: 7 521 lookups, 7 451 hits, 14 entries
hashed per lookup. Measured against the committed emitter on the same row, the
same work takes **334.2 s** (committed) and **335.7 s** (memoised) — the hit path
is cheaper than the rebuild and the miss path is a tuple copy, so the memo is
neutral there rather than harmful. Its retention is 190 MB of context text in
163 strings at `dense`'s peak, inside an 8 GiB cap.

**Three instruments, all env-gated, none of them touching an artifact byte:**

- `LTP_OBTRACE=<path>` writes one line per assumption — `idx`, the seconds
  since the previous one (`gap`), the seconds it took to write (`took`), its
  bytes, and the term the emitter was standing in. This is the instrument that
  separated "the emitter is slow" into "its texts are slow" and "the walk to
  the next text is slow" (below).
- `LTP_STACK=<n>` dumps every thread's stack into `$LTP_TRACE.stack` on every
  n-th trace sample, from the tracing thread itself. A `faulthandler` watchdog
  armed to dump on a timer wrote its first dump straight into the process's
  death — a run vanished at exactly t=120 s, the moment of its first dump — so
  the dump now happens on the thread that is already awake.
- `LTP_CENSUS=1` prints the byte split (export parse table, synthetic nodes,
  encoding memo, context memo, obligation list, memo dicts, const/beta/leaf
  axioms, largest proof, peak RSS) at the end of a run and on a `MemoryError`.

The trace grew `ctxby`, `ctxn`, `ctxcall`, `ctxhit` and `ctxscan` for this
section; every earlier column is unchanged.

#### Where the `nsOuterFock…singleTime` row's time actually goes

§13.30 ended this row at "a handful of rules a minute" and left the cause open.
`LTP_OBTRACE` splits every second of it into two numbers, and the split is not
the one the artifact sizes suggest. Over the 3-hour run's 24 383 assumptions:

| | seconds | share |
| --- | ---: | ---: |
| writing the assumption lines (`took`), 47.8 GB of text | 570 | 12.5 % |
| walking from one assumption to the next (`gap`) | 3 916 | 87.5 % |
| startup (export index, 1.94 GB + 1.94 GB statement spools) | ~640 | rest |

The 38 GB of axiom text §13.30 saw accreting in the tail is written at 60 MB/s
when it is written at all; what takes the minutes is the `infer` that precedes
it. And the gaps are extremely concentrated: **eleven** gaps exceed 200 s and
they sum to 6 324 s of the 8 282 s run — 76 % of the wall clock is eleven
single calls.

| assumption | `gap` s | `took` s | bytes | what it is |
| ---: | ---: | ---: | ---: | --- |
| 9 280 | 1 134 | 0.0 | 2 268 | one `infer` over a shared subtree, output two kilobytes |
| 9 279 | 429 | 85.4 | 5 798 976 831 | the gap is 5× the write of a 5.8 GB line |
| 24 380 | 1 908 | 13.1 | 1 799 569 526 | the longest single call of the run, 32 minutes |
| 24 382 | 523 | 71.8 | 3 857 649 407 | 9 minutes of walk, then 3.9 GB written |
| 24 379 | 81 | 147.1 | 8 254 713 455 | the largest line, 8.25 GB |

The stack samples agree with the split rather than the intuition: in the flat
regions they land in `Deriver.infer`/`_check` recursion, and in the writing
ones in `tree_json` → `Enc._enc_tokens`, never in a container or a memo.

**Can it finish?** Every counter says yes and none of them is unbounded: RSS sits
at **2 250 MB** with the cap at 10 GiB, `axmax` freezes at 8 254 713 455 B after
t=2 700 s, and both long runs converge on the same endpoint — **125 408 rules ·
24 035 obligations** (`ns-long2` reaches the rule count at t=6 965 s, this run
was 7 rules and 5 obligations short when it was stopped by hand at t=8 282 s,
short of its 3-hour clock). What it cannot do is finish inside a clock that has
been tried so far, and the reason is now named: a handful of individual `infer`
calls of 300–1 900 s each, which emit kilobytes apiece. The row's price is
~48 GB of axiom text and a **124 GB** obligation file, and its wall clock is
three to four hours on this host — twice what §13.30 assumed, and *time on
repeated inference*, not memory, not text.

What that leaves, in order: (1) the walk itself — `infer` is re-entered under
the same context for the same expression across those long gaps, and the `ctx`
memo's key (the environment tuple) is already the right key for a per-`(eid,
ctx)` result memo; (2) the row's clock, which should be **14 400 s** once (1)
lands — raising it before that just buys another unfinished run, which is what
§13.30's 7 200 s and this section's 10 800 s both measured; (3) the 124 GB
obligation file, which no consumer of this row needs in one piece.
