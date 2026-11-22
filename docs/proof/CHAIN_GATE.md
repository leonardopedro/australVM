# PC07 — the frozen chain and its gate

[COMPOSITION.md](COMPOSITION.md) proves the two-edge composition,
[SOURCE_IR.md](SOURCE_IR.md) reflects the clamp body, [PARSED_WGSL.md](PARSED_WGSL.md)
reflects the shader and transports the bound, and [GPU_PLAN.md](GPU_PLAN.md)
applies the same composition to G13's memory plan. This page is the first
**whole-chain** gate: one frozen fixture that declares the cut and the proof
graph, and one checker that replays the artifacts and refuses any disagreement.

- Fixture: `mm0/proof_chain/chain.toml` (declaration), assembled from
  `word.mm0` → `relation.mm0` → `clamp_source.mm0` → `clamp_wgsl.mm0` →
  `clamp_transport.mm0` with their `.mm1` proofs.
- Checker: `scripts/proof-chain check | report | selftest`.
- The assembled theory is `3 sorts, 21 term/def, 33 ax/thm`; nothing in the
  fixture is new mathematics — it is the artifacts above, measured together.

## The cut, as declared and as measured

| layer | function | proved bound | link from the previous layer | recorded identity |
|---|---|---|---|---|
| source | `source_clamp` | `clamp_upper` | — | `source_sha256` |
| ir | `ir_clamp` | `ir_inherits_source_bound` | `ir_clamp_matches_source` | `source_sha256`, `ir_sha256` |
| shader | `wgsl_clamp` | `wgsl_inherits_source_bound` | `wgsl_clamp_matches_ir` | `wgsl_sha256`, `ir_sha256` |

The checker does not take these rows on trust: for each subject it requires the
function to be a declared `def`, the bound theorem's conclusion to be literally
`ule (<function> x) u100`, the link theorem's conclusion to be
`ueq (<function> x) (<previous function> x)` for one shared variable, and the
subject's digest key to be present in the artifact header. `ir_sha256` is
recorded by three artifacts (source reflection, WGSL reflection, transport) and
must be the same digest in all of them — that is the fixture's measured
"same KernelIR on both sides of the cut" fact, not a comment. A changed shader
moves `wgsl_sha256`; a changed IR moves `ir_sha256` in all three.

## The declared proof graph

`[[applies]]` rows hold each theorem's dependency set to the names its proof
term mentions. Proof-position arguments count: the transport's proof is
`(transfer_through_eq (transfer_through_eq wgsl_inherits_source_bound edge) hn)`,
so it depends on PC05a's `wgsl_inherits_source_bound` and on PC03's
`transfer_through_eq` — a graph that counted only head applications would miss
the reuse, which is exactly the property this card has to demonstrate. PC06's
cache (`mm0/proof_chain/gpu_plan.closure`) uses the same definition — a bare
`name` in an `applies` cell is a mention, `name@n` an application — and its own
run holds the rule to the artifact where it bites (`clamp_transport`'s entry),
so the two graphs cannot disagree about the same proof without one of the two
gates failing.

The entry theorem is `transported_bound`:

```text
transported_bound (x i j y len n: u32){edge, hw, he, hb, hn}: ule n u100
```

Its transitive closure reaches two unproved word axioms (`ueq_sub_le`,
`umeet_lb_r`) and its own five premises, and the checker requires both lists to
match the manifest exactly — a new axiom leaf, a dropped dependency, or a
premise added to the statement is a failure, not a footnote.

## The residual register

| premise | class | meaning |
|---|---|---|
| `wgsl_edge x y` | interface-condition | `y` is the shader's value for this invocation; the caller establishes it |
| `wrapper_admits i` | runtime-premise | the wrapper admits the invocation (roles, guard, freshness) |
| `extent_guard i len` | runtime-premise | the runtime extent premise (`i <= arrayLength`) |
| `binding_index i j` | runtime-premise | the binding/index correspondence |
| `native_agrees y n` | external-model | the device returns the value the shader computes |

Every entry premise must have a row and every row must name an entry premise, so
the register is the statement's premise list — it cannot grow or shrink silently.
The classes are what separate the three kinds of debt the plan asks for:
`interface-condition` is a hypothesis on this invocation's data,
`runtime-premise` is supplied by the wrapper at run time, and `external-model`
is the one assumption the transport proof actually consumes (`hn`). Logical
gaps — statements with no proof — cannot appear here at all: MM0 refuses them
before the checker looks.

## Modes

`check` is the full policy: replay, graph, cut, shared digests, exact residual
closure, and the residual classes of `--profile` (`checked` allows all three
classes; `certified` allows only `interface-condition`, so the external model
assumption is a hard failure there). Exit 1 names every disagreement; exit 3
means `mm0-rs`/`mm0-c` are absent and prints a `[gap]` — never a pass.

`report` runs the same replay and the same structural checks and prints the
chain as the conditional theorem it is: the cut with its links, the entry
statement, each premise with its class, the axiomatic leaves, and a verdict of
`conditional`. It does not apply the profile policy, so a chain that `check
--profile certified` refuses still reports — a conditional theorem is a result,
a certificate is a policy.

`selftest` is the card's negative controls, in a scratch copy of the fixture:

| control | expected |
|---|---|
| a changed statement (`u100` → `u99` in the shader bound) | `mm0-c` refuses the replay |
| an incomplete proof file (the entry's proof removed) | `mm0-rs` refuses the proof file |
| a forged record (the entry's proof replaced by `ueq_refl`) | `mm0-rs` refuses the proof term |
| the shader subject deleted from the declaration | `the subject cut is declared as required` |
| a declared graph row that drops `wgsl_inherits_source_bound` | `the declared proof graph matches the replay` |
| the `native` residual row deleted | `each entry premise is classified` |
| one hex digit of `ir_sha256` changed in the WGSL artifact | `shared digests agree across the artifacts` |
| `check --profile certified` on the intact chain | `the residual classes satisfy the profile` |
| `report --profile certified` on the same chain | exit 0, `verdict: conditional` |
| `check` with no `mm0-rs`/`mm0-c` on `PATH` | exit 3, `[gap] PC07` |

A control that stops refusing is a failure, and so is one that refuses for the
wrong reason: each is matched against the specific check name it must trip.

## Reproduce

Bounded and cheap — no Lean corpus, no GPU, no OCaml build:

```sh
MM0=$(nix build "git+file://$PWD#mm0" --no-link --print-out-paths)
PATH="$MM0/bin:$PATH" python3 scripts/proof-chain check
PATH="$MM0/bin:$PATH" python3 scripts/proof-chain report
PATH="$MM0/bin:$PATH" python3 scripts/proof-chain selftest
```

`scripts/verify-invariants` runs `check` and `selftest` and prints `PC07` lines;
without the toolchain both are explained gaps.

## What this is not

- **Not GPU instruction certification.** The chain ends at a device value, not
  at emitted GPU instructions; the plan's native end-to-end claim is PC08+.
- **Not a verified parser or decoder.** The AST/IR/WGSL shapes are *recorded*
  reflections (`-- trusted-edge:` in the artifacts) and the digests pin the
  bytes those reflections were taken from; no parser or typechecker is proved.
- **Not an approval.** Nothing here is a W3 approval; the artifacts this page
  assembles are the ones the earlier cards own, and the entry theorem remains
  conditional on the four runtime/model premises above.
