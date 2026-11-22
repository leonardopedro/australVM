# PC04 — the real clamp source bound to KernelIR

[COMPOSITION.md](COMPOSITION.md) proves the composition and
[SEMANTICS.md](SEMANTICS.md) defines the word/map model. This page records the
first *reflection*: the checked typed body of `Pointwise.clamp` and the
KernelIR the subset pass actually built from it, tied to the modeled
`source_clamp` and to a proved bound.

Generator: `bin/gpu_proof`. Specification: `mm0/proof_chain/clamp_source.mm0`
(assembled after `word.mm0`); proofs: `.mm1`; gate:
`scripts/pc04-reflect`; reflection test: `test/GpuProofInterfaceTest.ml`.

## What is reflected

`examples/gpu/pointwise/Pointwise.aum` contains:

```austral
function clamp(x: Nat32): Nat32 is
    if x > 100 then
        return 100;
    else
        return x;
    end if;
end;
```

`Gpu_proof_interface.of_typed` reads the **checked monomorphic AST** of that
function and `of_ir` reads the **actual KernelIR** the subset pass produced:

```text
typed AST   MIf (MComparison (GreaterThan, MParamVar x, MIntConstant 100),
                 MReturn (MIntConstant 100), MReturn (MParamVar x))
KernelIR    If (Compare (GreaterThan, Param 0, U32 100),
                Return (U32 100), Return (Param 0))
```

Both stages hoist the comparison into a temporary/`let`, so the reflection
resolves those bindings before matching. `witness` requires the two
reflections to agree on the comparison, literal, branch value and parameter;
a disagreement is refused rather than reflected.

The emitted fragment defines the IR denotation and proves two facts:

```text
def ir_clamp (x: u32) = umeet x u100
ir_clamp_matches_source   : ir_clamp x = source_clamp x
ir_inherits_source_bound  : ir_clamp x <= u100
```

The first is reflexivity after unfolding (both definitions are the reflected
operator), the second is PC02's meet lower bound. The assembled theory is
`3 sorts, 11 term/def, 21 ax/thm`.

## The trusted edge

The mapping **from AST constructors and IR nodes to MM0 names** is recorded on
the artifact and is not proved:

```text
-- trusted-edge: the typed-AST and KernelIR nodes above name the two semantic
--   objects; mapping them to `source_clamp`/`ir_clamp` is recorded here, not
--   proved.
```

So the reported cut is: *parser/typechecker accepted this body* (checked
frontend), *the subset pass produced this IR* (checked by construction), and
*these AST/IR shapes denote `umeet x u100`* (trusted, recorded). What is proved
is the bound above that cut. No claim is made that the parser, typechecker or
decoder is verified.

## How the binding fails when it should

- **Reversed source condition.** Changing the fixture to `if x < 100` makes the
  real reflection produce `ujoin`, and the emitted `umeet_lb_r` proof no longer
  applies. `test/GpuProofInterfaceTest.ml` compiles the reversed fixture and
  checks the operator; `scripts/pc04-reflect` mutates the committed proof's
  `umeet` to `ujoin` and requires `mm0-rs` to refuse (mm0-c does not re-verify
  `def` bodies, so the control is on the proof that consumes the operator).
- **Wrong parameter mapping.** Tampering the IR comparison to read `Param 1`
  while the else branch reads `Param 0` makes `of_ir` refuse; the test asserts
  that refusal.
- **Changed literal or comparison in the IR.** `witness` refuses the mismatch.
- **Digest-only changes.** Digests are comments. `check_artifact` compares the
  recorded `-- reflect:` line and the `ir_clamp` definition against a fresh
  reflection, so editing a digest (which the test does) cannot make a false
  correspondence pass.
- **Stale fixture.** `scripts/pc04-reflect` recomputes
  `sha256(Pointwise.aui ++ Pointwise.aum)` and refuses a recorded
  `source_sha256` that no longer matches.

The reflection is structural on this one fixture shape: the condition must be
`<param> <cmp> <literal>` with the literal branch returning the literal and the
other returning the parameter. Other shapes are refused, not generalized; the
literal must be `100` because that is the constant PC02 binds to `u100`.

## Commands

```sh
# Regenerate the committed artifact from the real fixture and KernelIR.
LD_LIBRARY_PATH="$PWD:$PWD/safestos/cranelift/target/release" \
  nix develop "git+file://$PWD#ocaml" -c dune exec bin/gpu_proof.exe -- \
  emit examples/gpu/pointwise/Pointwise.aui \
       examples/gpu/pointwise/Pointwise.aum clamp mm0/proof_chain

# Re-check the committed artifact (gate uses this).
nix shell '.#mm0' nixpkgs#python3 -c python3 scripts/pc04-reflect

# Re-reflect and compare against the artifact without regenerating it.
... dune exec bin/gpu_proof.exe -- check <aui> <aum> clamp \
      mm0/proof_chain/clamp_source.mm0
```

`test/GpuProofInterfaceTest.ml` requires the committed files to be
byte-identical with a fresh emission, so a stale artifact is a red test.

## Still open

- The reflection edge is trusted; proving a parser/decoder correspondence is
  outside this fragment.
- Only the direct clamp shape is reflected; helpers are not (PC04's plan allows
  one helper only after the direct case).
- The IR→WGSL edge is PC05; this file stops at KernelIR. The parsed-WGSL side
  of that edge is now reflected in [PARSED_WGSL.md](PARSED_WGSL.md), which
  binds `wgsl_clamp` to this page's `ir_clamp`; the wrapper's roles and guard
  are checked separately there.
- Handing the reflected IR to the PC06 composition (so the plan inherits this
  bound) still needs the plan-side reflection, which PC05 supplies.
