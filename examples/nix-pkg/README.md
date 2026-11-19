# nix-pkg — Nix expressions translated to AustralVM, verified, packaged, executed

This directory is the joint test of two tools that were built separately:

* **`dnx-verify`** (`../../../dnx/crates/dnx-verify`) — reduces a Nix
  expression and an AustralVM fragment to DeltaNets and compares their
  canonical UNF digests, reporting the kernel's isomorphism witness when they
  match;
* **`avm-pkg`** (`../../safestos/pkg`) — the content-addressed package
  engine: derivations, signed `.avm` cells, a cold store, an ephemeral hot
  store, and a capability-gated loader.

Each `X.nix` / `Y.aum` pair is one *translation*: the author (an LLM, acting
as the translator) wrote the Nix expression, then wrote the AustralVM module
that must compute the same closed term. Nothing in a pair is generated from
the other side, so the verifier's verdict is a real check of the translation
rather than a re-print of an input.

## The pairs

| pair | Nix expression | AustralVM module | Nix value |
|---|---|---|---|
| `arith` | `(2 + 3) * 4` | `Arith.aum` | `20` |
| `apply` | `(x: x + 1) 41` | `Apply.aum` | `42` |
| `bool` | `1 < 2` | `BoolCheck.aum` | `true` |
| `float` | `1.5 + 2.25` | `FloatSum.aum` | `3.75` |
| `let` | `let base = 40; in base + 2` | `LetExpr.aum` | `42` |
| `version` | `(major: minor: major * 100 + minor) 2 12` | `Version.aum` | `212` |
| `sub` | `10 - 4` | `Sub.aum` | `6` |
| `mul` | `6 * 7` | `Mul.aum` | `42` |
| `nested` | `(x: y: x + y) 3 4` | `Add2.aum` | `7` |
| `not` | `!(1 < 2)` | `Not.aum` | `false` |
| `and` | `(1 < 2) && (2 < 3)` | `And.aum` | `true` |
| `multilet` | `let a = 1; b = 2; in a + b` | `MultiLet.aum` | `3` |
| `neg` | `-5 + 3` | `Neg.aum` | `-2` |
| `precedence` | `1 + 2 * 3` | `Precedence.aum` | `7` (module computes 9) |

`precedence` is a **declared mismatch**: the AustralVM parser is left to
right, so `((1 + 2) * 3)` computes 9 where Nix computes 7. `pairs.json`
carries `"expect": "different"` for it, so the batch passes only because the
verifier reports the difference — and the pair is never packaged. It is the
in-tree proof that a `[ok]` on the other thirteen is a real comparison.

Several pairs share a digest (`apply`/`let`/`mul` all reduce to `42`) because
that is the verifier's equality: canonical normal forms, not source text.
They remain distinct packages because the derivation identity also covers the
name and version.

`pairs.json` is the `dnx-verify` manifest for the set, so the directory can be
checked without this script:

```bash
cd ../../../dnx
./target/release/dnx-verify check --manifest \
  ../australVM/examples/nix-pkg/pairs.json
```

## Run it

```bash
examples/nix-pkg/run.sh
```

The script, in order:

1. **Verifies.** `dnx-verify check --manifest pairs.json` must meet every
   pair's expectation (13 equivalent, `precedence` different). Any mismatch,
   or any pair it cannot compare, aborts.
2. **Runs a second negative control.** `Apply.aum` with its literal `41`
   changed to `42` must be reported *different* with exit 1 (a real
   mismatch, not the exit-2 "could not compare" path).
3. **Writes derivations.** For each packaged pair it asks
   `dnx-verify nix-hash` for the Nix side's canonical net digest and emits
   `target/nix-pkg/plan.json` with one derivation per package: `name`,
   `version = 1.0.0`, `builder_symbol = austral.translate`, and `sources =
   [<the verified Nix net digest>]`. The derivation's *input identity is the
   net the translation was verified against*; `avm-pkg drv-hash` / `build
   --plan --for` compute the Merkle id from it with the engine's own
   calculus (nothing is hand-hashed).
4. **Builds packages.** `avm-pkg build <module>.aum --plan … --for <name>
   --sign <demo seed>` compiles the module (`--use-cps-jit`, totality pass
   on) and `cps2cell`, producing a signed cell stored under the derivation,
   with the input-closure digest recorded beside it.
5. **Round-trips.** Every cell is hydrated back through the hot store with
   `avm-pkg get` and compared byte for byte with the built image.
6. **Executes.** `exec-image.rs` (compiled by the script with plain `rustc`)
   is handed to `avm-pkg load --exec`: the loader gates `Execute` and every
   declared call against the cell's own table, relocates the image at
   `0x10000000`, and spawns the executor in the runner's sandbox with the
   image staged as `cell.json`. The executor maps the image, resolves the
   arithmetic helpers to fixed-address stubs, calls `main`, and prints the
   returned word. Integer and Bool results are read from `rax=`; Float64 is
   an IEEE-754 bit pattern in that same word (the CPS one-word ABI), so the
   line also carries `f64_bits=` and `float` is checked there. Every value is
   compared with the Nix value in the table.

Measured on this host, 2026-11-18 (release `avm-pkg`/`cps2cell`/`dnx-verify`):```
14 pair(s): 13 equivalent, 1 different, 0 error(s); expectations met
[ok] all 13 cells hydrated byte-identically
[ok] arith executes to 20        [ok] apply executes to 42
[ok] bool executes to 1          [ok] float executes to 3.75
[ok] let executes to 42          [ok] version executes to 212
[ok] sub executes to 6           [ok] mul executes to 42
[ok] nested executes to 7        [ok] not executes to 0
[ok] and executes to 1           [ok] multilet executes to 3
[ok] neg executes to -2
```

The store lives in `target/nix-pkg/store` and `target/` is git-ignored, so the
checkout stays clean; `plan.json` there is generated, not committed, because
its source digests are pinned to the kernel's serializer.

### Negative control for the execution step

A green run does not prove the check would notice the defect it was written
for, so `linker-control.sh` mutates the failure back in and rebuilds:

```bash
examples/nix-pkg/linker-control.sh
```

It requires `deny-execute.cedar` to refuse the cell with
`EPERM: capability denied for action "Execute"`, then reverts `link_with`'s
PC-relative fix to the double-subtraction, rebuilds `avm-pkg`, and requires
`load --exec` to stop returning 42 (the pre-fix linker jumps four bytes past
its target). It restores the source, rebuilds, and requires 42 again. The
control keys on the fixed line rather than on git state, so it works with the
fix committed or not, and restores the exact bytes it saved. CI runs it after
`run.sh`, which is what makes the job's execution step non-vacuous.

Environment overrides: `DNX_VERIFY`, `AVM_PKG`, `AVM_AUSTRAL`, `AVM_CPS2CELL`,
`AVM_NIXPKG_WORK`, `AVM_NIXPKG_STORE`, `AVM_NIXPKG_SIGN_SEED`. The default seed
is a fixed demo key — these are test cells, not a release signing key.

CI runs the same script in the `nix-pkg` job of
`.github/workflows/build-and-test.yml`.

## What this test found (and what was fixed for it)

Writing the execution half turned two engine defects into failures, and both
were fixed at the cause rather than worked around:

* **Internal calls were relocated four bytes off.** Cranelift emits
  `X86CallPCRel4` with addend `-4` (the rel32 field size); `avm-exec`'s
  `link_with` added the addend *and* subtracted `site + 4`, double-counting
  the field. Every cell with a call to another function in the same cell
  (`apply`, `nested`, `version`) jumped past its target and segfaulted on
  load. Fixed to the ELF rule `S + A - P`, with the linker's hand-built
  fixture corrected to the convention Cranelift actually uses.
* **`not` was a bitwise complement.** Opcode `0x19` used `bnot`, so
  `not true` returned `-2` — a truthy value — although the decoder's own
  comment and `CpsGen` describe a 0/1 complement. Fixed to `bxor 1`, pinned
  by `not_opcode_complements_zero_and_one` in
  `safestos/cranelift/tests/cps_tests.rs`.
* **Float literals were rounded into integers.** The CPS binary has one
  64-bit literal opcode, so `CpsGen` used to write
  `Int64.of_float (Float.round f)`: `1.5` and `2.25` both became `2`, and a
  Float64 `+` was a call to the *integer* `trappingAdd`. Float64 is now
  carried as its IEEE-754 bit pattern in the same word (`Int64.bits_of_float`,
  exact for fractions/negatives/NaN) and `Compiler_cps` lowers Float64
  `+ - * /` to `au_fadd`/`au_fsub`/`au_fmul`/`au_fdiv`, which bitcast, compute
  in `f64` and bitcast back; pinned by `float_helpers_bitcast_and_compute`.

None of these is visible from source-level verification; they are why the
execution step exists.

## Multi-cell images (a provider cell, not a stub)

`run.sh` links each cell against runtime stubs. `multi-cell.sh` links two
*cells*: `MathLib` exports `double`, and `DoubleApp` imports it, so the owner
cell carries one import and one call relocation that only a provider can
resolve.

```bash
examples/nix-pkg/multi-cell.sh
```

```text
== negative control (the owner alone must not link)
  [ok] without --provider: unresolved symbol "double" (exit 1)
== execution (owner + provider cell, resolved from the plan)
  [ok] double(42) resolved in the provider image (base 0x11000000) and returned 84
== negative control (a provider without a key is refused)
  [ok] provider without --key refused (exit 1)
== negative control (a provider signed by another builder is refused)
  [ok] provider signed by another builder refused (exit 1)
== callback execution (owner -> provider -> owner)
  [ok] provider's compute(42) called owner's tick and returned 53
```

The loader takes repeated `--provider <cell.avm>` flags, or `--plan <plan.json>
--for <name> --cells <dir>`: the plan names the closure, and every derivation
the owner depends on (transitively, dependency-first) is taken from
`<dir>/<name>.avm`. An explicit `--provider` whose file stem names the same
derivation wins, so one provider can be overridden without dropping the rest;
a dependency with no cell is a hard error naming the missing file, never a
silently smaller image. A plan-derived provider is also checked against the
derivation id the plan resolved: the cell records the id it was built for, so
a differently-built cell dropped in as `<name>.avm` is refused. Each provider is parsed and gated by **its own**
capability table — it runs in the same process, so `Execute` and its
`Call:<symbol>` list are judged too — and then `linker::link_cells` builds one
symbol table for the whole image and applies every cell's relocations at its
own base. That second part is what makes a provider that calls the owner or
the runtime work; the owner-only `link_with` leaves provider text unpatched.

Provider text is executable, so layout is not a trust decision: `load` requires
`--key <pubkey-hex32>` whenever a provider is supplied and verifies every
provider (and the owner) against it before linking. `build` prints that public
key in its report (`"pubkey"`), so a script can verify what it just built;
without a key the load is refused, and a cell signed by a different builder
fails the signature check. Neither control is a policy question — a provider
without a signature is not run at all.

Bases are assigned by the CLI, not guessed by the executor: the owner keeps
`--base` and provider *n* is linked for `base + n × 0x01000000`. The layout is
recorded in `cell.json` (`providers: [{base, text}]`), and the executor maps
each provider there after rejecting an unaligned base or one that overlaps
the owner or the stub page. `exec-image.rs` then calls the owner's `main` as
before; the call lands in the provider's mapped text.

The two modules are compiled **separately**, because one CPS emission carries
one module's functions: compiling both together would put `double` in the
owner's cell and delete the thing under test. The derivation plan records
`doubleapp` as depending on `mathlib`, which is the package-level shape of the
same edge, and that plan is now how the provider is found; a `.avm` cell still
does not carry its dependencies' bytes, which is why `--cells` names the
directory.

The second half of the script compiles a mutually-referencing pair: the owner
`Callback` defines `tick` and calls `compute`; the provider `Wave` defines
`compute` and calls `tick`. Module imports must be acyclic, so each side
reaches the other through `pragma Foreign_Import(External_Name => …)` and each
cell is compiled on its own. The CPS JIT logs an unresolved-symbol diagnostic
while compiling a cell whose callee is in the other cell (the CPS file is
written before that step, so emission is unaffected); the script records the
diagnostic in `$CB/*.err` and reports it as a note. Loading the owner alone
misses `compute`; loading it with the plan closure resolves both directions
and `main` returns 53 — 42 to the provider, `tick` back in the owner, `+1` in
the provider. This is the case the shared symbol table exists for.

## Limits of this test (read before trusting a pass)

* **The fragment is shared, and small.** `dnx-verify` refuses open terms,
  strings, paths, lists, attribute sets, closures and unreduced primitive
  functions. A real nixpkgs derivation (`stdenv.mkDerivation { … }`) is an
  attribute set and cannot be compared this way. The kernel translator's
  Austral grammar is also narrower than the compiler's: `if … then … else …
  end if;` is a parse error there (only `let`, `clone`, `destroy` and
  `return` statements are accepted), so conditionals are absent from the set
  even though both tools accept them separately.
* **Equality is the canonical net, not the computed value.** Two expressions
  that compute the same number through different normal forms hash
  differently. The verdict is exactly the kernel's `austral_defeq`, which is
  what a conversion step needs.
* **The grammars overlap but are not identical.** The kernel translator
  accepts `let x: Int64 = 2;`, while the AustralVM compiler requires
  `let x: Int64 := 2;`. The modules use `:=` so *one* file serves both. Write
  the translations as compiler-valid Austral and let the verifier be the
  stricter oracle.
* **The cell is not the source.** The equivalence is checked on the *source*
  net, and the execution step checks the value the current CPS lowering
  produces. The integer Pervasive primitives are stubs in `exec-image.rs`
  that do not trap on overflow, and Float64 uses the engine's one-word
  bit-pattern convention rather than the platform's `xmm0` return register —
  the two places where the test harness, not the compiler, supplies the
  meaning of the bits.
