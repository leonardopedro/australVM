# Bounded-fold source profile (`bounded-fold-v1`)

**Status, 2026-11-19.** G17a admitted and probed the shape; G17b expands it
into KernelIR and publishes ordinary bundles. The loop is gone from the
emitted kernel: it is one immutable KernelIR `let` per body execution, checked
against the node budget before it is built. No loop construct is added to
KernelIR, the bundle schema, or WGSL.

The source profile is separate from the numeric profile: `u32-modular-v1` /
`wgsl-f32-v1` say which arithmetic may appear, `bounded-fold-v1` says whether
one bounded loop may appear in the entry. It is opt-in and never inferred.

```sh
./austral compile examples/gpu/bounded-fold/Fold.aui,examples/gpu/bounded-fold/Fold.aum \
    --target-type=wgsl --gpu-source-profile=bounded-fold-v1 \
    --entrypoint=Fold:upTo31 --output=/tmp/fold-bundle
```

The profile name is recorded in the bundle's `evidence.source_profile`
(`v1` for the strict profile), so a published artifact says how its entry was
admitted. The Rust reader treats the field as optional: bundles written before
G17b still load.

## Admitted shape

Decided on the typed, monomorphic AST — resolved local IDs, types and literals,
never on source text:

* the entry keeps the v1 signature (one or two `Nat32` parameters, `Nat32`
  result);
* exactly one private accumulator, introduced by `var acc: T := …` **before**
  the loop;
* exactly one `for i from 0 to K` in the entry body, with `K` a compile-time
  literal in `0..31`; a second loop, a loop after the first, or a loop in a
  helper is refused;
* the loop body is exactly one assignment to that accumulator, from a v1
  scalar expression (the compiler's one-value temporary for the assigned
  expression is allowed);
* the entry returns the accumulator itself, once, at the end.

`LiftControlPass` rewrites the bounds into `Index`-typed compiler temporaries,
so the pass resolves those temporaries to their literal initializer. An `Index`
temporary without a literal is a runtime bound and is refused.

Refused by name: a literal bound above 31, a start other than 0, a second
mutable binding, a second write, nested or repeated loops, loops in helpers,
conditionals and early returns, `while`, `match`, borrows, discarded
expressions and effects, and an assignment through an address.

## Expansion

For an admitted fold with bound `K`:

1. The initializer is the first KernelIR `let` (the accumulator's first local).
2. Each body execution becomes one `let` of a **fresh** local that shadows the
   accumulator's source name; the update expression reads the previous local,
   so the KernelIR is a straight line of immutable bindings.
3. Statements after the loop read the last local, and the entry's `return`
   yields it.

Work is projected before the body is built: the expansion costs
`(K+1) × (1 + IR nodes of the update + the body size of every helper it calls)`.
A projection over the 100000-node budget is refused (message names the
projected nodes and the iteration count), and helper calls are counted per
iteration even though their definitions are shared. The expanded body is the
same KernelIR the emitters already handle, so the bundle format, the Rust
schema, the reference evaluator and the memory plan are unchanged.

This is unrolling, not a GPU loop: bundle size grows with `K`, bounded by the
32-iteration profile limit and the node budget.

## The endpoint is inclusive

Austral's specification and the C renderer (`lib/CRenderer.ml` renders
`i <= K`) execute the body `K+1` times. The CPS lowering used to emit `CmpLt`
in the `Mt.MFor` case, so it executed `K` times — `for i from 0 to 0` did
nothing under CPS and one update under C. That lowering now emits `CmpLte`
(`lib/Compiler_cps.ml`), and its inclusive behaviour is measured both ways:

* statically, `test/CpsLoopLoweringTest.ml` pins the emitted relation for
  both a hand-built `MFor` and real source (and fails 3/3 when the lowering
  is switched back to `CmpLt`);
* by execution, the JIT server returns the number of body executions:
  `countZero` (`to 0`) → `RESULT -1`, `countFour` (`to 4`) → `RESULT -5`,
  `countTen` (`to 10`) → `RESULT -11`, each function accumulating
  `trappingSubtract(acc, one)` once per iteration. Before the fix the same
  calls returned `0` for `to 0`.

G17b expands the same **inclusive** endpoint (`K+1` body executions), so C,
KernelIR/WGSL and the CPS JIT now agree on the iteration count.

```sh
# the JIT measurement (tc + --jit-server, so the CPS functions actually run)
printf 'call countZero\ncall countFour\ncall countTen\nexit\n' | \
  ./austral compile /tmp/Loops.aum --target-type=tc --jit-server --allow-all
# RESULT -1, RESULT -5, RESULT -11
```

## Fixture and measured evidence

`examples/gpu/bounded-fold/Fold.{aui,aum}`: `upToZero(x)` adds `x` once and
`upTo31(x)` 32 times, both with an unused index.

```sh
# admission probe, expansion counts, budget refusal (27 tests)
nix develop "git+file://$PWD#ocaml" -c dune exec test/GpuSubsetTest.exe
# [probe] G17a upToZero: binder=acc init=0 count=1 update=Intrinsic(Add,[Local 0;Param 0]) result=Local 0
# Ran: 27 tests ... OK

# C is the oracle (inclusive endpoint): 1 update and 32 updates at x = 7
./austral compile examples/gpu/bounded-fold/Fold.aui,examples/gpu/bounded-fold/Fold.aum \
    --target-type=c --entrypoint=Fold:main --output=/tmp/fold-c/Fold
cc -x c -O0 -o /tmp/fold-c/Fold.exe /tmp/fold-c/Fold -lm && /tmp/fold-c/Fold.exe
# 7
# 224

# the same oracle is a gated end-to-end suite
python3 test-programs/runner.py 026-gpu-bounded-fold
# 026-gpu-bounded-fold  001-c-oracle  PASS

# published bundles carry the profile; the WGSL is 33 lets for K=31
./austral compile … --target-type=wgsl --gpu-source-profile=bounded-fold-v1 \
    --entrypoint=Fold:upTo31 --output=/tmp/fold-bundle-31
# evidence.source_profile = "bounded-fold-v1", 33 `let`s (initializer + 32)

# reference and physical device agree at a ragged length
cat > /tmp/fold-inputs.json <<'EOF'
{"inputs": [[3, 7, 11, 4294967290]]}
EOF
avm-gpu reference /tmp/fold-bundle-31 --inputs /tmp/fold-inputs.json
# output [96, 224, 352, 4294967104]
avm-gpu run /tmp/fold-bundle-31 --inputs /tmp/fold-inputs.json --require-hardware
# backend wgpu, Intel(R) UHD Graphics 630 (CFL GT2), Vulkan, exit 0
# output [96, 224, 352, 4294967104]

# a bound of 0 is the identity (one update), on the device too
avm-gpu run /tmp/fold-bundle-0 --inputs '{"inputs": [[3,7,11]]}' --require-hardware
# output [3, 7, 11]
```

Mutation controls (each measured to change the result):

* changing `upTo31`'s literal bound from 31 to 1 gives
  `[6, 14, 22, 4294967284]` instead of `[96, 224, 352, 4294967104]`;
* changing that fold's update from `modularAdd(acc, x)` to
  `modularSubtract(acc, x)` gives `[4294967200, 4294967072, 4294966944, 192]`
  — the same 32 executions, a different operand.

A dynamic bound (`to x`) never reaches this pass: a loop bound must be an
`Index` expression, and a `Nat32` parameter cannot be cast to it, so the
frontend refuses first (`Cannot cast to Index from Nat32`).

## Not claimed

* A CPS/JIT proof of loop equivalence: the endpoint now agrees by
  measurement (above), not by a verified translation.
* A GPU loop instruction: expansion is static, so a `K`-fold kernel is
  `K+1` KernelIR nodes, not a loop the device iterates.
* Admission is a shape check, not a work bound on the per-iteration expression
  beyond the existing KernelIR node budget.
