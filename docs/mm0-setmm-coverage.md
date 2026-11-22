# set.mm coverage map for the BookProof corpus

Question this answers: for each part of the corpus, can its mathematics be stated and
proved over **set.mm converted to MM0** (`mm0/metamath/set.mm0`), or does it need new
development at set.mm's level? Measured on 2026-11-05.

## 1. Method

* Corpus: `../timepiece/BookProof/**/*.lean` — 883 modules. Declarations counted by
  regex (`theorem|lemma|def|abbrev`). Each module is assigned one *primary* area by
  keyword priority `GEOM > OPERATOR > MEASURE > ANALYSIS > PROB > FOUNDATIONS`, so a
  module that mentions both `Fock` and `Measure` counts as OPERATOR.
* Library: `mm0/metamath/set.mm` (Metamath source) and `set.mm0` (its MM0
  conversion). Counts are of `$p` (provable) and `$a` (axiom) lines and of keyword
  mentions.

Caveat: the corpus classification is **lexical**. A keyword in a docstring counts the
same as one in a proof term. Its error direction is to over-assign modules to
advanced areas, never to under-assign, so the "expressible" figures below are upper
bounds.

## 2. What the library offers

| metric | value |
|---|---|
| provable statements (`$p`) | 48,171 |
| — in the main library (lines 1–514,842) | 31,207 |
| — in the 59 mathboxes (lines 514,842+) | 16,964 |
| axioms (`$a`), incl. all definitions | 3,110 |
| MM0 conversion size | 20.7 MB spec / 548 MB proofs / 43 MB `.mmb` |
| `mm0-c set.mmb < set.mm0` | **exit 0, 0.43 s** |

Relevant blocks with anchors (line numbers in `set.mm`):

| block | anchor | `$p` in block |
|---|---|---|
| functions, finite sums | `df-mpt` 49341, `df-sum` 179827 | — |
| measure, volume, integral | `df-vol` 359084, `df-itg` 363508 | 5,091 (359,084–483,570) |
| Hilbert Space Explorer | `df-hba` 496898, `ax-hilex` 497235, `df-cm` 503252 | 1,707 (483,570–514,842) |
| matrices | `df-mat` 301011 | — |

Keyword mentions in `set.mm` (whole file, case-insensitive):

`Hilbert space` 694, `inner product` 256, `Fourier` 1,910, `Banach` 52,
`Lebesgue` 190, `derivative` 262, `matrix` 602, `probability` 66,
`random variable` 42, `prior` 20, `Gaussian` 20, `Markov` 8, `Bayes` 2,
`Hermitian` 51, `spectrum` 14, `eigenvalue` 16, `Fock` 2, `creation` 0,
`annihilation` 0, `compact operator` 0, `Laplacian` 0, `Sobolev` 0,
`ContDiff` 0, `Clifford` 1, `Pauli` 0, `Lorentz` 0.

One trap worth recording: **`df-lp` (line 316698) is `limPt`, the limit-point
operator, not an L^p space.** set.mm has no `L^p`/Sobolev scale.

## 3. What the corpus needs

| primary area | modules | declarations | source lines | set.mm verdict |
|---|---:|---:|---:|---|
| FOUNDATIONS (finite sets, order, groups, matrices) | 75 | 1,134 | 14,897 | **expressible** |
| PROB (priors, Bayes, entropy) | 48 | 515 | 7,250 | **expressible**, new defs needed (`entropy` absent) |
| ANALYSIS (normed/inner-product/Hilbert, Fourier) | 85 | 1,390 | 22,151 | **partial** — no Laplacian, Sobolev, smoothness |
| MEASURE (Lebesgue measure and integral) | 188 | 3,012 | 45,348 | **mostly expressible** — no `L^p` |
| OPERATOR (Fock, spectral, unbounded, Hermitian) | 405 | 10,504 | 153,941 | **new development** |
| GEOM (Lorentz, Pauli, Clifford) | 82 | 1,748 | 20,368 | **new development** |
| **total** | **883** | **18,303** | **263,955** | |

So of 18,303 declarations, **~3,039 (17%)** sit in areas where set.mm already has the
underlying mathematics, **~3,012 (16%)** in measure theory where it has the measure
and integral but not the function spaces, and **~12,252 (67%)** in operator theory
and Lorentz/Pauli geometry where set.mm has essentially nothing (`Fock` 2 mentions,
`compact operator` 0, `Lorentz` 0).

## 4. Two costs, not one

Restating is the price of admission and does not scale with proof difficulty:

1. **Statement translation.** Corpus statements are interaction-net encodings
   (`theorem ... : $ teq (tapp (tapp (tentity 3) ...) ttrue $`), e.g.
   `mm0/statements/BookProof.ChapterNoBestPrior.mm0` has 6 roots over 10,602 export
   declarations. Nothing in that form matches a set.mm statement, so every
   declaration must be re-expressed in set.mm's language before anything can be
   proved. There is no translator for this; the Lean source is the only readable
   statement of intent.
2. **Proof.** Once restated, proofs in the covered areas are citations of set.mm
   labels, which cost almost nothing (see `mm0/pilot/pilot.mm1`: 5 theorems, compile
   and `mm0-c` verification in seconds). In the uncovered areas the proofs are new
   formalization.

## 5. Port order

1. `BookProof.ChapterNoBestPrior` (4 declarations, finite sums over ℝ) — the
   reference port; see `mm0/pilot/`.
2. `Attention/ChapterSoftmax*` and the FOUNDATIONS/PROB modules (6–21 declarations
   each) — same shape, cheap.
3. The rest of the PROB family: `ChapterDutchBook` (18), `ChapterHierarchicalBayes`
   (12), `ChapterUniformPrior*` (6–7), `ChapterMaxEntropy` (11), plus
   `Density/ChapterDensityMarginalConditional` (7).
4. ANALYSIS modules that only need inner-product and Hilbert space, which the 1,707
   Hilbert Space Explorer theorems cover.
5. MEASURE modules once an `L^p` development exists.
6. OPERATOR and GEOM chapters: new mathematics, not a port.

## 6. Port status for the reference chapter

`mm0/pilot/ChapterNoBestPrior.mm1` proves six theorems over set.mm-as-MM0. All are
checked: `mm0-rs compile` exits 0 with no `sorry` warning, `mm0-c cnbp.mmb < joined.mm0`
returns **rc 0**, and three separate one-symbol alterations of the spec each return
**rc 255**.

| ported theorem | what it is | set.mm labels cited |
|---|---|---|
| `cbnp_singleton_sum` | `sum_(y in {a}) p y = p a` | `sumsn`, `fveq2` |
| `cbnp_indicator_sum` | `indsum` at the shape the chapter needs | `indsum`, `ax_mp`, `tru` |
| `cbnp_indicator_sum_singleton` | `sum_(y in H) ind{a}(y)*p(y) = p(a)`, the `+1` point mass | the two above, `eqtri`, `pm3_2i`, `syl`, `snssi`, `a1i` |
| `cbnp_indicator_sum_singleton_neg` | the same at the `-1` point mass, giving `(-1)*p(a)` | `mulass`, `mul12`, `mulcl`, `neg1cn`, `pm3_2an3`, `oveq2`, `sumeq2i` |
| `cbnp_eq_at` | the two point-mass tests give `p(a) = q(a)` | `breq1i`, `breq2i`, `mpbi`, `mpbir`, `mulm1`, `leneg`, `letri3`, `recn` |
| `cbnp_eq` | pointwise agreement on `H` forces `p = q` | `ralrimiv`, `eqfnfv`, `pm3_2i`, `tru` |

`cbnp_eq_at` is the chapter's `le_antisymm` step and `cbnp_eq` is its `ext x` / `funext`
step; together they carry the content of `eq_of_expectedUtility_ge_all`.

### The translation boundary

Lean quantifies over the function space (`forall u : Hyp -> R`). set.mm's `wral`
quantifies only over **setvars**, so instantiating such a hypothesis at a *compound*
class — the point mass is one — requires `rspa` plus `wsb` substitution rewriting of the
whole formula. The port therefore takes the two instantiations the Lean proof actually
performs as hypotheses: `hpos` (the `+1` point mass) and `hneg` (the `-1` point mass),
one pair per point. That is what makes the port checkable today; recovering the
`A. u` form from it is substitution plumbing rather than mathematics.

Consequently `not_uniformly_better` and `distinct_priors_each_preferred` are **not**
ported: each turns `not forall u` into an existential witness (`not_forall`, `not_le`),
which presupposes exactly the `A. u` hypothesis that the port replaces. Two separate
gaps stand between them and a proof: the substitution layer above, and the closure of
the hypotheses themselves — refuting `A. u. P u` means proving something *under* the
assumption `x e. H`, which MM0 has no rule for.

### Closing hypotheses automatically

`mm0/tools/closehyps.pl` supplies that missing rule as a tool: it implements the
classical deduction theorem over MM1 proof terms and derives the closed companion

```
pub theorem T_c (params) : $ (wi F1 (wi F2 ( ... C ))) $ = 'proof';
```

for any `pub theorem T` stated with hypotheses `F1 .. Fn`, using only `ax_1`, `ax_2`,
`ax_mp` and `id`. Every leaf is closed (`hk -> id`, `hi -> (ax_mp hi ax_1)`), every
hypothesis-free citation is wrapped by `ax_1`, and each argument of a cited rule is
peeled off that rule's closed companion with an `ax_2` step, so no node of the emitted
proof assumes anything. Verified on a two-hypothesis inference: the companion compiles
and `mm0-c` returns **rc 0**, while one altered symbol in the companion's statement
returns **rc 255**.

A cited rule that is itself stated with hypotheses needs *its* closed companion first.
Local theorems get one recursively. External set.mm rules are looked up first: set.mm
often states a rule's closed form directly under another label (`ax_1` is the closure of
`a1i`, `pm3.2` of `pm3_2i`, `breq1` of `breq1i`), so the tool canonicalises both sides
— replacing every non-constructor token by the index of its first occurrence, which is
invariant under variable renaming — and looks the shape up in an index of the 20,564
closed conclusions (66 usable shapes before the top level was parsed as a single
application; that bug hid the whole index). A one-step `ax_mp` composition of two
closed lemmas is tried next. What survives that is genuine new mathematics, and the
tool reports it rather than inventing it. Stage two is implemented for exactly that case: `closehyps.pl --mma
mm0/metamath/set.mm` decodes the rule's *own* set.mm proof with `SetMM::derive`, keeps
the proof tree, and runs the same deduction transform over it, with the one change the
first-order setting forces — a rule's term arguments are copied rather than peeled, so
only `$e` hypotheses become implications. Measured on this chapter (2026-11-06), the
following cited rules now close automatically, each emitted as `pub theorem
<label>_c` in dependency order: `mpd`, `a1i`, `syl`, `biimpd`, `mpbid`, `mpbi`,
`eqeq2i`, `eqtri`, `nsyl4`, `pm2_61i`, `impcon4bid`, `con4bid`, `_2thd`, `_2falsed`,
`con2i`, `nsyl`, `con3i`, `pm5_21ni`, `pm5_21nii`, `eleq2i`, `mpan2`.

| lemma | frontier after stage two |
|---|---|
| `cbnp_singleton_sum` | — (already closed: it has no hypotheses) |
| `cbnp_indicator_sum` | `indsum` |
| `cbnp_indicator_sum_singleton` | `indsum` |
| `cbnp_indicator_sum_singleton_neg` | `indsum`, `sumsn` |
| `cbnp_eq_at` | `indsum`, `sumsn` |
| `cbnp_eq` | `indsum`, `sumsn` |

`eqtri`, `mpbir`, `mpbi` and `ralrimiv` left the frontier. What remains is `indsum` and
`sumsn`, and both fail for one reason that belongs to the *spec*, not to the transform:
set.mm0 encodes the quantifier axioms with a **dependent hypothesis** —

```
axiom ax_gen {x3: setvar} (ph: wff x3):
  $ ph $ >
  $ wal x3 ph $;
```

— so citing `ax-gen` must pass the dependent wff `ph`, and the proof stream does not
carry it (set.mm pushes only the setvar). Its value is fixed by the step's own *result*
instead, so the decoder has to record each step's instantiated conclusion next to its
arguments: `_run_map` computes it, but the tree it returns keeps only the arguments.
That is a bounded change — record the result, then instantiate the axiom's hypothesis
pattern against it — and it is the last thing between the current tool and the six
companions. No such claim is made here: the table above is the measured state, and the
six closed companions do not exist yet.

**Why not generate them instead of decoding?** Because the emitter that turns a `dk` or
CNL statement into a proof term does it by *normalising in a convergent equational
fragment*, and set.mm's labels are not in one. Measured from the hint lists of
`metamath/set.mm` (48,157 proofs carry hint lists; transitive closure over each label's
direct references): `indsum` reaches **4,796 assertions / 53,342 references**, `sumsn`
**4,709 / 51,700**, `sumeq2i` **3,400 / 29,676** — against `eqtri` **258**, `mpbir`
**76**, `ralrimiv` **92**. A generator with no access to set.mm's proofs would have to
re-derive thousands of read theorems — order on the reals, finite sums, quantifier
manipulation — before it could state the goal. Decoding re-derives only the target's own
steps and the closure transform supplies the rest, which is why generation is used where
it applies (an equational fragment with a decision procedure) and decoding where it does
not.

### The decoder: resolved (2026-11-06)

**Resolved.** Both open questions below are settled: the prefix is the `$f` hypotheses
of the variables that occur plus every `$e`, **in declaration order** (set.mm does
interleave them), then the hint list; `Z` appends one map slot, which a later reference
reuses. With that the decoder verifies proofs instead of guessing at them:

| sample | accepted |
|---|---|
| first 400 assertions | **400 / 400** |
| 20,001 – 21,000 | **920 / 1,000** |
| 40,001 – 40,100 | **100 / 100** |

All seven labels that used to stop it decode with checked derivations: `eqtri` (13
steps), `sumsn` (10), `ralrimiv` (9), `mpbir` (8), `sumeq2i` (21), `dfbi1ALT` (58),
`indsum` (151). Three bugs were the difference: `%vartype` was populated only for
assertions, so no variable atom could be typed; `splice(@stack, -0)` cleared the whole
stack on a nullary constructor (`cdr`, `cmgp`); and unification compared bound values by
stringifying whole trees, which made dense proofs quadratic — that is what hung the
sweep near statement 40,000.

The full 48,135-proof sweep has **not** been completed: dense regions cost about 2 s per
proof (~17 h single-process), and ~8% of the 20k sample still fails. The narrative below
is kept as the record of how the answer was found; its conclusion ("unresolved") no
longer holds.

`mm0/tools/SetMM.pm` reads `metamath/set.mm` directly: comments, `${`/`$}` scopes,
`$v`/`$c`/`$f`/`$e`/`$d`, conclusions and proof sections. On this snapshot it parses
**120,560 statements, 51,211 assertions, 48,135 proofs**, with all **8,037,922**
hypothesis references resolving. That half is verified. The other half is not.

The format is documented in `mmpars.c` and Appendix B of the Metamath book: a proof is
`$= ( <label hints> ) <letters> $`, the letters index a label map, and a reference is
an optional base-5 high digit `U..Y` followed by one base-20 low digit `A..T` that
terminates it (`value = value*20 + c-'A'`). The code is explicit that the map is the
theorem's mandatory hypotheses followed by the hint list, and that `Z` names the step
just completed so later references reuse it instead of re-deriving it.

That rule does not fit this file. `a1i` has 13 mandatory hypotheses and 3 hints, so its
map has 16 entries, but its letters `ABADCABEF` decode to references
`0,1,0,3,2,0,1,4,5` — every one of them lands inside the 12-entry `$f` block, so the
proof would push nine expressions and never apply an assertion. Twenty candidate
constructions (hypotheses then hints, hints then hypotheses, `$e`-only prefixes, hints
only, all-earlier-assertions variants) crossed with both pop conventions, scored
against the two checks that cannot be faked — final stack height 1 and the top step's
conclusion shape equal to the theorem's own — close **0 to 2 of 102** sampled theorems.
So the letter encoding, not the map alone, differs from the documented one, and the
filed judgement is: unresolved, not "roughly working".

An oracle settles it instead of more guessing. `metamath-exe` builds here with plain
`gcc -O2 src/*.c` (no autotools needed) and reads this exact file: **254,788
statements, 3,076 `$a`, 48,135 `$p`, no errors**, i.e. the proofs are valid
current-format after all, and the fault is in this decoder, not the file. `show proof
<label>` then prints the decompressed reference sequence, which turns the map from a
guess into a measurement:

| theorem | letters | references | map recovered |
|---|---|---|---|
| `a1i` | `ABADCABEF` | 0,1,0,3,2,0,1,4,5 | `wph wps a1i.1 \| wi ax-1 ax-mp` |
| `mpbir` | `BACABDEF` | 1,0,2,0,1,3,4,5 | `wph wps mpbir.min mpbir.maj \| biimpri ax-mp` |
| `id` | `AAABZAAACAECD` | 0,0,0,1,(Z),0,0,0,2,0,4,2,3 | `wph \| wi ax-1 mpd` + one `Z` slot |

So the prefix is not `reqHypList` (13 entries for `a1i`, mostly unused `$f`): it is the
`$f` hypotheses of the variables that actually occur, then the `$e` hypotheses, then
the hint list — and `Z` really does append one map slot that a later reference reuses,
which the `id` row confirms (four pushes, `Z` tags `wi`, a later reference to slot 4
re-pushes it, and `show proof /normal` shows the resulting 14 steps). One case still
contradicts that rule: `a2i` (`ABCEEABEACEEDABCFG`, 18 references) puts `wi` where the
rule predicts the second `$e`. So the prefix rule is close but not settled, and the
remaining work is mechanical measurement against the oracle rather than design.

The oracle also removes the need to finish the decoder before using set.mm's proofs:
`show proof <label> / normal` emits the plain label sequence, which is exactly what the
deduction transform consumes, for any label and recursively for its dependencies.

### What the decoder now has right, and what still stops it

The decoder is not allowed to depend on any binary but the MM0 compiler, so its own
acceptance test is a real one: each assertion application pops its mandatory
hypotheses, the popped expressions are unified against the hypotheses' formulas, and
the final stack entry must unify with the theorem's conclusion. Measured against that
criterion on a 400-theorem sample: **68 accepted, 332 rejected** — a large gain on the
previous 2/102 and still nowhere near shipping.

Three facts were established along the way, each pinned by an oracle run:

1. **The pop count is not the declared hypothesis list.** `wi` carries twelve
 `$f wff ...` hypotheses in this file, but `metamath`'s own debug output for step 4 of
 `a1i` shows `sch $|$ wff ph $|$ wff ps $|$` against `ins $|$ wff ps $|$ wff ph $|$`,
 i.e. two popped entries. The count that reproduces every verified proof here is the
 `$f` hypotheses of variables *occurring in that statement's formula* plus every `$e`
 (`wi`:2 + `ax-1`:2 + `ax-mp`:4 = the 8 pops that balance `a1i`'s nine steps). This is
 now `_req_hyps` in `tools/SetMM.pm`.
2. **The letter field is the leading typecode too.** A statement's formula is stored as
 `<typecode> <expression>` (`|-`, `wff`, `class`, ...), so a parser that strips only
 `|-` turns `wi`'s conclusion into the atom `wff` and pushes nonsense. `_form_tree`
 now drops one leading token unconditionally.
3. **The remaining unknown is the semantics of `$f`-hypothesis stack entries.** `a1i`
 now reaches the right shape and fails only at the final `ax-mp`: the pushed `wps`
 entry binds `ps` to the bare variable `ps`, and the `maj` hypothesis then demands
 `( ps -> ph )`. Metamath evidently lets a `$f` entry leave its variable *open* rather
 than binding it, and reproducing that is the whole of what is left. The 332 rejections
 are overwhelmingly this one issue, not decoding errors.

A consequence worth stating plainly: a dump cache produced with the oracle is not an
option for the shipped pipeline, since a closure has to be recomputed for whatever
`set.mm` snapshot the operator builds against. The decoder must be finished.

## 7. Shape search tool

`mm0/tools/shapegrep.pl` indexes `set.mm0` and matches a statement shape against every
declaration's conclusion, reporting the instantiation found and the hypotheses owed:

```
$ perl mm0/tools/shapegrep.pl '(wi (wa (wcel A cr) (wcel B cr)) (wb (wceq A B) (wa (wbr A B cle) (wbr B A cle))))'
letri3  [theorem]
    (wi (wa (wcel A2 cr) (wcel B2 cr)) (wb (wceq A2 B2) (wa (wbr A2 B2 cle) (wbr B2 A2 cle))))
```

`-s` matches anywhere inside a conclusion, `-f` prints hypotheses, `-v` prints the
variable bindings found, `--name RE` filters by label, `-k` filters by declaration kind.
Every citation in the table above was located with it; without it each one meant grepping
labels and reading statements by hand.

## 8. Why no Attention/Softmax chapter is ported here

The smallest candidate by declaration count is `Attention/ChapterSoftmaxStability`
(6 declarations, 146 lines). It is not portable over set.mm as-is, for two reasons
that the coverage numbers in §3 already predict:

1. **Its definitions do not exist in set.mm.** The chapter reasons about
   `partition beta s = sum_j exp (beta * s j)` and `scoreSoftmax beta s j =
   exp (beta * s j) / partition beta s`. Neither softmax nor a partition function
   appears anywhere in set.mm, and it imports four sibling modules
   (`ChapterSoftmaxSharpness`, `ChapterSoftmaxOrder`, `ChapterSoftmaxFluctuation`,
   `ChapterSoftmaxMaxEntropy`) for the surrounding vocabulary (`logPartition`,
   `scoreSoftmax_pos`, …). A port would have to state those as new MM0 axioms
   first — at which point the result is "proved from added axioms", not "proved
   over set.mm", which is the opposite of what this document measures.
2. **Its analysis is real exp/log.** The proofs go through `Real.exp_add`,
   `Real.exp_le_exp`, `Real.log_le_log`, `Real.log_mul`, `Real.log_exp`,
   `abs_add_le`, `abs_sub_le_iff` and `Finset.sum_le_sum`. set.mm has `ce`
   (complex exponential) and a logarithm, but no softmax-shaped bridge lemmas, so
   each step would need its own development even after the definitions.

The same holds for the other small entries (`ChapterDeepLearningEnsemble`, 4
declarations): it is a fibrewise sum over the fibres of `train : Seed -> Model`,
i.e. `Finset.sum_bij` rearrangement of a prior induced by a map — clean
mathematics, but again about a prior that set.mm does not define.

Conclusion: these chapters belong to the 67% "new development" bucket of §3, not
to the 17% that ports by citation. Porting one honestly means adding its
definitions as MM0 axioms and proving the chapter from those — a different
artefact from the set.mm-backed port in §6, and one that needs the same
label-by-label effort per theorem.

## 9. Which Lean 4 route can end in `mm0-c`

The question is not "can a Lean 4 proof be re-checked" but "can a Lean 4 proof be
re-checked *by the MM0 compiler and nothing else*". Those have very different answers,
and the second one rules out the intermediate systems that look promising.

| route | verdict |
|---|---|
| Lean 4 → MM0 directly | not possible as a proof translation. MM0's checker applies named axioms (`ax_mp`-style); Lean's kernel checks CIC terms. There is no term-level map between them. |
| Lean 4 → nanoDa → ... | nanoDa is a Lean 4 *kernel* in Rust. It consumes `lean4export` NDJSON and prints accept/reject; it emits no language, so it is a terminus, not a bridge. |
| Lean 4 → Lambdapi | no such translator exists. The Dedukti ecosystem has Rocq (`CoqInE`), HOL-Light (`hol2dk`), Isabelle, Agda, Matita (`Krajono`) and PVS front ends; Lean appears only as an *output* target (`sttfaxport`, Lambdapi → Lean). |
| Lambdapi → MM0 | no *export target* (`lambdapi export -o` offers `dk`, `raw_dk`, `hrs`, `xtc`, `raw_coq`, `stt_coq`, `raw_lean`, `stt_lean`), and no Lambdapi/Dedukti translator, proof export or source file exists in this project: `grep -ril 'lambdapi\|dedukti\|LambdaPi\|lambda-?pi'` plus a `find` for `*.dk`/`*.lp`/`*.lpo` over `australVM/` and `unfer/` return only this document, one set.mm syntax token and one attribution note. The in-project MM0 translator that does exist is `unfer/logos/src/mm0/` — see the paragraph after this table. |
| Lean 4 statements → first-order logic → MM0 | the only route that terminates in `mm0-c`, and it is not a translation: the statements are restated in set.mm's language and the proofs are rebuilt from set.mm. This is what §6 does. |

**What the project does have.** `unfer/logos/src/mm0/` is a working MM0 emitter: 5,232
lines over `cnl.rs` (surface syntax), `proof.rs` (net and Peano encodings, proof-term
construction), `emit.rs` (certificate assembly, content-addressed theorem names, tool
invocation), `bridge.rs` (CoreIR → DeltaNet corroboration) and `kop.rs`, driving the
112-line theory `unfer/logos/mm0/logos_math.mm0` plus three expectation corpora
(`claims.txt` 99 lines, `upstream.txt` 174, `kop_claims.txt` 117). Measured 2026-11-06,
after `cargo build --release -p logos` (15 s; the previously shipped binary predates the
subcommand and answers `Unknown subcommand: mm0`):

- `logos mm0 ask "two plus three equals five"` → `verdict: true`, goal
  `teq (tapp (tapp tprim_add (ts (ts tz))) (ts (ts (ts tz)))) (ts (ts (ts (ts (ts tz)))))`,
  theorem `logos_462dbcf1b6537a1f`, with a DeltaNet node-permutation witness;
- `logos mm0 ask "two plus three equals six"` → `verdict: false` (`teq L R ⟹ tfals`);
- `logos mm0 theory --out` re-elaborates under `mm0-rs`: 2 sorts, 9 term/def, 18 ax/thm;
- `logos mm0 claims claims.txt` → **47 claims, 0 failures**, including the honest
  boundaries (`unproved for all x, y: x plus y equals y plus x`).

So the MM0 *emission back end* exists in Rust and is exercised end to end by
`australVM/scripts/logos2mm0`. What it does not have is a front end from a dependently
typed system: its input is Logos' controlled natural language lowered to `CoreIR`, and
its fragment is arithmetic over Peano terms with equality. A Dedukti route meant writing a front end that consumes `dk` and targets this back
end's statement and proof-term shape; that front end now exists and is described next.
The typing objection after it is what bounds the route, not a lack of tooling.

**The `dk` front end (2026-11-06).** `unfer/logos/src/mm0/dk.rs` plus `logos mm0 dk
<FILE.dk>` consume a Dedukti-style presentation (`symbol`, `rule`, `thm`, `def`, `type`)
and emit `logos_math.mm0` theorems that `mm0-rs` compiles and `mm0-c` re-checks:

- every `rule` is re-proved as its own theorem (`dk_rule_1 …`), so the rewrite system is
  checked against the theory rather than assumed — a rule the recursion laws do not
  justify is a translation error naming both normal forms;
- every `thm … := refl` becomes a `teq` theorem whose proof is the explicit chain of
  `add_z`/`add_s`/`mul_z`/`mul_s` and congruence steps that Dedukti's `refl` would have
  performed by conversion, so `mm0-c` checks each rewrite instead of trusting a reducer;
- `sym(p)` and `trans(p, q)` are translated: `sym` swaps the goal, and `trans` recovers
  its intermediate term by normalising the left side — the only value both halves can
  meet in a fragment decided by normalisation — and then checks each half against it. A
  citation (`thm … := earlier_thm`) is matched first-order against the named statement,
  which is the instantiation MM0's elaborator will infer. A `thm` whose sides do not
  convert is still refused, with both normal forms named.

Measured: `logos mm0 dk logos/mm0/dk/arith.dk` → 4 rules + 8 theorems, `2 sorts, 9
term/def, 30 ax/thm`, `mm0-c` **exit 0**; fed the emitted `.mm0` against the `.mmb`
directly, `mm0-c` alone also exits 0. The negative fixture `induction_gap.dk`
(commutativity) exits **1** with *"do not convert to a common normal form … the missing
rule is usually induction"*. Eight unit tests cover the parser, the fragment boundary,
citation matching and both composition forms, and the gate now holds the front end to
the same standard as the CNL path: `scripts/logos2mm0` runs both fixtures and reports
**13 passed, 0 failed, 0 gapped**.

What it is not: a λΠ kernel. MM0 is first-order, so the front end translates statements
and equational derivations and never checks a dependent type. The typing objection below
still stands, and it is why this route certifies arithmetic facts rather than Lean's
theorems.

Two of these deserve the reason rather than the fact. nanoDa is *the right tool for a
different job* — independently re-checking that a Lean proof is not a `sorry` — and it
composes well with a Lean-side audit. If the goal is "trust the Lean corpus", running
`lean4export` + nanoDa alongside the existing check is worth adding. But it cannot
produce anything MM0 consumes, so it cannot shorten the set.mm work.

The Lambdapi route fails deeper than tooling — not because a translator cannot be
written, but because of what it can hand over. λΠ-calculus modulo rewriting and CIC
both have dependent types, universes and rewrite/definitional equality; MM0 has none
of those — it is a first-order system with explicit sorts. Sending a Lambdapi
derivation to MM0 would require encoding types as first-order predicates, adding an
axiom that internalises the type checker, and then every Lean step becomes a step
*inside that encoding*. MM0 would still check its own axioms, so the result would be a
proof of "this encoding of the Lean term is well-typed", not of the theorem — and the
encoding axiom would be doing the mathematical work. That is precisely the trust
transfer this port is meant to avoid.

So the honest architecture is the one already in the tree: the Lean development is the
*specification* (what statements, with what dependencies), and the MM0 artefact is a
re-derivation in set.mm's language, closed-form, verified by `mm0-c` alone. What the
Lean side buys is the statement list and the proof obligation, not the proof term.

### 9.1 Re-measured 2026-11-22: the direct route runs, the Dedukti route still does not

The first row of the table above has aged badly, and it is worth saying so plainly:
**Lean 4 → MM0 directly is now the route in the tree**, as a proof translation and not a
statement port. `scripts/lean-typing-proof` reads the nanoda-checked `lean4export` NDJSON
and emits an MM0 *derivation* in the typing judgment of `docs/cic_typing.mm0`
(`tty ctx term type`), which `mm0-rs` elaborates and `mm0-c` re-checks. What the row got
right survives: MM0 has no term-level map for CIC, so the derivation goes through an
encoding — the delta-net agent signature of `docs/cic.mm0` (11 axioms) plus the typing
rules of `docs/cic_typing.mm0` (15 axioms, 152 lines) — rather than through a term-level
translation. The objection that matters is therefore not "is there a map" but "how big
is the trusted base, and is it checked".

Measured today on this host:

| measurement | value |
|---|---|
| `scripts/lean2mm0 check` (statement side) | **OK**, generated MM0/MM1 in sync with the pinned exports |
| `scripts/check-lean-typing-proof` (proof side) | **OK**, 12 artifacts across 3 modes match the golden |
| smoke row `abs_re_inner_pairVec_le`, default mode | 1 041 771 375 B `.mm0`, 1 075 343 397 B `.mm1`, 1 012 915 095 B sidecar, 70.6 s |
| same row, `LTP_OBLIG_SHARE=1` | identical `.mm0`/`.mm1`, sidecar 4 258 753 B, 31.4 s |
| largest row measured (`nsOuterFock…singleTime`) | 102 543 367 789 B `.mm0`, 2 617–3 762 s, peak 356–363 MiB |
| trusted base | 11 + 15 named axioms in the two theories; per artifact, 467 710 assumptions, of which 1 279 are constant types and 1 256 substitution conversions |

The ratios are the real finding: a one-gigabyte certificate for a three-binder lemma, and
a hundred-gigabyte one for the largest theorem, with the count of assumptions in the same
order as the term count. That is what an encoding of CIC into a first-order theory costs,
and it is the thing to attack next (§13.25–13.53 in `lean4mm0.md` are all that attack),
not the choice of intermediate language.

**The Dedukti/LambdaPi route, re-checked.** Nothing has changed in the tooling, and the
in-repo position is unchanged: no Lean → Dedukti translator exists (the ecosystem's front
ends are Rocq, HOL-Light, Isabelle, Agda, Matita, PVS; Lean appears only as an output
target), and no `*.dk`/`*.lp` file exists in `australVM/`. The one Dedukti-shaped
component in the workspace is `unfer/logos/src/mm0/dk.rs` — 979 lines consuming
`symbol`/`rule`/`thm`/`def`/`type` and emitting `logos_math.mm0` theorems whose rewrite
rules are re-proved rather than assumed. Read for what it would contribute, it settles
the question by its own fragment: the mapped symbols are `zero`, `succ`, `plus`, `times`
and `eq`, the file's two fixtures are 15-line arithmetic files, and it documents that it
"is not a λΠ kernel" — a `thm` whose proof is not `refl`/citation/`sym`/`trans` is
refused by name. A Lean export lowered to `dk` first would land in a front end that
cannot express dependent types at all.

So the comparison is not close, and not because Dedukti is a poor system:

| axis | Lean 4 → MM0 directly (in tree) | Lean 4 → Dedukti → MM0 |
|---|---|---|
| front end | exists (`lean2mm0` + `lean-typing-proof`, 5 288 lines) | would have to be written; none exists upstream |
| back end into MM0 | exists, `mm0-c`-checked, 26 axioms total | would have to be written; the existing `dk` path covers Peano arithmetic |
| what the intermediate system adds | nothing — the encoding is explicit in the two theories | a second reduction engine's conversion, i.e. a *larger* trusted base |
| checked by | `mm0-c` alone, on an artifact anyone can re-run | `mm0-c` on an encoding, plus Dedukti's own checker for the steps before it |
| cost | 1 GB / lemma, 102 GB / largest theorem | the same encoding, plus a translation layer whose proofs still need the same grounding |

A Dedukti hop cannot shrink the certificate, because the size comes from unfolding CIC
terms into a first-order encoding, not from the syntax chosen to carry them. It also
cannot improve the trust story: `dk.rs` re-proves its rewrite rules precisely because a
rewrite system handed over from another system is a trusted component, and λΠ-modulo
conversion would have to be trusted in exactly the same way. **Verdict: worse on every
axis that is measurable, and unavailable on the one that is not.**

### 9.2 Certificate size: shared subterms (measured 2026-11-22)

The 1 GB per lemma of §9.1 is repetitive, not deep: on the smoke row 467 710 assumptions carry
only 2 690 distinct bodies, and a handful of very large terms are repeated across thousands of
rule applications. The emitter therefore interns repeated subterms and writes them once as MM0
`def`s at the top of the artifact — `def sdK: Term = $ … $;`, nullary and sort-checked, so a use
site is three tokens instead of tens of kilobytes.

It is **the default since 2026-11-22, at a 1024 B threshold**, because the format is a promise
the gate pins: the flip re-recorded 6 of the 12 artifacts (both `.mm0`/`.mm1` pairs and the two
stop-mode files, table below) and left every sidecar hash untouched. `LTP_DEF_SHARE=0`
reproduces the pre-flip bytes, and any other value sets the threshold. The first attempt at the
flip moved 8 artifacts: the obligation JSON spelled `"rhs": "sd5"` with no `def` block in a file
`scripts/lean-net-cert` reads standalone, so `_put_json` now writes through `Enc.write_expanded`
— the sidecar is expanded text, the artifact carries the names.

The interning rule is fixed by the artifact order, not by a heuristic pass: a node is a `def`
candidate at its **first** use, decided by its expanded size; bodies are written in *reverse*
first-use order while a `live` set holds the names already emitted, so no body can refer
forward; and `digest()` — the emitter's own hash deciding when two nodes share a `ty_deq`
obligation — keeps hashing the *expanded* text, otherwise interning would split 7 923 formerly
identical conversions into extra assumptions. The spool path (`Enc.spool`, which compares an
on-disk declared type byte-for-byte) disables interning outright: a reconstructed wrap can never
carry the whole type's name, so `_spool_eq` would fail with “derived type is not the declared
type … declared sd0”.

Measured on the smoke row (`BookProof.Toolbox.ChapterFockPairPerturbation.ndjson`,
`abs_re_inner_pairVec_le`):

| artifact | pre-13.54 | `LTP_DEF_SHARE=1024`, interning off | at the default (interning on) |
|---|---|---|---|
| `.mm0` | 1 041 771 375 B | **354 580 487 B** (2.94×) | **334 175 929 B** (3.12×) |
| `.mm1` | 1 075 343 397 B | **388 152 509 B** (2.77×) | **367 747 951 B** (2.92×) |
| sidecar (obligations) | 1 012 915 095 B | unchanged | unchanged (sha256 `999d2bec…`) |
| wall clock | 70.3 s | 68.3 s | 64.4–70.9 s |
| assumptions | 467 710 (1 279 constant types, 1 256 substitution) | **identical** | **identical** |
| defs emitted | — | 1 694 | 1 560 |

The size cut is real but the assumption count is untouched, which is the honest reading: sharing
compresses the *text* of the derivation, not its obligations. Only 1 560–1 694 nodes clear a 1 KiB
threshold and they account for the whole 66–68 % saving, matching the earlier headroom measurement
on the three largest axioms. The two sharing layers compose rather than overlap: interning (§13.53)
hands the encoder fewer distinct ids, so 1 560 nodes reach a first use instead of 1 694 and the
20 MB between the last two columns is §13.53's contribution to the same command.

**Content is unchanged.** Expanding every `sdK` back into its uses and dropping the `def` block
gives 980 180 332 B of non-comment text on both sides, `cmp` clean against the default artifact.
(Comment lines are excluded from that comparison by necessity: they render truncated text, so a
truncated line that happens to contain a name expands to thousands of characters.)

**Across the book, sampled by size.** Six rows spanning 7 → 666 943 rule applications, each
emitted twice (default and shared, everything else held equal) and each pair re-elaborated and
re-checked: 48 388 → 48 531 B (7 rules, 1 def), 17 219 → 17 269 B (27), 83 807 → 83 857 B (321),
1 012 315 → 1 013 710 B (4 777, 48 defs), **67 199 500 → 53 831 176 B** (6 860, 309 defs, 1.25×)
and **222 469 921 → 167 243 432 B** (666 943, 2 262 defs, 1.33×; `mm0-rs` `3 sorts, 331
term/def, 135 856 ax/thm`, `mm0-c` exit 0). Every pair passes both tools, so the cut costs
nothing in checkability. Below ~5 000 rules the `def` block is a net 50–144 B, which is the
honest price of a size floor rather than a profitability test, and the 19 authored rows that the
sweep marks `emit` — all over the emitter's 10 GiB / 5 400 s budget — were **not** re-emitted, so
this is a sample and not a book-wide total. `docs/lean4mm0.md` §13.54 also records the two
defects this sweep found (binder domains compared shared against an expanded spool; `Enc.enc`
memoising a `sdK` name so a `def` body read back as its own name) and their fixes, which leave
the smoke row's bytes — and therefore this table's first row — unchanged.

**The real toolchain accepts the shared artifact.** Slice = both theories with their `import`
lines stripped, then the artifact's header, `term t_symN: nat;` payload declarations, the whole
def block and the first 2 000 axioms (215 219 297 B):

| step | result |
|---|---|
| `mm0-rs compile slice.mm1 slice.mmb` | **OK**, 2 m 19.7 s, 1 887 232 B `.mmb`, `3 sorts, 2146 term/def, 2026 ax/thm` |
| `mm0-c slice.mmb < slice.mm0` | **OK**, exit 0, 1.6 s |

The full artifact was *not* elaborated: 2 000 axioms cost 140 s, so 467 710 extrapolate to hours,
and this run reports measurements, not projections. Two traps found while setting the slice up,
both worth recording because they look like defects in the artifact and are not:

- `mm0-rs compile` takes its output as the **second positional** argument; `-o out.mmb` is accepted
  silently, writes nothing, and the empty `.mmb` then makes `mm0-c` fail with
  `Unable to memory map file`.
- Inside a `def` body every atom must be a *constant in scope*; an atom the toolchain does not
  know becomes a metavariable, and a body with more than 55 of them is rejected with
  `error: too many bound variables (max 55)`. A harness that strips the artifact's
  `term t_symN: nat;` lines reproduces that error on the first two defs and nothing else.

**Memory floor, re-probed.** §9.1 recorded 384 MiB as the floor for the 102 GB ns row; §13.55 of
`lean4mm0.md` is the re-probe the `sendfile` tail copy asked for, and the answer is that 360 MiB *does*
pass: 3 802 s, peak VmSize 356 MiB, exit 0, with the proof artifacts `cmp`-identical to the
432 MiB reference. The bisection then run at 356 MiB also completes (3 636 s, peak 355 MiB,
same `cmp`-identical artifacts), and the 354 MiB probe after it completes as well (3 564 s, peak
353 MiB, same `cmp`-identical artifacts). 352 MiB still dies in the axiom list.

**The floor is a band, not a byte count.** A concurrent four-way bisection then failed at 353, 354,
355 and 356 MiB (`beta_ax` dict growth at 351 MiB peaks; `_spool_eq` at 5 334 s for 356) while
357–360 MiB all passed 4 for 4, so 354 and 356 have each both passed and failed under the same cap
and env. The passing runs reach cap − 1; the failing ones die at the first growth they cannot
place. The quotable floor is therefore **357 MiB — the lowest ceiling under which this row has
never been observed to fail** — with "sometimes fails" at and below 356, and 384 MiB as the
configuration to use when determinism is worth 27 MiB. §13.55 of `lean4mm0.md` has the table.

The re-probe only says that with its environment, which is why the number is quoted with it:
`LTP_OBLIG_SHARE=1 LTP_OBLIG_CHUNK=2147483648 LTP_EXPRS_CACHE=20000 LTP_ENC_MAX=2MB
LTP_CENSUS=1 LTP_DEDUP_NODES=1`. Re-running the same cap at 360 MiB *without*
`LTP_DEDUP_NODES=1` reproduces §13.55's known control instead of a floor: `MemoryError` in
`infer` after 926 s at a 342 MiB peak, with the census showing synthetic nodes 43.9 MB and the
context memo 17.4 MB against 26.4 MB and 8.5 MB with interning on. The environment *is* the
measurement — same cap, different tables, different verdict — so a floor quoted without it is
not a floor.

### 9.3 The axioms: what the derivation uses and what MM0 could prove

`docs/cic.mm0` declares 11 axioms and `docs/cic_typing.mm0` declared 15 before this audit — it
declares 10 now, and the five that left are the subject of the last part of this section. Two questions matter for the
trusted base: which of them a derivation actually *uses*, and which could stop being axioms.

The first is measurable. The smoke row's `.mm1` header reports 1 929 299 rule applications, and
counting rule names in the proofs gives:

| axiom | uses in `abs_re_inner_pairVec_le` |
|---|---|
| `ty_app` | 629 783 |
| `ty_deq` | 465 159 (343 865 universe-level + 121 295) |
| `ty_abs` | 204 571 |
| `ty_var0` | 8 974 |
| `ty_varS` | 3 691 |
| the other 21 | **0** |

So the trusted base *exercised* by this artifact is five rules. The other 21 are inert here (they
serve other rows and the statement side), and the 467 710 per-instance assumptions — constant
types and `ty_deq` conversions — are where the actual weight sits.

The second question is answered per axiom. MM0 has no datatypes, no recursion, no induction and no
structural congruence schema; `tsubst`, `teval`, `valid_iso`, `tfield` and `tctx_deq` are opaque
constructors, and `tsucc`/`tzero` carry no axioms at all, so nothing about them is derivable. Under
those rules:

| axiom | verdict |
|---|---|
| `teq_refl`, `teq_symm`, `teq_trans` | irreducible as written: `teq` is declared opaque, so these are the only rules that conclude it. They *would* be theorems if `teq` were defined (Leibniz-style) instead of declared — a change of theory, not a proof obligation |
| `tsubst_beta`, `teq_subst` | irreducible: `tsubst` has no definition to unfold, and congruence has no schema |
| `teq_app` | irreducible: same congruence argument |
| `fold_nil`, `fold_cons`, `era_collapse` | irreducible: rewrite rules of the net are the content, not consequences |
| `prim_eval` | not provable, but *eliminable*: the emitter only needs ground instances, so the schema can become per-instance premises instead of a theory axiom |
| `net_isomorphism_step` | irreducible by design: the single place where an externally checked certificate enters, and `valid_iso` is opaque, so no derivation can reach it |
| `ty_var0` | irreducible: the only rule that reads a context entry |
| `ty_varS` | irreducible: weakening needs `tsucc` to mean something, and it has no axioms |
| `ty_pi`, `ty_abs`, `ty_app` | irreducible: they *are* the λΠ fragment |
| `ty_deq` | irreducible: congruence of `tty` under type equality, and MM0 has no congruence schema |
| `tdefeq_refl`, `tdefeq_symm`, `tdefeq_trans` | **eliminable, not provable**: at `def tdefeq (a b: Term): Prop = $ teq a b $;` these are literally `teq_refl`/`teq_symm`/`teq_trans`, so the theory can state them once (in cic.mm0) instead of twice. They cannot become *theorems* — see the toolchain note below |
| `tdefeq_iso` | **eliminable under the same alias**: it *is* `net_isomorphism_step` |
| `tctx_deq_nil` | **eliminable**: at `tctx_deq := teq` it is `teq_refl tnil` |
| `tctx_deq_cons` | not provable, and the alias does not help: it needs congruence of `teq` under `tcons`, i.e. one axiom traded for another |
| `tcert_witness` | not provable (`valid_iso` is opaque); *eliminable* by making the certificate the premise where it is used, which is what `scripts/lean-net-cert` already checks |
| `ty_proj`, `ty_ctx` | irreducible: they state the field and context-conversion relations that are supplied as data |

**The alias is now applied, as eliminations.** `docs/cic_typing.mm0` defines `tdefeq` and
`tctx_deq` as `teq`, and the five axioms above are gone rather than restated, so the file's
trusted base is **10 axioms, not 15**. Nothing references the removed names (the artifacts
declare their own per-instance axioms), and the typing rules that consume conversion —
`ty_app`, `ty_deq`, `tctx_deq_cons`, `ty_ctx` — elaborate unchanged, because a `def` of the
provable sort is accepted in a conclusion *and* in a hypothesis.

**Why not theorems — measured, because the first answer here was wrong.** The obvious version,
`theorem tdefeq_refl (a: Term): $ tdefeq a a $ = $ teq_refl a $;`, is *accepted by `mm0-rs`*
(exit 0) but only as `sorry`: `warning: theorem … contains sorry`, and then `mm0-c` refuses the
file — `at 125: 't1' uses sorry`, `error: some theorems used sorry`, exit 3. The rule
underneath is not about proofs at all: in MM0 mode `mm0-rs` says `(MM0 mode) theorems should
not have proofs` (proofs live in `.mm1`, as `mm0/gap_certificate.mm1` shows — that companion
verifies, exit 0), and in *both* modes a proof obligation whose statement is headed by a `def`
is rejected by `mm0-c` (`expecting a theorem`). A minimal theory isolates it: a def-headed
axiom conclusion verifies (exit 0), a def-headed hypothesis verifies (exit 0), a def-headed
theorem does not. The earlier note that Prop-valued `def`s are “already used in
`mm0/pairing/full2.mm0`” was wrong twice over: that `def eq` appears in no statement, and the
file does not compile under this toolchain (`def ifp missing value`).

**Verified after the change.** `mm0-rs join docs/cic_typing.mm0` → 3 sorts, 30 term/def, 21
ax/thm, `mm0-c` exit 0. The slice that proves the artifacts still fit: both theories with the
`import` line dropped, the artifact's header and `def` block, and its first 2 000 axioms
(215 224 418 B) → `mm0-rs` 2 146 term/def, 2 021 ax/thm in 2 m 19 s, `mm0-c` exit 0 — the
466 416 `tdefeq`-headed statements of the real artifact are def conclusions and check. One
harness trap: the slice boundary must be the artifact's first `^axiom` line, or the header
block re-declares `ax_const_0` and the verifier reports a spurious `unify failure` at
`ax_lev_0`.

Tally, after the change: of the original 26 axioms, 5 are eliminated by definition (a design
change — `tdefeq` is now exactly `teq`, so it is no longer *wider* than it), 5 are exercised by
this artifact, 2 can be demoted to premises, and **14 of the remaining 21 are irreducible**:
for each, its conclusion's head constant is not produced by any other rule, so it is
independent of the rest. Nothing in the theory is now provable that was not before; the gain is
the five axioms that are gone.

## Appendix — FOUNDATIONS and PROB modules

`name decls` pairs; zero-declaration entries are import aggregators or scratch files.

```
FOUNDATIONS (75 modules, 1,134 decls)
Attention/ChapterAttentionMixing 9      Attention/ChapterAttentionOVCircuit 9
Attention/ChapterAttentionSaturation 7  Attention/ChapterSoftmaxFluctuation 21
Attention/ChapterSoftmaxJacobian 21     Attention/ChapterSoftmaxStability 6
Attention/ChapterSoftmaxTemperatureMonotone 10   BalabanRG/Convergence 9
ChapterA4b 4                            ChapterAbelianDiagonal 7
ChapterBookBrstGaugeFixing 32           ChapterBookBrstInstances 24
ChapterCoreBoundsEsa 20                 ChapterF6 23
ChapterFreeFieldBornSignOrientationQuotient 8
ChapterFreeFieldBornSignOrientationSubgroup 5
ChapterFreeFieldBornSignRepresentation 6 ChapterFreeFieldConstraint 11
ChapterGaugeAdjointAlgebra 17           ChapterGaugeWeylResidual 9
ChapterGravityIrrep 10                  ChapterKoopmanLyapunovFarisLavine 19
ChapterMaschkeFiniteGroup 11            ChapterRotaryPosition 10
ChapterSE2 17                           ChapterScaledDotProduct 23
ChapterSinusoidalPosition 9             ChapterYangMillsBianchi 6
Definability/ChapterCountableDefinability 3  Definability/ChapterDefinabilityFragment 3
Definability/ChapterEll2Separable 6     Definability/ChapterPaFreeCompletion 8
Definability/ChapterRieszFischer 11     EsaClosure 0
FarisLavine 0                           FieldStrength/ChapterYangMillsFieldStrength 8
GravityAlgebra/ChapterGravityGenInverse 4  GravityAlgebra/ChapterGravityInvMetric 5
GravityAlgebra/ChapterGravityMetric 7   GravityAlgebra/ChapterGravityPolymomentum 36
GravityAlgebra/ChapterGravityProjDirectSum 6  GravityAlgebra/ChapterGravityProjector 8
GravityAlgebra/ChapterGravityRankSplit 5 GravityAlgebra/ChapterGravitySplit 10
GravityAlgebra/ChapterGravityTimeProj 7 HarmonicAnalysis/ChapterLegendrePolynomial 23
HarmonicAnalysis/ChapterSphericalBessel 12  HashimotoShiftInvert 0
HermiteCore 0                           NavierStokesFlow 0
Ns/ChapterNavierStokesHermiteFarisLavine 55  Ns/ChapterNavierStokesSignedShift 49
Qg/ChapterQgDerivativeRealization 49    Qg/ChapterScalaronNotQuadratic 5
RandomMap2Audit 0                       ReflectionPositivity/HalfSpaceAlgebra 20
ReflectionPositivity/WilsonLinkLattice 153  Sirk/ChapterKrylovShiftSpan 51
Sirk/ChapterSirkMultiShift 13           SirkSpectralGeometry 0
Sm/ChapterParityHypercharge 6           Sm/ChapterParityZ4 7
Sm/ChapterSmFieldStrengthSplit 23       Starobinsky 0
Textbook/ChapterA3s 4  Textbook/ChapterA3t 4  Textbook/ChapterA3u 8  Textbook/ChapterA3v 4
Textbook/ChapterA4g 15                  Toolbox/ChapterHermiteBandCalculusHigher 51
WeylSL2/ChapterWeylSL2Group 17          WeylSL2/ChapterWeylSL2Unipotent 75
YangMillsFriedrichs 0                   YangMillsHermite 0   _scratch_harm 0

PROB (48 modules, 515 decls)
Attention/ChapterAttentionCalibration 6  Attention/ChapterAttentionFreeEnergy 8
Attention/ChapterAttentionMarkov 17      Attention/ChapterAttentionMasking 18
Attention/ChapterAttentionPrior 10       Attention/ChapterAttentionTemperature 9
Attention/ChapterCoherentGeometry 8      Attention/ChapterDeepLearningEnsemble 4
Attention/ChapterDeepLearningMAP 6       Attention/ChapterDeepLearningSampling 8
Attention/ChapterSoftmaxBorn 17          Attention/ChapterSoftmaxDivergence 11
Attention/ChapterSoftmaxMaxEntropy 8     Attention/ChapterSoftmaxSharpness 14
Attention/ChapterThermalMaxEntropy 7     ChapterBaryonAsymmetry 8
ChapterBijectionProbability 10           ChapterComputableScarcity 8
ChapterCrossEntropyGradient 11           ChapterDoubleSlit 9
ChapterDutchBook 18                      ChapterE2 14
ChapterEntropy 9                         ChapterEntropyTemperature 9
ChapterEulerComplexQuat 10               ChapterEulerCountableChain 15
ChapterEulerDensityMatrix 15             ChapterEulerStochastic 10
ChapterFiniteArithmeticPrior 7           ChapterFiniteBayesHierarchy 8
ChapterGaugeMechanicsCharge 26           ChapterGaugeParametrization 10
ChapterHierarchicalBayes 12              ChapterHierarchicalBayesComposition 14
ChapterIrreversible 15                   ChapterLogPartitionConvex 7
ChapterMaxEntropy 11                     ChapterNoBestPrior 4
ChapterPriorDependence 6                 ChapterPriorOdds 5
ChapterSequentialBayes 8                 ChapterSqueezedGaussStates 29
ChapterStatisticalModelTheory 10         ChapterTotalVariance 14
ChapterUniformPrior 6                    ChapterUniformPriorPosterior 7
Density/ChapterDensityMarginalConditional 7  Reconstruction/ChapterSymmetryEntropy 12
```
