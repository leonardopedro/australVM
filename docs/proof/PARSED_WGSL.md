# PC05a — the parsed WGSL clamp bound to KernelIR

[SOURCE_IR.md](SOURCE_IR.md) reflects the checked typed AST and the KernelIR.
This page reflects the third subject: the **parsed Naga module** the runtime
actually validates, i.e. the `kernel.wgsl` the pipeline publishes — not a
retyped model of the sample and not the WGSL text read as a string.

Generator: `avm-gpu reflect-wgsl <bundle>` (Rust, `safestos/gpu/src/
wgsl_reflect.rs`). Specification: `mm0/proof_chain/clamp_wgsl.mm0` (assembled
after `word.mm0` and `clamp_source.mm0`); proofs: `.mm1`; gate:
`scripts/pc05-reflect`; tests: `safestos/gpu/tests/wgsl_reflect.rs`.

## What is reflected

`austral compile --target-type=wgsl --entrypoint=Pointwise:clamp` publishes
this shader for the fixture:

```wgsl
fn scalar_entry(p0: u32) -> u32 {
    let v0: bool = (p0 > 100u);
    if v0 {
        return 100u;
    } else {
        return p0;
    }
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&input0)) { return; }
    output0[i] = scalar_entry(input0[i]);
}
```

The reflection reads, from the parsed module:

- the compute entry (`main`) and the single function it calls
  (`scalar_entry`), with the call occurring exactly once;
- that helper's one parameter — its name, position and width (32 bits; Naga
  records bytes, the reflection records bits) — and its unsigned result;
- the condition `p0 > 100u`: the comparison direction and the literal, with the
  parameter as the left operand;
- the two branch returns: the high branch returns the compared literal, the low
  branch returns the parameter;
- the observable: the call result is stored into the `read_write` storage array
  at the same index the `read` array was loaded from (identity output index),
  with the workgroup size recorded from the entry.

Then it requires the same facts from the bundle's `kernel_ir` (`If (Compare
(GreaterThan, Param 0, U32 100), Return (U32 100), Return (Param 0))`, with the
condition resolved through the compiler-hoisted `let`). The two reflections
must agree on comparison, literal, branch value, parameter index, width and
workgroup size; `witness` refuses any disagreement.

The emitted fragment adds one definition and two theorems to PC04's:

```text
def wgsl_clamp (x: u32) = umeet x u100
wgsl_clamp_matches_ir        : wgsl_clamp x = ir_clamp x
wgsl_inherits_source_bound   : wgsl_clamp x <= u100
```

Both definitions unfold to the reflected operator, so the witness is
reflexivity and the bound is PC02's meet lower bound.

## The wrapper is checked separately

The reflection answers "what does the scalar helper compute". The storage roles,
the bounds guard and the freshness of the output store are a different question,
answered by the existing wrapper audit (`safestos/gpu/src/wrapper.rs`), which
`avm-gpu reflect-wgsl` runs first and reports as its own field
(`"wrapper_audit": "pass"`). A role change is refused twice over: the audit
refuses `input0` as `read_write` ("input binding 0 is not read-only") before the
reflection is reached, and the reflector refuses a set of storage globals that
is not one read array plus one read_write array.

## The trusted edge

Parsing the shader and reading the helper's nodes as this fragment is
**recorded, not proved** — the same status PC04's AST/IR edge has:

```text
-- trusted-edge: parsing the shader and reading the scalar helper's nodes
--   above as this fragment is recorded here, not proved. ...
```

A digest never turns a mismatched correspondence into a pass.
`avm-gpu check-reflect` re-parses the bundle's WGSL, re-reflects both sides, and
compares the artifact's `-- reflect-wgsl:` line and its `wgsl_clamp` definition.
Both recorded digests (`-- wgsl_sha256=`, `-- ir_sha256=`) must be the current
subjects', so an artifact that was regenerated from a changed shader is refused;
and because the comparison is on the reflected lines rather than the digests, an
artifact whose digest *matches* its own mutated text is refused too.

## Regenerating and checking

`scripts/pc05-reflect` needs three things, and it names whichever is missing
before skipping anything (exit 3, which the gate reports as a `[gap]` with that
same line):

- the compiler, at `_build/default/bin/austral.exe` (`dune build bin/`);
- `safestos/gpu`'s debug checker, at
  `safestos/gpu/target/debug/avm-gpu` (`cargo build --bin avm-gpu`);
- `mm0-rs` and `mm0-c` for the replay, which `devShells.default`, `.ocaml` and
  `.lean` all carry as the flake's `packages.mm0` — outside those shells,
  `nix build "git+file://$PWD#mm0"` and put its `bin/` on `PATH`.

```bash
# Emit from a fresh bundle (the gate uses this path).
./_build/default/bin/austral.exe compile \
  examples/gpu/pointwise/Pointwise.aui,examples/gpu/pointwise/Pointwise.aum \
  --target-type=wgsl --entrypoint=Pointwise:clamp --output=/tmp/clamp.avkg
safestos/gpu/target/debug/avm-gpu reflect-wgsl /tmp/clamp.avkg --out mm0/proof_chain

# Re-reflect and compare a committed artifact without regenerating it.
safestos/gpu/target/debug/avm-gpu check-reflect /tmp/clamp.avkg \
  --artifact mm0/proof_chain/clamp_wgsl.mm0

# The whole gate, including the mutation controls and the MM0 replay.
python3 scripts/pc05-reflect
```

`scripts/pc05-reflect` re-emits the artifact and requires byte identity with the
committed copies, then mutates the parsed shader three ways — reversed
comparator, changed literal, changed storage role — recomputing `wgsl_sha256` in
the bundle so it still loads, and requires a refusal for each. With
`mm0-rs`/`mm0-c` present it assembles and replays the chain and requires a
`umeet`→`ujoin` proof copy to be refused; without them it reports a `[gap]`,
never a pass. The toolchain comes from the flake:

```bash
MM0=$(nix build "git+file://$PWD#mm0" --no-link --print-out-paths)
PATH="$MM0/bin:$PATH" python3 scripts/pc05-reflect
#   [detail] word+clamp_source+clamp_wgsl assembled and accepted
#   [detail] a reflected ujoin does not check the umeet proof
#   [pass] PC05a parsed-WGSL reflection of the real clamp
```

## The bound travels through the edge (PC05b)

`mm0/proof_chain/clamp_transport.mm0` takes the bound that PC05a proved of the
parsed shader and moves it to the value the device returns:

```text
wgsl_edge                      : wgsl_clamp x = y          -- the target is the shader's value
extent_guard i len              : i <= len                  -- the runtime extent premise
binding_index i j              : i = j                     -- the binding/index correspondence
native_agrees y n              : y = n                     -- the device returns the shader's value
shader_edge_bound              : y <= u100
shader_edge_bound with native  : n <= u100
```

Both steps are PC03's `transfer_through_eq`, the first applied over PC05a's
`wgsl_inherits_source_bound`, so the earlier theorem is visible in the proof
term rather than in a comment — with that proof removed the transport no longer
compiles, which is the `drop-import` control.

### Four premises, one register, a strict profile

`transported_bound` carries all four residual premises as hypotheses:

| premise | term | consumed by |
| --- | --- | --- |
| wrapper | `wrapper_admits i` | PC06's bundle claim |
| extents | `extent_guard i len` | PC06's bundle claim |
| bindings | `binding_index i j` | PC06's bundle claim |
| native | `native_agrees y n` | the transport proof |

Only `native` is consumed here; the other three are *recorded*, and the artifact
says so instead of pretending otherwise. `wrapper_admits` is deliberately
uninterpreted — nothing in this fragment can prove it, and no axiom is allowed
to make it look proved.

The **strict profile** (`scripts/pc05b-transport` without the toolchain) refuses
an artifact that shrinks those obligations: a missing premise row, a premise
re-labelled onto another term, a premise dropped from the statement, a premise
"proved" by a theorem of the artifact, a third theorem, an `axiom`, a changed
`-- subject:`, or a `-- bound:` that disagrees with the reflected literal. The
replay refuses the rest: removing the consumed premise from the statement or the
proof, or moving the edge onto another operator, no longer checks.

No Why3 arithmetic goal corresponds to these premises — the GPU chain has no
Why3 artifact — so nothing is left green on a prover assertion; the bound is
MM0-discharged by PC02 and PC05a.

### A changed shader, a stable statement

The transport records the edge it came from (`-- edge_sha256=`) and the edge's
own subjects (`-- wgsl_sha256=`, `-- ir_sha256=`). A changed shader moves that
evidence and the transport must be re-emitted; the theorem statements are
byte-identical, because they are stated over `wgsl_clamp` and the reflected
bound rather than over a digest. The script measures both halves.

```bash
MM0=$(nix build "git+file://$PWD#mm0" --no-link --print-out-paths)
scripts/pc05b-transport --write      # regenerate after clamp_wgsl.mm0 moves
PATH="$MM0/bin:$PATH" python3 scripts/pc05b-transport
#   [pass] PC05b transport of the clamp bound through the reflected edge
```

## Still open

- The parser→model reflection is trusted. A checked Naga decoder is a separate
  artifact (PC05a's own instruction: record the edge, do not claim it).
- Only this clamp shape is reflected: one scalar helper, one comparison, one
  literal, width 32. A second literal needs the shared word theory extended
  first (`u100` is the only pinned word constant).
- The reduction's modular sum law is PC05b's, not this page's.
- The MM0 replay is a `[gap]`, never a pass, on a host without `mm0-rs`/
  `mm0-c`. It has been run green with the flake's `mm0` package (the assembled
  theory is `word` + `clamp_source` + `clamp_wgsl`, and the `ujoin` control is
  refused).
- Three of PC05b's four premises are recorded, not discharged; PC06's bundle
  gate is what consumes them, and the reduction's modular sum law follows later
  as the card says.
