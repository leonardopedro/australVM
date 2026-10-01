# ENGRAM — content-addressed conditional memory for the Total Austral Subset

**Status:** normative for Workstream E (`PLAN_liquid_types.md` §9), stages E1–E5.
Pinned 2026-10-01 at E1.

This document is the spec that E2–E5 implement and the golden corpus in
`../corpus/engram_keys.tsv` tests against. Where it repeats the plan it is
deliberately identical; where the plan was silent it makes a decision, and each
decision is marked **[decision]**.

---

## 1. The idea, in one paragraph

DeepSeek's Engram paper (arXiv:2601.07372, reproduced in `../engram.md`) gives an
LLM a conditional memory: hashed suffix N-gram keys → O(1) static-embedding
lookup → context-aware gating → residual fusion. Workstream E keeps the paper's
lookup module and changes only the *keys*. Instead of surface N-grams, every text
fragment is compiled by logos' pipeline to an interaction net, reduced to its
**unique normal form**, and addressed by that normal form. The key is the
content, so paraphrases that denote the same thing collide in one table slot,
and the lookup stays O(1) and *deterministic* — computable at corpus ingest time,
before any decode step.

Nothing here replaces logos. E2 is a **consumer** of `unfer/logos`: it reuses
`harper_gate::lint`, `ccg::parse_sentence`, `core_ir::*`, `deltanet::compile_to_net`,
`reducer::reduce`, `deltanet::unf::{canonical_serialize, unf_hash}` and
`deltanet::ted`, and adds no new compiler, no new hash, and no second net
representation. **[decision]** The hash is the *existing* `deltanet::unf::unf_hash`
— already SHA-256 over the canonical serialization — not a new digest. logos
already depends on `sha2`, so E2 adds no dependency.

---

## 2. Keys

### 2.1 "Structural N-grams"

For decode position `t` and granularity `g`:

| `g` | segment | paper analogue |
|---|---------|----------------|
| `window` | token window of order `n = 2..N` (N = 3 default, paper §4.1) | `E_{n,k}` |
| `subderiv` | every CCG sub-derivation → own CoreIR → net → UNF | compositional n-gram |
| `sentence` | the whole fragment → reduced net → UNF | whole-pattern |

`window` is kept for two reasons, and both matter: it is the paper's baseline
so the E4 ablation has something to compare against, and it is the fallback when
the fragment cannot be parsed (see §4).

### 2.2 `EngramKey` byte layout

Versioned, fixed width, little-endian-free (byte arrays are copied verbatim):

```
offset  size  field          notes
------  ----  -------------  ------------------------------------------------
0       4     magic          b"ENGM"
4       2     layout_version u16 LE; currently 1
6       1     granularity    0 = window, 1 = subderiv, 2 = sentence
7       1     flags          bit0 = key is a fallback (§4), rest reserved 0
8       4     depth          u32 LE; window order n, or sub-derivation depth
12      32    unf_hash       SHA-256 of canonical_serialize(reduced net)
44      32    ted_hash       SHA-256 over the TED canonical serialization
76      8     l1_weight      f64 LE; NaN when the key carries no L1 weight
------  ----  -------------  ------------------------------------------------
84 bytes total
```

**[decision]** `unf_hash` comes **first among the hashes** because it is the
stable addressing key: `E_g` is a `HashMap<EngramKey, Embedding>` whose lookup
and dedup statistics key on `unf_hash` alone (§3). `ted_hash` is a secondary
index that merges algebraically-equal fragments without changing the primary
address, per §2.4 item 3.

**[decision]** `l1_weight` is `f64` with `NaN` as "no weight", because a `0.0`
probability is a real L1 value (E5) and must not be confused with absence.

### 2.3 Granularity lattice

`window ⊂ subderiv ⊂ sentence`: every `window` key is also derivable at a
coarser reading, and every `subderiv` key is a strict refinement of its
`sentence`. A lookup at granularity `g` **does not** fall back to a coarser
granularity — a miss is a miss, so that coverage statistics stay meaningful.
The lattice is recorded so E3 can report dedup per level without re-deriving
segmentation.

### 2.4 "Unique normal form up to accounted isomorphisms"

Exactly the quotient logos already computes:

1. **reduction-order independence** — confluence of net reduction, machine-checked
   in `unfer/logos`'s `lean/Confluence.lean` (diamond, Church–Rosser, UNF
   uniqueness), exported via lean4export and re-checked in nanoda, and
   corroborated at runtime by the double-reduction self-check (`verified` in the
   `uk_logos_compile` report).
2. **node-index / α-isomorphism** — `deltanet::unf::canonical_serialize` walks the
   rooted port structure and never emits node indices, so isomorphic wirings
   serialize to identical bytes.
3. **algebraic isomorphism** — `deltanet::ted` canonicalizes the Int64 fragment
   to sorted polynomials over ℤ/2⁶⁴ (commutativity, distribution, like terms), so
   `ted_hash` merges `x + 1` with `1 + x`.
4. **not accounted** — graph isomorphism of *unreduced* nets. Out of scope:
   keys are always computed on reduced normal forms.

---

## 3. Denotational nuance (read this before trusting a dedup number)

Keys identify **normal forms**, so keys are *denotational*, not surface, and two
different sentences can collide:

> `John adds two three` and `Bob adds three two` both reduce to `5` and collide
> at `sentence` and `subderiv` granularity.

**[decision]** This is the intended quotient and is **not** a bug. Surface
identity is preserved by using `window` keys, or by consulting derivational
metadata alongside the key. E3 therefore reports dedup ratio **per granularity**
and never as a single headline number: a high `sentence` dedup ratio with a low
`window` ratio is a signal about the lexicon, not a win.

---

## 4. Fallback rule

A fragment is reduced to a key by:

```
harper_gate::lint → ccg::parse_sentence → core_ir::compile_to_core_ir
  → core_ir::linearity::insert_linearity → deltanet::compile_to_net
  → reducer::reduce (≤ 1M iterations; stuck → fallback) → unf::unf_hash
  + TED canonicalization → ted_hash
```

mirroring `logos::translate` steps 3–5 and the `uk_logos_compile` report fields.

If any stage fails — the fragment is outside the CNL grammar, the lexicon does
not cover it, or reduction hits the iteration cap — the key is the `window` key
for the fragment's token window, with `flags` bit0 set. **[decision]** A
fallback key is stored and looked up normally, but tagged, so E3 can report the
**parse rate** as a first-class metric. A silently-degraded corpus is the failure
mode this whole feature risks, and a tagged fallback is what makes it visible.

**[decision]** Reduction is capped at 10⁶ iterations because E2 must not be able
to hang the ingest of a hostile corpus. The cap is a *skip*, not an error.

---

## 5. API

E2–E3 live in `../unfer/logos/src/engram/` (new module inside the existing crate,
not a new crate).

```rust
// key derivation — E2
pub fn engram_key(fragment: &str, granularity: Granularity) -> Result<EngramKey, KeyError>;
pub fn window_key(tokens: &[String], n: usize) -> EngramKey;
pub fn subderiv_keys(tree: &DerivationTree) -> Vec<EngramKey>;
pub fn segment(fragment: &str, granularity: Granularity) -> Vec<EngramKey>;

// table + ingest — E3
pub struct EngramTable { /* per-granularity HashMap<EngramKey, Embedding> */ }
impl EngramTable {
    pub fn new() -> Self;
    pub fn insert(&mut self, key: EngramKey, emb: Embedding);
    pub fn get(&self, key: &EngramKey) -> Option<&Embedding>;
    pub fn ingest(&mut self, corpus: &Corpus) -> IngestStats;
}

// stats — E3. Every field is per granularity; see §3.
pub struct IngestStats {
    pub dedup_ratio: BTreeMap<Granularity, f64>,
    pub ted_merge_rate: BTreeMap<Granularity, f64>,
    pub parse_rate: BTreeMap<Granularity, f64>,
    pub lookup_p50_ns: u64,
    pub lookup_p99_ns: u64,
}
```

**[decision]** `parse_rate` is part of `IngestStats`, not a debug log line: §4
makes it the metric that says whether the corpus is actually reaching the UNF
path.

---

## 6. Golden corpus

`../corpus/engram_keys.tsv`, ≥30 groups, one row per (group, member, granularity).

Columns: `group`, `member`, `granularity`, `expect` (`collide` | `distinct`),
`note`.

- `expect=collide` — members in a group that **must** produce the same
  `unf_hash` at that granularity. Paraphrases and algebraic variants land here.
- `expect=distinct` — members that must **not** collide. Near-misses land here;
  these are the rows that catch an over-eager canonicalization.

**[decision]** The corpus records *expectations*, not digests, because E2 does not
exist yet. E2 fills the digest columns in when it lands, and the digest columns
are what pin the implementation; until then the expectation columns are the
specification and a failing row means the implementation, not the corpus, is
wrong.

---

## 7. What E1 does not decide

- **E4** needs `deepseek-ai/Engram` cloned and a rev pinned; the ablation design
  is in the plan.
- **E5** (`l1::split_l1` weighted keys) reuses `logos::l1::split_l1` as-is; the
  aggregation rule is `logos::l1::aggregate_results`.
- **E6** (`uk_engram_lookup`/`uk_engram_store`) follows the existing S29
  registration checklist and is **additive only** — the frozen `uk_*` contract
  rules apply.
- Reduction cost at decode time is still open (plan §9 open question 1): the
  `window` path is O(1) by construction, but `subderiv`/`sentence` reduce a net.
  Mitigation is the deterministic memo cache; E3 measures the hot `window` path
  separately.