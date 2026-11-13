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
| **code fingerprint in the certificate** | `-- code: cps=<sha256> obj=unrecorded target=host`, re-derived from `--emit-cps` bytes by `verify-invariants` | **exists, recorded not checked** (§9) |
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
-- code: cps=6299960658a4a79cbbe45e42a8b8ece931bc05c33f30113ef57e70dfcc95865a obj=unrecorded target=host

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

Two absences deserve emphasis because they are the ones that make the SOTA claim
currently indefensible:

1. **Every certificate we emit is about a ground computation.** `mm0/gap_certificate`
   is 9 structural / 0 grounded, but all nine are *closed*: they reduce to `ttrue`.
   Lanyon's cheapest theorems range over a state vector `U` with `U_L = U_R` as a
   hypothesis. We have never emitted a certificate with a free variable.
2. **`verify` regenerates the statement from the same export it verifies.** A
   weakened definition is reproduced faithfully by regeneration, so the drift gate
   cannot see it. This is exactly the hole `comparator` was built to close.

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
  `verify_pairing.py` still reports 15/15. All seven artifacts were approved
  and checked end to end during S1, then the approvals were removed: approval
  is an owner commit that touches only `mm0/approved/`, not something this
  plan writes for itself. An approval that pins a theory file's hash becomes a
  `[gap]`, not a refusal, on a machine where that file is not provisioned.

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
-- code: cps=62999606…95865a obj=unrecorded target=host
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

**Still open:** the object hash stays `unrecorded` — this path writes no object
file, and `code-status` says so rather than implying otherwise. Recording it
means taking the cranelift-emitted code bytes from the bridge (`au_*`), which is
honest work but not what unblocks W8: the ISA model consumes the CPS layer.

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

### W8 — An ISA model that only fills the gaps  *(nothing started; 1–2 weeks after W2/W3)*

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
| ISA = decode/step relation over registers and memory | `mm0/isa.mm0` | **no — this is the new artifact** |

The ISA model never restates a source-level guarantee. It states the two
transitions it alone can prove, taking the layer-1 certificates as **hypotheses**:

- `encdec` — for the emitted subset, `decode (encode i) = i`.
- `step_refines` — one ISA step (state → state′) implements one CPS-IR step,
  **assuming** the IR-level typing and linearity invariants that
  `linearity_certificate` / `layout_certificate` already establish.
- `bounds_from_layout` — the per-access memory obligations the step relation
  needs are discharged by citing the layout discipline's block bounds, not by
  re-proving arithmetic. This is precisely "the model fills in the gaps": each
  gap is a named hypothesis with a citation, and `verify-invariants` refuses an
  uncited one.

Consequences that must be stated and not softened: this makes the ISA model
*conditional* on our own proofs, so a bug in the linearity certificate would
propagate into the machine-code claim. That is the honest trade — the
alternative is two independent proofs of linearity, and the plan's §7 discipline
applies: the citation must be pinned (as in W7) so a drifted certificate cannot
be cited.

**Subset discipline (measured, not assumed).** The model covers exactly the
instructions the emitter produces for the certified fragment — the same mechanism
as `SYMBOL_REGISTRY`'s census, with an `ISA_REGISTRY` beside it:

```
austral compile examples/verified-kernel/flux1d/flux1d.aum --use-cps-jit \
  --emit-cps=/tmp/flux1d.cps        # the IR the model must cover
scripts/isa-census                 # [gap] does not exist: would diff emitted opcodes
                                   # against mm0/isa.mm0's declared instruction table
```

An uncovered opcode is a gate failure, not a footnote — the census is what stops
the model drifting into "the ISA" when the claim is "the bytes we emit".

**[gap] The model itself.** Neither repo ships one: `mm0/statements/` holds
`BookProof.*` only, and the mm0 project's own theories (Peano, set.mm) contain no
machine-code semantics, so this is authored or translated, not imported. The two
plausible sources, and what each costs:

- **Sail** (the maintained RISC-V/AArch64 models): a new emitter, `sail2mm0`,
  sibling to `lean2mm0`, and a translation whose fidelity is itself an unproved
  claim. Cheapest path to a *trusted-by-provenance* semantics.
- **Authored from the ISA manual** for a small subset (RV64I integer core), the
  Lanyon-style choice: fewer rules, each reviewable, no translator in the TCB.

Both are weeks of work and neither is on the W0–W7 critical path, which is why
this is last and unscored.

**Exit criteria (named, not met):** `mm0/isa.mm0` passes `mm0-rs compile` +
`mm0-c`; `scripts/isa-census` reports zero uncovered opcodes for the flux1d IR;
`verify-invariants` has a negative control where a hand-edited certificate
citation is refused. Until those three exist this row is unscored in §5 — a
number here would be a claim without a command.

---

## §5 Sprint order and exit criteria

| sprint | workstreams | exit criterion (a command, not a feeling) |
|---|---|---|
| S1 | W0, W3 | `scripts/mm0-comparator` rejects all three negative controls; `lean2mm0 check` byte-identical — **met 2026-11-12** |
| S2 | W1 | `examples/verified-kernel/flux1d` builds and runs — **met 2026-11-12**, and re-met 2026-11-13 through `--use-cps-jit`; `make test` — **met 2026-11-13** (161 tests, 18 suites, 0 failures on a clean `_build`); the obligation-carrier half is unblocked but unstarted, and the `netcert` acceptance is reassigned to W2 because `netcert` does not normalize arithmetic (W1, item 3) |
| S3 | W2 | `mm0/flux1d_certificate` passes `mm0-rs compile` + `mm0-c`; contains a free variable; budgets pinned — **met 2026-11-12**, and re-met 2026-11-13 with 4 obligations (`structural=6 grounded=15 certificates=4 assumed=1`); approval pending |
| S4 | W2 (mutation), W4 | `au2mm0 selftest` trips both anchors; `assumed` pinned and gated — **met 2026-11-13** (4 anchors; `assumed=1` from `uniform_state`, witness `l=1 r=1`) |
| S5 | W5, W6 | `code=` and `float-gap:` lines emitted and gated; claim-string check green — **met 2026-11-13**; fingerprint **checked 2026-11-13** through the bridge (`84 passed`), object hash still `unrecorded` and labelled so; probe measured `max relative deviation 0.000e+00` |
| S6 | W7 | CNL corpus answers kernel-content questions with the expected verdicts, including one honest `unproved:` — **met for content 2026-11-13** (`theory: kernel` in `unfer/logos/mm0/claims.txt`: four citations + one `unproved:`); a question-answering layer over the corpus is not built, by the section's own warning |
| S7 | — | scoreboard §1 re-measured end to end; this document updated with numbers — **met for W0–W7 2026-11-13 (§3.3)** |
| S8 | W8 | `mm0/isa.mm0` compiles and `mm0-c` accepts it, `scripts/isa-census` reports zero uncovered opcodes, and an uncited gap-hypothesis is refused — **not started**, and deliberately unscored: the artifact, the census script and the negative control do not exist, so there is nothing to measure |

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
2. **Fingerprint checked vs recorded** (W5). Checked costs a new S29-registered
   `uk_*` symbol; recorded costs nothing but weaker claim. Recommend recorded
   first.
3. **Exact fragment width.** W2 starts at `Int64`/`Nat64`. Widening to a fixed-point
   or rational model is what makes the certificates cover real numerics; it is a
   separate theory change and should not be smuggled into S3.
4. **Where the flux example lives:** `examples/verified-kernel/` (new tree) vs
   extending `examples/kernel/UnferKernel`. New tree keeps the `uk_*` demo intact.
5. **W8 ISA source:** translate a Sail model (`sail2mm0`, translator in the TCB) vs
   author an RV64I integer subset from the manual (more review, no translator).
   Recommend authoring the subset first — it is what the census will show we emit —
   and treating a Sail translation as the follow-up that widens it.
6. **W8 conditioning:** the ISA theorem takes our linearity/layout certificates as
   hypotheses, so it inherits their bugs. The alternative is an independent proof
   at ISA level, which duplicates the guarantee in a weaker fragment. Recommend the
   conditioned form with pinned citations.

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
