# Formalization Plan: `arctic_authority` in Lean 4 via Aeneas

**Executor:** leanstral (`vibe --agent lean`), driven phase by phase.
**Goal:** machine-checked Lean 4 proofs that the `arctic_authority` authorization-decision
logic is correct, obtained by translating the Rust code to pure Lean functions with
Aeneas/Charon, plus a hand-written Lean model of the mathematics from the Arctic paper
(`../arctic.md`) that the code-level theorems are connected to.

All paths below are relative to `/media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/australVM/arctic_authority`
unless absolute. Sibling repos:

- Aeneas: `/media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/aeneas` (present, **not yet built**; `charon/` not yet cloned; pinned Charon commit in `charon-pin` = `909ff09ad0f1…`)
- Arctic protocol crate (dependency `arctic`): `/media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/dynamic-arctic`
- Paper: `/media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/australVM/arctic.md`

Already installed on this machine: `opam` (/usr/bin/opam), `rustup`, `cargo`, `nix`,
`elan` + `lake` (`/media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/.elan/bin`).
Aeneas' Lean backend pins **Lean `leanprover/lean4:v4.31.0`** (see
`aeneas/backends/lean/lean-toolchain`); elan will fetch it automatically on first `lake build`.

---

## 0. Scope and verification strategy

`arctic_authority/src/lib.rs` (278 lines) is a thin authorization-decision layer:

| Item | Role |
|---|---|
| `AuthzResult` | `Allow`/`Deny` |
| `group_pk_from_bytes` | decompress 32-byte Ristretto point |
| `ArcticAuthEngine::new` | holds group pk + `HashMap<String, DelegationCertificate>` |
| `register_certificate` | verify threshold Schnorr sig over `serde_json::to_vec(&cert)`, then insert |
| `revoke` | remove by delegatee pk |
| `check(principal, action, resource)` | Deny unless action=="Call", cert registered, unexpired (`SystemTime::now`), and capability ∈ cert or `"*"` |

**What we formalize with Aeneas (code level):** the decision logic — lookup, expiry,
capability matching, registry state transitions. This is the part where a bug silently
grants kernel capabilities, and it is pure logic, ideal for Aeneas.

**What we formalize by hand in Lean/Mathlib (paper level):** the *correctness* (not the
ROM security proofs) of the mathematics in `arctic.md` §3–5: Lagrange interpolation,
Schnorr verification relation, VPSS₁ (Gen/Verify/Agg/Recover), and Arctic's
sign₁/sign₂/combine producing signatures that satisfy Schnorr.Verify. Plus one genuinely
information-theoretic security lemma (VPSS₁ uniqueness under honest majority, which needs
no crypto assumptions — only "a degree ≤ t−1 polynomial is determined by t points").

**What is explicitly out of scope (trusted / axiomatized), documented in `TCB.md`:**

1. `arctic_core::verify` (curve25519-dalek Ristretto arithmetic) — axiomatized as an
   abstract predicate `sigOk : PubKey → ByteList → Signature → Bool`, *linked* to the
   paper-level Schnorr relation by one axiom.
2. `serde_json::to_vec` — axiomatized as an **injective** encoding
   `encodeCert : Cert → ByteList` (injectivity is the actual security-relevant property:
   two distinct certificates must not share a signed byte string; serde_json on a struct
   with fixed field order satisfies it).
3. Unforgeability of Arctic / pseudorandomness of VPSS₁ (DL + ROM, forking lemma) —
   stated as named assumptions, not proved. (Stretch goal, Phase 6.)
4. Charon, Aeneas, the Lean kernel, and the Rust compiler.

**Key strategy decision — extract a pure core crate, not `arctic_authority` itself.**
Charon cannot realistically translate `curve25519-dalek`, `serde_json`, `std::HashMap`,
or `SystemTime::now()` (the last is impure and *must* not appear in extracted code).
So Phase 2 refactors the logic into a new zero-dependency crate
`arctic_authority_core` that `arctic_authority` then calls (behavior unchanged, existing
8 tests must still pass), and Charon/Aeneas run on the core crate only. This is the
standard Aeneas workflow and the highest-probability path.

---

## Phase 1 — Install and build the toolchain

Acceptance: `aeneas` binary exists, translates its own Lean tutorial test, and the
Aeneas Lean library builds with `lake`.

### 1.1 OCaml switch + deps (per `aeneas/README.md`)

```bash
cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/aeneas
opam switch create 5.3.0 || opam switch 5.3.0   # idempotent
eval $(opam env --switch=5.3.0)
opam install -y calendar core_unix domainslib easy_logging menhir \
  ocamlformat.0.27.0 ocamlgraph odoc ppx_deriving ppx_deriving_yojson \
  progress unionFind visitors yojson zarith
```

Fallback if opam packages fight the system OCaml: use the Nix flake instead
(`nix develop` inside `aeneas/` provides OCaml, Rust and Charon pinned; nix is installed).

### 1.2 Charon (pinned) + Aeneas build

```bash
cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/aeneas
make setup-charon    # clones ./charon at the commit in ./charon-pin and builds it
                     # (uses rustup or nix, both installed; installs the pinned
                     #  nightly Rust toolchain — needs network)
make                 # builds aeneas (dune)
ls bin/              # expect: aeneas (plus charon symlinks under ./charon/bin)
```

Sanity check the pipeline end-to-end on Aeneas' own tutorial before touching our code:

```bash
cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/aeneas
make test-tutorial 2>/dev/null || (cd tests/lean && lake build Tutorial)
```

### 1.3 Lean side

```bash
elan toolchain install leanprover/lean4:v4.31.0
cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/aeneas/backends/lean
lake exe cache get   # pulls the Mathlib olean cache Aeneas' library depends on — large download
lake build           # must succeed; this is the `Aeneas` Lean library our proofs import
```

**Record in the work log:** exact `charon --version`, `aeneas` commit, Lean toolchain,
so extraction is reproducible.

---

## Phase 2 — Refactor Rust into an extractable pure core

Acceptance: `cargo test` in `arctic_authority` still passes all 8 tests; new crate
`arctic_authority_core` builds with **zero external dependencies** and `#![no_std]`-style
purity (no clocks, no I/O, no HashMap).

### 2.1 New crate `arctic_authority_core`

Create `arctic_authority/core/` (workspace member or path dep) with `src/lib.rs`
containing *only* data types + pure functions. Proposed shape (leanstral may adjust
names, not semantics):

```rust
// No deps. No std collections beyond Vec. No String — use Vec<u8> so Aeneas'
// standard library support (Vec, integers) covers everything.

pub enum AuthzResult { Allow, Deny }

pub struct CertData {
    pub delegatee_pk: Vec<u8>,
    pub expires_at: u64,
    pub capabilities: Vec<Capability>,
}

pub enum Capability { Wildcard, Named(Vec<u8>) }

/// The registry as an association list (HashMap is not extractable;
/// an assoc list with last-insert-wins or unique-key invariant is).
pub struct Registry { pub certs: Vec<CertData> }

/// Pure decision function. `now` passed explicitly (was SystemTime::now()).
/// `action_is_call` passed explicitly (was `action == "Call"` on &str).
pub fn check_at(reg: &Registry, principal: &[u8], action_is_call: bool,
                resource: &[u8], now: u64) -> AuthzResult;

/// Pure registration: `sig_valid` is the *result* of arctic_core::verify,
/// computed by the impure shell. Returns Some(new registry) iff sig_valid.
pub fn register(reg: &Registry, cert: CertData, sig_valid: bool) -> Option<Registry>;

pub fn revoke(reg: &Registry, delegatee_pk: &[u8]) -> Registry;
```

Semantics must be *exactly* those of today's `lib.rs`:
deny unless call-action; lookup by `delegatee_pk`; deny if `now >= expires_at`
(note: `>=`, expiry instant itself is expired); allow iff some capability is `Wildcard`
or `Named(resource)`; `register` replaces an existing entry for the same key
(HashMap::insert semantics); `revoke` removes it.

### 2.2 Rewire `arctic_authority/src/lib.rs` as a thin impure shell

`ArcticAuthEngine` keeps its public API and its `HashMap`, **or** (preferred, smaller
gap) switches its internal storage to `core::Registry` and only does at the boundary:
signature verification (`arctic_core::verify` on `serde_json::to_vec(&cert)`),
`SystemTime::now()`, `String` ⇄ `Vec<u8>`/`Capability` conversion (`"*"` ⇔ `Wildcard`).
Run `cargo test` — all 8 existing tests green, unmodified except for internal plumbing.
The Rust-side gap that remains (shell faithfully forwards to core) is small enough to
audit by eye and is covered by the existing tests; note it in `TCB.md`.

### 2.3 Charon annotations

If anything non-extractable leaks in, mark it opaque with Charon attributes
(`#[charon::opaque]`) rather than growing the TCB silently — but the goal is that the
core crate needs **no** opaque items.

---

## Phase 3 — Extract to Lean with Charon + Aeneas

Acceptance: generated Lean files compile inside the proof package.

```bash
export AENEAS=/media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/aeneas
cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/australVM/arctic_authority/core
$AENEAS/charon/bin/charon cargo --preset=aeneas   # produces arctic_authority_core.llbc
$AENEAS/bin/aeneas -backend lean arctic_authority_core.llbc \
    -dest ../verify/ArcticAuthority/Extracted
```

(Exact flags drift between Aeneas versions — if these fail, consult
`$AENEAS/charon/bin/charon --help`, `$AENEAS/bin/aeneas --help`, and imitate
`$AENEAS/Makefile` test targets / `$AENEAS/tests/lean/Tutorial`.)

Create the Lean proof package `arctic_authority/verify/`:

```
verify/
  lakefile.lean          -- package ArcticAuthority; requires the Aeneas library by path:
                         --   require aeneas from "../../../aeneas/backends/lean"
  lean-toolchain         -- leanprover/lean4:v4.31.0 (copy from Aeneas backend)
  ArcticAuthority/
    Extracted/…          -- GENERATED by aeneas, never hand-edited
    Model/               -- Phase 5: paper mathematics
    Specs.lean           -- Phase 4: spec predicates + theorem statements
    Proofs/              -- Phase 4/5 proofs
    TCB.md               -- running list of axioms/assumptions with justification
```

Extraction is re-runnable: add `make extract` at repo top of `arctic_authority/` doing
the two commands above, so Rust changes propagate mechanically.

---

## Phase 4 — Code-level theorems (the core deliverable)

Prove in `Proofs/`, about the **extracted** functions (Aeneas emits monadic Lean;
use the Aeneas `progress` tactic and its std-lib lemmas — see
`$AENEAS/tests/lean/Tutorial` for the idioms). All theorems `sorry`-free.

Registry well-formedness invariant `WF reg`: keys (`delegatee_pk`) pairwise distinct.

- **T1 (deny by default).** `lookup reg p = none → check_at reg p a r now = Deny`.
- **T2 (action gate).** `¬ action_is_call → check_at … = Deny`.
- **T3 (expiry).** `lookup reg p = some c → now ≥ c.expires_at → check = Deny`.
- **T4 (soundness of Allow).** `check_at reg p true r now = Allow →
  ∃ c, lookup reg p = some c ∧ now < c.expires_at ∧
  (Capability.Named r ∈ c.capabilities ∨ Capability.Wildcard ∈ c.capabilities)`.
- **T5 (completeness).** Converse of T4.
- **T6 (register).** `register reg c true = some reg' → lookup reg' c.delegatee_pk = some c
  ∧ ∀ p ≠ c.delegatee_pk, lookup reg' p = lookup reg p ∧ WF reg → WF reg'`;
  and `register reg c false = none`.
- **T7 (revoke).** `lookup (revoke reg p) p = none` and other keys untouched; preserves `WF`.
- **T8 (authority invariant, the headline theorem).** Define the reachable-state
  predicate: every registry obtained from `∅` by a sequence of
  `register (sig_valid := sigOk gpk (encodeCert c) s)` / `revoke` steps.
  Then: **if `check_at reg p true r now = Allow`, there exists a certificate `c` and
  signature `s` with `sigOk gpk (encodeCert c) s = true`, `c.delegatee_pk = p`,
  `now < c.expires_at`, and `r` granted by `c`** — i.e. *every Allow is backed by a
  threshold-signed, unexpired, capability-granting certificate*. (`sigOk` and
  `encodeCert` are the Phase-0 axioms; injectivity of `encodeCert` is what lets T8 say
  the *registered* cert is the *signed* cert.)

Each theorem = one commit. After each, `lake build` must be green.

---

## Phase 5 — Paper-level model (`Model/`, Mathlib)

Hand-written Lean over an abstract group, following `arctic.md` §3–5. Use Mathlib:
`CommGroup`/`ZMod q` (or an abstract `Field F` + `CommGroup G` with a `zpow` action),
`Lagrange.interpolate`, `Polynomial.degree`.

- **M1 Schnorr.** `SchnorrVerify (pk : G) (m : Msg) (σ : G × F) : Prop :=
  g ^ σ.2 = σ.1 * pk ^ (H₃ σ.1 pk m)` (hash as an abstract function parameter).
- **M2 Lagrange facts** (mostly Mathlib): interpolation, and Eq. 3 of the paper:
  `L'_{aᵢ}(j) = 0` for `j ∈ aᵢ`, `L'_{aᵢ}(0) = 1`.
- **M3 VPSS₁ definitions** (paper §4.3): `KeyGen` output shape (replicated shares over
  `A = [n].choose (t-1)`), `Gen`, `Verify` (Eq. 5–6: commitments-to-coefficients,
  last `|C| − t` must be `1`), `Agg`, `Recover`. Deterministic algorithms → plain
  Lean functions; `KeyGen` randomness → universally quantified share vector.
- **M4 VPSS₁ correctness** (paper Eq. 2 + "Correctness" paragraph): honest shares
  `dᵢ = f(i)` for `f = Σᵢ H(φᵢ,w)·L'_{aᵢ}`, `deg f ≤ t−1`; hence `Verify = 1`,
  `Agg = g^(f 0)`, `Recover = (f 0, g^(f 0))` for every coalition `|C| ≥ μ`.
- **M5 Arctic correctness** (paper §5, Fig. 6): an honest run of
  Sign₁/Sign₂/Combine over coalition `C`, `|C| ≥ 2t−1`, produces `σ` with
  `SchnorrVerify pk m σ`. This is the paper's correctness claim, and it is exactly the
  statement that justifies axiom `sigOk` ≈ M1 at the code level.
- **M6 VPSS₁ uniqueness, honest-majority (information-theoretic — no ROM needed).**
  If `μ ≥ 2t−1`, at most `t−1` corrupt, then any two coalitions whose commitment sets
  pass `Verify` and agree with honest parties' commitments yield the same `Agg` output:
  each coalition contains ≥ t honest points; a polynomial of degree ≤ t−1 "in the
  exponent" is determined by t points (injectivity of `fun x => g ^ x` for `g` a
  generator of prime order q). This is the one *security* property fully provable here.
- **Link axiom (one line, in TCB.md):** `sigOk gpk msg σ = true ↔ SchnorrVerify … `
  instantiated at the Ristretto group — trusted because curve arithmetic stayed in Rust.

Milestone order: M1, M2 first (cheap, mostly Mathlib), then M3–M4, then M6, then M5.
M5 requires transcribing Fig. 6 from `arctic.md` (read lines 547–761) and cross-checking
against `dynamic-arctic/src/arctic_core.rs` (`sign1`, `sign2`, `combine`, `verify`) —
where paper and code disagree, formalize the paper and file a note.

---

## Phase 6 — Stretch goals (only after Phases 1–5 are `sorry`-free)

- State (not prove) VPSS₁ verifiability & pseudorandomness and Arctic unforgeability
  as Lean `Prop`s with named hypotheses (DL, ROM), so `TCB.md` points at formal
  statements rather than prose.
- Extract `dynamic-arctic`'s `lagrange.rs` with Aeneas and prove it against M2
  (it is nearly dependency-free, so a realistic second extraction target).
- Fuzz/differential tests: `cargo test` comparing shell vs. core on random inputs.

---

## Execution protocol for leanstral

Run each phase as its own session, from the right working directory:

```bash
vibe --agent lean --workdir /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/australVM/arctic_authority \
  --add-dir /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/aeneas \
  --add-dir /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/dynamic-arctic \
  -p "Execute Phase N of FORMALIZATION_PLAN.md: <phase title>. Stop at the phase's acceptance criteria." \
  --max-turns 80
```

Rules for the agent:

1. **One phase per session; meet the acceptance criteria before moving on.**
   Re-read this file at session start; keep a running `verify/WORKLOG.md`
   (what was done, tool versions, deviations from plan and why).
2. **Never edit `Extracted/`** — fix the Rust and re-run `make extract` instead.
3. **`lake build` green after every commit; no `sorry`/`admit` in committed proofs**
   (a `Proofs/WIP.lean` excluded from the default target is allowed while iterating).
4. Every new axiom goes into `TCB.md` with a one-paragraph justification; if a proof
   seems to need a new axiom, stop and reconsider first.
5. When Aeneas/Charon reject something, prefer simplifying the Rust core (it exists to
   be extractable) over patching the toolchain; consult `$AENEAS/tests` for supported
   idioms and `$AENEAS/tests/lean/Tutorial` for proof patterns (`progress`, `scalar_tac`).
6. Cite the paper: each Model/ definition carries a doc-comment naming the
   `arctic.md` section/equation it transcribes.

---

# Coordination with `RandomMap2.md`

`FORMALIZATION_PLAN.md` and `RandomMap2.md` are **completely independent work
packages** — zero shared files, zero shared dependencies, zero overlapping
 deliverables. This section defines the guarantee so that different
LLM-Lean-specialists can execute them **in parallel without any risk of
duplicated work or file conflicts**.

### Separation guarantee

```
FORMALIZATION_PLAN.md                          RandomMap2.md
─────────────────────                          ────────────
australVM/arctic_authority/                    RiemannProof/
├── arctic_authority/src/lib.rs               ├── RiemannProof/SchoenfeldPRA.lean
├── arctic_authority/core/                     ├── RiemannProof/RandomMap2.lean
├── verify/                                     ├── RiemannProof.lean
│   ├── ArcticAuthority/                        └── RiemannProof/RandomMap2.md
│   │   ├── Extracted/  (generated by Aeneas)
│   │   ├── Model/
│   │   ├── Specs.lean
│   │   └── Proofs/
│   ├── TCB.md
│   └── lakefile.lean
└── FORMALIZATION_PLAN.md

Shared resources: NONE.
  - No file is imported by both packages.
  - No dependency is shared (Aeneas/Charon vs. Mathlib/Lp).
  - No symbol name collisions possible.
  - No directory overlap.
```

### What each specialist must NOT do

| Specialist | Must NOT touch | Must NOT create |
| :--- | :--- | :--- |
| **FORMALIZATION_PLAN** (arctic_authority) | `RiemannProof/` directory, `RandomMap2.lean`, `SchoenfeldPRA.lean` | Any file outside `australVM/arctic_authority/` and `aeneas/` |
| **RandomMap2** (decoupled framework) | `australVM/arctic_authority/` directory, `verify/`, `Extracted/`, `arctic_authority_core.llbc` | Any file outside `RiemannProof/` (except reading `FORMALIZATION_ROADMAP.md` for coordination) |

### Parallel execution protocol

When both specialists run simultaneously:

1. **Each specialist operates exclusively within its own directory tree.**
   FORMALIZATION_PLAN never writes to `RiemannProof/`. RandomMap2 never
   writes to `australVM/arctic_authority/`.
2. **Aeneas extraction output (`Extracted/`) is never hand-edited** —
   the FORMALIZATION_PLAN specialist re-runs `make extract` after Rust changes.
   RandomMap2 never touches this tree.
3. **`lake build` invocations are directory-scoped.**
   - FORMALIZATION_PLAN: `cd australVM/arctic_authority/verify && lake build`
   - RandomMap2: `cd RiemannProof && lake build`
   These are independent builds with no shared object files.
4. **No shared coordination file is needed.** The two plans have no
   dependencies on each other. If the author later decides to connect them
   (e.g. use the decoupled framework's `OuterWaveFunction` as the verification
   target for Aeneas-extracted code), that will be a new work package with
   its own specification.
5. **If a specialist encounters an error in shared infrastructure** (e.g.
   the Aeneas toolchain breaks), it reports the issue but does NOT modify
   the shared file — it belongs to the other specialist's track.

### Definition of done (both tracks)

| Track | Done when |
| :--- | :--- |
| **FORMALIZATION_PLAN** (arctic_authority) | Phases 1–5 complete, `lake build` green, all 8 tests pass, `TCB.md` up to date |
| **RandomMap2** (decoupled framework) | All 4 items PROVED (`outer_inner_reduces_to_head` compiles), `lake build` green, `#print axioms` confirms only `propext`/`Classical.choice`/`Quot.sound` |

### What a parallel pass looks like

Both specialists can run simultaneously with **zero coordination overhead**:

- **Specialist A** (FORMALIZATION_PLAN): Phase 1 (install toolchain) or Phase 2
  (refactor Rust core) or Phase 3 (extract with Aeneas). No blocked items.
- **Specialist B** (RandomMap2): R1 (fix `SchoenfeldPRA` exports) or R3
  (Phase 4 epistemological payoff). R2 (move `MeasurableSpace`/`BorelSpace`)
  should be deferred — it touches `SchoenfeldPRA.lean` which is in the
  FORMALIZATION_PLAN's dependency chain if Aeneas needs the Substrate type.

**Result: full parallelism, zero duplication risk, independent builds, independent verification.**
