# PLAN_sota_kernel_verification — spec → kernel → certificate, at the state of the art

**Status (2026-11-12): plan only. Nothing here is implemented.** This document
is the execution schedule for the next agent; every item names the file it
touches, the command that decides it, and the test that would falsify it. It is
written *after* reading the two reference systems (Lanyon's IdealMHD pipeline;
DeepMind's AlphaProof Code / libsignal) against what already exists in this
tree, so §3 is measurement, not aspiration.

Companion documents: `docs/lean4mm0.md` (§7 ground certificates, §9 trust
budget, §11 set.mm statements, §12 corpus pipeline), `docs/cic.mm0`,
`docs/cic_typing.mm0`, `mm0/pairing/README.md` (the statement-pairing rule),
`scripts/logos2mm0` (the CNL gate's five invariants), `PLAN_liquid_types.md`
(refinements the kernels will eventually want).

Honesty rule (inherited from `PLAN_valen` §5 and `PLAN_avm_pkg`): anything that
cannot run on this host is recorded as **not covered**, never as passing. Every
number quoted in a status line must be produced by a command in this document.

---

## §1 The claim to be earned

Both references are, in different ways, publishing *claims*. Ours has to be a
claim too, and each clause needs a test that can fail:

| # | Claim | Falsified by |
|---|---|---|
| C1 | One language carries the specification, the kernel, and the certificate source. No Racket/Lean/C three-way split. | A workstream artifact whose certificate source is hand-written outside Austral. |
| C2 | Symbolic obligations (identities over a *class* of states), not only ground computations, are certified. | The comparator accepting a certificate that only instantiates one state vector. |
| C3 | The approved statement, not the generated one, is what the proof is checked against. | A weakened definition passing `verify` unchanged. |
| C4 | The compiled artifact the proof is about is content-addressed, so "same kernel" is decidable. | Two different kernels sharing a certificate hash, or one kernel losing its hash across a rebuild. |
| C5 | The root of trust is a frozen verifier small enough to audit, and the trusted theory is a pinned line count. | TCB growth without a `BUDGETS` row. |
| C6 | A question in controlled natural language gets an answer whose truth is the certificate's verdict. | The gate passing while the certificate is tampered (invariant 5 of `scripts/logos2mm0`, generalized). |

C1–C2 are Lanyon's territory. C3, C5 are AlphaProof's. C4 and C6 are ours
already, and are the parts worth defending hardest — neither reference has them.

---

## §2 Reference audit — adopt / reject

### Lanyon (IdealMHD)

Mechanism: a small declarative DSL (state vector, fluxes, wave speeds,
assumptions) is expanded by a deterministic compiler into **sibling** artifacts —
Lean 4 proof text and C kernels — from one AST. The proofs are cheap because the
obligations are ring/field identities closed by `simp; ring_nf; field_simp;
split_ifs <;> linarith`. The float gap is bridged by generated runtime
assertions at a fixed tolerance, which they correctly call tests, not proofs.

- **Adopt:** the shape (small human/agent artifact → deterministic amplification),
  sibling emission from one source of truth, and the *discipline* of naming the
  proof boundary in the artifact (we already do this better: `-- trust-split:`).
- **Adopt with a twist:** their generated runtime assertions. Ours must be
  emitted *from the spec expression the certificate is about*, never hand-written
  in the kernel, or the assertion and the theorem drift apart (W6).
- **Reject:** three languages. Racket for the spec, Lean for the proof, C for the
  kernel is three parsers, three semantics, and two chances to disagree silently.
  Austral + CPS IR + `mm0-c` is one pipeline with a single arbiter.
- **Reject:** `ring_nf` as a dependency we cannot audit. Our substitute is not a
  tactic search — it is normalization plus a congruence chain, which is what
  `scripts/lean2mm0` already emits for ground redexes (W2 lifts it to symbolic).

### AlphaProof Code (libsignal)

Mechanism: a human-approved Lean specification (trusted) + a definition-only
AArch64 semantics (trusted, ~8k lines) + 40–80 agents producing 6.5M lines of
untrusted proof + a **gatekeeper** that accepts only when `comparator` confirms
the proof proves *exactly* the held-out statement and its whole dependency
closure, using only Lean's standard axioms. Specification strength is defended by
three techniques: assumed-leaf accounting, vacuity checks that the hypotheses are
jointly satisfiable, and mutation of the *code* to prove the theorem becomes
false.

- **Adopt (highest value):** the held-out statement + dependency-closure
  comparator. We have the *ingredients* (`mm0/pairing/gen_mm0.py` already strips
  proofs to statements; `-- theory coverage:` already enumerates used theory
  names) and none of the *gate* (W3).
- **Adopt:** `assumed=N` as a pinned budget next to `structural=/grounded=`
  (W4). `scripts/lean-typing-check` already computes the count and throws it
  away after printing.
- **Adopt:** mutate the certified subject, not the emitter (W1 acceptance, W3
  negative control). §11's corrupted-`set.mmu` anchor is our only instance.
- **Adopt, wherever it is cheap:** "try to falsify before proving" — `scripts/
  lean-net-cert` already asks the kernel for a not-equivalent verdict; make that
  the default pre-check for every new obligation class.
- **Reject:** the agent swarm. Our obligations are the kind that admit a
  deterministic certificate; hiring 80 agents to search for one is a strategy for
  a different problem class, and it would put an unauditable megabyte count in the
  TCB's shadow.
- **Adopt, differently (W8):** the definition-only-semantics split. Their ISA
  model is trusted and their Lean proofs are not; here the *source-level* proofs
  already exist, so W8's ISA layer takes them as hypotheses instead of restating
  linearity, refinements and memory safety one level down.
- **Note the asymmetry honestly:** AlphaProof's claim is about the *bytes that
  run* and explicitly excludes the Rust compiler from trust. Our cranelift path
  keeps the OCaml compiler, the bridge, and Cranelift inside trust. W5 narrows
  this; it cannot close it, and the plan must not pretend otherwise.

---

## §3 Current state, as measured in this tree

Exact (grep/ls-verified for this plan):

| capability | where it lives now | state |
|---|---|---|
| frozen MM0 root of trust | `mm0-c` over `.mmb`; CI job `phase-d` | **exists**, in use |
| trusted theory, line-pinned | `docs/cic.mm0` 122 lines, `docs/cic_typing.mm0` 152 lines, `docs/memsafe.mm0` 111, `docs/service_isolation.mm0` 99 | **exists** |
| ground certificates + emitter | `scripts/lean2mm0` 2206 lines (modes `generate`/`check`/`verify`/`selftest`), 7 artifacts in `mm0/*.mm0` | **exists** |
| trust-split budget | `BUDGETS` in `scripts/lean2mm0`, `-- trust-split:` emitted, gated by `scripts/verify-invariants` | **exists** |
| theory-coverage label | `-- theory coverage: used=… unused=…`, scanned from generated decls only | **exists** |
| statement-level corpus (any Lean module) | `scripts/lean-corpus-mm0`, `lean-corpus-sweep`, `lean-corpus-trust`, `mm0/statements/` | **exists** — but statements encode *types*, not content |
| content-level statement over set.mm | `mm0/quasi_rh_7_8.{mm0,mm1}`, `SETMM_LABELS`, 50,031 statements re-checked | **exists**, one instance |
| statement pairing rule + generator | `mm0/pairing/gen_mm0.py` 228 lines, `verify_pairing.py` 191 lines, 10 negative controls + 1 tolerated variant | **exists** |
| CNL → certificate, with tamper anchor | `scripts/logos2mm0` (Peano fragment), `lib/formalize_plugin.ml` (`cnl_` constants) | **exists**, small fragment |
| proof-term derivation check | `scripts/lean-proof-scan`, `lean-typing-proof`, `lean-typing-check` | **exists** |
| kernel obbligation oracle | `safestos/cranelift/.../netcert`, `uk_austral_defeq`, `scripts/lean-net-cert` | **exists** |
| C backend | `--target-type=c` (`lib/CliParser.ml`) | **exists** |
| symbolic (class-of-states) certificate | `mm0/flux1d_certificate.{mm0,mm1}` — 4 obligations over `docs/kernel_alg.mm0`, 15 grounded + 6 structural facts, one hypothesis eliminated by substitution, `mm0-rs` + `mm0-c` accept the `peq` chain | **exists** (flux1d; approval pending) |
| **kernel-spec surface (flux algebra) in Austral** | `examples/verified-kernel/flux1d/` (spec + execution side, obligations sidecar) | **partial** (no CPS-IR carrier yet) |
| comparator: approved statement vs generated | `scripts/mm0-comparator` (diff + closure hash + theory hash, 3 negative controls) | **exists** (S1); `mm0/approved/` empty on purpose |
| **`assumed=N` budget** | `-- assumed:` + `-- witness:` emitted by `au2mm0`, pinned in `BUDGETS` and gated; `lean-typing-check --assumed-json` for Lean artifacts | **exists** (flux1d: `assumed=1`; lean2mm0 gap below) |
| **code fingerprint in the certificate** | `-- code: cps=<sha256> obj=<sha256> target=host`, both digests **checked** by `verify-invariants` — CPS from the bridge's `austral_cps_fingerprint`, object from the AVCE capture of the cranelift-emitted code, `arch=` named | **exists, checked** (W5) |
| **ISA model of the emitted subset** | `mm0/isa.mm0` + `isa.mm1` (23 opcodes; `encdec` **proved** as 23 theorems from the table, `step_refines` **proved** as 3 more; 1 citation reported content-unchecked), `ISA_REGISTRY`, `lib/Cps_codec.ml`, `scripts/isa-census --corpus` | **exists, codec and step relation proved** (W8/S8) |
| **generated float probe** | `mm0/flux1d_probe.c` emitted from the certified AST; compiled and run by `verify-invariants` | **exists** (tested, not proved) |
| **GPU codegen** | — | **absent, and out of scope (§6)** |

### §3.1 Baseline (measured 2026-11-12, before W0/W3)

```
$ bash scripts/verify-invariants 2>&1 | tail -1
verify-invariants: 65 passed, 0 explained gap(s), 0 failed

$ ls mm0/*.mm0 | wc -l
7

$ for f in mm0/*_certificate.mm0; do grep -m1 trust-split "$f"; done
-- trust-split: structural=9  grounded=0 certificates=9     (gap)
-- trust-split: structural=3  grounded=5 certificates=8     (layout)
-- trust-split: structural=17 grounded=0 certificates=17    (linearity)
-- trust-split: structural=2  grounded=0 certificates=2 evidence=9 (service_isolation)
-- trust-split: structural=9  grounded=0 certificates=9     (quasi_rh)

$ wc -l docs/cic.mm0 docs/cic_typing.mm0 docs/memsafe.mm0 docs/service_isolation.mm0 | tail -1
  484 total
```

Every count below is from one of those commands; a number that no command in
this document produces does not belong in it.

### §3.2 After S1 (measured 2026-11-12)

```
$ git diff --exit-code mm0/          # W0: no generated byte changed
(clean)
$ python3 scripts/lean2mm0 check
lean2mm0 check: OK — generated MM0/MM1 in sync with the pinned exports
$ python3 scripts/lean2mm0 selftest
... OK — all four anchors refuse tampered input
$ python3 scripts/mm0-comparator selftest
clean approval accepted; changed numeral refused; weakened theory refused;
tampered approval refused; OK — all three negative controls refuse
$ wc -l docs/kernel_alg.mm0
149 docs/kernel_alg.mm0
$ mm0-rs compile docs/kernel_alg.mm0 /tmp/k.mmb && mm0-c /tmp/k.mmb < docs/kernel_alg.mm0
4 sorts, 20 term/def, 38 ax/thm      (both exit 0)
$ python3 mm0/pairing/verify_pairing.py
15/15 checks passed - pairing rule verified
$ bash scripts/verify-invariants 2>&1 | tail -1
verify-invariants: 67 passed, 7 explained gap(s), 0 failed
  (+1 pass: `W2: docs/kernel_alg.mm0 accepted by mm0-rs + mm0-c`)
```

The seven gaps are `W3: <artifact> has no approved statements`: they are the
approval queue, not a regression. Approval is an owner commit that touches
only `mm0/approved/` (see W3), so this plan does not fill it in itself.

After the owner approved all seven (commit `1e9e74a4`, touching only
`mm0/approved/`) and W1's example gate was wired in:

```
$ python3 scripts/mm0-comparator check
mm0-comparator: 7 approved, 0 gap(s)
$ bash scripts/verify-invariants 2>&1 | tail -1
verify-invariants: 75 passed, 0 explained gap(s), 0 failed
$ cd examples/verified-kernel/flux1d && make && echo $?
0
```

With W1's example gate and `au2mm0`'s two gates wired in:

```
$ bash scripts/verify-invariants 2>&1 | tail -1
verify-invariants: 77 passed, 0 explained gap(s), 0 failed
```

### §3.3 After S4/S5 (measured 2026-11-13)

```
$ bash scripts/verify-invariants 2>&1 | tail -1
verify-invariants: 81 passed, 1 explained gap(s), 0 failed
  [gap] W3: flux1d_certificate has no approved statements (owner commit)
  [pass] W1: Flux1d compiles through the CPS JIT and its four obligations hold …
  [pass] W2/W5/W6: flux1d certificate and probe re-emit byte-identically …
  [detail] flux1d probe: 4 identit(ies), 1000 states, max relative deviation 0.000e+00
  [pass] W6: no certificate claims a float path is proved

$ grep -E "trust-split|assumed:|code:" mm0/flux1d_certificate.mm0
-- trust-split: structural=6 grounded=15 certificates=4
-- assumed: 1 (uniform_state: l = r (a uniform interface))
-- code: cps=cd65d0b46a9f8bc017953572031fc1e2c2d9b374296800f4ca11de776b5ae3c3 obj=unrecorded target=host
-- (re-measured later the same day with the object digest recorded: obj=ceed355c…, see §3.4)

$ cd examples/verified-kernel/flux1d && make && echo $?
CPS JIT: Generated 5 functions (7773 bytes)
CPS JIT: 5 functions compiled
0

$ python3 scripts/au2mm0 selftest
au2mm0 selftest: sign flip refused; division refused as a gap; false obligation
not emitted; new assumption refused; OK
```

The approval queue is empty and stays empty by construction: a new artifact
arrives unapproved, so it shows up as a `[gap]` until an owner commit holds its
statements out.

### §3.4 After W8 + object fingerprint (measured 2026-11-13)

```
$ bash scripts/verify-invariants 2>&1 | tail -1
verify-invariants: 93 passed, 1 explained gap(s), 0 failed
  [pass] W5 (checked): the certificate's cps digest is the bridge's digest of the emitted bytes
  [pass] W5 (checked): the certificate's object digest is the bridge's digest of the compiled code
  [pass] W8: mm0/isa.mm0 + isa.mm1 accepted by mm0-rs + mm0-c, no sorry
  [pass] W8: encdec table and its proofs are current
  [pass] W8: a table entry moved off its byte breaks its proof
  [pass] W8: every gap-hypothesis in the ISA model is cited and pinned
  [pass] W8: scripts/isa-census covers the emitted opcodes
  [pass] W8: the corpus census covers every opcode the examples emit
  [pass] W8: isa-census refuses a model opcode the decoder rejects
  [gap]  W3: isa has no approved statements (owner commit)

$ ./scripts/cps-fingerprint --object /tmp/flux1d.cps
ceed355c0e8a150e68bc0e61439eb62e85fa48eb6f50807f6d637205ec07d2cb arch=aarch64

$ ./scripts/isa-census --corpus
isa-census: 9 modules — 768 opcode read(s), 11 distinct, 23 declared, 23 opcode(s) declared in mm0/isa.mm0
  per module: flux1d.cps: 9 distinct, 557 read(s); fib.cps: 7 distinct, 24 read(s); …;
              kernel.cps: 7 distinct, 18 read(s) — the `uk_version` demo;
              memory-ops.cps: 9 distinct, 43 read(s) — deref/store
  used: 0x01 int_lit, 0x02 var, 0x03 let, 0x04 app, 0x07 return, 0x08 if, 0x10 cmp_lt,
        0x14 cmp_eq, 0x16 and, 0x20 deref, 0x30 store
  result: covered — every opcode used is declared in the decoder and in the artifact

$ scripts/obj-drift-control        # the W5 negative control, measured
before: cps=cd65d0b4… obj=ceed355c… out=0
patched: a semantics-preserving emitter edit (function order), out=0, obj=5b75e8bfdd58…
gate:   [FAIL] W5 (checked): the certificate's obj digest is the bridge's digest …
obj-drift-control: held — … the gate refused it, and the pinned digests returned
```

Note what the census exposes: the certified kernel reads 9 of the 23 declared
opcodes and the whole example corpus reads 11 (it gained `deref` and `store`
once a module existed that used them — see below). Arithmetic in the certified
fragment lowers to `App Austral.Pervasive::trappingAdd`, not to `0x05 add` — that
opcode is emitted only by the while-loop increment in `Compiler_cps.ml` — so
"the emitted subset" is 9 opcodes for the corpus and 23 for the emitter as a
whole, and both numbers are measured rather than asserted.

**What the memory opcodes cost to measure.** `0x20 deref` and `0x30 store` were
declared, decoded by the JIT and proved in the model, but *no module could emit
them*: a dereference was lowered to a call to the `__deref` pseudo-function, and
a store through a reference was lowered to an assignment to a variable named
`__assign_target` that nothing reads — the store was silently dropped. Both are
fixed in `lib/Compiler_cps.ml` (`MDeref → Deref`, `MAssign` on a pointer-like
target → `Store`), and `examples/memory-ops` exercises them: it stores 7 through
a write reference, reads it back, and exits 0. The fix is byte-visible — flux1d's
CPS shrank by 765 bytes (`cps=cd65d0b4…`) while its *object* digest did not
change (`obj=ceed355c…`), which is the expected relation when a pseudo-call is
replaced by the load the backend was already emitting.

Two absences deserve emphasis because they are the ones that make the SOTA claim
currently indefensible:

1. **Every certificate we emit is about a ground computation.** `mm0/gap_certificate`
   is 9 structural / 0 grounded, but all nine are *closed*: they reduce to `ttrue`.
   Lanyon's cheapest theorems range over a state vector `U` with `U_L = U_R` as a
   hypothesis. We have never emitted a certificate with a free variable.
2. **`verify` regenerates the statement from the same export it verifies.** A
   weakened definition is reproduced faithfully by regeneration, so the drift gate
   cannot see it. This is exactly the hole `comparator` was built to close.

### §3.5 After W9.1/W9.2, the opcode producers and the L11 pins (measured 2026-11-18)

```
$ nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants | tail -1
verify-invariants: 157 passed, 8 explained gap(s), 0 failed
  [pass] W9: mm0/clif.mm0 + .mm1 accepted by mm0-rs + mm0-c
  [pass] W9: the CLIF model still claims no theorem
  [pass] W9.3: clif_step is vocabulary — declared, stated, never axiomatized
  [pass] W9.3: the refinement's citations still match their artifacts (5 pinned)
  [pass] W9.3: certification state matches mm0/approved/
  [detail] clif2mm0: OK — 20 form(s), 854 instruction(s) over the corpus, 0 theorems
  [pass] W9: mm0/clif.mm0 is what the corpus lowering measures
  [pass] W9: a form the lowering does not emit is refused
  [detail] 23 opcodes: 15 mapped to a converted model declaration,
           8 structural (no ISA instruction), 0 not covered
  [pass] W9.1: the pinned RV64 model converts and mm0-c accepts it
  [detail] REFUTED as stated: load store
  [pass] W9.3: load/store are refuted as the model's execute_LOAD/execute_STORE
  [detail] 3 sorts, 8 term/def, 0 ax/thm
  [pass] W9.3: the memory projection is stated as vocabulary, accepted by mm0-c
  [pass] W9.3: the projection's citations still match their artifacts (3 pinned)
  [pass] W9.3: the toy model proves the stated projection, not a neighbour
  [pass] W9.3: the toy's citation digest refuses a changed claim
  [detail] 3 sorts, 18 term/def, 48 ax/thm
  [pass] W9.3: the toy projection's theorems check without sorry
  [pass] W9.3: the toy's theorem and axiom counts agree with the plan and AGENTS (28 theorem(s), 20 axiom(s))
  [pass] W9.3: the toy's shape figures match what mm0-rs counts (3 sorts, 18 term/def, 48 ax/thm)
  [detail] mutations refused: toy_jit_store_visible(refused),
           toy_jit_store_load_other(refused), toy_jit_two_cells_preserved(refused),
           toy_jit_fourth_store_drops_oldest(refused), toy_jit_recent_cells_readable(refused),
           toy_jit_third_store_kept(refused), toy_jit_refresh_keeps_other_cell(refused),
           toy_jit_refresh_keeps_other_cell(refused), toy_jit_two_cells_preserved(refused),
           toy_jit_refresh_keeps_held_cells(refused), toy_jit_refresh_no_new_held(refused),
           toy_width4_load_after_wide_store(refused),
           toy_width4_store_load_roundtrip(refused)
  [pass] W9.3: the toy's proofs depend on its axioms (mutating one refuses them)
  [detail] toy-witness: 20/20 axioms hold in the witness; 6/6 rejected candidates fail as they must
  [pass] W9.3: the toy's axioms hold in a concrete model (no counterexample)
  [pass] W9.3: the witness exercises all 20 axioms the toy declares
  [detail] toy-witness: 5 atom(s) (0 = unset), 16276 memories (every shape of at most three cells)
  [pass] W9.3: the witness's verdicts do not change when the domain grows (27 rows at 4 and 5 atoms)
  [detail] lean-witness: 20 axiom(s) proved for an arbitrary atom domain, 6 rejected candidate(s) proved false
  [pass] W9.3: a Lean development proves the same 20 axioms
  [pass] W9.3: the Lean development proves the toy's 28 theorems too
  [pass] W9.3: every Lean declaration rests on Lean's standard foundation only (48 audited)
  [pass] W9.3: the Lean file's code carries no sorry/admit/native_decide/ofReduceBool
  [detail] injected sorry at line 808: lean accepted the file (exit 0), the scan refused it
  [pass] W9.3: the no-sorry scan refuses an injected sorry that Lean itself accepts
  [pass] W9.3: the no-sorry scan does not count banned words inside comments
  [pass] W9.3: both witnesses reject the same candidates
  [detail] lean mutations refused: put-no-truncation(refused@197),
           drop-keeps-written-address(refused@131), notfull-weakened-to-four(refused@229),
           narrow4-becomes-identity(refused@599), keep-order-becomes-put(refused@353)
  [pass] W9.3: the Lean model's proofs depend on its definitions (mutating one refuses them)
  [pass] W9.3: the composition table's counts match the files it records (4 row(s), 0 from mm0/approved/)
  [pass] W9.3: the composition table's closure digests match the files it records (4 row(s), 0 from mm0/approved/)
  [pass] W9.3: every field word a composition cell names has a literal comparison (8 word(s))
  [detail] one axiom dropped from the approval: store_projection_toy(stmts cell 70, approved 69) store_projection_toy(says 20 axioms, approved 19) store_projection_toy(declarations 69, approved 68)
  [pass] W9.3: an approved copy that drops an axiom fails the composition counts
  [pass] Phase D: store_projection.mm0 declares its own vocabulary (no imported theory)
  [gap]  W3: isa has no approved statements (run scripts/mm0-comparator approve isa, then commit mm0/approved/)
  [gap]  W3: slot_layout has no approved statements (run scripts/mm0-comparator approve slot_layout, then commit mm0/approved/)
  [gap]  W3: store_projection has no approved statements (run scripts/mm0-comparator approve store_projection, then commit mm0/approved/)
  [gap]  W3: store_projection_toy has no approved statements (run scripts/mm0-comparator approve store_projection_toy, then commit mm0/approved/)
  [gap]  W3: liquid_split has no approved statements (run scripts/mm0-comparator approve liquid_split, then commit mm0/approved/)
w3-pending: isa(193 stmt) liquid_split(91 stmt) slot_layout(23 stmt) store_projection(11 stmt) store_projection_toy(70 stmt)
w3-approved: flux1d_certificate(100 stmt) gap_certificate(107 stmt) layout_certificate(69 stmt) linearity_certificate(67 stmt) linearity_certificate_spans(71 stmt) quasi_rh_7_8(3 stmt) quasi_rh_certificate(99 stmt) service_isolation_certificate(44 stmt)
  [pass] W3: the plan's §W3 stmts column matches what each artifact holds (5 row(s), 0 approved)
  [pass] W3: every pending §W3 row's artifact re-derives into a staging (5 row(s))
  [pass] W3: an approved §W3 row passes when the approved copy holds what it says
  [pass] W3: a §W3 row that disagrees with mm0/approved/ is refused
  [pass] W3: the plan's §W3 digest column matches the file it describes (5 row(s), 0 approved)
  [pass] W3: one altered statement makes the comparator refuse the approved copy
  [pass] W3: a quoted statements-sha256 that does not hash the statements is refused
  [pass] W3: every §W3 emitter cell names a real command (5 row(s))
  [pass] W3: every approved §W3 row's emitter still produces its approved copy (0 row(s))
  [pass] W3: an approved §W3 row whose emitter left the registry is refused
  [pass] W3: the recorded probe evidence still names live gate lines (21 row(s))
  [pass] W3: every §W3 state cell says whether the row is approved or still waiting on the owner (5 row(s))
  [pass] W3: stage-approvals answers an unknown artifact name with exit 64
  [pass] W3: stage-approvals' artifact list is the §W3 table's rows (5 name(s))
  [pass] W3: stage-approvals stages exactly §W3's 5 pending row(s) (exit 0)
  [pass] W3: stage-approvals stops staging an artifact once its approved copy exists
  [pass] W3: stage-approvals refuses a named artifact that is already approved
  [pass] W3: stage-approvals refuses an artifact that equals its approved copy
  [pass] W3: comparator self-test (all three negative controls refuse)

$ nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants | tail -1
verify-invariants: 156 passed, 9 explained gap(s), 0 failed
  # + W9.1 (no lean shell) and the Lean halves of W9.3 are explained gaps here;
  # python3 comes from the OCaml shell, so the scripts run.
$ bash scripts/isa-census --corpus | head -1
isa-census: 12 modules — 905 opcode read(s), 23 distinct, 23 declared, 23 opcode(s) declared in mm0/isa.mm0
  result: covered — every opcode used is declared in the decoder and in the artifact
$ grep '^-- code:' mm0/flux1d_certificate.mm0
-- code: cps=c10ca9a9…4d11 obj=fac893da…e1df target=host
  # re-emitted by the merge's incoming branch; the statements are byte-identical,
  # only this line moved (obj is host-native: arch=aarch64, so it is a
  # documented gap on an x86_64 host — see AGENTS.md's W5 note)
$ nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-drift-controls all
== liquid-golden (0s)
w3-drift-controls: [gap] liquid-golden: why3 not on PATH, so the pinned-golden check is
                   an explained [gap]; this control is not covered in this shell
                   (run it under the default devShell, which carries why3)
  [gap] not covered in this shell (named above)
...
w3-drift-controls: 13 case(s) held, 1 not covered in this shell, in 35m19s with -j 1
$ LD_LIBRARY_PATH=… dune runtest                              # exit 0
OK / OK / … (15 executable(s), 0 failures)
```

**Measured 2026-11-20 at `8b8eb323`, on an aarch64 host, after `make bridge`:**
the `ocaml` shell reports `166 passed, 6 explained gap(s), 0 failed`
([transcript](docs/data/gate/2026-11-20-ocaml-shell.log)) and the `lean` shell
`175 passed, 4 explained gap(s), 0 failed`
([transcript](docs/data/gate/2026-11-20-lean-shell.log)). The transcript block
above is the branch this was merged from and reads `156 passed, 9 explained
gap(s)`; the merge added checks and closed three of those gaps. In that `ocaml`
shell the six gaps were the four §W3 approvals plus `W9.1: RV64 model not
re-converted` and `W9.3: the Lean witness proofs not checked`, and the
certificate printed `arch=aarch64` there, so its `W5 (checked)` object digest
compared for real: the `[pass]` means the bytes the bridge compiled are the
bytes the certificate pinned.

**Re-measured 2026-11-21 at `d2099104` on this x86_64 host, in all three
shells:** `176 passed, 2 explained gap(s), 0 failed` under
`nix develop "git+file://$PWD"`, `161 passed, 5 explained gap(s), 0 failed`
under `.#ocaml`, and `162 passed, 4 explained gap(s), 0 failed` under `.#lean`.
None of those numbers is a correction of the ones above; each is a different
shell over the same checks, and the differences close exactly. The four
approvals closed their four `[gap] W3: <artifact> has no approved statements`
lines and added one `[pass] W3: <artifact> matches its approved statements` line
each. `.#ocaml` carries `lean` but not `why3`, so the 15 per-file Liquid passes
of the 2026-11-20 shell are the three `Liquid golden…`/`Liquid subset` gaps
there — hence `161 + 15 = 176` against the default shell and `5 − 3 = 2` — while
the eight Lean witness passes replace the `W9.3` gap. `.#lean` re-converts the
RV64 model as well, turning `W9.1` into a pass (`162 = 161 + 1`, `4 = 5 − 1`).
The certificate on this host records `arch=x86_64` with `cps=c10ca9a9…` and
`obj=b138fd6e…`; on a host whose certificate recorded the other arch, the same
check normalises those two lines and reports the incomparable digest as a gap
rather than a failure — see AGENTS.md's W5 note.

One gap is shared by all three shells and is a coverage loss, not a shell fact:
`[gap]  W3: the §W3 row check's approved-row control not run (no  row carries a
count)`. With every §W3 row approved, the two controls `an approved §W3 row
passes when the approved copy holds what it says` and `a §W3 row that disagrees
with mm0/approved/ is refused` have no row left to pick, so they stopped running;
the gate reports that as a gap, never as a pass.

Of the
figures quoted, only `clif2mm0` moved for a reason worth naming: the quoted 854
is now **811 instructions** over the same 12 modules (869 on the branch this was
merged from, before `ab9edf22` stopped initialising a cell for a borrow, which
the merged lowering includes), with the form set unchanged at 20 and the
movement in counters only (one-argument `call` 35 → 50, two-argument `call`
63 → 48, `store` 37 → 52), while the opcode census did not move at all (905
reads, 23 of 23 opcodes) — so this is a CLIF-layer counter change, not a
CPS-layer one. The `w3-approved` sizes and the certificate's statements are
unchanged; its `-- code:` line was re-emitted for this host (see above), which is
the intended relation: a lowering change the CLIF layer measures moves only the
artifact that measures it.

`make test` was not re-run as one target, because its first half is the 35-minute
drift run quoted above; the two halves were measured separately instead, and
each one is the same command the target runs. `drift-controls` exits 0 with
`13 case(s) held, 1 not covered in this shell`: `liquid-golden` is the uncovered
one, because the gate guards the pinned-golden check on `command -v why3` and
reports it as an explained `[gap]` under `.#ocaml`, so the drifted copy has no
`[FAIL]` to produce (the `default` shell that carries why3 cannot be built on
this aarch64 host — `alt-ergo`, see AGENTS.md). Before the case learned to say
that, it failed and `make test` stopped there — the reason this is a `[gap]`
line now rather than a green case is that the shell really cannot produce the
evidence, and a control that silently returns 0 would be the false pass.
`dune runtest` then passed on the same tree: 15 executables, 0 failures. Not
re-run in this measurement: the seven read-only `*-drift-control` steps that
`drift-controls` also drives (the L11b control was run on its own, below; the
L11a control and `obj-drift-control` were not), and the test-program runner.

The three `[FAIL]`s this work produced before those two re-emits were the
expected ones — the certificate pinned the CPS and object digests of the *old*
lowering. They are refreshed with `au2mm0 emit … --code … --obj …`, and the
certificate's statements (and therefore `mm0/approved/flux1d_certificate.stmts`)
are unchanged: only the `-- code:` line moved. That is the intended relation —
a lowering change is a digest diff, not a re-approval.

---

## §4 Workstreams

Sizes are estimates, and each workstream ends at a gate command. W1–W3 are the
critical path; W4–W7 are parallelizable but should not start before W3 lands,
because they all assume an approved-statement substrate.

### W0 — Shared MM0 emission, factored not forked  *(0.5–1 day)*

**Why:** W2 needs a second frontend (Austral/CPS → MM0) and the tree already has
a 1.5k-line emitter whose value is its emission logic, `prim_eval` bridging,
trust-split accounting and coverage labelling. Duplicating that is the mistake
`docs/lean4mm0.md` §0 forbids.

- Extract from `scripts/lean2mm0` a stdlib-only module — `scripts/mm0_emit.py`
  (or a package dir `scripts/mm0lib/`) — containing: declaration writer, literal
  numeral declarations, `prim_eval` bridge emission, trust-split/coverage line
  writers, `BUDGETS` table shape.
- `scripts/lean2mm0` imports it. Behaviour must be **byte-identical**:
  `scripts/lean2mm0 check` still passes, `scripts/verify-invariants` Phase D
  unchanged.
- **Acceptance:** `scripts/lean2mm0 verify` exit 0 and `git diff --exit-code
  mm0/` after regeneration; `scripts/lean2mm0 selftest` still trips both anchors.
- **Landed (2026-11-12):** `scripts/mm0_emit.py` holds `SIGNATURE_TERMS`,
  `trust_split_line`, `coverage_line`, `pinned_split`, `axiom`/`theorem`/
  `pub_theorem`/`defn`/`grounded_axiom`/`prim_bridge`, `numeral_decl`/
  `numeral_lines`, and `check_spec_proof_parity`/`check_proof_refs`.
  `scripts/lean2mm0` imports it and aliases `EmitError` to `TranspileError`.
  `git diff --exit-code mm0/` is clean and `check` + `selftest` pass — the
  refactor is byte-identical, which is the only acceptable outcome.

  **Second frontend migrated (2026-11-12):** `scripts/ledger2mm0` now takes
  `coverage_line`, `trust_split_line`, `defn`, `theorem` and `pub_theorem`
  from `mm0_emit`; its own copies of the first two were deleted. This is the
  migration the shared module existed for — the two emitters that *forked* it
  rather than one that was extracted from it. Accepted the same way:

  ```
  $ python3 scripts/ledger2mm0 check
  ledger2mm0 check: OK — certificates are in sync with the pinned ledger and theory
  $ python3 scripts/ledger2mm0 selftest   # both anchors still trip
  $ python3 scripts/ledger2mm0 verify     # mm0-rs + mm0-c, both artifacts, both anchors
  ```

  Still forked and deliberately left: `mm0/pairing/gen_mm0.py` (it *filters* an
  existing `.mm1`, it does not emit declarations) and `scripts/service_iso2mm0`
  (its axioms and theorem pairs are interleaved with domain logic per pair; a
  shared-writer pass there would be a rewrite, not a deletion, and buys nothing
  until a third frontend needs the same shapes).

### W1 — A kernel-spec surface in Austral (Lanyon's stage 1, miniaturized)  *(1–2 weeks)*

**Why:** the missing input. Not a general-purpose DSL — a *numerical kernel
algebra* carrier restricted to the Total Austral Subset, so `fold` and `match` are
the only iteration and liquid refinements can later attach (`PLAN_liquid_types.md`).

- New example tree `examples/verified-kernel/flux1d/` with:
  - `Flux1d.aui` — the spec: `type State: Free` (density, momentum, energy as
    `Int64` in the exact slice), declared assumptions as comments adjacent to the
    refinements they will become, and the flux as pure Austral function
    (`flux : State -> State`), plus the numerical flux `numFlux : State -> State
    -> State`.
  - `Flux1d.aum` — the kernel side: the same functions compiled for execution, no
    separate C, no separate proof text.
  - `kernel.toml` + a manifest grant (reuse `examples/kernel/`'s pattern) so the
    `modhost`/`uk_*` gate is exercised, not bypassed.
- The spec carries three *obligations* as annotations the compiler preserves into
  CPS IR (extend `CellAttribute`/`Compiler_plugin` carrier the way
  `LiquidPragma` was added in `lib/Common.ml`): `consistency` (`numFlux u u =
  flux u`), `conservation` (interface jumps sum to flux differences),
  `symmetry` (permutation invariance of the residual).
- **Acceptance (all four must hold):**
  1. `make test` green with the new example in the suite.
  2. `dune build` emits CPS IR containing the obligation nodes:
     `./austral --emit-cps=… examples/verified-kernel/flux1d/…` then grep the
     obligation tags.
  3. `NETCERT`/`netcert` decides each obligation's two sides *not equivalent*
     when the kernel is mutated (see W2 negative control) — i.e. the pipeline can
     see the difference before any UI is built.
  4. **Mutation of the subject:** flip one sign in `flux` in a copy of the example
     and require the later W2 certificate step to fail loudly. This is the
     AlphaProof strength test, applied to the spec rather than the emitter.
- **Sizing note:** if the obligation carrier proves invasive to the compiler, the
  fallback is a sidecar `.auspec` file parsed by a new pass — record the decision
  in this document rather than silently changing scope.

**Landed 2026-11-12 (S2, partial):** `examples/verified-kernel/flux1d/` exists —
`Flux1d.aui` (spec surface), `Flux1d.aum` (execution side), `Makefile`,
`README.md`, and **`flux1d.obligations.json` (the sidecar)**. The kernel is a
two-point flux `numFlux(l,r) = 2·f(l) − f(r)` with `f(x) = 3x + x²`; the three
obligations are `consistency` (`numFlux u u = flux u`), `conservation`
(`delta a b + delta b c + delta c a = 0`, the telescoping cycle) and `symmetry`
(`delta l r + delta r l = 0`). `make` compiles with the repo-root `austral` and
runs: `main` checks all three on one state vector and exits 0; the run is gated
by `verify-invariants` as `W1: Flux1d compiles through the CPS JIT and its four
obligations hold at the example state` (four since W4 added `uniform_state`).

**Re-measured 2026-11-13:** the example now compiles and runs through
`--use-cps-jit` — `CPS JIT: Generated 5 functions (7773 bytes)` /
`5 functions compiled`, exit 0 — which is the same CPS the W5 fingerprint
records. Reaching that took two bridge fixes, both found by this gate rather
than by reading the code: the JIT had no `And`/`Or` arms (every Austral module
that conjoints conditions was rejected as `Unknown opcode: 0x16`), and `MDeref`
lowered to a `__deref` callee that nothing registered (the JIT now treats it as
the one-word load `__slot_get` already was).

Acceptance, measured:

1. builds and runs — **met** (`make`; exit 0), and the gate above is now part of
   `scripts/verify-invariants`. `make test` — **met 2026-11-13** on a clean tree
   (`rm -rf _build austral`): `161 tests`, 18 suites `OK`, `0 FAILED`, exit 0,
   under `nix develop .#ocaml` (see the toolchain note below for why not `default`).
2. obligation nodes in CPS IR — **not met, no longer blocked**: the carrier is
   still a compiler change nothing has started on, but the OCaml side now builds
   and tests here, so the `[gap]` that stood here is gone and this is ordinary
   work again. The sidecar remains the fallback this section already sanctions.
3. `netcert` distinguishes a mutated kernel — **not met, and now measured to be
   the wrong instrument**: `netcert` compares the *symbolic expression* it parses
   (`sym_expr_a`/`sym_expr_b` in its JSON) and does not normalize arithmetic, so
   it reports not-equivalent for `2*(3*x + x*x)` vs `6*x + 2*x*x` and for
   `2*f(u) - f(u)` vs `f(u)` alike:

   ```
   $ safestos/cranelift/target/release/netcert '2 * (3 * x + x * x)' '6 * x + 2 * x * x'
   {"equivalent":false,"sym_expr_a":"(Mul64 2 (Mul64 (Add64 (Mul64 3 x) x) x))",
    "sym_expr_b":"(Mul64 (Mul64 (Add64 (Mul64 6 x) 2) x) x)",…,"witness":null}
    exit=1
   ```

   So as an obligation oracle it would answer *not equivalent* for every pair,
   mutated or not — a check that cannot fail is not a check. The algebraic
   direction is W2's (`auv2mm0 selftest` already refuses the weakened scheme,
   item 4 above); `netcert` stays what it is good at, deciding *conversion* of
   typing derivations (`scripts/lean-net-cert`). Recording this reassignment here
   rather than leaving an acceptance criterion pointed at a tool that cannot meet
   it.
4. mutation of the subject — **met at the checker level** by
   `scripts/au2mm0 selftest`, but the mutation that bites is in **`numFlux`**, not
   `flux`: all three obligations hold for *any* physical flux, so weakening
   `flux` is invisible to them by construction (measured: flipping `flux`'s 3 to
   −3 left all three obligations true). Weakening the scheme
   (`numFlux = 2·f(l) − f(r)` → `f(l) − f(r)`) makes `consistency`
   `f − f = f` false and is refused. That distinction is the useful part: these
   statements constrain the numerical scheme, and W2 must add a statement that
   constrains the constitutive law if the flux itself is to be certified.

**The toolchain, measured 2026-11-13.** Getting `make test` to run on this host
took three separate faults, and only the last is in this repo:

1. `nix develop` could not realize its shell at all. The pinned channel's
   `alt-ergo` fails inside its *own* build — `ocamlopt` reports
   `I/O error: as --debug-prefix-map …`, i.e. no assembler is reachable in that
   sandbox — and because `devShells.default` carries the prover, the whole shell
   is unbuildable, taking the OCaml toolchain down with it (this host is
   `aarch64-linux`; the failure reproduced with 36 GB free, so it is not the disk
   that a first attempt blamed). `devShells.ocaml` is now the same nixpkgs minus
   why3/alt-ergo/z3: `dune 3.23.1`, `OCaml 5.4.1`.
2. Two `dune`-independent hangups: `builtins.getFlake` on this working tree copies
   it into the store (`refi/target` alone is 14 GB), so the flake is addressed as
   `git+file://$PWD#ocaml`; and 30 GB of `unfer/target/debug` plus 21 GB of a
   stale `_build` had to go.
3. `test/dune` omitted `../lib/liquid/golden/vc-fold-premises.mlw` from
   `LiquidVCTest`'s `(deps …)`, so the golden was never staged and the test could
   only pass on a `_build` that already contained it. Fixed; that is why the honest
   number above comes from `rm -rf _build austral` rather than a warm tree.

`dune build @runtest --force` still reports 9 failures in `LiquidVCTest`
(`compiling … failed (exit 127)`, empty stderr) — dune re-runs those tests under
its sandbox, where `compiler ()` computes a path to `austral.exe` the sandbox does
not contain. `make test` (the documented entry point) does not take that path and
is green; the forced run is recorded here so nobody mistakes it for a pass.

The sidecar's JSON shape is the shape an obligation node would serialise to, so
when the carrier lands the prover's input does not change — `defs` (named
polynomials with parameters), `obligations` (`lhs`/`rhs` expression trees over
`var`/`int`/`add`/`sub`/`mul`/`neg`/`call`), and `assumptions` for the Int64
wraparound, which is stated in the certificate header rather than left implicit.

### W2 — Symbolic certificate emitter (the actual gap)  *(2–3 weeks)*

**Why:** C2. This is the one component neither we nor anyone open-source has
here, and the reason Lanyon's numbers look impossible.

Shape, deliberately reusing existing machinery:

```
CPS IR obligations ──reflect──▶ integer polynomial normal form (ℤ)
                                    │
                          normalize both sides (Horner / lex order)
                                    │
                    equal coefficients ⇒ congruence chain of teq steps
                                    ▼
              <artifact>.{mm0,mm1} over docs/kernel_alg.mm0 ──▶ mm0-rs ──▶ mm0-c
```

**Landed so far (2026-11-12): the reflection half.** `scripts/au2mm0` reads the
obligation document, substitutes calls, and decides every obligation by two
independent routes — symbolic expansion to a coefficient dict, and exact
sampling at four deterministic points — refusing when they disagree. It refuses
out-of-fragment operators (`div`/`mod`/`float`) as a **gap** (exit 2), and its
`selftest` owns two anchors: a weakened scheme is refused, and a division is
refused as a gap. `emit` is deliberately **not implemented and exits 2**: a
certificate needs a proof chain (`peq` steps, one ring law each), and this tool
will not write a statement it cannot prove. That prover is W2's remaining body
of work; the design it must implement is pinned below.

Prover design (do not improvise a weaker one): normalize `psub lhs rhs` to a
flat multiset of *unit-coefficient* monomials (a coefficient `c` becomes `c`
repeated summands, so no literal arithmetic is needed — `padd_neg` cancels
pairs and `padd_zero` clears the spine), materialise each literal as a grounded
decimal fact `peq (tint nC) <sum of ones>` (the same posture as `teval` in
`docs/cic.mm0`), and emit the chain as `peq_trans` steps carried through
subterm positions by `peq_cong_add`/`peq_cong_mul`/`peq_cong_neg`. Coefficient
arithmetic never appears, so `litadd`/`litmul` stay unused for this fragment.

- **New theory `docs/kernel_alg.mm0`** — *written 2026-11-12, extended 2026-11-12*:
  164 lines, `mm0-rs compile` and `mm0-c` both exit 0 (4 sorts, 20 term/def,
  44 ax/thm). The five laws added since the first draft (`padd_zero_l`,
  `pneg_add`, `pmul_neg_l`, `pmul_neg_r`, `pneg_zero`, `pmul_zero_l`) are the
  ones a normalizer needs to move a sign from a sum to its terms; each is an
  identity of every commutative ring, i.e. the same standard as `padd_assoc`,
  and the artifact header says which of them a certificate used.

**Landed (2026-11-12): the first symbolic certificate.** `scripts/mm0_ring.py`
is a proof-producing normalizer, and `scripts/au2mm0 emit` writes
`mm0/<stem>_certificate.{mm0,mm1}`:

```
$ python3 scripts/au2mm0 emit examples/verified-kernel/flux1d/flux1d.obligations.json
au2mm0: wrote mm0/flux1d_certificate.mm0 and flux1d_certificate.mm1
        (3 certificate(s), 2 helper(s), 15 grounded fact(s))
au2mm0: flux1d: mm0-rs + mm0-c accepted the certificate
$ grep trust-split mm0/flux1d_certificate.mm0
-- trust-split: structural=5 grounded=15 certificates=3
```

(W4 superseded these numbers with a fourth obligation: the committed artifact now
reads `-- trust-split: structural=6 grounded=15 certificates=4` and
`-- assumed: 1`; the block above is the S3 measurement it was met on.)

What that means, precisely: the three statements in
`mm0/flux1d_certificate.mm0` are the kernel's obligations written as
polynomial identities (`consistency`: `2·f − f = f`; `conservation`:
the cycle residuals sum to 0; `symmetry`: `delta l r + delta r l = 0`), and
`mm0-c` re-checks a `peq` chain for each — one ring law per step, with the
coefficient arithmetic grounded as 15 visible `cf_` decimal facts, each
re-checked by exact integer sampling before it is written. The chain is closed
by the theory's single trusted `int_ring_transfer`.

The proof design was validated against the root of trust *before* the
normalizer was written: `mm0` proof applications take no term arguments, so a
step is a bare law name (`(padd_comm)`) whose arguments unify from the statement
being proved. The two derived lemmas the normalizer needs (`ring_swap`,
`ring_pmul_swap`, three law applications each) are emitted as `pub theorem`s in
the artifact, so `mm0-c` checks them too.

**What W2 still owes:**
1. **Approval.** `mm0/approved/flux1d_certificate.stmts` does not exist, so
   `scripts/mm0-comparator check` reports `7 approved, 1 gap` and
   `verify-invariants` carries exactly one explained gap. That is the held-out
   review step, not a bug: the statements are the thing a reader audits.
2. **Hypotheses.** An obligation with hypotheses is refused as a gap (exit 2),
   not silently weakened; flux1d's three are unconditional. The theory already
   supports the hypothesis form (`peq p q` under `rcomp` equalities).
3. **Beyond degree-2/one state.** The prover is general in its fragment
   (`+ − ×`, integer literals) and has no dimension limit, but no other kernel
   exercises it yet.
4. **The reflection step is trusted.** `mm0-c` cannot see kernel → polynomial.
   `check` decides it twice (expansion + exact sampling) and `emit` refuses on
   disagreement, which is the most this shape can do; a *proof* of reflection
   needs the CPS-IR carrier of W1 acceptance 2.
  Changing it is a theory change: edit its line in `BUDGETS` in the same commit.
  Target ≤ 150 lines, defined-only except for *one* transfer rule per class:
  - `tint`, `tadd`, `tmul`, `tneg` with `teq` congruence rules — exact integer
    spine, no floats;
  - `poly_nf` as a *definition* (the normal form function), so a certificate is a
    computation over it rather than a search;
  - **`int_ring_transfer`** — the single trusted bridge from ℤ-polynomial
    coefficient cancellation to the ℝ-valued statement of record. Its
    justification is written down once (the ring homomorphism ℤ → ℝ), reviewed,
    and pinned by hash. Every kernel identity in every future dimension
    instantiates it; this is the exact posture of `prim_eval` and
    `net_isomorphism_step`.
- **New frontend `scripts/au2mm0`** over `scripts/mm0_emit.py` (W0): reads the
  obligation nodes from CPS IR JSON, reflects the Austral expression to
  polynomials, normalizes, emits the chain. Refuses (exit 2, never 0) when an
  operation has no exact integer model — e.g. a division or a float literal.
- **Dual-path cross-check, mandatory before emission** (the `lean2mm0 selftest`
  posture): the reflected polynomial evaluated by an independent evaluator must
  equal the net-reduced value of the same obligation. A disagreement is a hard
  error.
- **`BUDGETS` rows**, pinned per artifact as `symbolic=N transfer=M
  soundness-assumed=K`, with `-- trust-split:` extended and gated by
  `scripts/verify-invariants`.
- **Acceptance:**
  1. `mm0/flux1d_certificate.{mm0,mm1}` in `mm0/`, `mm0-rs compile` exit 0
     (warnings are proof holes), `mm0-c` exit 0.
  2. At least one certificate contains a **free variable** — the artifact header
     states the hypothesis set (`u_L = u_R`), and the proof script's first line is
     a quantified statement, not `teq ttrue ttrue`.
  3. Mutation: flipping a coefficient in the reflected polynomial must be caught
     by the dual-path check (test, not prose).
  4. `scripts/lean2mm0 selftest` gains a sibling `scripts/au2mm0 selftest` with
     the same two-mutation table.
- **Explicit non-goal inside W2:** floats. Any identity over IEEE-754 values is
  emitted as a `-- float-gap:` annotation pointing at the exact statement it is a
  test *of* (W6). Do not certify floating-point equality.

### W3 — Comparator: approved statements, held out  *(3–5 days)*  ← highest value per day

**Why:** C3, and the one AlphaProof component we lack entirely.

- `mm0/approved/<artifact>.stmts` — the statement skeleton, copied by hand into a
  reviewed commit. Format: the output of the *existing* pairing generator
  (`mm0/pairing/gen_mm0.py` already emits public statements with proofs stripped;
  reuse it, do not write a second stripper).
- `scripts/mm0-comparator` — new script, stdlib-only:
  1. regenerate the artifact into a tempdir;
  2. extract statements (`gen_mm0.py` path);
  3. canonicalize local names by stream order;
  4. **diff against the approved file**; any difference is a refusal;
  5. **dependency-closure hash**: hash (sha256, sorted) every `term`/`def`/`axiom`
     name the statement transitively depends on, and every theory file it imports,
     recording both in the approved file. This is the step that catches a weakened
     definition, which a statement diff alone cannot.
- Gate: `scripts/verify-invariants` runs `mm0-comparator` for every artifact in
  `mm0/` when `mm0/approved/` contains its entry, and reports a `[gap]` for
  artifacts not yet approved — never a silent pass.
- **Negative controls (all three required, modelled on `mm0/pairing`'s ten):**
  1. change a numeral in the *source* obligation ⇒ statement diff must fire;
  2. weaken a definition in `docs/cic.mm0` ⇒ closure hash must fire;
  3. tamper with one line of the approved copy ⇒ comparator must refuse (proves
     it reads the approved file, not its own output).
- Approval is a commit that touches only `mm0/approved/`, so the diff shows a
  human decided. Same posture as `BUDGETS` widening.
- **Landed (2026-11-12):** `scripts/mm0-comparator` implements `approve`,
  `check` and `selftest`. It reuses `mm0/pairing/gen_mm0.py` (no second
  stripper), runs the owning emitter's `check` first so the compared `.mm1` is
  known fresh, canonicalizes local names by stream order, and pins
  `closure-sha256`, `theory-sha256` and `held-out-sha256` in the approved file.
  `selftest` refuses all three controls (changed numeral, weakened
  `docs/cic.mm0`, tampered approval). `scripts/verify-invariants` runs it per
  artifact and reports a `[gap]` for each unapproved one — 7 at S1, by design.
  `mm0/pairing/gen_mm0.py` gained `import` passthrough (the set.mm layer's
  stream contains one, and the generator had no `import` keyword);
  `verify_pairing.py` still reports 15/15.  All seven artifacts were approved
  and checked end to end during S1, then the approvals were removed: approval
  is an owner commit that touches only `mm0/approved/`, not something this
  plan writes for itself.

  **Owner approvals (asked 2026-11-15, landed 2026-11-20), one row each.** Each
  row was a separate `[gap]` line in `verify-invariants`, so a partially-approved
  set showed up as a shrinking count rather than a single unhelpful "W3 open":
  the four rows below were the four `[gap] W3: <artifact> has no approved
  statements` lines in §3.5 until the owner committed all four. A fifth row that
  some transcripts quote — `mm0/liquid_split.mm0`, emitted by
  `scripts/liquid-split-mm0` — is **not** in this table: neither the artifact nor
  the emitter exists in any commit, so the table would be asserting files the
  tree does not have, and the checks that read the table (its `stmts` column, its
  emitter cell, its staging) would be reading a claim that cannot be measured.
  It belongs here in the commit that lands both. The `state`
  cell is the record of the decision in both directions — setting it back to
  `` `[gap]`, awaiting owner commit `` withdraws an approval, and the gate reads
  the rest off the two states (see the state-column paragraph below). What an
  approval lands is the statement of record, never a proof: `isa`'s 26 theorems
  and the toy's 28 carry their proofs in their `.mm1`, `store_projection` has
  none to carry, and `mm0/clif.mm0` cites the approval while still claiming
  `0 theorems`.

  | artifact | what the approval holds out | stmts | emitter that owns it | state | statements-sha256 |
  |---|---|---|---|---|---|
  | `mm0/isa.mm0` | the converted ISA model's declarations plus the 23 encdec instances and 3 step refinements | 193 | `scripts/isa-encdec check` — **registered** in `EMITTERS` | `[pass]`, approved in `mm0/approved/` | `2619b5ec9337db280718eb78055f8df76f7a700ae80a05b7fc6bd241db01359a` |
  | `mm0/slot_layout.mm0` | the layout proof's definitions and statements | 23 | none — hand-written spec | `[pass]`, approved in `mm0/approved/` | `98dd6399e157800065a56b11a2d99f29724113ed42a5bc600e222efd220013c4` |
  | `mm0/store_projection.mm0` | the load+store projection claim, three pinned cites, a certificate naming the approval | 11 | none — states its own vocabulary | `[pass]`, approved in `mm0/approved/`; the claim itself stays unproved (§W9.3) | `5c201a800d1190c5074ecc07cbb5bd3c1f74dbf05148097f064ece4ec73af5a2` |
  | `mm0/store_projection_toy.mm0` | the toy model's 28 theorem *statements* and its 20 cited axioms (proofs live in the `.mm1`, checked by `mm0-c`) | 70 | none — compile+check is the gate | `[pass]`, approved in `mm0/approved/` | `abed1393972165e893db24bc34325017ae4f7b94be3284f8f4e33415369a220e` |

  The `stmts` column is **checked**, not remembered. `verify-invariants` runs
  `mm0-comparator approve --dry-run` for every artifact — pending *and* approved —
  prints a `w3-pending:` and a `w3-approved:` line …

  ```
  $ nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants \
      | grep -E '^w3-(pending|approved):|the plan.s §W3'
  w3-approved: flux1d_certificate(100 stmt) gap_certificate(107 stmt) isa(193 stmt) layout_certificate(69 stmt) linearity_certificate(67 stmt) linearity_certificate_spans(71 stmt) quasi_rh_7_8(3 stmt) quasi_rh_certificate(99 stmt) service_isolation_certificate(44 stmt) slot_layout(23 stmt) store_projection(11 stmt) store_projection_toy(70 stmt)
  [pass] W3: the plan's §W3 stmts column matches what each artifact holds (4 row(s), 4 approved)
  [pass] W3: every pending §W3 row's artifact re-derives into a staging (0 row(s))
  [pass] W3: an approved §W3 row passes when the approved copy holds what it says
  [pass] W3: a §W3 row that disagrees with mm0/approved/ is refused
  [pass] W3: the plan's §W3 digest column matches the file it describes (4 row(s), 4 approved)
  [detail] one altered statement: check refused it (unmodified staging: exit 0)
  [pass] W3: one altered statement makes the comparator refuse the approved copy
  [pass] W3: a quoted statements-sha256 that does not hash the statements is refused
  [pass] W9.3: the composition table's counts match the files it records (4 row(s), 4 from mm0/approved/)
  [pass] W9.3: the composition table's closure digests match the files it records (4 row(s), 4 from mm0/approved/)
  [detail] one axiom dropped from the approval: store_projection_toy(stmts cell 70, approved 69) store_projection_toy(says 20 axioms, approved 19) store_projection_toy(declarations 69, approved 68)
  [pass] W9.3: an approved copy that drops an axiom fails the composition counts
  [pass] W3: every §W3 emitter cell names a real command (4 row(s))
  [pass] W3: every approved §W3 row's emitter still produces its approved copy (4 row(s))
  [pass] W3: an approved §W3 row whose emitter left the registry is refused
  [pass] W3: stage-approvals stages exactly §W3's 0 pending row(s) (exit 0)
  [pass] W3: stage-approvals stops staging an artifact once its approved copy exists
  [pass] W3: stage-approvals refuses a named artifact that is already approved
  [pass] W3: stage-approvals refuses an artifact that equals its approved copy
  [pass] W3: the recorded probe evidence still names live gate lines (6 row(s))
  [pass] W3: every §W3 state cell says whether the row is approved or still waiting on the owner (5 row(s))
  [pass] W3: stage-approvals answers an unknown artifact name with exit 64
  [pass] W3: stage-approvals' artifact list is the §W3 table's rows (5 name(s))
  [pass] W3: stage-approvals stages exactly §W3's 5 pending row(s) (exit 0)
  [pass] W3: stage-approvals stops staging an artifact once its approved copy exists
  [pass] W3: stage-approvals refuses a named artifact that is already approved
  [pass] W3: stage-approvals refuses an artifact that equals its approved copy
  ```

  The `statements-sha256` column is checked the same way, because a quoted digest
  is either compared with the staging that produces it or it is a copied hex
  string nobody can verify. The gate stages each row into its own scratch
  directory — `mm0/approved/` is never written, since approval is an owner
  commit — reads the digest back from the header the comparator wrote, and
  compares. That also exercises the staging path on every run, which is the one
  thing a script that must not write the real directory cannot otherwise
  demonstrate.

  … and then parses this table and diffs the pending line against it. The
  approved line is a report, not an enforcement: an approved artifact is held by
  its statement diff, but the *size* printed next to it is what makes a shrunk
  artifact visible in the gate output — measured 2026-11-15 by deleting one axiom
  from `mm0/flux1d_certificate.mm1`, which moved its entry to `(99 stmt)` and
  turned `[FAIL] W3: flux1d_certificate matches its approved statements`, then
  restoring the file. A `?` in either line means "could not measure here" (the
  comparator refused: no `python3`, or an emitter reporting a stale `.mm1`), never
  zero — a zero would read as "this artifact no longer says anything", which is a
  finding of its own and must not be spelled the same way as an unavailable tool. Disagreement fails the gate in
  both directions, naming the row: a cell that no longer matches the file
  (`isa(table says 192, file says 193)`) and half an approval — a pending row
  whose approved copy is already committed (`isa(says unapproved, but …/isa.stmts
  exists — flip this row's state cell)`) or an approved row with no copy to check
  it against (`isa(says approved, but …/isa.stmts does not exist)`). The second
  direction is what an approval used to leave behind: the row stayed pending, so
  its count was compared with a staging and the gate said nothing about the copy
  the owner had just committed.

  A **pending row's artifact must also re-derive now**, before anybody tries to
  approve it. The `stmts` check above can only compare a number it managed to
  measure, so an artifact the comparator cannot stage — a stale `.mm1` its emitter
  refuses, a generator that broke, a registry entry that went away — used to come
  out as "no pending artifact of that name is measurable here", which reads like a
  missing tool rather than a finding about the artifact, and the owner met the
  refusal only at approval time. Every §W3 pending row must now have produced its
  `w3-pending:` number (`[pass] W3: every pending §W3 row's artifact re-derives
  into a staging (4 row(s))`), and the failure names the rows plus the comparator's
  own first line. Measured 2026-11-17 by appending a blank line to `mm0/isa.mm1`
  (which makes it stale against the encoder) and restoring it afterwards: `[FAIL]
  W3: every pending §W3 row's artifact re-derives into a staging` with
  `§W3 pending re-derivation: isa(emitter: scripts/isa-encdec check) did not
  stage; what the comparator said: isa([gap] scripts/isa-encdec check failed for
  isa: isa-encdec: FAIL …isa.mm1 is s…)`, and the `stmts` row message switching to
  `isa(row gives 193 statements, but its artifact does not re-derive here — see the
  re-derivation check)`. The failure names the row's own emitter because that is
  what an owner follows next, and it is the table's `emitter that owns it` cell
  that says which command refused the `.mm1` — not something to work out from the
  artifact's name.

  The **emitter column is checked the same way**, because a column saying which
  command owns an artifact is a claim like `stmts` is. `mm0-comparator emitters`
  prints the registry the comparator actually uses (`<stem>\t<script>\t<mode>`,
  the single source of truth for `EMITTERS`), and `verify-invariants` parses this
  table's fifth column and requires, per row: a cell that names a command
  (`` `scripts/isa-encdec check` ``) must name a file that exists and is
  executable, with the same script *and* mode the registry holds — so a renamed
  emitter, or a mode change, is red; a cell that says `none` must be *genuinely*
  unowned, so registering an emitter for `slot_layout` makes the table wrong
  rather than merely incomplete; and a cell in neither form fails, which is what
  stops the column drifting back into prose. Measured 2026-11-15 with four
  drift controls on copies of this table (a nonexistent script, `isa` changed to
  `none`, a real-but-unregistered `scripts/toy-witness` on `store_projection`, and
  a free-form cell): each one turned `[FAIL] W3: every §W3 emitter cell names a
  real command` with the row named, and the committed table passes (4 rows).
  For a *pending* row that check is static: it proves the command *is there* and
  *is the one registered*, not that it runs, because running it is what
  `mm0-comparator approve`/`check` does when an approval exists
  (`emitter_check`). An *approved* row is the case where the static form is not
  enough: its `stmts` and digest columns read a committed file, so they keep
  passing after the emitter is retired or its output changes, and nothing would
  re-derive that file. So the gate approves each approved row again into a
  scratch root — one more `approve`, which runs the row's emitter, or the
  artifact itself for a `none` row — and requires the result to be byte-identical
  to the committed copy: `[pass] W3: every approved §W3 row's emitter still
  produces its approved copy (1 row(s))` with `isa` approved, and with a copy
  holding one statement fewer
  `isa(the emitter produces today's statements, …/isa.stmts holds the approved
  ones)`. Byte-identical rather than count-equal, because the claim is that this
  command *owns this file*, and a re-derived approval that merely matches in size
  is a different file with the same shape.

  That check has its own negative control, so it is not a claim that has never
  fired: the same code is handed a registry with `isa` retired — what unregistering
  its emitter produces — and the row that names `` `scripts/isa-encdec check` ``,
  and must report `isa(the row says 'scripts/isa-encdec check', the registry says
  'nothing' for it)`; with the real registry over an identical copy it must report
  nothing, which is the half that proves it is not always complaining. It is
  `[pass] W3: an approved §W3 row whose emitter left the registry is refused`, and
  it needs no approval to exist: the copy it reproduces is a fresh staging, so the
  control runs on today's table. The registry is passed in rather than read from
  the path, so the retirement is simulated without editing the repository.

  That control checks what the function *returns*; whether the gate's output
  changes is a different claim, so it is measured end to end too, the way
  `scripts/obj-drift-control` measures W5. `scripts/w3-emitter-drift-control`
  copies this table, rewrites `isa`'s emitter cell to name
  `` `scripts/toy-witness check` `` — an executable command the registry does not
  hold for `isa`, which is what a retirement leaving the cell behind looks like
  from the table's side — and runs `scripts/verify-invariants` against the copy
  through `AUSTRALVM_PLAN`. Measured 2026-11-17: exit 1, `verify-invariants: 138
  passed, 6 explained gap(s), 2 failed`, `[FAIL] W3: every §W3 emitter cell names
  a real command` with `isa(cell says 'scripts/toy-witness check', EMITTERS says
  'scripts/isa-encdec check')`, and every §W3 line that does not read that row
  still passing. Two failures, not one, because the in-gate control re-reads the
  same cell — the gate saying the same thing twice, which is why the script's list
  of unchanged lines omits it rather than pretending otherwise.

  Every measured perturbation is recorded as data, not only as prose:
  [`docs/data/w3/probes.tsv`](docs/data/w3/probes.tsv) carries one row per
  perturbation (artifact, what was done, the check that fired, the message it
  produced, the date), and the gate re-reads the file on every run — each
  `gate_line` must still be text `scripts/verify-invariants` emits, each artifact
  must still be a §W3 row or a committed `mm0/` artifact (or the Liquid golden) —
  the twelve `copy`-row controls name the file they moved — a row without a
  measurement date is refused, and each
  `expected_message` is checked as a *skeleton*: it carries `…` wherever the gate
  interpolates a stem, a command or a count (or assembles the sentence from
  pieces), and every literal chunk between the ellipses, four characters or
  longer, must still appear verbatim in the script. That last part is what keeps
  a recorded message from describing output the gate can no longer produce; the
  ellipses are not shorthand for "roughly this". Two further columns record what a
  gate run ended with where the probe *was* a run — `gate_exit` and `gate_total`,
  `-` where the probe is exercised in-gate instead — and both are re-read for
  shape: a numeric exit must be non-zero (a perturbation that exits 0 caught
  nothing) and the total must read `<n> passed, <n> explained gap(s), <n> failed`,
  while a `-` in one column with a number in the other is refused because nobody
  could reproduce it. That is `[pass] W3: the recorded probe evidence still names
  live gate lines (8 row(s))`. A total is the gate as it stood on the row's date,
  not today's: the gate grows, so an older row keeps the smaller total it
  measured (`stale-mm1`'s `128 passed` predates the §W9.3 controls) while its
  exit status and its message remain the claims the check re-reads. The perturbations are deliberately not re-run (appending to
  `mm0/isa.mm1` or editing `EMITTERS` is a measurement, not a routine check), but
  a citation that no longer exists is caught. Measured 2026-11-17 by renaming one
  recorded `gate_line`: `[FAIL] W3: the recorded probe evidence still names live
  gate lines` with `§W3 probe evidence: stale-mm1(gate line 'W3: every pending row
  re-derives' is not in the gate)`. The exit/total half was measured the same day by
  setting the drift row's recorded exit to 0, which the run refused with
  `drift-whole-gate(records exit 0, so the perturbation caught nothing)`. The
  message half was measured by accident, and
  kept: the first recorded form of the stale-`.mm1` message spanned a `)` the gate
  builds from two pieces, so the run refused it with `stale-mm1(expected message
  chunk ') did not stage; what the comparator said: ' is not in the gate)` and
  `139 passed, 6 explained gap(s), 1 failed` — which is the check working, and the
  reason the recorded row now marks that boundary with `…` rather than quoting a
  sentence no single line of the script contains.

  **The thirteen former `copy` rows.**
  `scripts/w3-drift-controls` is those thirteen controls under one entry point
  (`<case>`, `--list`, or `all`), plus `cert-approved` for the digest the
  certificate's approval citation carries — fourteen cases. Each case builds a
  scratch copy, drifts one claim in it, runs the whole gate with the read-only
  override that points at the copy, and requires exit 1 with the expected
  `[FAIL]` line — and the `[pass]` lines that must not move. Since 2026-11-19
  that second half is universal
  rather than "where a second check reads the same copy": nine cases
  (`phase-d-theory-coverage`, `phase-d-imported-theory`,
  `phase-d-self-vocabulary`, `phase-d-measured-coverage`, `isa-hypothesis-pin`,
  `witness-axiom`, `memsafe-edge`, `cert-approved`, `w3-approved-row`) asserted
  only their failure, so a gate that had broken wholesale would have satisfied
  them. They now also require `Phase D: gap_certificate pins its trust split (9
  structural / 0 grounded)` and `W3: every §W3 state cell …`, neither of which
  reads the drifted file or the scratch approval root. Two overrides
  were added for this, both read-only relocations rather than grants:
  `AUSTRALVM_MM0_DIR` (the model directory the Phase D, W8 pin, witness and
  memsafe checks read) and `AUSTRALVM_LIQUID_GOLDEN` (the extracted OCaml the
  subset check diffs). Measured 2026-11-18: all twelve hold — eleven at `140
  passed, 7 explained gap(s), 1 failed`, `isa-hypothesis-pin` at `139 passed, 7
  explained gap(s), 1 failed`, and `witness-axiom` at `137 passed, 8 explained
  gap(s), 3 failed`, the extra two being the count checks that read the same
  appended axiom. Each case's whole gate log is kept under
  `$W3_DRIFT_LOGS` (default `/tmp/w3-drift-controls`), one directory per
  invocation and one per case inside it — `runs/<id>/<case>/01-g.log`, with the
  case's printed report as `report.txt` beside it — and that log is where the
  recorded totals come from. A run no longer overwrites its case's previous log:
  the file per case name was how a red row lost the transcript that would have
  explained it. The count is bounded — a run removes all but the newest
  `W3_DRIFT_KEEP` (default 20) run directories, `0` disabling pruning — and the
  run's summary lists, per case, the gate logs in its own directory, so the
  transcript of a red case can be found from the summary alone. `make drift-controls` runs
  `scripts/w3-drift-controls all` and then the `*-drift-control` scripts, and
  `make test` depends on it — the Makefile states the cost, because every control
  is a full gate run (~4.5 min). A case cannot be made cheaper (the case *is* the gate
  run), so `-j N` runs independent ones at once — and a run writes its own wall
  clock down rather than leaving it in a transcript: the runner appends one row
  per run to `$W3_DRIFT_LOGS/timings.tsv`, and to `$W3_DRIFT_RECORD` when that
  is set. The rows kept in the tree are `docs/data/w3/drift-timings.tsv`:

  ```
  date	rev	jobs	requested	wall_s	per_case_s	held	gap	outcome	cases	died
  2026-11-19	de8b11b2	1	14	3763	268	14/14	0	held	all	?
  2026-11-19	de8b11b2	2	14	2649	189	14/14	0	held	all	?
  ```

  `held` is `held/requested` and does not count a case the shell could not
  exercise, so a run that reported `13 case(s) held, 1 not covered` records
  `13/14` with `gap` = 1 instead of `14/14`; `cases` names what was asked for,
  which is what ties a row to the run directory that holds that case's logs.
  `died` names the cases that did not hold and left no gate transcript at all —
  the subshell ended before `gate_run` wrote its first line, which is also what
  a case killed by the host looks like (empty report, empty status, no log) —
  so a `failed` row carries names instead of only a count. `-` there means every
  case that ran reached the gate (a control that ran and lost its assertion is
  not a death: its transcript is in its run directory), and `?` means the row
  was written before the column existed. Rows written before `gap` and `cases`
  cannot be attributed to a case, and are not the ledger's evidence for anything.

  That is all 14 cases in **62m43s** at `-j 1` (268 s each) against **44m9s** at
  `-j 2` (189 s of wall clock per case, ~379 s of work, because two gate runs
  share the core) — 18m34s, 30%, off. The second core is worth 1.42x rather than
  2x because a single gate run is itself mostly serial, and every case still
  holds under `-j 2` (the same 14 `holds` as at `-j 1`), which is the check that
  the cases really are independent: own scratch tree, own `TMPDIR`, repository
  read-only. Those two rows predate the recorder and were entered from the runs'
  own gate logs; every row after them is written by the run itself. Reproduce
  with `W3_DRIFT_RECORD=docs/data/w3/drift-timings.tsv bash
  scripts/w3-drift-controls -j 2 all`, under `nix develop`. The five read-only
  `*-drift-control` scripts are separate scripts and write no row of their own;
  measured the same way, they take 22m44s serial against 17m22s
  at `-P 2`, all five holding — so the two parallel halves together go from
  **1h25m27s to 1h01m31s**, 23m56s or 28% of the wall clock. `make
  drift-controls` therefore runs at `DRIFT_JOBS` (default `nproc`) and keeps
  `scripts/obj-drift-control` — the one control that edits the tree and
  rebuilds the compiler — last and serial; that one is unchanged and excluded
  from the totals above.

  The thirteenth case is the row that used to be the exception. `W3: $_stem
  matches its approved statements` only runs for an artifact that has an
  approval, and the §W3 table's own rows are all still pending — so the case
  brings its own approved root instead of waiting for one: `w3-approved-row`
  copies `mm0/approved/` into scratch, changes one approved axiom's text
  (`quasi_rh_7_8`'s single axiom, `cc0` → `cc1`) and leaves the file's digest
  alone, which is precisely a statement weakened after the fact. Measured
  2026-11-18: exit 1, `140 passed, 7 explained gap(s), 1 failed`, the single red
  line being `[FAIL] W3: quasi_rh_7_8 matches its approved statements` with the
  other 7 gaps and every other check unmoved. Nothing is approved by it —
  `mm0/approved/` is read and copied, never written, and the scratch copy is
  deleted with the run — so the owner's commit remains the only way an approval
  enters the tree.

  **Which checks have no measured failure.** Every check in the gate was counted
  against the evidence, and the counter is `scripts/check-census` — the numbers
  below are its output on this tree, not a hand count. Of 225 `check` statements,
  137 distinct texts remain once a trailing count parenthetical (`($n name(s))`,
  `($_w3_rows row(s), …)`, `(9 structural / 0 grounded)`) is dropped: it reports a
  number, not a different claim. 22 of those 137 have a measured perturbed-tree
  run (`probes.tsv`, one whole-gate run each) and every one of the rest carries a
  row in the table below, so `scripts/check-census` reports 0 distinct texts with
  neither: no check in the gate is unread now, and there are no `copy` rows —
  the last one, `W3: $_stem matches its approved statements`, is measured by
  `w3-approved-row` from a scratch approved root rather than by an approval in
  the tree. Every `copy` row (thirteen perturbed-tree runs, one per row,
  `make drift-controls`) is now a case in `scripts/w3-drift-controls`, which is
  why the `copy` column is empty and not thirteen rows.
  `class` is one of five. **control** — the check already perturbs a scratch copy
  (or stages into a scratch root) on every run and requires the refusal, so its
  red path is observed without a second experiment. **derived** — the check
  recomputes a claim from the tree (a digest, a byte identity, a count, a name
  list, a regenerated table) and compares it with a recorded cell or pin, so a
  drifted claim on either side turns it red by construction. **copy** — a control
  of the `probes.tsv` shape is possible (copy the data, drift one claim, point an
  override at the copy) and is not written yet; this is the one class that is a
  real gap, and it now holds no rows: each of the thirteen former members is a
  case of `scripts/w3-drift-controls`, one `probes.tsv` row apiece. **tool** — the check only runs when an external tool is present
  (`mm0-c`, Lean, `why3`/`z3`, `cc`), so its observed failure mode so far is a
  missing tool, not a drifted claim. **none** — the check asserts a fact about the
  implementation (a file exists, a source line says something, a scan finds no
  banned word, a name is computed per artifact), so there is no second side to
  compare and testing it means editing the repository, which no control here does.
  A `control`, `derived` or `none` row is not a failure of evidence: the first two
  are evidence already, and the third's subject is the tree itself. A check that
  already perturbs a scratch copy every run is therefore listed as `control`
  rather than `none` — that is what reading the body decides, and it is why the
  table cannot be produced by a keyword scan.

  | check | class | what stands in the way |
  |---|---|---|
  | `$_cert_claim` | none | the name is computed per certificate claim; the check repeats per claim |
  | `$name` (Phase D loop) | none | the name is computed per generated document |
  | `H12: notes implemented dir has the four categories` | none | a directory layout; nothing to drift |
  | `H8: austral_cps default archetype declared` | none | greps a source file |
  | `H8: ecmascript archetype runtime branch present` | none | greps a source file |
  | `H8: manifest parser reads the archetype key` | none | greps a source file |
  | `H8: tidepool (Haskell effects) backend module exists` | none | a file's existence |
  | `H8: cap-std Rust backend module exists` | none | a file's existence |
  | `L10 Cycle B: liquid_measure tenant registered in Vm_plugin.boot` | none | greps a source file |
  | `L10 Cycle B: liquid_measure is a distinct tenant from liquid` | none | compares two source strings |
  | `L10 Cycle B: LiquidMeasure is the extracted module, listed in dune` | none | greps a build file |
  | `L11a: LiquidIndexClosure is a module of austral_liquid` | none | greps a build file |
  | `L11a: the liquid pass runs the index-closure audit before emitting` | none | line numbers out of one source file, compared to each other — recomputed from the tree, nothing recorded to drift |
  | `L11a: the index-closure audit has its own test suite` | none | greps `test/dune` for a suite name |
  | `L11b: LiquidIndex is a module of austral_liquid` | none | greps a build file |
  | `L11b: the sorted index language has its own test suite` | none | greps `test/dune` for a suite name |
  | `L11b: the sorted index test does not drive the compiler` | none | asserts an absence in one dune stanza |
  | `L11c: LiquidSplitPilot is a module of austral_liquid` | none | greps a build file |
  | `L11c: the split pilot runs before the .mlw is emitted` | none | line numbers out of one source file, compared to each other |
  | `L11c: the split pilot has its own test suite` | none | greps `test/dune` for a suite name |
  | `L11c: the split pilot has an end-to-end suite` | none | a directory's existence |
  | `L11c: the split pilot's facts are checked to erase` | none | greps a test file for a case name |
  | `L11a: the ordering pin has a negative control` | none | the control script's existence; the gate cannot run it, because it runs the gate |
  | `L11a: the ordering control moves the copy through the gate's own hook` | none | greps the control for the hook it names, and the gate for that hook |
  | `L11b: the conversion pin has a negative control` | none | the control script's existence; the gate cannot run it, because it needs `dune` |
  | `L11b: the conversion control mutates the rule LiquidIndex declares` | none | compares one source line in `LiquidIndex.ml` with the control's view of it |
  | `L11b: the conversion control re-checks the tree after restoring` | none | greps the control for its digest and restore steps |
  | `PC02: flipping clamp_upper's conclusion is refused` | control | a scratch `.mm1` with the conclusion flipped, refused on every run |
  | `PC02: word.mm0 + word.mm1 accepted, clamp bounds and map laws proved` | tool | runs `mm0-rs` + `mm0-c`; a missing toolchain is the failure mode observed so far |
  | `W5: the staleness note is silent when the artifact is newest, and names the newer binary` | control | a scratch artifact/binary pair is stamped both ways on every run and the note must be empty in one and name the file in the other |
  | `Liquid golden $name parses under Why3` | tool | needs `why3`; the `.mlw` copy is the driftable data |
  | `Liquid golden $name type-checks under Why3` | tool | needs `why3` |
  | `Liquid golden vc-fold-premises.mlw proves in full under z3` | tool | needs `why3` + `z3` |
  | `Liquid subset extraction matches its pinned golden` | control | `scripts/w3-drift-controls liquid-golden` appends a line to an `AUSTRALVM_LIQUID_GOLDEN` copy and the diff refuses it; the one case conditional on the shell — the gate guards that check on `command -v why3`, so a prover-less shell reports `[gap] … not covered in this shell` rather than a false hold |
  | `Liquid subset library extracts with the existing driver` | tool | needs the extraction driver |
  | `Phase D: lean2mm0 emitter present` | none | a script's existence |
  | `Phase D: generated certificate artifacts present` | none | files' existence |
  | `Phase D: gap_certificate pins its trust split` | control | `phase-d-trust-split-gap` rewrites the split to `structural=8` in an `AUSTRALVM_MM0_DIR` copy |
  | `Phase D: layout_certificate pins its trust split` | control | `phase-d-trust-split-layout` widens the same line to `structural=4` |
  | `Phase D: $name declares its own vocabulary (no imported theory)` | control | its negation is the same drift as the coverage row below — `phase-d-self-vocabulary` deletes the marker and the artifact falls through to the refusal |
  | `Phase D: $name only claims rules its theory declares` | control | `phase-d-imported-theory` adds a `used=` name the theory never declares |
  | `Phase D: $name states its measured coverage` | control | `phase-d-measured-coverage` makes `-- clif-forms:` disagree with the forms the file declares |
  | `Phase D: $name states its theory coverage` | control | `phase-d-theory-coverage` deletes the coverage line, `phase-d-self-vocabulary` deletes the self-declared marker |
  | `S21: ZENODO_SYMBOLS (uz_*) registered separately` | none | greps the symbol registry |
  | `S28: AuthorizationEngine trait present` | none | greps a source file |
  | `S29: UNFER_SYMBOLS table present` | none | greps a source file |
  | `S29: UNFER_SYMBOLS registers into the Cranelift JIT` | none | greps a source file |
  | `S29: per-module CPS compile consumes the same census` | tool | needs a build |
  | `S29: symbol_sync test cross-checks EXPECTED_SYMBOLS.txt` | none | runs the repository's own test |
  | `S30: cadabra2-cli subprocess, not a dependency` | none | an architectural claim about source |
  | `U11: the CPS lowering's adapter descriptors match the golden` | derived | re-emits the descriptors from the fixture and diffs them with `test-programs/adapters/adapters.golden` |
  | `W3: $_stem matches its approved statements` | control | `w3-approved-row` copies the whole approved root, weakens one approved axiom's text and leaves its digest alone, then runs the gate through `AUSTRALVM_APPROVED_ROOT`; `mm0/approved/` is only read, so no control approves |
  | `W3: stage-approvals stages exactly §W3's pending rows` | control | the staging runs against a scratch root every run; the row is its refusal path |
  | `W6: generated C probe compiles and passes at tolerance 1e-8` | tool | needs `cc` |
  | `W8: hypotheses are cited, pinned, and the unchecked count matches its pin` | control | `isa-hypothesis-pin` rewrites `-- unchecked-citations:` to 1 in an `AUSTRALVM_MM0_DIR` copy |
  | `W8: mm0/slot_layout.mm0 + .mm1 accepted, in_bounds_alloc proved` | tool | needs `mm0-rs` + `mm0-c` |
  | `W9: mm0/clif.mm0 + .mm1 accepted by mm0-rs + mm0-c` | tool | needs the mm0 toolchain |
  | `W9.3: clif_step is vocabulary — declared, stated, never axiomatized` | control | `clif-vocabulary` prepends an `axiom` line to an `AUSTRALVM_CLIF` copy |
  | `W9.3: the refinement's citations still match their artifacts` | control | `clif-citations` flips one `-- cites:` digest in an `AUSTRALVM_CLIF` copy |
  | `W9.3: a Lean development proves the same axioms` | tool | needs Lean |
  | `W9.3: the Lean development proves the toy's theorems too` | tool | needs Lean |
  | `W9.3: the witness exercises every axiom the toy declares` | control | `witness-axiom` appends an axiom to a toy copy that the witness does not hold; the two count checks read the same file and fire with it, which its log shows |
  | `logos mm0: CNL → DeltaNet → MM0 corpus verifies (and its anchor fires)` | tool | needs the `unfer/logos` pipeline; the check owns its own tampered-certificate anchor |
  | `memsafe: theory, emitter, both fixtures, ledgers and certificates present` | none | files' existence |
  | `memsafe: certificate + ledger emitter present and executable` | none | a file's mode |
  | `memsafe: fixtures exercise every automaton edge` | control | `memsafe-edge` drops `t_acq` from both `-- transitions exercised:` lines in an `AUSTRALVM_MM0_DIR` copy |
  | `memsafe: $stem pins its trust split` | none | the name is computed per artifact |
  | `service-isolation: pins, theory and both certificate halves present` | none | files' existence |
  | `service-isolation: emitter present and executable` | none | a file's mode |
  | `service-isolation: trust split is structural-only` | none | parses a header's claim |
  | `$name` | none | the check helper's own argument; every name it prints is generated |
  | `S28: DenyAll is the default (fail-closed)` | none | greps `auth.rs` for the struct; no second side to compare |
  | `S28: check() refuses without an installed engine` | none | greps `auth.rs`; no second side to compare |
  | `W1: Flux1d compiles through the CPS JIT and its four obligations hold at the example state` | tool | needs the built `austral` binary and `libaustral_cranelift_bridge.so`; runs the example end to end |
  | `W2/W5/W6: flux1d certificate and probe re-emit byte-identically and mm0 re-accepts it` | derived | re-emits the certificate, the `.mm1` and the probe, and compares the bytes with the committed three |
  | `W2: docs/kernel_alg.mm0 accepted by mm0-rs + mm0-c` | tool | needs `mm0-rs` + `mm0-c` |
  | `W3: a quoted statements-sha256 that does not hash the statements is refused` | control | drifts the digest in a scratch staging and requires the refusal |
  | `W3: a §W3 row that disagrees with mm0/approved/ is refused` | control | runs the row function against a scratch approved root, missing copy then stale cell |
  | `W3: an approved §W3 row passes when the approved copy holds what it says` | control | the positive branch of the same scratch-approval probe |
  | `W3: an approved §W3 row whose emitter left the registry is refused` | control | drops `isa` from a scratch registry and requires the refusal |
  | `W3: every §W3 state cell says whether the row is approved or still waiting on the owner` | none | validates the plan's cells against the two allowed shapes; the red path is a plan edit |
  | `W3: one altered statement makes the comparator refuse the approved copy` | control | alters one statement in a scratch staging |
  | `W3: stage-approvals answers an unknown artifact name with exit 64` | control | runs `stage-approvals` with an unknown name against a scratch root |
  | `W3: stage-approvals refuses a named artifact that is already approved` | control | names an artifact that a scratch approved root already holds |
  | `W3: stage-approvals refuses an artifact that equals its approved copy` | control | stages a copy byte-identical to the one in a scratch approved root |
  | `W3: stage-approvals stages exactly §W3's $_sa_pending_n pending row(s)` | control | stages every pending row into a scratch root and counts the headings |
  | `W3: stage-approvals stops staging an artifact once its approved copy exists` | control | stages into a scratch root that already holds one approved copy; that heading must be absent |
  | `W3: stage-approvals' artifact list is the §W3 table's rows` | derived | compares the script's artifact list with the §W3 rows it is generated from |
  | `W3: the plan's §W3 digest column matches the file it describes` | derived | recomputes each quoted digest from the file or approved copy the row names |
  | `W3: the recorded probe evidence still names live gate lines` | derived | re-reads `probes.tsv` and checks each row against the gate it cites |
  | `W5 (checked): the certificate's cps digest is the bridge's digest of the emitted bytes` | derived | recomputes the CPS digest through the bridge and compares the recorded one; also mutates a CPS copy |
  | `W5 (checked): the certificate's obj digest is the bridge's digest of the emitted object` | derived | recomputes the object digest through the bridge |
  | `W5 (checked): the bridge the gate loads is the release artifact it was built from` | derived | compares the repo-root copy against the one `make bridge` writes; a stale root copy measuring the kernel with an old bridge is the silent pass `scripts/obj-drift-control` found once |
  | `W6: no certificate claims a float path is proved` | none | scans the committed artifacts for the claim; testing it means editing one |
  | `W7: a certificate with a flipped coefficient is refused` | control | flips one coefficient in a scratch certificate and requires the refusal |
  | `W8: a table entry moved off its byte breaks its proof` | control | moves an `enc_add` entry to another byte in a scratch `.mm1` |
  | `W8: an unchecked citation cannot be re-labelled as checked` | control | rewrites the unchecked-citation count in a scratch model |
  | `W8: an uncited or stale hypothesis is refused` | control | zeroes one cited digest in a scratch model |
  | `W8: encdec table and its proofs are current` | derived | regenerates the table from the encoder and compares it with the committed one |
  | `W8: isa-census refuses a model opcode the decoder rejects` | control | appends a bogus opcode to a scratch model |
  | `W8: mm0/isa.mm0 + isa.mm1 accepted by mm0-rs + mm0-c, no sorry` | tool | needs `mm0-rs` + `mm0-c` |
  | `W8: scripts/isa-census covers the emitted opcodes` | tool | needs the built `isa_census` binary and an emitted CPS file |
  | `W8: the allocation-monotonicity proof depends on its direction` | control | drops the symmetry argument in a scratch `.mm1` and requires the failure |
  | `W8: the corpus census covers every opcode the examples emit` | tool | needs the built `isa-census --corpus` path |
  | `W9.1: the pinned RV64 model converts and mm0-c accepts it` | tool | needs `RISCV_MODEL`, `lean` and `lean4export` |
  | `W9.3: a Lean development proves the same $(printf '%s\n' ` | tool | needs Lean; the axiom list is then compared with the `.mm0` |
  | `W9.3: an approved copy that drops an axiom fails the composition counts` | control | drops one axiom from a scratch approval and requires the count to move |
  | `W9.3: both witnesses reject the same candidates` | tool | needs `rustc` and Lean; compares the two rejection lists |
  | `W9.3: every Lean declaration rests on Lean's standard foundation only` | tool | needs Lean and its `#print axioms` output |
  | `W9.3: every field word a composition cell names has a literal comparison` | derived | reads the field words the composition cells name and finds each comparison in the gate |
  | `W9.3: load/store are refuted as the model's execute_LOAD/execute_STORE` | tool | needs `nix` and `scripts/mem-refines` |
  | `W9.3: the Lean development proves the toy's $(printf '%s\n' ` | tool | needs Lean; the theorem list is then compared with the `.mm0` |
  | `W9.3: the Lean file's code carries no sorry/admit/native_decide/ofReduceBool` | none | scans the Lean source for banned words; the scan's own red path is the two controls below |
  | `W9.3: the Lean model's proofs depend on its definitions (mutating one refuses them)` | control | mutates five definitions in scratch copies and requires each proof to fail |
  | `W9.3: the memory projection is stated as vocabulary, accepted by mm0-c` | tool | needs `mm0-rs` + `mm0-c` |
  | `W9.3: the no-sorry scan does not count banned words inside comments` | control | puts a banned word inside a comment in a scratch file and requires no hit |
  | `W9.3: the no-sorry scan refuses an injected sorry that Lean itself accepts` | control | injects a `sorry` Lean accepts into a scratch copy and requires the scan to refuse it |
  | `W9.3: the projection's citations still match their artifacts` | derived | recomputes each pinned citation digest |
  | `W9.3: the toy model proves the stated projection, not a neighbour` | derived | compares the toy's pin with the projection's digest |
  | `W9.3: the toy projection's theorems check without sorry` | tool | needs `mm0-rs` + `mm0-c` |
  | `W9.3: the toy's axioms hold in a concrete model (no counterexample)` | tool | needs `rustc` for `scripts/toy-witness` |
  | `W9.3: the toy's citation digest refuses a changed claim` | control | drifts the claim in a scratch copy and requires the pin to read stale |
  | `W9.3: the toy's proofs depend on its axioms (mutating one refuses them)` | control | mutates thirteen axioms in scratch copies and requires each proof to fail |
  | `W9.3: the toy's shape figures match what mm0-rs counts` | derived | compares the plan's and AGENTS' shape figures with what mm0-rs counts |
  | `W9.3: the toy's theorem and axiom counts agree with the plan and AGENTS` | derived | compares each document's theorem and axiom counts with the file's |
  | `W9.3: the witness exercises all $_ax_n axioms the toy declares` | derived | compares the file's axiom names with the witness's `[hold]` lines |
  | `W9.3: the witness's verdicts do not change when the domain grows` | derived | runs the witness at 4 and 5 atoms and compares the verdicts |
  | `W9: a form the lowering does not emit is refused` | control | appends a form the lowering does not emit to a scratch model and requires the refusal |
  | `W9: mm0/clif.mm0 is what the corpus lowering measures` | derived | regenerates the model from `clif_dump` and compares it with the committed file |
  | `memsafe: $ledger_stem in sync with its fixture and the checker` | derived | recomputes each ledger from its fixture and compares it with the checker's |

  The **state column is checked too**, and for the same reason: "`[gap]`" is a
  status, not an explanation. It tells a reader the gate is red without saying
  whether anyone is working on the artifact, whether something blocks it, or
  whether it is an owner decision waiting to be taken — and the third is what it
  always is here, because an approval is an owner commit into `mm0/approved/`
  and nothing in this repository may self-approve. So each row's state cell has
  exactly two shapes, and it is also the *record* of the decision:

  | cell | means |
  |---|---|
  | `` `[gap]`, awaiting owner commit `` | still pending; the row's numbers are checked against today's artifact |
  | `` `[pass]`, approved in `mm0/approved/` `` | approved; the row's numbers are checked against `mm0/approved/<stem>.stmts` |

  Both may be followed by `; ` and an artifact-specific reason
  (`store_projection` uses that for "the claim itself stays `[gap]`"). A cell
  that says `pending`, `TBD`, a bare `[gap]`, an empty reason after `;`, or a
  *different* reason such as `blocked on X` fails with the row named — the last
  one deliberately, since a row that is blocked for another reason is still
  waiting on the owner and the cell would then understate what the gate is
  showing. A cell that says approved without naming `mm0/approved/` fails too.
  Measured 2026-11-15 with four drift controls on copies of this table (bare
  `[gap]`, `pending`, `… ;` with nothing after it, and `blocked on the model
  rebuild`): each turned `[FAIL] W3: every §W3 state cell says whether the row is
  approved or still waiting on the owner` with the row and the offending text,
  and the committed table passes (4 rows). That measures the cell's *shape*; that
  the state it names reaches the files is a second claim, and it is measured end
  to end by `scripts/w3-approval-state-drift-control`: it copies this table,
  rewrites `isa`'s cell to `` `[pass]`, approved in `mm0/approved/` `` with no
  `mm0/approved/isa.stmts` behind it, and runs the gate against the copy through
  `AUSTRALVM_PLAN`. Measured 2026-11-18: exit 1, `verify-invariants: 137 passed,
  7 explained gap(s), 4 failed`, `[FAIL] W3: the plan's §W3 stmts column matches
  what each artifact holds` with `[detail] §W3 table vs measured: isa(says
  approved, but …/isa.stmts does not exist)`, and exactly the three other lines
  that read the same cell — the digest column, the approved-row emitter check and
  the staging count — following it, while the shape check and the emitter column
  still pass. Recorded as the `approval-claim` row of
  `docs/data/w3/probes.tsv`.

  **An approval is one edit, and the gate derives the rest.** Because the state
  cell says which file the row's numbers must match, approving an artifact is
  committing `mm0/approved/<stem>.stmts` and flipping that cell — not deleting
  the row. Everything downstream is then read off the two states: the `stmts`
  and digest columns are compared with the approved copy for approved rows and
  with a fresh staging for pending ones (the pass lines report the split:
  `(4 row(s), 4 approved)` today), the `w3-pending:` line shortens and
  `w3-approved:` grows without anyone editing a second list, and
  `scripts/stage-approvals` stops staging the artifact because its approved
  copy now exists. Half an approval is refused rather than half-made: a pending
  row whose approved copy *does* exist fails with `… exists — flip this row's
  state cell`, and an approved row with no copy, or one whose copy holds a
  different number of statements, fails naming the copy. Measured 2026-11-17 on
  copies: `isa`'s cell flipped to approved against a scratch approved root →
  `(4 row(s), 1 approved)`, `w3-pending:` down to three, the digest check still
  green, `3 staged, … 1 already approved`; an approved cell with the copy
  removed → `isa(says approved, but …/isa.stmts does not exist)`; an approved
  copy holding one statement fewer → `isa(table says 193 statements, the
  approved copy holds 192)`.
  Measured 2026-11-15 by perturbing both at once: `1 failed`, with exactly those
  two messages. The entries are in the table's row order; the fix for a stale
  cell is to read the `w3-pending:` line, not to re-derive the count by hand. The
  same counts via the owner's route (`approve --approved-root /tmp/appr2 …` then
  `check --approved-root /tmp/appr2 …`) gave `4 approved, 0 gap(s)`. The flow needs no hand-written files from the
  owner: approve into a scratch root, `check` against it, read the statements
  (`approve --dry-run <artifact>` prints exactly those), then commit
  `mm0/approved/<artifact>.stmts` — a commit touching only `mm0/approved/`.
  `isa`'s approval is anchored to a current artifact because its emitter is
  registered; the other three are hand-written specifications with no emitter,
  which the comparator prints (`no emitter registered — compared as committed`)
  instead of skipping silently. Each row leaves the table on its own:
  `approve <artifact>` against the real root, `git add
  mm0/approved/<artifact>.stmts`, and its `[gap]` becomes `[pass] W3:
  <artifact> matches its approved statements` — so approving `isa` alone moves
  the §3.5 gap count from 4 to 3 and says exactly which one moved. An approval that pins a theory file's hash
  becomes a `[gap]`, not a refusal, on a machine where that file is not
  provisioned.

### W4 — Strength budget: `assumed=N`  *(1–2 days)*

- `scripts/lean-typing-check` gains `--assumed-json` (count + names, from the
  leaves it currently reports as `assumed`).
- `scripts/lean2mm0` / `scripts/au2mm0` write `-- assumed: N (names…)` into each
  artifact header; `BUDGETS` pins N per artifact.
- Vacuity check, cheap version: assert that each obligation's hypothesis set is
  jointly satisfiable by exhibiting one concrete state that meets it, in the
  artifact as a `-- witness:` line. (AlphaProof proves satisfiability; exhibiting
  a witness on a 1D state vector is enough at this scale and costs nothing.)
- **Acceptance:** raising `assumed` without editing `BUDGETS` fails
  `verify-invariants`; a mutation adds one assumed leaf and is refused.

**Landed 2026-11-13.** `flux1d.obligations.json` gained `uniform_state`
(`l = r ⇒ delta l r = 0`), the first obligation with a non-empty `hypotheses`
array. `au2mm0` eliminates such a hypothesis by *substitution on the locus*:
solve `a = b` for a variable that occurs in no other monomial with integer
coefficients (else `[gap]`), substitute into both sides, and count the surviving
hypothesis set as `assumed`. A hypothesis the prover cannot eliminate is counted
and pinned, never assumed silently; `BUDGETS["flux1d"]` is now
`{structural: 6, grounded: 15, certificates: 4, assumed: 1}`, and
`verify-invariants` refuses a rise without the pin. Each obligation also emits
`-- witness: l=1 r=1`, found by grid search over
`HYPOTHESIS_GRID = (1, 0, -1, 2, -2, 3)`; a grid miss is reported as a `[gap]`
("may be vacuous"), not as a claim that no witness exists. The fourth selftest
anchor covers this path.

**Open:** `lean2mm0` still emits no `-- assumed:` line, because its inputs carry
no assumed leaves to count (`mm0_emit.assumed_line` is the helper it would call,
and `lean-typing-check --assumed-json` is the source it would read). Measured
example: `lean-typing-check --assumed-json mm0/proto/_simp_1_1_typing.mm1` ⇒
`count=1 names=['ax_body']`. Wiring it into `lean2mm0` is a one-line gate but
needs a Lean module whose export actually contains an `assumed` leaf.

### W5 — Narrow the TCB: code fingerprint in the certificate  *(1 week)*

**Why:** AlphaProof's strongest claim is that the proof is about the bytes that
run. We cannot reach that without a machine-code semantics, but we can make the
gap *legible and checkable*.

- Every certificate artifact carries `-- code: cps=<sha256> obj=<sha256>
  target=<triple>` — the CPS IR hash and the cranelift-emitted object hash for the
  kernel the certificate is about.
- Decision point (§9): whether the fingerprint is *checked* (requires a
  CPS-IR-level entry point in the bridge, which is a new `uk_*` symbol and
  therefore a full S29 checklist) or merely *recorded* (no new symbol; the OCaml
  side already has the CPS IR bytes at `--emit-cps` time). **Default to
  recorded**, then promote to checked only if a regression test can be written
  that fails when the object changes without the certificate changing.
- **Acceptance:** rebuild the kernel after a semantics-preserving edit that alters
  emitted code ⇒ `code.obj` differs, certificate verdict unchanged, and the
  status line says `code-drift`. A semantics-changing edit ⇒ `netcert` says
  *not equivalent* ⇒ certificate refused.

**Landed 2026-11-13 (recorded route).** `au2mm0 emit … --code <cps> --target …`
writes

```
-- code: cps=cd65d0b4…a3c3 obj=unrecorded target=host
-- code-status: recorded (cps = sha256 of the --emit-cps bytes); obj unrecorded — no object file is written by this path
```

`verify-invariants` re-derives the CPS with
`austral compile … --use-cps-jit --emit-cps=<tmp>` and re-emits; the digest is
byte-stable across runs (same sha256 twice, 7,773 bytes, 5 functions), so a
certificate whose kernel moved fails the gate. `target=host` rather than
`uname -m` keeps the committed artifact host-neutral.

**Promoted to checked, 2026-11-13.** The decision point above is settled the
cheap way: the fingerprint is now computed *by the runtime*, without adding a
`uk_*` symbol, so no manifest grant and no S29 checklist are involved.

- `safestos/cranelift/src/fingerprint.rs` exports
  `austral_cps_fingerprint(bytes, len) -> *mut c_char`: sha256 (`sha2`, already
  in the lock at 0.10.9 as a transitive dep) over exactly the bytes the JIT is
  handed. It is an ordinary FFI export, like `austral_defeq_translate` —
  deliberately *not* callable from compiled Austral code. Three unit tests,
  including the published digest of the empty input rather than a value
  compared to itself:

  ```
  $ cargo test --release --lib fingerprint
  test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 49 filtered out
  $ nm -D target/release/libaustral_cranelift_bridge.so | grep austral_cps_fingerprint
  000000000012301c T austral_cps_fingerprint
  ```

- `scripts/cps-fingerprint <file>` asks that export for a file's digest (ctypes), so
  the gate's number comes from the crate that compiles and runs the bytes, not
  from the emitter hashing what it just wrote. `--selftest` additionally holds
  the Rust digest against `hashlib` on three inputs and requires a one-byte
  change to be visible.
- `verify-invariants` holds the certificate's committed `cps=` to the bridge's
  digest of the freshly `--emit-cps`'d bytes, and requires a mutated copy of
  those same bytes to fingerprint *differently* — otherwise the comparison would
  be true of nothing. Measured:

  ```
  [pass] W5 (checked): the certificate's cps digest is the bridge's digest of the emitted bytes
  verify-invariants: 84 passed, 0 explained gap(s), 0 failed
  ```

  and the control, with the committed digest edited to start `0…`:

  ```
  [FAIL] W5 (checked): the certificate's cps digest is the bridge's digest of the emitted bytes
  exit=1
  ```

  The artifact was restored with `git checkout -- mm0/flux1d_certificate.mm0`
  afterwards; the run above is the only one that saw the tampered header.

**Object hash recorded and checked, 2026-11-13.** The bridge gained
`austral_object_fingerprint(bytes, len)` and `austral_object_arch()` (same
plain-FFI route, no `uk_*`): the module is compiled through the JIT's own
`compile_cps_to_clif` into a `CaptureModule`, and the digest is sha256 over
`CaptureModule::encode_avce()` — the same bytes `avm-exec` consumes. `au2mm0
takes `--obj <digest> --obj-arch <arch>`, so the certificate now says

```
-- code: cps=cd65d0b4…a3c3 obj=ceed355c…d2cb target=host
-- code-status: recorded (cps = sha256 of the --emit-cps bytes, obj = sha256 of the
   AVCE capture of those bytes, arch=aarch64)
```

and `verify-invariants` re-derives both: the CPS digest from the fresh
`--emit-cps` bytes, the object digest from a fresh compile of those bytes. A
semantics-preserving edit to the emitter moves `obj=` and fails the gate; the
demonstration that the two digests are independent is the negative control below.

Two limits are written into the code and the gate instead of being smoothed over:

- **Architecture.** The capture is host-native (`cranelift_native`), so the
  digest is only comparable on the same `arch`. The gate compares it when the
  recorded `arch=` matches `uname -m` and reports a `[gap]` otherwise — never a
  pass, never a failure. On this host: `arch=aarch64`.
- **Process scope.** cranelift-module allocates `FuncId`s from a process-global
  counter, so the *second* module compiled in one process encodes different
  declaration indices and fingerprints differently. The digest is defined for
  the first compilation of a fresh process, which is what `scripts/cps-fingerprint`
  and the gate both do. This is why the unit test asserts well-formedness and
  non-collision, while stability across runs is held by the gate (which compares
  a fresh process's digest to the pinned one on every run) — a claim is tested
  where it is true.

**A failed comparison says which build it is about (measured 2026-11-19).** The
digests describe one `austral` and one bridge build, so a tree that has been
rebuilt since the certificate was recorded fails with the same two red lines as
a drifted one. The gate now attaches the mtimes when they explain it: the
perturbation below, with the artifact's mtime held at its recorded
`13:57:34` and `austral` at `14:08:25`, exits 1 with `152 passed, 8 explained
gap(s), 5 failed` — the re-emit claim, the `obj` digest, and the three citation
checks that hash the artifact — and both W5 lines carry

```
      staleness: — austral is newer (2026-11-19T14:08:25) than
      mm0/flux1d_certificate.mm0 (2026-11-19T13:57:34), so the recorded digests
      may describe an older compiler build; re-emit it: python3 scripts/au2mm0
      emit examples/verified-kernel/flux1d/flux1d.obligations.json --code <cps>
```

The same perturbation with the artifact's mtime left at *now* fails the same
five checks and prints no staleness line, which is the intended split: the note
is a diagnosis of *why*, never a skip of the comparison. It cannot be a refusal
on mtime alone, because a rebuild of identical sources leaves the digests equal
and the tree green — measured on this host with `austral` 34 minutes newer than
the certificate, and CI writes the artifact at checkout before `make bridge`
writes the binary, so an mtime-only refusal would be red on every run. The note
is itself a comparison with a control: `[pass] W5: the staleness note is silent
when the artifact is newest, and names the newer binary` stamps a scratch pair
both ways on every run. Restored: `sha256 924ac3f0…f94` for the certificate,
gate green again.

**The drift control, measured 2026-11-13.** A digest that never moves would be
worthless, so the W5 claim is tested by making the emitter move without changing
behaviour: `scripts/obj-drift-control` reverses the function order in the CPS
module (semantic — the JIT indexes functions by name), rebuilds, re-emits, runs
the program, and then puts everything back. Measured end to end:

```
before: cps=cd65d0b4… obj=ceed355c…d2cb  program exit=0
patched: cps=18928686dd5a… obj=5b75e8bfdd58…cb5f  program exit=0   (same output)
gate:   [FAIL] W5 (checked): the certificate's object digest is the bridge's
        digest of the compiled object (aarch64)
restored: cps=cd65d0b4… obj=ceed355c…d2cb  (pinned values return)
```

So the object digest is sensitive to emitted code, the certificate is not
silently accepted after such a change, and the control leaves a green tree (it
restores and rebuilds on every exit path, including failure). Since 2026-11-19
the run also requires two lines that read neither the certificate nor the
emitter — `[pass] W3: every §W3 emitter cell names a real command` and
`[pass] Phase D: gap_certificate pins its trust split (9 structural / 0
grounded)` — because the expected W5 `[FAIL]` would also be printed by a gate
that had broken wholesale; it was the one drift control with no green
assertion at all.

Re-measured 2026-11-19 on the *clean* layout this tree now builds — no `.so` at
the repo root, only `safestos/cranelift/target/release/` — which is why the
control resolves the bridge the way the gate does instead of reporting a
`[gap]`:

```
before: cps=5dfcdfe8c6dd… obj=26bb3ab35216…50cc out=0
after:  cps=a3652e1cdf82… obj=524f7432048b…cead out=0      (same exit status)
gate:   [FAIL] W5 (checked): the certificate's obj digest is the bridge's
        digest of the emitted object (x86_64)
held:   pinned cps/obj returned after the restore; lib/CpsGen.ml byte-identical
``` It is not in
`verify-invariants` because it rebuilds the compiler twice; it is a script a
reader can run, with the result above pinned in this document.

**Also measured (and it changes the acceptance text above):** the `code-drift`
experiment cannot be demonstrated with `netcert`, because `netcert` does not
normalize arithmetic — it reports *not equivalent* for `2*(3*x + x*x)` vs
`6*x + 2*x*x` (W1, item 3). The semantics-preserving-edit half is demonstrated
today by the byte-identity comparison of the re-emitted certificate instead,
which is what the gate actually does.

### W6 — The float gap, stated instead of hidden  *(2–3 days)*

- The C backend (`--target-type=c`) and the JIT path both emit, from the *same*
  reflected expression W2 certified, a validity probe for the identity —
  generated, never hand-written. This is Lanyon's `..._valid(...)` idea, with the
  drift risk removed by construction.
- The certificate header records `-- float-gap: idealization=integer exact,
  probe=generated tolerance 1e-8, status=tested-not-proved`. No artifact may claim
  otherwise; the wording is gated by `scripts/check-todo-ledger`-style string
  check in `verify-invariants` (a claim that says "proved" next to a float path
  fails the gate).

**Landed 2026-11-13.** `au2mm0` renders `mm0/flux1d_probe.c` from the *same*
post-substitution AST as the `peq` chain (one `c_expr` renderer, both consumers),
with the `float-gap:` line in the certificate header and
`* Status: tested-not-proved. … this says what was tested.` in the probe's own
header. 1,000 states. Measured:

```
$ cc -O2 -o /tmp/probe mm0/flux1d_probe.c -lm && /tmp/probe
flux1d probe: 4 identit(ies), 1000 states, max relative deviation 0.000e+00
```

`verify-invariants` compiles and runs the probe (a missing `cc` is a `[gap]`,
never a pass), compares the emitted source byte-for-byte with the committed one,
and runs the claim-string check: every `mm0/*_certificate.mm0` is scanned for a
line containing both "float" and "proved" unless it also says
not-proved/unproved/tested-not-proved.

### W7 — Question answering over the kernel corpus  *(1–2 weeks, last)*

**Why:** C6 is ours alone, and `scripts/logos2mm0` proves it works — but only over
a Peano fragment. Extending it needs W2's content-level statements, because
§12's corpus statements encode *types* (`teq <type-code> ttrue`), which answer
"what is the signature" and nothing else. This is the honest limit of the current
CNL feature and should be written into its README before anything else here.

- Extend the CNL claim corpus (`unfer/logos/mm0/claims.txt`) with kernel-content
  sentences over `mm0/flux1d_certificate` statements: "the numerical flux agrees
  with the physical flux when the states agree" ⇒ provable; "the scheme is
  second-order accurate" ⇒ must return `unproved:` (the fragment's boundary,
  recorded rather than hidden, exactly as `upstream.txt` does).
- Keep invariant 5 (tampered certificate rejected) and extend it to the new
  artifact: a flipped coefficient in the certificate must be refused.
- **Do not** build a natural-language agent loop on top until the corpus can state
  a kernel property as content. An agent that emits into a type-level encoding can
  only be answered vacuously.

**Landed 2026-11-13 (the corpus half; no agent loop).** `unfer/logos` has a third
fragment (`unfer/logos/src/mm0/kernel.rs`), selected by a `theory: kernel` line in
a claims file. It answers by **citation**: a sentence is
lowered to the same polynomials the certificate used (expanding the kernel's
definitions from the obligations sidecar), and it is `proved` when some theorem of
`mm0/flux1d_certificate.mm0` states exactly the same identity up to renaming the
variables — hypothesis included, which is why `delta l r = 0 when l = r` cites
`uniform_state` while `delta l r = 0` does not.

The citation is **pinned**, and that is the part that makes it worth something: the
certificate's `-- Source:` line carries the sha256 of the sidecar it was emitted
from, and a sidecar that does not hash to it is refused rather than believed. A
sidecar that grew an obligation the certificate never proved is refused too (the
theorem has to exist by name). So a citation cannot outlive the artifact it cites.

`unfer/logos/mm0/claims.txt` gained the five lines this section asked for
(`theory: kernel`):

```
proved: for all u: numerical flux of u and u agrees with flux of u
proved: for all l, r: delta of l and r plus delta of r and l equals zero
proved: for all a, b, c: delta of a and b plus delta of b and c plus delta of c and a equals zero
proved: delta of l and r equals zero when l equals r
unproved: the scheme is second order accurate
```

Measured (`scripts/logos2mm0`, which now passes `--kernel-certificate` and counts
the shapes):

```
[pass] claim corpus certifies (52 lines, every expectation held)
[pass] kernel corpus cites flux1d_certificate (4 obligation(s), 1 honest unproved)
[pass] kernel corpus refuses a certificate whose pinned sidecar digest does not match
```

What the fragment deliberately cannot do: refute. "Not an identity" is not
"false", and turning one into the other is a semantic claim about the numeric
domain this layer has no proof of, so such sentences come back `unproved` with the
reason — including a true identity the certificate happens not to state. The
`unproved: the scheme is second order accurate` line is the same boundary stated
as content: order of accuracy is a property of the scheme, not an identity in
ℤ[x⃗], so it must never start "proving".

The tamper anchor is now two-sided: `verify-invariants` flips a coefficient in the
`consistency` theorem's statement and leaves the proofs alone, and mm0 has to
refuse the result (a citation of a statement that is no longer proved would be
evidence about nothing); `logos2mm0` tampers the pinned digest instead.

**Still open:** question answering *over the corpus* (the W7 title) — the
fragment decides sentences, and there is no query layer that finds the sentences
for a question. That is additive on top of a corpus that can now state kernel
content, and the plan's warning stands: it must not be built on the type-level
encoding.

### W8 — An ISA model that only fills the gaps  *(model + census landed 2026-11-13; proofs conditional)*

**Why:** AlphaProof Code's strongest structural idea is a *definition-only*
semantics of the instruction set (~8k trusted lines) with the proofs about it
untrusted. We have the opposite half: real proofs about linear types, liquid
refinement types and memory safety, but they are about CPS IR, not about the
bytes. Building an ISA model from scratch here would prove the three guarantees a
second time at a lower level, in a weaker fragment, and the two proofs would be
free to disagree. So the model is designed to **consume** them.

**The layering (this is the whole design).** Three semantics layers, and each
theorem crosses exactly one boundary:

| layer | artifact | already exists? |
|---|---|---|
| source = Austral judgments (linearity, layout/memory safety, refinements) | `mm0/linearity_certificate`, `mm0/layout_certificate`, `mm0/gap_certificate`, each with its ledger's sha256 in the header | **yes** (re-emitted, gated) |
| IR = the CPS machine built by W2's theory | `mm0/*_certificate.mm0` theory lines + `kernel_alg.mm0` | **yes** |
| ISA = decode/step relation over registers and memory | `mm0/isa.mm0` | **yes — landed 2026-11-13** |

The ISA model never restates a source-level guarantee. Two of its three
statements are now **proved**, and only the memory one is a citation:

- `encdec` — for the emitted subset, `decode (encode i) = i`. **Proved**, 23
theorems in `mm0/isa.mm1`, from the 23-row table that `ISA_REGISTRY`,
`lib/Cps_codec.ml` and `test/CpsCodecTest.ml` hold to each other.
- `step_refines` — one ISA step (state → state′) implements one CPS-IR step.
  **Proved** (2026-11-14) as `add_refines`/`mul_refines`/`sub_refines`: both
  sides are defined to compute the same `radd`/`rmul`/`radd∘rneg` into the same
  destination register, so `req_trans` + `req_symm` close it. The premises
  (`linear`, `in_bounds`) stay in the statement but are *not used by the proof* —
  which is the measurement that the old axiom was double-counting, not a gap.
- `bounds_from_layout` — the per-access memory obligations the step relation
  needs are discharged by citing the layout discipline's block bounds, not by
  re-proving arithmetic. This is the one remaining named hypothesis with a
  citation, and `verify-invariants` refuses an uncited or stale one.

Consequences that must be stated and not softened, in the order they now hold:
the byte layer is no longer assumed at all, and the step relation is no longer
assumed; what is still *conditional* is the memory obligation, which inherits any
bug in the layout certificate. That is the honest trade — the alternative is two
independent proofs of block bounds — and the plan's §7 discipline applies: the
citation must be pinned (as in W7) so a drifted certificate cannot be cited.

**Subset discipline (measured, not assumed).** The model covers exactly the
instructions the emitter produces for the certified fragment — the same mechanism
as `SYMBOL_REGISTRY`'s census, with an `ISA_REGISTRY` beside it:

```
austral compile examples/verified-kernel/flux1d/flux1d.aum --use-cps-jit \
  --emit-cps=/tmp/flux1d.cps        # the IR the model must cover
./scripts/isa-census /tmp/flux1d.cps   # decoder vs artifact vs measured
```

An uncovered opcode is a gate failure, not a footnote — the census is what stops
the model drifting into "the ISA" when the claim is "the bytes we emit".

**[gap] → answered 2026-11-13.** Neither repo ships a machine-code semantics, so the
model was authored for the emitted subset rather than imported (the Lanyon
choice): 23 opcode rules, no translator in the TCB, every rule a definition or a
cited hypothesis. Sail remains the route if the claim ever needs to leave this
emitter; the two costs below are unchanged.

- **Sail** (the maintained RISC-V/AArch64 models): a new emitter, `sail2mm0`,
  sibling to `lean2mm0`, and a translation whose fidelity is itself an unproved
  claim. Cheapest path to a *trusted-by-provenance* semantics.
- **Authored from the ISA manual** for a small subset (RV64I integer core), the
  Lanyon-style choice: fewer rules, each reviewable, no translator in the TCB.

Both are weeks of work and neither is on the W0–W7 critical path.

**Landed 2026-11-13 — the model.** `mm0/isa.mm0` is authored (not translated),
`-- layer: ISA`, coverage line `used=… unused=…`, and three `-- cites:` lines
pinning `linearity_certificate`, `layout_certificate` and `kernel_alg` by sha256.
5 sorts, 23 opcode rows (`-- op 0xNN name`), and — since the encdec and
step-refinement work below — **26 proved theorems**, one cited hypothesis
(`bounds_from_layout`). The `run_*` / `ir_*` pairs remain `axiom`s on purpose:
they are the definition of the step relation, the AlphaProof definition-only
posture, and a reader audits them by eye rather than a citation.

**Landed 2026-11-13 — encdec, proved instead of assumed.** The first version of
this section was one axiom over uninterpreted `enc`/`dec`, annotated
`status=measured-not-proved`. That was nearly empty: an uninterpreted `enc` says
nothing about any byte, so the axiom was also true of a model that encoded every
instruction as 0. MM0 has no pattern matching or recursion, so a codec cannot be
definitionally *implemented* in the theory — but it can be *stated* finitely, one
equation per emitted opcode, and then the round trip is a theorem of those
equations:

```
term ins_add: Ins;
axiom enc_add: $ beq (ienc ins_add) n5 $;      -- the instruction is byte 0x05
axiom dec_add: $ ieq (idec n5) ins_add $;      -- and byte 0x05 is it
theorem encdec_add: $ ieq (idec (ienc ins_add)) ins_add $ = '(ieq_trans (beq_cong_dec enc_add) dec_add);
```

`scripts/isa-encdec` generates that block for all 23 entries of `ISA_REGISTRY`
into `mm0/isa.mm0`, and derives `mm0/isa.mm1` (the MM0 proof file: same
statements, every theorem `pub theorem ... = '(proof)`) from it, so the two cannot
drift. What is trusted is now the *table* — 23 lines saying which byte
introduces which instruction — and that table is checked three ways rather than
believed once: `scripts/isa-census` against the JIT's own decoder,
`test/CpsCodecTest.ml` against the OCaml encoder that `lib/CpsGen.ml` now uses
(`lib/Cps_codec.ml`), and `scripts/verify-invariants` against mm0-c.

Measured (the split matters: mm0-rs compiles `isa.mm1`, mm0-c re-checks those
proofs against the `isa.mm0` specification — feeding mm0-c the proof file is
`invalid command keyword`, and compiling the specification alone would accept
missing proofs):

```
$ mm0-rs compile mm0/isa.mm1 /tmp/isa.mmb && mm0-c /tmp/isa.mmb < mm0/isa.mm0
  5 sorts, 68 term/def, 91 ax/thm — exit 0, no `sorry`
$ scripts/isa-encdec
isa-encdec: 23/23 encdec instances proved from the table; mm0/isa.mm0 and mm0/isa.mm1 are current
$ cargo test --release --lib CpsCodec  # (OCaml side: dune exec test/CpsCodecTest.exe)
Ran: 5 tests … OK   # round trip, 256-byte exactness, injectivity, names, table = artifact
```

Both controls are live in the gate: moving one table entry off its byte
(`enc_add` 0x05 → 0x06) makes `mm0-rs` refuse the proof that depended on it, and
staling a citation makes `isa-hypotheses` fail.

**Landed — the census, over a corpus.** The first census measured flux1d alone
and reported "8 of 23 opcodes used", which is a fact about one kernel rather than
about the emitter. `scripts/isa-census --corpus` now compiles every example whose
CPS path produces a function (`flux1d`, `fib`, `union`, `record`, `generic-union`,
`named-argument`, `identity`) and censuses the union, reporting each module's
contribution:

```
isa-census: 9 modules — 768 opcode read(s), 11 distinct, 23 declared, 23 opcode(s) declared in mm0/isa.mm0
  per module: flux1d.cps: 9 distinct, 557 read(s); fib.cps: 7 distinct, 24 read(s); …;
              kernel.cps: 7 distinct, 18 read(s) — the `uk_version` demo;
              memory-ops.cps: 9 distinct, 43 read(s) — deref/store
  used: 0x01 int_lit, 0x02 var, 0x03 let, 0x04 app, 0x07 return, 0x08 if, 0x10 cmp_lt,
        0x14 cmp_eq, 0x16 and, 0x20 deref, 0x30 store
  declared but not used here (covered by the model, 14): 0x05 add, 0x06 sub, 0x09 while, 0x0a match, …
  result: covered — every opcode used is declared in the decoder and in the artifact
```

The union is 9, not 23: `while`, `match`, `mul`, `store`, `deref` and the other
comparisons are declared and proved but never emitted by this corpus, and the
output says so instead of implying the subset is fully exercised. `hello-world`
and `memory` are excluded because the JIT cannot compile them today (`__embed`),
which the script reports as a `[gap]` if it ever stops being true rather than
silently dropping them.

**How it is measured.** `safestos/cranelift/src/isa.rs` holds `ISA_REGISTRY`
(the same shape as `SYMBOL_REGISTRY`: opcode, name, operands, class), plus
`note`/`clear`/`snapshot`. The measurement is the *decoder's own* count: the
seven opcode-read sites in `cps.rs` call `crate::isa::note(code)`, so the number
comes from the parser that runs the bytes, not from a second decoder written to
agree. `src/bin/isa_census.rs` diffs decoder vs artifact vs measured for one or
many modules and exits 1 on any uncovered or overstated opcode, 2 when it cannot
run.

The census controls are live in the gate: `-- op 0x7f bogus` added to a copy of
the model makes the census fail (the decoder does not accept `0x7f`), and
stripping or staling a citation makes `isa-hypotheses` fail. What remains open is one hypothesis with a real vocabulary.
`mm0/slot_layout.mm0` defines `in_bounds (s) (i)` as membership of `i` in the
slot list the state carries, and proves `in_bounds_alloc`: allocating a slot
never loses one that was already in bounds. `isa.mm0` states the same definition
(and `scripts/isa-hypotheses` refuses the two copies if they differ), so
`bounds_from_layout` is now `content=checked` instead of `unchecked`: the cited
artifact defines the predicate and reasons about it. What it cannot prove is the
premise itself — that the compiler's layout pass lists every slot its code later
dereferences. That pass is OCaml; mm0 sees its output as `slots s`, never its
code. The difference from where this section started is that the claim now has a
vocabulary to be false in, and its invariant is checked rather than asserted.

---

## §4b W9 — the IR model and the ISA we did not write (owner-directed 2026-11-14)

**The direction, in the owner's words compressed to one sentence:** the
machine-code claim should be *assembled from the proofs we already have* — liquid
types, linear types, memory safety — with a Cranelift-IR model in MM0 bridging
them to an ISA model, and the ISA model should be one that already exists rather
than one this repo authored.

What that changes about W8, stated plainly: W8's `mm0/isa.mm0` is an in-repo
model of *the emitted subset* — honest, but authored here, so "the ISA" in it
means "the instructions our decoder accepts". W9 does not add more instructions
to that file; it replaces its provenance (an external model, converted) and
inserts a layer between the CPS certificates and it (the Cranelift IR). The
reuse the owner asks for is the whole point: the layer-1 artifacts stay the
source of the typing/linearity/bounds facts, and the new layers *cite* them the
way `bounds_from_layout` now cites `slot_layout.mm0` — vocabulary defined,
invariant proved, one premise about the compiler left visible.

### W9.1 ISA model: convert, do not author

**Status 2026-11-14 — converted, accepted, and the diff is closed.** An external
RV64 model is pinned and converted by one command, `scripts/rv64-model-mm0`, in
two stems because the Lean model splits along exactly the line the emitter does:

```bash
nix develop "git+file://$PWD#lean" -c bash scripts/rv64-model-mm0
# [INFO] ALU model    : …/riscv-lean-ccdfd676/share/riscv-lean/RISCV/Instructions.lean
# [INFO] riscv_instructions: export 340046657 bytes, 6355791 records
#   roots: 83 of 55494 exported declarations
# 3 sorts, 30 term/def, 94 ax/thm
# [INFO]   mm0-c accepted (83 declarations)
# [INFO] memory model : …/sail-riscv-lean-11d8fa21/lib/lean (Lean_RV64D)
# [INFO] rv64_memory: export 782800603 bytes, 13689978 records
#   roots: 2 of 167940 exported declarations
# 3 sorts, 30 term/def, 13 ax/thm
# [INFO]   mm0-c accepted (2 declarations)
# 23 opcodes: 15 mapped to a converted model declaration, 8 structural
#             (no ISA instruction), 0 not covered
```

**Two pins, each converted from its own specification.** The ALU half is
**opencompl/riscv-lean** at `ccdfd67647b334a830e220c4caef0f067cd9e15c`
(`flake.nix` `packages.riscv-lean`,
`sha256-SaV/eT7LUbcrKut4R0KbG503t2jVj/8jXCjDFHEagCg=`, exposed as
`$RISCV_MODEL`). The memory half is the Sail-generated model that repo itself
depends on: **opencompl/sail-riscv-lean** at `11d8fa21` (`Lean_RV64D`) plus
**rems-project/lean-sail** at `0794631`, the exact commits in the upstream
`lake-manifest.json` — the lakefile asks for `rev = "main"`, the manifest is the
lock, and those two hashes are that lock. `packages.sail-riscv-lean` builds them
to oleans (28 min, cached in the store) with the git requirement rewritten as a
path requirement and both `lean-toolchain`/`lake-manifest.json` removed, so the
build is offline and the flake's Lean 4.30.0 compiles sources upstream wrote for
4.29.0 (measured). Both are fetched, not vendored, so each conversion's input
has a name and a digest.

The ALU pin ships exactly one file, `RISCV/Instructions.lean`, and that is the
honest part of the choice:

- it is the only file in that repo with **no imports**, so it elaborates
  without Lake, without network, and without the `Lean_RV64D` git dependency
  (a lake dep at `rev = "main"` — unpinnable as a flake input, unfetchable in a
  sandbox) that the repo's default target `RISCV.Basic` requires;
- the repo's `lean-toolchain` names Lean 4.32.2; this file elaborates under the
  flake's 4.30.0 in 2.2 s (measured), so no toolchain bump was needed. The
  version gap is not "probably fine" — the file compiles, and BitVec APIs that
  changed between those releases are in the files it does not import;
- what that costs is stated in the diff, not hidden: that file is **ALU + M +
  Zba/Zbb/Zbs** (630 lines, 83 `def`s, `namespace RV64`) and has no loads, no
  stores and no branches. That is why there is a second stem: the memory
  declarations (`execute_LOAD`, `execute_STORE`) are in the Sail-generated
  `LeanRV64D.InstsEnd`, and converting them meant paying the Sail cost rather
  than writing a substitute.

The conversion reuses the pipeline this repo already trusts
(`scripts/lean2mm0-stmts`, statements of record as MM0 axioms — structure only;
*truth* stays with the Lean kernel and nanoda's re-check of the export, exactly
as `docs/lean4mm0.md` scopes it): olean → `lean4export` → roots via
`scripts/lean2mm0`'s `Export` reader (dotted names: `RV64.add`,
`LeanRV64D.Functions.execute_LOAD`) → `lean2mm0-stmts emit --only` →
`mm0/statements/<stem>.{mm0,mm1,roots}` → `mm0-rs compile` → `mm0-c < .mm1`
**exit 0** for both stems. The two exports are 340 MB / 55,494 decls and 783 MB /
167,940 decls; the second is 13× larger because `InstsEnd` is a 45 MB generated
file, and the export is the cheap part of that pin — the build is the expensive
part.

One measured trap, now encoded in the script: `lean2mm0-stmts verify`'s own
mm0-c leg is a false negative — it runs `mm0-c <mmb>` with stdin `/dev/null`,
which exits 255 for known-good artifacts (`cmd: null`, `cmds:` empty). The
canonical form is `mm0-c <mmb> < <stem>.mm1`, which is what the script runs and
what `verify-invariants` does for every other artifact.

**Coverage, stated as the diff the script prints:** of the 23 `ISA_REGISTRY`
opcodes, **15 map to a converted declaration** (`int_lit→RV64.li`, `add→add`,
`sub→sub`, `mul→mul`, `and→and`, `or→or`, `not→xori`, the four orderings→`slt`,
`cmp_eq`/`cmp_neq`→`xor`, and `deref→LeanRV64D.Functions.execute_LOAD`,
`store→execute_STORE`), 8 are CPS structure with no instruction of its own
(`var`, `let`, `app`, `return`, `if`, `while`, `match`, `string_lit`), and **0
are uncovered**. The mapping is closed by the script (`isa.rs` gaining an opcode
with no entry fails it) and "mapped" is checked against *the artifact the entry
names*, so a declaration that is not there prints `NOT IN ARTIFACT` and fails
rather than counting as covered. The W9.1 acceptance limb — "every opcode the
JIT decodes is in the converted model" — is therefore **met**, with one
qualification that belongs in the same sentence: `structural` means "no RISC-V
instruction of its own", not "verified", and the CPS→instruction shape for
`if`/`while`/`match` is Cranelift's (`mm0/clif.mm0` measures it).

**The gate, and its control.** `verify-invariants` runs `scripts/rv64-model-mm0
check` when the `lean` shell is active (`$RISCV_MODEL` set) and fails on drift;
outside it the check is an explained `[gap]`, never a pass. Drift detection was
mutation-measured: appending one line to a copy of the statements and pointing
`OUT` at that directory gives `[FAIL] drift: … differs from a fresh conversion`,
exit 1. That control also caught a real defect in this script's first version —
the export was written to `mktemp -d`, and `lean2mm0-stmts` records the export's
*path* in the artifact header, so every `check` failed against an unchanged
tree (`/tmp/nix-shell.XOabRm/…` vs `/tmp/nix-shell.UAs9Zy/…`). The export now
lives at the gitignored `mm0/exports/riscv_instructions.ndjson`, the convention
the 2124 committed statement files already follow; the artifact is reproducible
byte-for-byte across runs.

Each mapping entry is a claim about semantics that the conversion does **not**
prove: `mm0-c` accepts the declarations as statements about `BitVec`/the Sail
monad, and nothing here says the JIT's `mul` is the model's `mul` or that the
JIT's `store` is Sail's `execute_STORE`. That is W9.3's obligation, and the
mapping table is its vocabulary — with the caveat that a table entry is
currently an argument from reading, not a theorem.

The toolchain it runs on (provisioned 2026-11-14, `flake.nix` `packages.lean` /
`packages.lean4export` / `packages.riscv-lean` / `devShells.lean`):

```bash
nix develop .#lean -c bash -c 'lean --version; lake --version'
# Lean (version 4.30.0, commit v4.30.0, Release)
# Lake version 5.0.0-src+v4.30.0 (Lean version 4.30.0)
```

`lean4export` is built here rather than fetched (`pkgs` has no package for it)
against *this* input's `lean4`, tag v4.30.0, and emits format 3.1.0 — the same
NDJSON version the pinned fixtures and `scripts/lean2mm0` expect, measured by
exporting a one-definition file (339 MB for a `def x : Nat := 1`, which is why
the model conversion is a size problem as well as a semantic one). Its usage
takes the module name and writes NDJSON to **stdout**:
`LEAN_PATH=<dir> lean4export <Module> > out.ndjson`. A second positional
argument is parsed as another module name, panics in `decodeNameLit`, and leaves
no output file — the first thing to get wrong here.

The two costs W8 predicted are both still visible: the converter
(`lean2mm0-stmts`) is in the TCB and is not itself verified (what `mm0-c`
checks is that the emitted statements are well-formed in `cic.mm0`'s term
language, not that they are the Lean definitions' meaning), and the coverage
question is answered by a hand-written mapping table rather than by a proof of
correspondence. What changed is the third one — the Sail cost was paid rather
than avoided: the memory stem needs a Lake project, two pinned git
dependencies and a 28-minute derivation, and it is worth it because the
alternative was either an uncovered `deref`/`store` or a memory model authored
here, which is the thing W9.4 forbids.### W9.2 Cranelift-IR model in MM0

`mm0/clif.mm0` — sorts for values, types and instruction forms of the *subset
of CLIF the bridge emits*, derived from the bridge's own output rather than from
the Cranelift manual:

- an emitter, `scripts/clif2mm0`, dumps the CLIF for a module (the bridge
already builds it into a `CaptureModule`) and emits one declaration per
instruction form it sees, in the spirit of `mm0_emit.py`'s coverage line;
- the CPS→CLIF lowering stays trusted (it is Rust code, and `mm0-c` cannot see
it), so the model's first job is to make that lowering's *output shape* a
measured fact: the census discipline, one layer down;

**Status 2026-11-14 — the measurement exists.** `safestos/cranelift/src/bin/clif_dump.rs`
wraps the capture backend, intercepts each `Context::func` at define time (before
the backend lowers it) and reports one line per instruction — mnemonic, operand
count, result count, operand/result types — so the output is a shape, not a
pretty-print. `scripts/clif2mm0` measures the same corpus as the opcode census
(both now call `scripts/cps-corpus`, so the two "emitted subsets" cannot drift
apart) and writes one constructor per measured form, with the arity in the
argument list and the type tuple in the name:

```
term iadd__i64_i64__r_i64 (a b: val): op;   -- 58 use(s), 2 operand(s), 1 result(s)
```

Measured after the `sub`/`mul`/`match` producers landed, and again after the
merge: **20 forms, 811 instructions over 12 modules** (`i64`/`i8` only; `iconst` at both widths, `iadd`,
`band`, `bor`, `bxor`, `icmp`, `uextend`, `load`, `store`, `call` at four
arities, `brif`, `jump`, `return`, and the trapping pair `ssub_overflow`/
`smul_overflow` — the exact list is in `mm0/clif.mm0`). The
count is lower than the 23 opcodes on purpose: the CLIF layer is coarser (`call`
covers `app`; `load`/`store` are the memory path) and the mapping between the two
layers is W9.3's job, not a name match. The two additions over the first
measurement (18 forms) are the arithmetic instructions, and they appear because
the CPS layer started emitting them — the corpus grew by a `*` in
`examples/control-flow`, not by new forms in Cranelift. The count then moved
853 → 854 on 2026-11-18 when `not` (opcode 0x19) stopped being a bitwise `bnot`
and became `bxor a, 1`: a `bnot` turns 1 into -2, which is truthy, so `not true`
was returning -2. The form named `bnot` left the model, `bxor` entered it, and
the extra `iconst 1` each of the corpus's `not`s now emits is the instruction
count's +1. It moved again 854 → 869 on 2026-11-19, re-measured over the same
12 modules: the form set is still 20 and only counters moved (one-argument
`call` 35 → 50, two-argument `call` 63 → 48, `store` 37 → 52). Merging the
branch that carried `ab9edf22` (a borrow no longer initialises a cell) took it
to **811** over the same corpus, form set still 20, and `scripts/isa-census`
still reads 905 opcodes over 23 of 23 — so the change is at the CLIF layer, not
in the CPS the two layers share.

**Corpus coverage is now complete and still pinned.** The corpus grew on
2026-11-14 from 9 to 12 modules: `examples/control-flow` exists only to drive the
opcodes the other examples never emitted (`while`, `cmp_gt`, `cmp_lte`,
`cmp_gte`, `cmp_neq`, `or`, `not`, the `for`-loop `add`, and — since the last
three producers landed — `sub` via its `i - 1` and `mul` via `Scale`'s `x * k`),
and `examples/generic-record` + `examples/ffi` contribute `string_lit`. The union
is **23 of 23 opcodes** (905 reads over 12 modules), and `scripts/isa-census`
*checks* that the unexercised set is exactly its `UNREACHABLE_OPCODES` pin — now
**empty**, failing in both directions: an opcode that becomes uncovered without
a reason written in the pin fails, and so does a pin that still names an opcode
the corpus emits. Closing the last three was a lowering change, not a corpus
trick:

- `a - b` and `a * b` arrive as calls to
  `Austral.Pervasive::trappingSubtract`/`trappingMultiply`, and
  `Compiler_cps.trapping_arith_op` now emits `Sub`/`Mul` for those two callees.
  The JIT implements both opcodes with the *same* trap-on-overflow contract as
  the methods (`cps.rs`'s `emit_trapping_arith` branches to the registered
  symbol rather than using wrapping `isub`/`imul`), so the instruction is a new
  encoding of old semantics. Measured at runtime: `6 * 7 - 7 = 35` exits 0, and
  `maximum_int64 * 2` / `minimum_int64 - 1` still die with `Overflow in
trappingMultiply (Int64)` / `… trappingSubtract …` (exit 255) instead of
  wrapping. `safestos/cranelift/tests/cps_tests.rs` pins all four behaviours
  without a compiler in the loop: the two opcodes' values (including operand
  order) run in-process, and the two overflow cases re-exec the test binary as
  a child, because the trap is `process::abort` — asserted as SIGABRT plus the
  message, `29 passed, 1 ignored` in that file (measured 2026-11-14). `+` keeps its call deliberately: 0x05 doubles as the `for` loop's
  increment, and that path is left alone;
- `Mt.MCase` lowers to `Match` (0x0A) instead of a `CmpEq` + `If` chain over the
  tag slot, so tag dispatch is one instruction (and the union example's opcode
  count falls from 7 distinct to 6 because the comparison disappears).

The negative control the pin always had still holds: naming an opcode the corpus
emits fails the gate with `… is in the unreachable pin but the corpus now emits
it`.

What is checked on every `verify-invariants` run: `mm0-rs` compiles
`mm0/clif.mm1` and `mm0-c` checks the committed declarations against it (a
mismatched declaration is refused — measured, not assumed); `-- proves: 0
theorems` is pinned, and mm0 will not accept a `theorem` here without its proof
in the `.mm1`, so this layer cannot decorate itself later; `scripts/clif2mm0`
re-runs the measurement and diffs the file (a form the lowering starts or stops
emitting is a diff, not stale prose), with a negative control that appends a
form the lowering never emits and requires the check to fail.

What it still is **not**: a semantics. The CLIF→ISA refinement is W9.3 and
remains unproved. The object digest no longer stands alone — it now has a
companion that says what instructions the bytes were made of, and (since
2026-11-14) the *name* of the step relation the refinement has to be about — but
"shape matches" is not "semantics refines", and the plan keeps them apart: the
step relation is vocabulary without a definition, and no theorem mentions it.

### W9.3 Refinement: CLIF step → ISA step, reusing the certificates

The proof obligation is `step_clif_refines : clif_step s = isa_step (lower s)`
for the emitted subset, with the layer-1 facts entering as hypotheses:

**Refuted first, and measured (2026-11-14).** The tempting shortcut — "the
emitted memory op *is* the model's instruction" — is false as stated, for both
opcodes, and the refutation is now a gate rather than a caveat:
`scripts/mem-refines` reads the committed `mm0/clif.mm0`
(`load__i64__r_i64`: 1 operand, 1 result; `store__i64_i64__r_none`: 2 operands,
0 results; width in the form name) and the pinned model source
(`LeanRV64D/InstsEnd.lean:69238`/`:67365`: `execute_LOAD` with 5 arguments,
`execute_STORE` with 4, both `: SailM ExecutionResult`, both
`assert (width ≤b xlen_bytes)`, both able to return an `E_*` fault from
`vmem_read`/`vmem_write`). Different arity, different result, different address
space — no encoding makes one the other, so `verify-invariants` pins
`[pass] W9.3: load/store are refuted as the model's execute_LOAD/execute_STORE`.

**What survives is stated, with citations, in `mm0/store_projection.mm0`:**
`aligned8 a ⇒ in_linear_space a ⇒ store_projects s a v` and
`… ⇒ load_projects s a` over uninterpreted state transformers
(`jit_store_i64`/`isa_store_w8`, `jit_load_i64`/`isa_load_w8`) — the projection
that makes `execute_STORE` at width 8 the same state change as `store.i64` on
an aligned address (and `execute_LOAD` the same read as `load.i64`) *once the
register read, the immediate addition and the never-taken fault branch are
projected away*. The file has no `axiom` and no theorem, cites `clif.mm0` /
`rv64_memory.mm0` / `isa.mm0` by digest, and says `-- certificate: [gap]`;
`mm0-rs` + `mm0-c` accept it (`3 sorts, 8 term/def, 0 ax/thm`), which is a
statement about well-formedness, not truth.

**And the shape of it is proved, on a toy model — `mm0/store_projection_toy.mm0`
+ `.mm1`.** The toy memory is **three cells**, and its capacity is characterised in
the four cases such a memory has. A write either hits an address the memory
already holds, or it does not:

| case | axiom | what it says |
|---|---|---|
| refresh (address held) | `at_put_held_other`, `at_put_held_keeps`, `at_put_held_no_new` (+ `put_held`, `put_held_other`) | the written cell takes the new value, the other cell is untouched, and the **set of held addresses is unchanged** |
| free cell | `at_put_other` (guarded by `notfull m`) | a write leaves every other address's content alone |
| full, evicting | `at_drop` | after four pairwise-distinct stores the **earliest** address reads `unset` (four, because a three-cell memory evicts on the fourth store — the depth is the parameter, so the axiom must not hide it) |
| full, keeping | `at_keep_prev` | the oldest of the three survivors still reads the value it was given |

Two guards are load-bearing rather than decorative. `at_put_other` needs
`notfull`: an unconditional "writes never disturb other addresses" describes
*unbounded* memory and would contradict `at_drop`. `put_held_other` needs
`notfull` too: an unguarded "a held cell survives a write elsewhere" is false,
because a write to a fresh address in a full memory evicts the oldest cell,
which may be the held one. Both are the kind of side condition the real
obligation carries too (as `aligned8`/linearity).

`notfull` is deliberately **not closed under `put`**, and that is a correction,
not an omission: an earlier revision of the toy carried
`put_notfull` ("a write to a memory with a free cell leaves it with a free
cell"), which cannot hold together with `at_drop` and `at_put_other` in a
capacity-2 memory — the third distinct write finds no free cell by definition.
What the proofs actually need is the one-store fact, so the axiom is now
`put_init_notfull` ("the state after one store on `init` has a free cell") and
nothing claims the closure. The refresh case is what exposed this: telling a
refresh from an append requires knowing which addresses are held, which is what
made the closure's failure visible instead of latent.

**"A refresh never evicts", stated without arithmetic.** The property a program
depends on when it stores repeatedly to one address is that repetition cannot
push a live cell out, which sounds like a counting claim — "the number of held
addresses does not change" — and the toy has no numbers in it. So it is proved
as membership in both directions a size could move in, which is the same
statement and needs no induction over a count:

| direction | axiom | theorem |
|---|---|---|
| no held address is lost | `at_put_held_keeps` (guarded by `held m x`: a write to a *fresh* address in a full memory does evict) | `toy_jit_refresh_keeps_held_cells` |
| no address becomes held | `at_put_held_no_new` (no refresh guard needed — see below) | `toy_jit_refresh_no_new_held` |
| and the consequence | `at_put_held_other` twice | `toy_jit_repeated_refresh_never_evicts`: store `a`, store `b`, refresh `a` twice, and a load of `b` still returns `vb` |

Two facts from building this are worth recording, because both are the kind a
hand-written witness would have kept:

- `at_put_held_keeps` **needs** its `held m x` guard. Without it the statement
  "a held cell survives a write elsewhere" is false, and `scripts/toy-witness`
  prints the memory that kills it (`m=[(0,0),(1,0)] a=2 b=1` — the write to `2`
  takes the cell `1` was using).
- `at_put_held_no_new` does **not** need that guard, and my first draft had one.
  The checker reported it as `[FAIL] … holds in the witness, so the .mm0 rejects
  it for no reason`, which is the message for a candidate that is *true*: `put`
  introduces no address except the one it writes, so "a held address that is not
  the written one was already held" needs no premise about the write at all. The
  guard came out, which made the axiom stronger and the rejection honest — the
  first time this checker has changed an axiom rather than caught a bad theorem.

**The read half of the refresh case, and why it needs no guard.** A refresh that
keeps the other cell is only half of what a program observes; the other half is
that the refreshed address reads back the *new* value — otherwise an
implementation that silently dropped every write would satisfy the refresh case.
So the case is closed from both sides: `toy_jit_refresh_visible` and
`toy_isa_refresh_visible` prove, over the same trace (store `a`, store `b`,
refresh `a`), that a load at `a` returns `va2`, while
`toy_jit_refresh_keeps_other_cell`/`toy_isa_refresh_keeps_other_cell` prove that
a load at `b` still returns `vb`.

The asymmetry between the two halves is the interesting part: the *other-cell*
theorems need `vne a b` (and, underneath, `held`/`notfull`), the *refreshed-address*
theorems need nothing at all. `at_put` says a write is visible at the address it
wrote for any memory, so what a write returns at its own address does not depend
on capacity or on what else is held; whether the *other* cell survives does. A
first draft of these two carried `vne a b` to mirror their companions, and
`mm0-rs` refused it with `warning: Unused hypothesis` naming the argument — the
guard was in the statement only to look symmetric, so it came out, which made the
theorems stronger and the file's claim sharper. (Warnings are errors for the
"no `sorry`" check in spirit: an unused hypothesis is a premise the statement
does not need, and saying less is what a certificate is for.)

**A second width, so "the two sides agree" is not a width-8 accident.** The toy
adds width 4 beside width 8: `jit_store_i32`/`isa_store_w4` and
`jit_load_i32`/`isa_load_w4`, five more theorems. `Val` holds no bits, so a
narrower width cannot be a truncation of a stored bit pattern — it is an
uninterpreted `narrow4` applied to the value, carrying only the two laws every
truncation has, and the witness checker ties it to a function that actually loses
information (`x % 2`, so the discarded "narrowing is the identity" candidate
fails at `x=2`):

| axiom | says |
|---|---|
| `narrow4_idem` | narrowing twice is narrowing once |
| `narrow4_cong` | equal values narrow to equal values — without it the toy's `veq` is too weak to push anything through `narrow4` |

The width then shows up in a **proof**, not just in a name: the width-4 load
definitions narrow on the way out, so

- `toy_width4_load_after_wide_store` — a width-4 read of a cell written at width 8
  returns `narrow4 v`, by congruence on `at_put`; and
- `toy_width4_store_load_roundtrip` — a width-4 store read back at width 4 returns
  that same value, because congruence gives `narrow4 (narrow4 v)` and idempotence
  collapses it (a `veq_trans` through that middle term).

The other three (`toy_jit_store32_visible`, `toy_store32_projection`,
`toy_load32_projection`) are the width-8 shape one width down, and they are the
honest limit of what this buys: the two width-4 sides are *again the same term*,
so what is proved is that the projection's shape does not mention 8. It is not a
proof that Cranelift's `i32` truncation is `narrow4`, nor that the ISA's 4-byte
write is `put` — those stay uninterpreted, exactly as at width 8.

**A second width-8 store, so the assumption behind the projection is visible.**
Every projection theorem in the toy is true by construction: both sides are the
same `put`, so `toy_store_projection` is `veq_refl` after unfolding and says
nothing about whether a *different* store would also project. That is the
assumption the toy was making, and the file now states it by adding
`isa_store_w8_keep`, a store with the **other refresh order**: when the written
address is already held, the value the cell already has stays (first write wins),
where `put` keeps the newest. It is an uninterpreted `term` with two laws,
`keep_at_put_held` (a refresh changes nothing readable at that address) and
`keep_at_put_other` (no other address moves — and, unlike `put`, this needs no
`notfull`, because a store that keeps the old value adds no cell and cannot
evict). It cannot be a `def`: a different ordering is not a composition of
`put`, which is itself part of what the assumption is.

Where the projection then fails:

| theorem | what it says |
|---|---|
| `toy_keep_store_invisible` | a store under this ordering is **not** visible at the address it wrote — the mirror of `toy_jit_store_visible` |
| `toy_keep_refresh_misses_the_write` | on the *same refresh trace* as `toy_jit_refresh_visible` (store `a`, store `b`, refresh `a`), this definition reads `va` — the value from before the refresh — where the JIT side reads `va2` |

The two definitions agree on every trace that only takes free cells, which is why
the assumption is easy to miss: none of the toy's projection theorems exercises a
differing ordering. And the failure is machine-checked in both witnesses rather
than left to the reader: `scripts/toy-witness` and `ToyWitness.lean` both carry
the candidate `keep_store_visible` ("the second store is visible at the address
it wrote") and **require it to fail**, so a checker that silently accepted the
no-op model would be certifying the definition it was supposed to be testing.

A witness for the axiom set is short enough to state in the header: `Mem` a list
of at most three `(address, value)` pairs, `put m a v` the pair `(a,v)` consed
onto `m` with any pair at `a` removed and the result truncated to three, `get` the
first matching pair else `unset`, `held m a` = some pair has address `a`,
`notfull m` = fewer than three pairs, `veq` equality on `Val`. Under it a refresh
keeps the other pairs, a write elsewhere into a non-full memory keeps them, three
distinct writes into a fresh memory all stay readable, and the fourth evicts the
earliest while the three survivors keep their values.

**The witness is executed, not asserted.** Prose is how a false witness survives
review, so `scripts/toy-witness` (`scripts/toy-witness.rs`, one file, no
dependencies, compiled with `rustc`) is the same witness as code, and it checks
every axiom — universally quantified — over all memories of at most three cells
on four atoms (four because `at_drop` needs four distinct addresses; a smaller
domain would make the eviction axiom vacuous rather than cheap):

```
$ nix develop "git+file://$PWD#ocaml" -c bash scripts/toy-witness
toy-witness: 4 atom(s) (0 = unset), 4369 memories (every shape of at most three cells)
[hold]   at_drop                      26843136 instance(s)
…
[reject] put_notfull                      4368 instance(s) — m=[(0,0),(0,0)] a=1 v=0 -> put=[(1,0),(0,0),(0,0)] has no free cell
[reject] at_put_other_unguarded         209712 instance(s) — m=[(0,0),(0,0),(1,1)] a=2 b=1 v=0 -> get 1 was 1 now 0
[reject] put_held_other_unguarded       119232 instance(s) — m=[(0,0),(0,0),(1,0)] a=2 b=1 v=0 -> put=[(2,0),(0,0),(0,0)] no longer holds 1
[reject] at_drop_at_two_cells          6710784 instance(s) — m=[] x=0 y=1 z=2 -> get x=1 (three stores did not evict)
[reject] keep_store_visible              69904 instance(s) — m=[] a=0 v=1 -> keep=[] reads 0 as 0, not 1
[reject] narrow4_is_identity                 4 instance(s) — x=2 -> narrow4 x=0
[model]  model_equivariant              366996 instance(s)
toy-witness: 6/6 refutation(s) transport to every larger atom domain (model commutes with injective renaming); the axioms are proved for an arbitrary atom type in mm0/toy_witness/ToyWitness.lean
toy-witness: 20/20 axioms hold in the witness; 6/6 rejected candidates fail as they must
```

The six `[reject]` rows are the check's real content: the axioms the toy
*considered and dropped* must fail, and each fails with the memory that kills it.
`put_notfull` is the closure axiom removed in the previous revision — the machine
form of the correction above — and the two unguarded preservation axioms fail for
the reason their guards exist. `at_drop_at_two_cells` is the eviction axiom the
two-cell revision of the toy carried: with a third cell three stores no longer
evict, which is exactly the parameter the extension added, so a witness that let
it pass would be certifying a memory it had not modelled. A candidate that
*held* would be reported
`[FAIL] … holds in the witness, so the .mm0 rejects it for no reason`; an axiom
that failed would print its counterexample. Both directions matter, so the run
is a check on the guard choice, not just on the axioms.

Thresholds, stated rather than implied: the `[hold]` rows are exhaustive over a
finite domain (4 atoms, `0` = `unset`; the gate re-runs the witness at
`--domain 5` on every run and requires all 27 row verdicts to be identical — ~72 s
of enumeration, so a size artifact would be caught rather than remembered;
`--domain 6` is `[gap]`: it did not finish inside 240 s here). Those rows are a check of the
model's *code*, not a proof over an arbitrary `Val`; the arbitrary-`Val` proof is
the Lean development above. The `[reject]` rows need no threshold at all: a
counterexample memory embeds in every atom type with at least four atoms, and the
`[model] model_equivariant` row checks the commutation with an injective renaming
that the transport needs (§"Where the witness argument breaks"). So the domain
size is an input to an enumeration, not the reach of a claim.
`verify-invariants` runs it (a `[gap]` where `rustc` is absent, never a silent
pass) and also compares the axiom names it holds against the `axiom` declarations
in the `.mm0`, so an axiom added to the artifact without teaching the model about
it is a red gate — the same drift control the theorem count gets.

**The same witness, once more, in a second checker.** The Rust program is a
*model*: it enumerates cells and reports counterexamples. A program I wrote can
be wrong in the same direction as the axioms it checks, so the toy's axiom set
is also written as a Lean 4 development, `mm0/toy_witness/ToyWitness.lean`, with
the same `put`/`get`/`held`/`notfull`/`narrow4` — and there it is a proof, not a
check: each axiom is a theorem about an **arbitrary atom type** `α` with
decidable equality and a default atom for `unset`, so there is no atom-domain
threshold left to state. The proofs are structural on the list that models a
memory (induction for the `drop`/`get`/`held` lemmas, and the four-store
eviction sequence is *computed*: after four distinct stores the list is exactly
the three latest cells, which is where `at_drop` and `at_keep_prev` both read
off). The capacity lives in the model's type — `Cap` is the type of lists of at
most three cells, which is what `put` builds — rather than in an extra
hypothesis on each axiom, which is why the statements below read exactly as they
do in the artifact; two of them are false on a four-cell list, so putting the
carrier in the type is also the honest place for it. `narrow4` needs numbers, so
those two axioms are stated at `Fin n` for every `n ≥ 2`: two atoms is the
smallest domain a narrowing to two values can inhabit, not a scale chosen to
keep a check cheap. Each rejected candidate is a theorem about its *negation*,
exhibited at `Fin 4` (a refutation needs one domain). Two checkers disagreeing
would be the finding; on this set they agree, and they fail differently (the
Rust witness prints the killing memory; Lean refuses the proof).
The file proves the toy's **28 theorems** as well as its 20 axioms, and the same
way `store_projection_toy.mm1` does: each is a chain of axiom instances
(`toy_jit_fifth_store_drops_next` is `at_drop` fired with a one-store memory;
`toy_jit_repeated_refresh_never_evicts` is `at_put_held_other` twice). That is a
stronger statement than "the axioms are consistent" — the axioms are *enough* for
what the artifact claims, and if one were missing the theorem that needs it would
not close. The width-4 theorems take the narrowing as a parameter with
idempotence as its only law, because congruence is free when `veq` is equality;
the `Narrow` section supplies a concrete lossy `narrow4` for every `Fin n`, so
that hypothesis is not vacuous.

`verify-invariants` runs `lean` on the file in the `lean` shell (a `[gap]`
elsewhere) and requires three printed lists to agree: the axiom names against the
`.mm0`'s, the rejected candidates against the Rust witness's `[reject]` rows, and
the theorem names against the `.mm0`'s 28 — plus the count of `theorem toy_…`
declarations in the file against the length of that (literal) list, so a theorem
cannot go missing while the list still claims it. A candidate that only one
checker refuses, or a theorem only one side carries, is a red gate. The whole file
elaborates in ~1.7 s (measured 2026-11-16): it used to be a kernel sweep over a
finite atom set and took 44 s.

**Where the witness argument breaks.** The point of having the witness is to say
plainly where it stops being evidence, and the Lean file is where those limits
are machine-checked rather than asserted:

- `put_notfull` is *false* in the model (proved: `put_notfull_breaks`), and so
  are `at_put_other`/`put_held_other` without their `notfull` guards
  (`at_put_other_unguarded_breaks`, `put_held_other_unguarded_breaks`) and
  `narrow4 = identity` (`narrow4_is_identity_breaks`, at `x = 2`), and so is the
  two-cell eviction axiom the toy used to carry (`at_drop_at_two_cells_breaks`:
  three distinct stores no longer evict). These five are theorems, not comments —
  the gate would go red if one of them stopped being refutable.
- The model has **at most three cells**; `put` is `((a,v) :: filter (·.1 ≠ a)).take 3`.
  There is no arithmetic in it, which is why "refresh never evicts" had to be
  proved as membership in both directions (`at_put_held_keeps` /
  `at_put_held_no_new`) rather than as a count.
- `veq` is equality on `Val`, which makes consistency cheap: a degenerate model
  with `veq := ⊤` satisfies every axiom. The content comes from the instance
  proofs and the rejected candidates, not from consistency alone — which is
  exactly why the rejected rows are the check's real content.
- `narrow4 := x % 2` is *a* function satisfying the two laws, not Cranelift's
  truncation; the width-4 side proves the projection's *shape* does not mention 8,
  not that `i32` truncation is this function. The real `narrow4` is uninterpreted
  at both widths.
- The *Rust* arm runs over a finite atom domain (4 atoms by default, `0 = unset`),
  and since 2026-11-16 no claim depends on that number. Its two directions are
  separated and one of them is domain-free:
  a **refutation** at this domain is a refutation for every atom type with at
  least four atoms, because the counterexample memory embeds in α under an
  injective renaming — and that transport is checked, not assumed: the
  `[model] model_equivariant` row renames every atom by two injections (one
  order-preserving, one not) into a strictly larger domain and requires `put`,
  `get`, `held`, `notfull` and `keep` to commute with them, so a model that
  compared atoms by anything but equality would be caught. The rows that
  *assert* (the 20 axioms) are a check of the model's code against the artifact's
  statements at one domain; coverage over an arbitrary atom type is the Lean
  proof's job and is not claimed here. `--domain 5` still gives the identical
  verdict on every row, which is a regression check on the enumeration rather
  than evidence of coverage. What neither arm establishes is that `Val` is
  `Fin n` — the model interprets the abstract sort, and the axioms are consistent
  with it, which is a different claim from consistency with the compiler.

**The toy cites the claim by digest, and the check must be able to fail.** The
toy's header carries `-- cites: mm0/store_projection.mm0 sha256=…`, and
`verify-invariants` compares it with the current file: if the claim is edited and
nobody re-reads the toy, the proof is certifying a statement the repository no
longer makes, which is the one drift a prover-only gate cannot see. A comparison
that cannot fail is decoration, so the gate re-runs the same rule against a copy
of the claim with `store_projects` renamed (the toy untouched in that run) and
requires the pin to be reported stale — `[pass] W9.3: the toy's citation digest
refuses a changed claim`. Two failure modes of the control itself are checked
rather than assumed: an edit that does not apply (`the drift edit did not apply,
so it controls nothing`) and a pin that accepts the renamed claim (`a renamed
claim still matches the toy's pin`). Measured 2026-11-15 by weakening the
comparison to `return 0` in a copy of the gate: the real check still passed and
the control went red with the second message, which is the bug it exists to
catch; with an edited claim and an un-updated pin, both checks fail.

- **Toy theorem count: 28** — pinned, not just described: `verify-invariants`
  fails if `mm0/store_projection_toy.mm0` declares a different number of
  `theorem`s, if the `.mm1` proves a different number, or if this line or the one
  in `AGENTS.md` still carries the old count. Add a theorem to the toy and both
  lines move with it, or the gate is red.
- **Toy axiom count: 20** — pinned the same way, against the `.mm0`'s `axiom`
  declarations and the `.mm1`'s, so adding an axiom (as the refresh case did)
  fails the gate until both documents say so. The count is what a reader uses to
  size the trust footprint of the toy, which makes a stale one a misstatement
  rather than a typo.
- **Toy shape: 3 sorts, 18 term/def, 48 ax/thm** — the same figures, pinned to
  the *compiler's* summary line rather than to a recount here: `mm0-rs` counts
  `provable sort Prop` as a sort and a `grep '^sort '` does not, so a hand count
  would pin 2 sorts and be wrong from the start. `verify-invariants` takes the
  line `mm0-rs` prints and requires it at **every** site that quotes the shape,
  not just at this bullet: the §3.5 transcript's `[detail]` and `[pass]` lines,
  this bullet in the plan and in `AGENTS.md`, and the prose below. One anchor is
  what lets a retired figure survive — the toy's `term/def` count already grew
  from 12 to 17 when the width-4 definitions landed, and a gate that pinned only
  the bullet would have stayed green while the transcript quoted the older file.
  The figure is never written into `verify-invariants` itself: a comment quoting
  it is a ninth site, and the one that historically went stale.

28 theorems check under `mm0-c` with no `sorry` — `3 sorts, 18 term/def, 48 ax/thm`,
20 of them axioms: a fresh memory reads `unset`; the JIT-side store
is visible at the written address; the width-8 model side is; the two agree
there; the two load definitions return the same value; while a cell is free a
store leaves other addresses alone; two distinct stores into a fresh memory
both stay readable (the composition of `at_put_other` through
`put_init_notfull` with `at_put` by `veq_trans`); a **refresh** — write `a`,
write `b`, write `a` again — leaves `b`'s value readable, which is what
`at_put_held_other` says and what a model that evicted on every write would get
wrong; a refresh also leaves `b` *held* and creates no address that was not held
(`toy_jit_refresh_keeps_held_cells`, `toy_jit_refresh_no_new_held`), and **two**
refreshes at `a` still leave `b`'s value readable
(`toy_jit_repeated_refresh_never_evicts`) — repetition at one address cannot
evict, which is the property behind the counting statement above; and, one width
down, a width-4 store is visible at its address (`toy_jit_store32_visible`), the
two width-4 sides agree (`toy_store32_projection`, `toy_load32_projection`), a
width-4 load of a width-8 cell returns the narrowed value
(`toy_width4_load_after_wide_store`), and a width-4 store read back at width 4
returns it too (`toy_width4_store_load_roundtrip`);
`toy_jit_third_store_kept` reads the first of three distinct stores back — the
counting step `put2_init_notfull` exists for; a **fourth** distinct store drops
the earliest address; a **fifth** drops the next oldest, which is the same
`at_drop` axiom fired with a one-store memory as its starting point — the axiom
is stated for an arbitrary `m` precisely so it fires at any depth, which is what
makes the fifth store a theorem and not a new assumption; and what the fourth
store *kept* is still readable (`toy_jit_recent_cells_readable`, from
`at_keep_prev`), with the projection still holding on that four-store
memory. The store-then-load-elsewhere theorems need both `vne a b` and
`notfull m`: with either dropped `mm0-rs` refuses, so the toy's stand-ins for
the real claim's side conditions are load-bearing, not decoration. The refresh
theorem needs `put_held_other` for a reason worth stating: after writing `a`
then `b`, `a` is still held *only* because a write elsewhere into a memory with
a free cell keeps it — without that step the model could not tell a refresh from
an append at all.

**The toy has a mutation control, so its theorems cannot be vacuous.**
`verify-invariants` rewrites an axiom's conclusion and requires the proof of the
theorem that used it to be refused — thirteen mutations, one per substantive axiom:
`at_put`'s read-after-write becomes "read unchanged" (must break
`toy_jit_store_visible`), `at_put_other`'s preserved value becomes `unset`
(must break `toy_jit_store_load_other`), `veq_trans`'s conclusion becomes
reflexive (must break `toy_jit_two_cells_preserved`), `at_drop`'s `unset`
becomes the written value (must break `toy_jit_fourth_store_drops_oldest`),
`at_keep_prev`'s `vy` becomes `vx` (must break
`toy_jit_recent_cells_readable`), `at_put_held_other`'s preserved value becomes
`unset`, `put_held`'s "the address written is held" becomes "the *value* written
is held" (both must break `toy_jit_refresh_keeps_other_cell`),
`put_init_notfull` becomes `notfull init` (must break
`toy_jit_two_cells_preserved`), `put2_init_notfull`'s two-store state becomes
`init` (must break `toy_jit_third_store_kept`), `at_put_held_keeps`'s kept address becomes the
written one (must break `toy_jit_refresh_keeps_held_cells`), and
`at_put_held_no_new`'s `held m y` becomes `held init y` (must break
`toy_jit_refresh_no_new_held`), `narrow4_cong`'s narrowed right-hand side becomes
the value itself (must break `toy_width4_load_after_wide_store`), and
`narrow4_idem`'s collapse becomes the value itself (must break
`toy_width4_store_load_roundtrip`). Measured 2026-11-16, all thirteen:
`mutations refused: toy_jit_store_visible(refused),
toy_jit_store_load_other(refused), toy_jit_two_cells_preserved(refused),
toy_jit_fourth_store_drops_oldest(refused), toy_jit_recent_cells_readable(refused),
toy_jit_third_store_kept(refused), toy_jit_refresh_keeps_other_cell(refused),
toy_jit_refresh_keeps_other_cell(refused), toy_jit_two_cells_preserved(refused),
toy_jit_refresh_keeps_held_cells(refused), toy_jit_refresh_no_new_held(refused),
toy_width4_load_after_wide_store(refused), toy_width4_store_load_roundtrip(refused)`.
`at_put_held_other`'s parameters are named `(m) (x y w)` rather than `(m) (a b v)`
so its conclusion line is textually distinct from `at_put_other`'s: the mutation
control rewrites the first textual occurrence, and two axioms whose conclusion
lines read identically would let one row mutate the wrong axiom and report a
`[FAIL]` that is about the control, not the proof.
A mutation that fails to apply, or a proof that still checks after its axiom was
falsified, is a `[FAIL]` — the first would mean the axiom is gone, the second
that the theorem was never using it. This is the one thing a prover cannot check
about itself: `mm0-c` verifies statements against artifacts, and a mutated file
is by construction not the artifact.

**And the same control against the Lean model, because a proof's failure mode is
different.** A theorem can stop depending on the *model* rather than on an axiom:
weaken a definition and the file still elaborates, having proved something about a
different memory. So `verify-invariants` mutates the definitions themselves and
requires elaboration to fail — five rows, each attributed by line rather than by
message: `take 3` → `take 4` (no eviction), `drop` keeping the written address,
`notfull` weakened to four cells, `narrow4` becoming the identity, and `keep`
becoming `put` (which would make the *rejected* candidate `keep_store_visible`
true, so the refutation would be the thing that broke). Measured 2026-11-16:
`lean mutations refused: put-no-truncation, drop-keeps-written-address,
notfull-weakened-to-four, narrow4-becomes-identity, keep-order-becomes-put` — the
gate appends the line each refusal was reported at, and that number moves as the
file grows, so only the notes are quoted here. Each refusal is *below* the
definition it edited, which is what makes it about the proof and not about the
edit's syntax. Three control-defect directions are
red rather than silent, all three measured on scratch copies: an edit that
matches nothing (`edit-did-not-apply`), an edit that applies but changes nothing
(`still-checks`), and an edit that breaks the syntax at the edited line
(`refused-at-edited-line`).

**What the proofs rest on, printed rather than assumed.** The file ends with
`#print axioms` for all 48 declarations it proves (the toy's 28 theorems and the
20 theorems that state its axioms), the way `hydro_lean/Hydro/AxCheck.lean` audits
its own headlines, and `verify-invariants` requires every line to name a subset of
`propext`, `Classical.choice`, `Quot.sound` — Lean's standard foundation, pulled in
by `simp`/`omega` — and all 48 to be audited. What the audit is *for*: `sorryAx`
would appear in that list, so a `sorry` that still elaborated could not hide;
`veq_*` and both `keep_at_put_*` rules report that they depend on no axioms at all.
No axiom of the toy appears as an axiom anywhere in the development — that is the
claim the file has been making since it stopped enumerating, and this is the
listing that makes it checkable.

The audit covers the 48 declarations it names, and nothing else, so it cannot see
a `sorry` in a lemma that nothing prints — dead code, or a helper a later edit
stops using. That is why `verify-invariants` *also* scans the file's code for
`sorry`, `admit`, `native_decide` and `ofReduceBool` after stripping comments:
the scan needs the real thing (Lean block comments nest, so it is a small lexer,
not a `sed` one-liner) and the audit needs the scan, since `#print axioms` of a
theorem says nothing about the lemmas it never names. The gate therefore carries
both directions itself: it appends `theorem scan_control_injected : True := by
sorry` to a scratch copy, requires exactly one hit at the injected line, and —
the point of the control — first requires `lean` to **accept** that copy with
exit 0, so the scan is shown to catch what elaboration does not. The reverse
control appends the same four words inside a line comment and a nested block
comment, requires zero hits, and requires that copy to elaborate too, so a
scanner that flagged comments fails rather than quietly passing. Both checks are
load-bearing by construction: with the scanner stubbed out the injection control
reports `[FAIL]`, and with a scanner that returns a hit unconditionally the
comment control does too (both measured).

**The control itself was tested by making the mutations no-ops** (2026-11-15),
which is the only way to know a negative control is not a rubber stamp. Three
scratch copies of `scripts/verify-invariants` with one edit each, all run in the
`ocaml` shell, all reporting `1 failed` and exit 1:

| scratch edit | expected detail | measured detail |
|---|---|---|
| patterns that match nothing | `(mutation did not apply)` ×3 | `mutations: …(mutation did not apply) … (mutation did not apply) … (mutation did not apply) (3 of 3)` |
| patterns that apply but change nothing (`provable sort Prop;` → `provable sort Prop; `) | `(still checks)` ×3 | `mutations: …(still checks) …(still checks) …(still checks) (3 of 3)` |
| patterns that break the file elsewhere (delimiter → `[ ]`) | `(refused, but not at that theorem)` ×3 | `mutations: …(refused, but not at that theorem) ×3 (3 of 3)` |

So a vacuous control is a red gate in all three of its failure directions, not a
silent pass.
The actual work — that Cranelift computes `put` and that Sail's
`vmem_write`/`vmem_read` at width 8 do too, and that the fault branch is not
taken on an aligned linear address — is exactly what the toy sets aside by
definition, and its header says so. What the toy buys is narrower and real: the
obligation's *shape* is consistent (the projection is not circular or vacuous
once made concrete), and a gate ties the proved statement to the stated one, so
a proof cannot quietly drift onto an easier neighbour.

So the plan's position is: the strong claim is refuted, the weak one is written
down and unproved in general, its shape is proved for a toy model, and the
difference between the three is enforced by gates instead of a footnote.

The rest of the obligation stands as before:

**Status 2026-11-14 — stated, cited, and refusal-tested; not proved.**
`mm0/clif.mm0` now declares the relation and nothing more:

```
sort St;                        -- a machine state: registers + memory, uninterpreted
term clif_step (s: St) (o: op) (t: St): Prop;
```

`mm0-c` accepts it as `5 sorts, 21 term/def, 0 ax/thm` — the relation is a
*symbol*, not an axiom: a `term …: Prop` says the vocabulary exists, an `axiom`
would say the relation holds, and only the latter is a claim. The obligation
itself is written where the file's reader will see it (`clif_step s o t ⇒
isa_step (lower o) s t`), with the layer-1 artifacts it may draw on cited by
pinned digest — `mm0/isa.mm0` plus the same four certificates `isa.mm0` cites
(`linearity`, `layout`, `flux1d`, `slot_layout`).

What is **certified** is nothing, and the file says so: `-- certificate: [gap]`
because `mm0/approved/` has no `isa` approval. That is not modesty, it is the
approval rule — a certificate is an owner commit, so this layer cannot mint one
for itself. The state is enforced in four places rather than described:
`verify-invariants` fails the artifact if an `axiom`/`theorem` appears in it, if
any `-- cites:` digest stops matching the file it names, if the obligation line
or the `clif_step` declaration disappears, or if `mm0/approved/isa.stmts`
exists while the file still claims `[gap]` (then the citation and the proof must
arrive together). Each was mutation-tested: adding `axiom mischief …` and
touching `mm0/approved/isa.stmts` turns them red (measured 2026-11-14: `101
passed, 5 failed` — the three above plus, by construction, mm0-c's acceptance
and the drift check, since the mutated file is no longer what the generator
emits), and the tree is restored afterwards.

The remaining work is exactly what it was, now with a place to put it:

- `linear s`, `typed s` and `in_bounds s i` are already the vocabulary of
  `isa.mm0`, and `slot_layout.mm0` is the pattern for making one of them
  provable instead of cited (define the predicate, prove its preservation,
  keep one sentence about the compiler visible);
- the Cranelift layer must *not* restate the certificates: its hypotheses cite
  `linearity_certificate.mm0`, `layout_certificate.mm0` and the liquid evidence
  with pinned digests, and `scripts/isa-hypotheses`' content check is what keeps
  a citation from being decorative — the failure W8 spent a day fixing;
- exit criterion (when it exists): `mm0-c` accepts the refinement artifact, a
  mutated lowering that changes one emitted instruction is *refused* by it, and
  the hypothesis list is content-checked with a pinned count.


**What the four pending approvals would cover, exactly (2026-11-16).** §W3
records four artifacts waiting on an owner commit — `isa`, `slot_layout`,
`store_projection`, `store_projection_toy`, counted as 193 + 23 + 11 + 70
statements. Those counts are the *comparator's* (declarations plus the notation a
file needs to round-trip), and a count is not a list, so this subsection records
what each approval actually hands the gate. Measured, not remembered:
`scripts/mm0-comparator approve --dry-run <artifact>` writes nothing, and the
digests below come from scratch copies written with
`--approved-root /tmp/appr`, so nothing under `mm0/approved/` was created.

| artifact | stmts | composition | `closure-sha256` |
|---|---|---|---|
| `isa` | 193 | 5 sorts + `Prop`, 70 terms, 1 def (`in_bounds`), 65 axioms, 26 theorems = 168 declarations; + 24 numeral-notation lines (`prefix n0: $0$ prec max;` … for `n0`–`n48`) + 1 `delimiter` | `27de0884e3408cd45800b07dd118f0fb2cd977ed15e868843c626dbeb32d0aed` |
| `slot_layout` | 23 | 3 sorts + `Prop`, 7 terms, 1 def (`in_bounds`), 7 axioms, 2 theorems = 21 declarations; + 1 numeral line + 1 `delimiter` | `2be35aa287735ea593c71ad5787b1b34dc725dcaccdece6460a26366db5e3114` |
| `store_projection` | 11 | 2 sorts + `Prop`, 8 terms, **0 axioms, 0 theorems** | `26699dbc9a27c1c4ae87bfc55ddd140bc85c24f5d082fac8c449fd56bf7f4554` |
| `store_projection_toy` | 70 | 2 sorts + `Prop`, 10 terms, 8 defs, 20 axioms, 28 theorems = 69 declarations; + 1 `delimiter` | `09842af8888899541cef6b58221d346616fe64310cc6b102506f512f81cedcbf` |

The composition and `closure-sha256` columns are **counted, not read back**:
`verify-invariants` compares the declared `N sorts` (adding `Prop` back when the
cell names it), `N terms`, `N defs`, `N axioms`, `N theorems`, the "= N
declarations" sum (which counts every `sort` line, `Prop` included: 6 + 70 + 1 +
65 + 26 = 168 for `isa`), the numeral and `delimiter` lines, and the closure
digest, with the file a run wrote —
`[pass] W9.3: the composition table's counts match the files it records (4
row(s), 0 from mm0/approved/)` and `… closure digests match the files it
records (4 row(s), 0 from mm0/approved/)`. The breakdown is not decoration: a
row is **counted from `mm0/approved/<stem>.stmts`** once that file exists,
because the row is a record of an approval and the approval is those bytes; a
row still pending is counted from a fresh staging. `0 from mm0/approved/` today
is the four approvals being un-made, and it moves on its own when one lands. If
the staged copy and the approved copy differ — an artifact edited after its
approval — the row reports `staged bytes differ from the approved copy` instead
of describing whichever copy was read, so a drift cannot keep the counts green.

Changing the toy row's `20 axioms` to `21` is refused as
`store_projection_toy(says 21 axioms, staging 20)` (measured), so the cell that
says what an approval *contains* cannot drift from the approval while its
statement count stays right.

That measurement is in-gate, so the whole gate is measured too, the way §W3's
emitter cell is: `scripts/w93-composition-drift-control` copies this plan,
rewrites the toy row's composition cell from `20 axioms` to `21 axioms` — leaving
the `70` stmts cell alone, which is the point, since the stmts check stays green
and only the composition column can catch it — and runs `verify-invariants`
against the copy through `AUSTRALVM_PLAN`. Measured 2026-11-17: exit 1,
`verify-invariants: 140 passed, 6 explained gap(s), 1 failed`, detail
`store_projection_toy(says 21 axioms, staging 20)`, `[FAIL] W9.3: the composition
table's counts match the files it records`, and the closure digests, the
approval control, and §W3's stmts and emitter checks all still passing — one
drifted count, one failure, nothing else moved. The run is recorded as the fourth
`docs/data/w3/probes.tsv` row. Its `expected_message` skeleton,
`…(says … axioms, … …)`, pins the field word to the message frame: each field is
compared by its own literal (`$art(says $claimed axioms, $src $ax)`), not by
`(says $claimed $label, …)` with the word read from a loop variable, so the
chunks the gate re-checks (`(says `, ` axioms, `) are adjacent to each other in
the statement that prints them rather than merely somewhere in the script. The
source word (`staging` or `approved`) stays interpolated, and is therefore
vocabulary rather than adjacency: a row is counted from whichever of the two
files applies, so making the word literal would need one format per field *and*
file — fourteen hand-written strings — because any single format used for both
files carries the variable again, and that duplication would be a second copy of
the field mapping sitting under the first. What can be literal without that
duplication is checked instead: `[pass] W9.3: every field word a composition cell
names has a literal comparison (8 word(s))` re-reads the four cells, extracts
every counted word (`2 sorts`, `20 axioms`, `1 def`, `24 numeral-notation lines`,
backticks stripped, `def` mapped to `defs` and `numeral…` to `numerals`), and
refuses a word no per-field literal compares — so a field added to a cell cannot
go uncounted. Measured the same day by spelling `axioms` as `axiom` in its
comparison: `[detail] §W9.3 fields: axioms(no literal comparison)` and
`139 passed, 6 explained gap(s), 2 failed`.

That the closure column is its own claim, not a by-product of the counts, is
measured the same way: `scripts/w93-closure-drift-control` copies this plan and
flips the first hex digit of the toy row's `closure-sha256` cell, leaving every
count cell — `70` included — alone. Measured 2026-11-17: exit 1,
`verify-invariants: 140 passed, 6 explained gap(s), 1 failed`, `[FAIL] W9.3: the
composition table's closure digests match the files it records`, detail
`store_projection_toy(says '19842af88888…', staging '09842af88888…')` — the
row's digest against the one the file hashes to, twelve characters each — while
the counts line, the approval control, and §W3's stmts and emitter checks still
pass. So a row can be right about what an approval *contains* and wrong about
which bytes it contains, and only this column catches that; the run is recorded
as the fifth `docs/data/w3/probes.tsv` row. Its skeleton is vocabulary-level for
the same reason the composition row's source word is: the file word in
`says '<row>' <src> '<file>'` is the variable `$_c_src` (`staging` or
`approved`), so `staging` is checked as a word the script contains rather than
text adjacent to the digests — the frame `(says '` and the fact that both
counts are printed are checked, the adjacency is not.

The refinement's certification state is measured the same way, because it is the
claim a reader would most like to believe: `W9.3: certification state matches
mm0/approved/` reads `mm0/clif.mm0`'s `-- certificate:` line and requires it to
say `[gap]` while no `mm0/approved/isa.stmts` exists, and to name the approval
once one does. `scripts/w93-certification-drift-control` copies `clif.mm0`,
rewrites that one line to claim `mm0/approved/isa.stmts` — which does not exist —
leaves the vocabulary, citations and forms alone, and runs the gate with
`AUSTRALVM_CLIF` pointing at the copy (the override moves the four checks that
read that file — the `proves: 0 theorems` pin, the vocabulary, the pinned
citations and the certificate line; the corpus re-derivation still compares
generated bytes with the committed path). Measured 2026-11-17: exit 1,
`verify-invariants: 140 passed, 6 explained gap(s), 1 failed`, `[FAIL] W9.3:
certification state matches mm0/approved/`, detail `no isa approval, yet
clif.mm0 does not say [gap]`, with the `clif_step` vocabulary and the five pinned
citations still passing — one uncertified claim, one failure. Recorded as the
`certification-claim` row of `docs/data/w3/probes.tsv`.

The approved branch — an approval exists, so a `[gap]` line is the drift — is
measured too, and until the owner's approvals landed it took moving one path: the
check asked `$repo_root/mm0/approved/isa.stmts` directly, which no override could
reach, so it now asks `$_appr_root/isa.stmts` — the directory
`AUSTRALVM_APPROVED_ROOT` already names for the §W3 rows. Measured 2026-11-18 with
a scratch approval: exit 1, `verify-invariants: 124 passed, 17 explained gap(s),
7 failed`, `[FAIL] W9.3: certification state matches mm0/approved/`, detail
`…/approved/isa.stmts exists — clif.mm0 must cite it, not a gap`. Seven failures,
not one, and the row said why: a scratch approval is inconsistent with the rest of
the tree *by construction* — it is not the approved bytes, so the §W3 stmts
column, the pending re-derivation, the composition counts and closure digests, the
toy's axiom count and the staging count all move with it. A consistent approval is
the owner's commit, so once the four §W3 approvals landed that branch became the
tree's own state, and `scripts/w93-certification-drift-control` measures it in
both directions from a copy of `mm0/clif.mm0` (`[gap]` with no approval, an
uncited approval with one). What no scratch approval could reach is the digest the
citation carries: a model may name the approval and still pin bytes it no longer
holds, which is how a citation rots. `scripts/w3-drift-controls cert-approved`
flips one hex digit on the copy's `-- cites: mm0/approved/isa.stmts` line, the copy
read through the existing read-only `AUSTRALVM_CLIF`. Measured 2026-11-20: exit 1,
`verify-invariants: 178 passed, 0 explained gap(s), 1 failed`, `[FAIL] W9.3: the
refinement's citations still match their artifacts`, detail `stale citations:
mm0/approved/isa.stmts`, with the certificate line itself, the `Phase D` trust
split and the §W3 state cells still passing — one rotted citation, one failure.
The model's other self-claim is
driftable the same way, and measured: a copy of `mm0/clif.mm0` whose `-- proves:
0 theorems` line says `1`, run with `AUSTRALVM_CLIF` at the copy, turns
`[FAIL] W9: the CLIF model still claims no theorem` and nothing else —
`verify-invariants: 140 passed, 7 explained gap(s), 1 failed`, 2026-11-18, the
`proves-line-drift` row of `docs/data/w3/probes.tsv`.
And the approved path is exercised, not assumed:
the gate copies the toy staging, deletes its first `axiom` line, and counts the
result *as an approval* through the same function the rows use — it must come
back `store_projection_toy(says 20 axioms, approved 19)` and fail, which is
`[pass] W9.3: an approved copy that drops an axiom fails the composition counts`.
The root the rows read is `mm0/approved/` unless `AUSTRALVM_APPROVED_ROOT` names
another directory — the same override the comparator's `--approved-root`
provides, and the one measurement above used: with that variable set to a
scratch root holding a `slot_layout` approval whose first axiom was deleted, the
gate printed `(4 row(s), 4 from mm0/approved/)`,
`slot_layout(staged bytes differ from the approved copy)
slot_layout(says 7 axioms, approved 6)` and one `[FAIL]` (measured, 2026-11-16).
The variable grants
nothing: it moves which directory is *read*, and it is also the comparator's
default for `approve` *and* `check` (the flag wins when both are given), so a
reader who names a root once does not approve into one directory and check
against another. That matters because the two have to agree: an override holding
only the artifact under test made every other approved artifact look pending
until `check` was brought onto the same root.

All four are byte-identical to what `mm0-comparator approve` would write (the
`statements-sha256` headers are `2619b5ec…`, `98dd6399…`, `5c201a80…`,
`abed1393…`); the digest is what pins the bytes, because the canonical form
**erases names** — a declaration appears as `term l4 (x: l0): l0;` whatever it
was called in the source. So the names below are the source names, and the
digest is what an owner is actually approving.

**`isa` (193).** Axioms, 65: the 19 model rules `req_refl`, `req_symm`,
`req_trans`, `radd_comm`, `radd_assoc`, `rmul_assoc`, `rmul_distrib`,
`reg_setreg_same`, `reg_setreg_other`, `run_add_def`, `run_sub_def`,
`run_mul_def`, `run_move_def`, `ir_add_sem`, `ir_mul_sem`, `ir_sub_sem`,
`ieq_trans`, `beq_cong_dec`, `bounds_from_layout`; then 23 encoder axioms and 23
decoder axioms, one pair per emitted instruction form — `enc_`/`dec_` ×
{`int_lit`, `var`, `let`, `app`, `add`, `sub`, `return`, `if`, `while`,
`match`, `cmp_lt`, `cmp_gt`, `cmp_lte`, `cmp_gte`, `cmp_eq`, `cmp_neq`, `and`,
`or`, `mul`, `not`, `deref`, `string_lit`, `store`}. Theorems, 26: the 23
`encdec_*` round trips (one per form, `theorem` not `axiom` — the pair of
encoder/decoder laws is what proves them) plus the 3 step refinements
`add_refines`, `mul_refines`, `sub_refines`. Terms, 70: the 23 `ins_*`
constructors of the same form list; the 24 numerals `n0`–`n10`, `n16`–`n25`,
`n32`, `n33`, `n48`; `radd`, `rmul`, `rneg`, `req`, `reg`, `setreg`, `neq`,
`linear`, `typed`, `scons`, `mem`, `slots`; `run_add`, `run_sub`, `run_mul`,
`run_move`; `ir_add`, `ir_mul`, `ir_sub`; `ienc`, `idec`, `ieq`, `beq`. One def,
`in_bounds`.

**`slot_layout` (23).** Terms: `n0`, `snil`, `scons`, `mem`, `seq`, `slots`,
`alloc`. Def: `in_bounds`. Axioms: `mem_here`, `mem_there`, `seq_refl`,
`seq_symm`, `seq_trans`, `mem_cong`, `slots_alloc`. Theorems: `mem_alloc`,
`in_bounds_alloc`. Sorts: `nat`, `Slots`, `St`.

**`store_projection` (11).** Terms: `aligned8`, `in_linear_space`,
`jit_store_i64`, `isa_store_w8`, `jit_load_i64`, `isa_load_w8`,
`store_projects`, `load_projects`. Sorts: `St`, `Val`. Nothing else — approving
this file approves a *vocabulary*, which is why its own header says
`-- certificate: [gap]` and why the approval does not turn the projection claim
into a proved one.

**`store_projection_toy` (70).** Terms: `init`, `unset`, `narrow4`, `put`,
`get`, `notfull`, `held`, `vne`, `veq`, `isa_store_w8_keep`. Defs:
`jit_store_i64`, `isa_store_w8`, `jit_load_i64`, `isa_load_w8`,
`jit_store_i32`, `isa_store_w4`, `jit_load_i32`, `isa_load_w4`. Axioms, 20:
`veq_refl`, `veq_trans`, `vne_sym`, `narrow4_cong`, `narrow4_idem`,
`get_init`, `init_notfull`, `put_init_notfull`, `put2_init_notfull`, `at_put`,
`put_held`, `put_held_other`, `at_put_held_other`, `at_put_held_keeps`,
`at_put_held_no_new`, `at_put_other`, `at_drop`, `at_keep_prev`,
`keep_at_put_held`, `keep_at_put_other`. Theorems, 28: `toy_init_unset`,
`toy_jit_store_visible`, `toy_isa_store_visible`, `toy_store_projection`,
`toy_load_projection`, `toy_jit_store_load_other`,
`toy_isa_store_load_other`, `toy_jit_two_cells_preserved`,
`toy_isa_two_cells_preserved`, `toy_jit_refresh_keeps_other_cell`,
`toy_isa_refresh_keeps_other_cell`, `toy_jit_refresh_visible`,
`toy_isa_refresh_visible`, `toy_jit_refresh_keeps_held_cells`,
`toy_jit_refresh_no_new_held`, `toy_jit_repeated_refresh_never_evicts`,
`toy_jit_third_store_kept`, `toy_jit_fourth_store_drops_oldest`,
`toy_jit_fifth_store_drops_next`, `toy_jit_recent_cells_readable`,
`toy_fourth_store_projection`, `toy_jit_store32_visible`,
`toy_store32_projection`, `toy_load32_projection`,
`toy_width4_load_after_wide_store`, `toy_width4_store_load_roundtrip`,
`toy_keep_store_invisible`, `toy_keep_refresh_misses_the_write`. Sorts: `Val`,
`Mem`.

Two things this pins down that a count could not:

- **What an approval buys, and what it does not.** It holds out the statements:
  the gate then refuses any statement, closure or theory mismatch against the
  approved copy. It does not make `store_projection`'s claim true (the file has
  no axioms and no theorem; the claim stays `[gap]`, and the toy is the only
  place any of this is proved), and it does not upgrade `isa`'s encoder laws to
  facts about the compiler — they are the model's, cited as such.
- **The counts are gate-pinned, the names are not.** `verify-invariants` prints
  `w3-approved: flux1d_certificate(100 stmt) gap_certificate(107 stmt)
  isa(193 stmt) … store_projection_toy(70 stmt)` every run (the `w3-pending:`
  line is empty today, all four rows approved), so a statement added to any of
  the four moves that line and this table has to move
  with it; the closure digests
  above are the rule for the *names*, since the canonical form does not keep
  them. `scripts/check-links` will not catch a stale name in a list like this —
  the enforcement is that a re-approval changes the digest in this table, which
  is a diff a reviewer sees.
- **Both digest columns are re-derived, not trusted.** The `closure-sha256` cell
  above and §W3's `statements-sha256` cell are the same four stagings: the gate
  stages each §W3 row into its own scratch directory, reads the digest back out
  of the file the comparator wrote, and compares — which is the line
  `[pass] W3: the plan's §W3 digest column matches the file it describes (4
  row(s), 0 approved)` — the trailing count is the approved/pending split, and an
  approved row is read from `mm0/approved/<stem>.stmts` instead of a staging. So the
  hex strings quoted here are produced by a run, not copied from the previous
  one: add or remove a statement, or let an emitter drift, and that line turns
  red before an owner is asked to approve the changed bytes. Two controls keep
  the comparison honest: one statement is altered in a staged copy and
  `mm0-comparator check` must refuse it (`statements differ from the approved
  file`), then only the quoted digest is altered and it must refuse that too
  (`does not hash it`) — the exact shape of a cell that has drifted from the
  bytes it claims to pin.

This is also what the distributed plan's M4/D24 hangs off: it says "preserve
existing W3 pending counts and approved statements; do not mark approval as
implementation completion", i.e. a deployment claim that cites these artifacts is
refused until the owner commits the four files, and citing them after approval
still carries the `[gap]` where the plan says `[gap]`.

**Staging, so the diff is reviewable without approving anything.** The four
files are produced by `scripts/stage-approvals`, which writes them under a
scratch root (`--root`, default `$TMPDIR/australvm-approve-staging`) and prints
exactly the diff an owner would be committing — against nothing for a first
approval, against the approved copy for a re-approval (`--approved-root`,
default `mm0/approved`, read-only and never written):

```
$ bash scripts/stage-approvals --stat-only
== isa
   193 statement(s), staged at /tmp/australvm-approve-staging/isa.stmts
   closure-sha256: 27de0884e3408cd45800b07dd118f0fb2cd977ed15e868843c626dbeb32d0aed
   statements-sha256: 2619b5ec9337db280718eb78055f8df76f7a700ae80a05b7fc6bd241db01359a
   212 line(s) would be added (no approved copy to diff against)
… slot_layout 35, store_projection 21, store_projection_toy 147 …
stage-approvals: 4 staged, 0 refused, 0 nothing to approve, 0 already approved; scratch root /tmp/australvm-approve-staging
stage-approvals: mm0/approved/ was not touched — the diff above is the decision
```

Without `--stat-only` it prints the full `git diff --no-index` for each artifact
(423 added lines across the four, measured 2026-11-16), which is what "the diff
is the human decision" means in the comparator's own header: the names in the
list above are gone from the file, so the only thing readable in the diff is the
*shape* of each statement plus the digests. `mm0/approved/` is never written by
this script, and `verify-invariants` keeps reporting the four `[gap] W3` lines,
so staging cannot be mistaken for approval.

**Its own exit codes are gated, because a staging run that "worked" is not
self-evident.** Three cases decide whether a person reading the output is being
told the truth: a name that is not `mm0/<stem>.mm0` must be a usage error
(**64**) rather than the comparator's `refused <name>` line — that line is the
message for a genuinely stale artifact, so a typo used to read like one — the
rows §W3 calls pending must actually stage (**0**, four `== <artifact>` headings,
`4 staged, 0 refused, 0 nothing to approve, 0 already approved`), an artifact
whose staged copy is byte-identical to its approved copy must be **refused as
nothing to approve** rather than printing an empty diff under a heading, which
reads like an approval of nothing, and an artifact *named by hand* that is
already approved must be **refused before anything is staged** (1): the diff it
would print is a re-approval, and an owner who half-remembered the state would
read that as a first approval. Re-approval is therefore opt-in with
`--reapprove` — the no-argument form never needs the flag, since it derives its
artifacts from the approved root and so never reaches that case. Measured
2026-11-16 and 2026-11-17: `nope` → 64, `--bogus` → 64, no arguments → 0 with
four headings, `store_projection_toy` named against an approved-root copy of
itself → 1 with `store_projection_toy is approved — pass --reapprove to stage a
re-approval diff` and no staging line, the same artifact with `--reapprove` and
an identical copy → 1 with `nothing to approve: byte-identical to …`. The gate
pins all four as `[pass] W3: stage-approvals answers an unknown artifact name
with exit 64`, `… stages exactly §W3's 4 pending row(s) (exit 0)`, `… refuses a
named artifact that is already approved` and `… refuses an artifact that equals
its approved copy`, so the script cannot quietly lose its usage error, stage a
re-approval under a heading that reads like a first one, or start calling a no-op
a success. The named-approved control deliberately hands the script a copy that
is *not* byte-identical, so its refusal cannot be the byte-identity check in
disguise.

**The pending set is derived, not listed twice.** `scripts/stage-approvals` has
`artifacts_all=(isa slot_layout store_projection store_projection_toy)` and no
`pending=` array: the no-argument run stages an artifact only when
`--approved-root` holds no `<stem>.stmts` for it, so a committed approval is
enough to stop staging it. The gate holds the derivation to §W3 from both sides
— `[pass] W3: stage-approvals' artifact list is the §W3 table's rows (4
name(s))` (an artifact named in one and not the other is reported with which
side is missing it) and `… stages exactly §W3's 4 pending row(s) (exit 0)`
(every pending row must produce a `== <artifact>` heading, an approved row must
not, and the summary must report that count with `0 refused`). A third control
runs the no-argument form against a scratch approved root holding one artifact
and requires the others to stage with `1 already approved;` in the summary —
`[pass] W3: stage-approvals stops staging an artifact once its approved copy
exists`. Together those are what make an approval one edit: flip the state cell,
and the staging set, the counts it prints and the gate's three lines move with
it. Measured 2026-11-17 by flipping `isa` to approved against a scratch root:
`(4 row(s), 1 approved)`, `stages exactly §W3's 3 pending row(s)`, summary `3
staged, 0 refused, 0 nothing to approve, 1 already approved`, 140 passed and one
fewer explained gap — because the comparator's `check` and this table now read the
same root (`AUSTRALVM_APPROVED_ROOT` moves the default for both, and the gate
passes `--approved-root` to `check` too), so the approved row is reported as
`[pass] W3: isa matches its approved statements` instead of a second gap. A
scratch root is then a *complete* view of "approved" for that run, which is what
makes the two sides comparable at all: an override that held only `isa.stmts`
would make every other approved artifact look pending, and did, before `check`
was brought onto the same root.

**What the `store_projection_toy` approval must contain.** Written down because
"approve the toy" is otherwise a judgement about a file nobody can read: the
staged artifact is 70 canonical statements with every name erased, so a copy
that differs by one axiom's position is a different axiom set with the same
count. The commitment is one file, `mm0/approved/store_projection_toy.stmts`,
and its content is exactly what

```
$ bash scripts/stage-approvals store_projection_toy --root /tmp/approve-toy
== store_projection_toy
   70 statement(s), staged at /tmp/approve-toy/store_projection_toy.stmts
   closure-sha256: 09842af8888899541cef6b58221d346616fe64310cc6b102506f512f81cedcbf
   statements-sha256: abed1393972165e893db24bc34325017ae4f7b94be3284f8f4e33415369a220e
   147 line(s) would be added (no approved copy to diff against)
```

writes — 148 lines, header `-- mm0-comparator v1 — approved statements for
store_projection_toy`, the two digests above, then the 70 statements. The
digests are the whole check: `abed1393…` is §W3's seventh cell and the value the
gate re-derives on every run, so a copy that is not byte-identical to this
staging fails `scripts/mm0-comparator check` or the digest comparison, whichever
it reaches first. `isa`, `slot_layout` and `store_projection` follow the same
shape with the digests in the table above.

What the commit has to be, and what changes with it:

- **The commit touches `mm0/approved/store_projection_toy.stmts` and nothing
  else** — `git show --stat HEAD` must list one path. Copying the staged file in
  is the approval; regenerating it inside the commit would make the diff the
  emitter's opinion rather than the owner's decision.
- **The row stays; its state cell flips.** Change
  `` `[gap]`, awaiting owner commit `` to `` `[pass]`, approved in `mm0/approved/` ``
  in §W3. That one edit is the whole of it: the `stmts` and digest cells are then
  compared with `mm0/approved/store_projection_toy.stmts` (they already hold those
  numbers), `w3-pending:` loses `store_projection_toy(70 stmt)`, the
  `w3-approved:` report gains it, the five `[gap] W3: … has no approved
  statements` lines in §3.5 become four, and the `row(s)` counts go on saying
  five with `1 approved`. `scripts/stage-approvals` stops staging it on its own,
  and no list anywhere else has to shrink — if one is left un-flipped (the copy
  committed, the cell still `[gap]`) the gate names the row and says `flip this
  row's state cell`; if the cell says approved and the copy is missing or holds a
  different number of statements, it names the copy.
- **§W9.3's composition table moves with it**, keeping `09842af8…` and
  `abed1393…` as the record of what was approved; the counts stay gate-pinned
  through the `w3-approved:` line once the row is gone.
- **Nothing about the claim changes.** The approval holds out the toy's 70
  statements, so the gate now refuses a statement, closure or theory mismatch
  against that copy. It does not make `store_projection`'s projection claim
  proved (that file still has no axioms and no theorems), it does not touch
  `isa`, and it does not turn the toy's 28 theorems into statements about the
  JIT — they are about a three-cell model whose 20 axioms `scripts/toy-witness`
  executes over a five-atom domain and `mm0/toy_witness/ToyWitness.lean` proves
  over an arbitrary atom type.

### W9.4 What must not happen

- An ISA model authored here and called "the ISA" (W8's own warning, now a
  non-goal rather than a caveat).
- A CLIF model that restates the CPS certificates in different words — that
  doubles the trusted base instead of reusing it.
- A refinement proof that assumes the lowering it is proving: the lowering's
  output must be the thing measured (`scripts/clif2mm0`), and any assumption
  about it must appear as a pinned, content-checked hypothesis.

---

## §5 Sprint order and exit criteria

| sprint | workstreams | exit criterion (a command, not a feeling) |
|---|---|---|
| S1 | W0, W3 | `scripts/mm0-comparator` rejects all three negative controls; `lean2mm0 check` byte-identical — **met 2026-11-12** |
| S2 | W1 | `examples/verified-kernel/flux1d` builds and runs — **met 2026-11-12**, and re-met 2026-11-13 through `--use-cps-jit`; `make test` — **met 2026-11-13** (161 tests, 18 suites, 0 failures on a clean `_build`); the obligation-carrier half is unblocked but unstarted, and the `netcert` acceptance is reassigned to W2 because `netcert` does not normalize arithmetic (W1, item 3) |
| S3 | W2 | `mm0/flux1d_certificate` passes `mm0-rs compile` + `mm0-c`; contains a free variable; budgets pinned — **met 2026-11-12**, and re-met 2026-11-13 with 4 obligations (`structural=6 grounded=15 certificates=4 assumed=1`); approval pending |
| S4 | W2 (mutation), W4 | `au2mm0 selftest` trips both anchors; `assumed` pinned and gated — **met 2026-11-13** (4 anchors; `assumed=1` from `uniform_state`, witness `l=1 r=1`) |
| S5 | W5, W6 | `code=` and `float-gap:` lines emitted and gated; claim-string check green — **met 2026-11-13**; both fingerprints **checked 2026-11-13** through the bridge (`97 passed`): `cps=cd65d0b4…` and `obj=ceed355c… arch=aarch64`, the latter gated only on a matching arch (a mismatch is a `[gap]`); the drift control (`scripts/obj-drift-control`) moves `obj=` under a semantics-preserving edit and the gate refuses it; probe measured `max relative deviation 0.000e+00`. **Re-pinned 2026-11-14** after the `sub`/`mul` lowering change: `cps=5dfcdfe8…`, `obj=ad3340b6… arch=aarch64`, statements unchanged (§3.5).**Re-pinned 2026-11-19** by the branch this was merged from, from an aarch64
host: `cps=c10ca9a9…` (which is what this tree's compiler produces — the cps
digest check is `[pass]` here), `obj=fac893da… arch=aarch64`, statements
unchanged, so the object-digest comparison is the documented arch-mismatch
`[gap]` on an x86_64 host |
| S6 | W7 | CNL corpus answers kernel-content questions with the expected verdicts, including one honest `unproved:` — **met for content 2026-11-13** (`theory: kernel` in `unfer/logos/mm0/claims.txt`: four citations + one `unproved:`); a question-answering layer over the corpus is not built, by the section's own warning |
| S7 | — | scoreboard §1 re-measured end to end; this document updated with numbers — **met for W0–W7 2026-11-13 (§3.3)** |
| S9 | W9 | an external ISA model converts and `mm0-c` accepts it; `mm0/clif.mm0` is emitted from the bridge's own CLIF; the CLIF→ISA refinement is accepted with content-checked hypotheses — **W9.2 met 2026-11-14**: `mm0/clif.mm0` is emitted by `scripts/clif2mm0` from `clif_dump`'s measurement of the 12-module corpus (**20 forms, 811 instructions**, re-measured after the arithmetic lowering, again after the `not` fix, and again when the CLIF counters moved in 2026-11-19 — the opcode census did not move; 23 of 23 CPS opcodes emitted, `UNREACHABLE_OPCODES` empty), accepted by `mm0-rs` + `mm0-c`, drift-gated with a negative control, and pinned to **0 theorems** because it models shape, not semantics; **W9.1 met 2026-11-14**: two pinned models convert and `mm0-c` accepts both — opencompl/riscv-lean at `ccdfd676` (83 `RV64.*` declarations → `mm0/statements/riscv_instructions.*`) and the Sail-generated `Lean_RV64D` at `11d8fa2` + `lean-sail` at `0794631` (`execute_LOAD`/`execute_STORE` → `mm0/statements/rv64_memory.*`, built by `packages.sail-riscv-lean` in ~28 min); the `ISA_REGISTRY` diff is closed in both directions (**15 mapped, 8 structural, 0 uncovered**), the table is checked against the artifacts it names, `check` re-converts both stems, `verify-invariants` runs it in the `lean` shell while reporting an explained `[gap]` elsewhere, and `.github/workflows/w9-isa-model.yml` runs it weekly (the memory stem's build is 28 min, so it is not a PR gate); the refinement's most tempting shortcut is **refuted and gated**: `scripts/mem-refines` shows the emitted `load`/`store` (1 operand/1 result, 2 operands/0 results) cannot be `execute_LOAD`/`execute_STORE` (5 and 4 arguments, returning `SailM ExecutionResult`, able to fault), and the surviving projection claim is stated with pinned citations in `mm0/store_projection.mm0` for both opcodes (`3 sorts, 8 term/def, 0 ax/thm`, no axiom, no theorem, `[gap]`) so only that weaker claim is left open, and its *shape* is proved on a toy model (`mm0/store_projection_toy.mm0` + `.mm1`: a three-cell memory whose capacity is characterised by four cases — refresh, free cell, evicting, keeping — 28 theorems from 20 cited axioms, at two widths, including a refresh keeping the other cell and its held-set proof, three distinct stores all readable and the fourth dropping the earliest, the fifth dropping the next, with an informal list model for consistency *that `scripts/toy-witness` executes* (every axiom checked over all three-cell memories on a four-atom domain — four because the eviction axiom needs four distinct addresses — and the six rejected axiom candidates each shown to fail), no `sorry`, gated to cite the current claim by digest — with a control that renames the claim and requires the pin to refuse — and to reuse its four transformer names, and mutation-tested in thirteen ways so no theorem survives its own axiom being falsified); and W9.3 is *stated* without being proved: `mm0/clif.mm0` declares `clif_step` as vocabulary (no axiom, no theorem), writes the obligation with five pinned citations, and carries `-- certificate: [gap]` until the owner's `isa` approval exists — a state `verify-invariants` mutation-tests; the direction and its non-goals are recorded in §4b |
| S8 | W8 | `mm0/isa.mm0` compiles and `mm0-c` accepts it, `scripts/isa-census` reports zero uncovered opcodes, and an uncited gap-hypothesis is refused — **met 2026-11-13** (`97 passed, 0 failed`; the corpus census reports 768 opcode reads over 9 modules — including a `uk_version` call and a module that derefs and stores — 11 distinct used, 23 declared, `result: covered`; all four controls trip). **Re-measured 2026-11-14**: 905 reads over 12 modules, **23 distinct**, 23 declared, `UNREACHABLE_OPCODES` now empty (the `sub`/`mul`/`match` producers landed). **Stronger than the criterion, twice:** `encdec` is no longer an axiom (23 theorems from a 23-entry table that `lib/Cps_codec.ml`/`test/CpsCodecTest.ml` and the census both check), and the three `step_refines` axioms are now theorems too. The last hypothesis now has a vocabulary: `mm0/slot_layout.mm0` defines `in_bounds` as slot-list membership and *proves* `in_bounds_alloc` (allocation never drops a slot), so `bounds_from_layout` is `content=checked` and what remains is one sentence about the compiler's layout pass |

S1 before S2 is deliberate: the comparator must exist before the first artifact
worth comparing is produced, otherwise we will retro-fit approval onto generated
statements and defeat the purpose.

---

## §6 Non-goals (recorded so they are not re-litigated)

- **GPU kernels.** No PTX/SPIR-V/OpenCL path exists in `lib/` and Cranelift has
  no GPU target. The honest prerequisite is CPS IR → portable C (already have
  `--target-type=c`); a GPU backend is a separate plan with its own verification
  story, not a slide in this one.
- **Replacing Lean as the source of truth.** `nanoda` re-checking of exports stays.
  W2 replaces *tactics*, not the kernel verdict.
- **Convergence/full CIC.** `cic.mm0` is an agent signature, not a dependent type
  theory. The `tsubst`-calculus question from `docs/lean4mm0.md` §7 stays open and
  is not on this path.
- **Agent swarms.** One deterministic emitter that refuses loudly beats forty
  workers that converge.
- **PDE well-posedness, CFL, divergence-free preservation.** Same boundary Lanyon
  draws; state it in the artifact rather than implying it.
- **Verifying "the ISA".** W8 models the instruction subset the emitter actually
  produces, measured by a census. A full-ISA semantics is a different project with
  a different trust story.

---

## §7 Measurement and honesty discipline

- Every status line added by this plan is produced by a command in §5; a number
  without a command is deleted, not softened.
- `[gap]` is the only way to say "could not run" — the `scripts/verify-invariants`
  convention, already used for missing `python3`, `why3`, and `mm0-rs`.
- No `TODO`/`FIXME` in any code file (`.rs`/`.sh`/`.py`/`.toml`) unless rowed in
  `docs/TODO_LEDGER.md`; work items live here and in plan §5, not in comments.
- Attack the anchors first: for each of W2/W3, write the mutation and watch it
  fire *before* the happy path is wired into CI, or the cross-check will be a
  decoration (the lesson of `docs/lean4mm0.md` §9.3).

---

## §8 Trust ledger

| item | trusted before | trusted after | note |
|---|---|---|---|
| `mm0-c` | yes | yes | unchanged, ~frozen |
| theory lines (`cic.mm0` + …) | 484 | 484 + 164 (`kernel_alg.mm0`) | pinned by `BUDGETS` |
| `int_ring_transfer` | — | **yes** | one rule, one review, hash-pinned |
| generated certificates | no | no | re-checked, never trusted |
| `mm0/approved/*.stmts` | — | **yes** | this is the statement of record |
| OCaml compiler + bridge + Cranelift | yes | yes | W5 records, does not remove |
| `mm0/isa.mm0` (W8) | — | **yes, when it exists** | definition-only semantics is trusted by construction; the proofs over it are not |
| reflected polynomial reflection | — | **yes** | a reflector bug is a soundness bug; this is why the dual-path check is mandatory and why the reflector is ≤ 500 lines |

The last row is new risk and must be stated in the README: a wrong reflection is
a wrong theorem, and the only defence is the independent evaluator, which is a
*different* implementation of the same idea, not a re-run of the same code.

---

## §9 Decisions that need the owner

1. **Spec carrier:** compiler-preserved annotations (W1 option A) vs sidecar
   `.auspec` (option B). A gives tighter coupling and a mutation test that bites;
   B avoids touching CPS IR. Recommend A with B as fallback.
2. **Fingerprint checked vs recorded** (W5) — **settled 2026-11-13, without an
   owner decision.** Checked turned out not to require a `uk_*` symbol at all: a
   plain FFI export (`austral_cps_fingerprint`, `austral_object_fingerprint`)
   is not callable from compiled Austral code, so there is no manifest grant and
   no S29 checklist. Both digests are checked; what the owner still owns is the
   `arch=` policy below.
2b. **Arch policy for the object digest.** The capture is host-native, so
   `obj=` is comparable only on the recorded architecture; the gate reports a
   `[gap]` on a different host rather than passing or failing. Alternative:
   record one digest per target triple (needs a cross-compile path the bridge
   does not have). Recommend keeping the single arch-named digest.
3. **Exact fragment width.** W2 starts at `Int64`/`Nat64`. Widening to a fixed-point
   or rational model is what makes the certificates cover real numerics; it is a
   separate theory change and should not be smuggled into S3.
4. **Where the flux example lives:** `examples/verified-kernel/` (new tree) vs
   extending `examples/kernel/UnferKernel`. New tree keeps the `uk_*` demo intact.
5. **W8 ISA source** — **taken 2026-11-13: authored**, for the 23 emitted
   opcodes. Translate a Sail model (`sail2mm0`, translator in the TCB) vs
   author an RV64I integer subset from the manual (more review, no translator).
   Recommend authoring the subset first — it is what the census will show we emit —
   and treating a Sail translation as the follow-up that widens it.
6. **W8 conditioning** — **taken 2026-11-13: conditioned, pinned.** The ISA
   axioms take our linearity/layout certificates as hypotheses, so they inherit
   their bugs; each cite carries a sha256 and `scripts/isa-hypotheses` refuses a
   stale one. The alternative is an independent proof at ISA level, which
   duplicates the guarantee in a weaker fragment. **Answered 2026-11-14:** the
   encoder exists (`lib/Cps_codec.ml`, the table the emitter writes through, with
   the round trip proved for all 23 entries) and the step relation is now proved
   rather than assumed (`add_refines`/`mul_refines`/`sub_refines` in
   `mm0/isa.mm1`). One citation remains, `bounds_from_layout`, which is a layout
   fact and not a step fact.

---

## §10 Checklist for the next agent

1. Read `docs/lean4mm0.md` §7, §9, §11, §12 and `mm0/pairing/README.md` first;
   this plan assumes them.
2. `make bridge` (root `libaustral_cranelift_bridge.so` staleness — AGENTS.md),
   then `make all`, then `nix develop` for every `dune` command.
3. `nix-shell -p python3 --run "bash scripts/verify-invariants"` — record the
   baseline pass/gap/fail counts in this file before changing anything.
4. S1 (W0 + W3) has landed; start at W1. Do not start W2 before W3's negative
   controls pass — they do, so the ordering constraint is satisfied.
5. ~~Approve the seven existing artifacts~~ — done (`1e9e74a4`, `mm0/approved/`
   alone; 75 passed / 0 gaps). The next approval is W2's
   `mm0/flux1d_certificate`, which must arrive unapproved so the diff is
   reviewable.
6. Update §3 and §5 with measured numbers as each sprint closes; delete any claim
   this document makes that the tree stops supporting.
