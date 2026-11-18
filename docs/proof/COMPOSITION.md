# PC03 relation, composition and premise discharge

This is the composition layer over [SEMANTICS.md](SEMANTICS.md). It states what
a lowering edge is, proves the identity and two-edge composition, and shows the
same composition twice: once with the IR bound left as an explicit pending
premise, and once with that exact premise discharged by PC02's `clamp_upper`.

The specification is `mm0/proof_chain/relation.mm0`; the checked proofs are
`mm0/proof_chain/relation.mm1`. `scripts/pc03-compose` assembles and checks
them; `scripts/verify-invariants` runs that script.

## Assembly, not import

MM0 has no import form: a specification is one file. PC03 therefore reuses PC02
by **assembly** — `scripts/pc03-compose` concatenates `word.mm0` +
`relation.mm0` and `word.mm1` + `relation.mm1` and checks the concatenation with
`mm0-c`. `relation.mm0` is a fragment with no `provable sort`, `delimiter`,
`sort`, or `axiom` of its own, and the check refuses it if one appears: the word
carrier, `source_clamp` and `clamp_upper` can only come from the shared prefix.
The assembled theory is `3 sorts, 14 term/def, 27 ax/thm`, and the 8 new
theorems are PC03's. `compose_bound` is the shared algorithm theorem that
[GPU_PLAN.md](GPU_PLAN.md) (PC06) applies a second time, at G13's launch size.

## Edges and observation

An edge is a *definition*, not an uninterpreted symbol with a premise-free rule:

```text
edge_src_ir x i  =  (source_clamp x = i)      -- source → admitted IR
edge_ir_tgt  i y =  (i = y)                   -- IR → target observation
observes     x y =  (x = y)
```

The equality is oriented left-to-right because PC02's order law
(`ueq_sub_le`) rewrites the left argument of `ule`; this is why the edges read
`source_clamp x = i` rather than the other way round. A changed edge has to
change this equation and re-check, rather than being granted by a new symbol.

`step3` is the trace vocabulary: the three states before and after one step.
Nothing assumes a step preserves the bound. `step_transfer` consumes the
*successor* edge facts explicitly, so a trace is a chain of checked edges and
the step hypothesis only restricts which states the lemma applies to.

## What is proved

| Theorem | Claim |
|---|---|
| `transfer_through_eq` | `p <= b`, `p = q` ⊢ `q <= b`, for an arbitrary bound `b` |
| `compose_bound` | two equality edges (`x = i`, `i = y`) and `x <= b` ⊢ `y <= b` |
| `identity_transfer` | the identity edge at `b := u100` (parameter instantiation) |
| `edge1_bound` | `edge_src_ir x i` ⊢ `i <= 100`, from PC02's `clamp_upper` |
| `compose_conditional` | two edges plus the pending premise `i <= 100` ⊢ `y <= 100` |
| `compose_discharged` | the same two edges, no pending premise ⊢ `y <= 100` |
| `step_transfer` | the bound after a step whose successor states satisfy both edges |
| `observes_chain` | the target observes the source clamp's result |

`transfer_through_eq` is stated at an arbitrary `b`, so `identity_transfer` is
an instantiation of it, not a second assumption. `compose_bound` composes the
two edges once at any bound; `compose_discharged` is that theorem at
`b := u100` with `clamp_upper` as the source bound. `compose_conditional` has
three premises, one of them the pending edge premise `ule i u100`;
`compose_discharged` has two. The script prints both counts and requires
exactly one premise to disappear.

## Trust boundary

The fragment adds **no axioms**. Everything new is a term, a definition or a
theorem proved from `word.mm0`'s order laws and `clamp_upper`. It does **not**
claim:

- that a real lowering edge has these equations (PC04/PC05);
- that the IR bound `ule i u100` holds for a compiled module before the
  discharge is applied — that is the pending premise, and it is a hypothesis
  until then;
- that a source or target *step* preserves anything; only that when the
  successor states satisfy both edges, the bound transfers;
- any safety or progress property beyond the single upper bound;
- that the assembled specification is a general theory of traces: a trace is a
  finite chain of these step lemmas, and there is no induction principle here.

Finite tests are controls. `scripts/pc03-compose` also samples words to check
the discharged chain stays in bounds and to exhibit the necessity witness
(`x = 5, i = 101, y = 101` satisfies `edge_ir_tgt` but not the bound), which is
why the conditional statement cannot drop its pending premise.

## What the controls refuse

The script mutates the checked artifacts and requires refusal:

1. replacing the pending premise with the unrelated `ueq i i` — `mm0-c` refuses
   the existing proof;
2. using the source edge `h1` where the pending premise is needed — `mm0-rs`
   refuses the proof term;
3. applying `compose_bound` to an edge fact where the bound belongs
   (`compose_bound (h1) (h2) (h1)`) — the third component is not a bound and
   the proof is refused.

A control that stops refusing is a failure. Both the conditional and the
discharged statements are checked by `mm0-c` before these mutations run, so a
proof that no longer proves the statement is caught even if the mutations still
fail.

## Reproduce

```sh
nix shell '.#mm0' -c bash -c '
  cat mm0/proof_chain/word.mm0 mm0/proof_chain/relation.mm0 > /tmp/all.mm0
  cat mm0/proof_chain/word.mm1 mm0/proof_chain/relation.mm1 > /tmp/all.mm1
  mm0-rs compile /tmp/all.mm1 /tmp/all.mmb && mm0-c /tmp/all.mmb < /tmp/all.mm0
'
python3 scripts/pc03-compose
```

## Still open

- Source reflection (the parsed Austral body really is `source_clamp`) is PC04.
- The actual IR→WGSL/Naga edge and its model are PC05; this file's target is
  the admitted observation, not a shader.
- The edge equations here are the clamp chain's. The second application, to
  G13's memory plan, is [GPU_PLAN.md](GPU_PLAN.md) (PC06); any further
  application needs its own edges and its own checked composition.
