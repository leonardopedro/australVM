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
never a pass.

## Still open

- The parser→model reflection is trusted. A checked Naga decoder is a separate
  artifact (PC05a's own instruction: record the edge, do not claim it).
- Only this clamp shape is reflected: one scalar helper, one comparison, one
  literal, width 32. A second literal needs the shared word theory extended
  first (`u100` is the only pinned word constant).
- The reduction's modular sum law is PC05b's, not this page's.
- The MM0 replay of `clamp_wgsl.mm0` has **not** been run on a host without the
  MM0 toolchain; it is a recorded gap until `mm0-rs`/`mm0-c` run it green.
