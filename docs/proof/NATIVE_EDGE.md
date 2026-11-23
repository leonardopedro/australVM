# PC08 — one captured native fragment, and the edge that stays open

[CHAIN_GATE.md](CHAIN_GATE.md) ends PC07's chain at a device value. This page is
the first native layer under it: a real host fragment, captured from the same
compilation the JIT runs, decoded, and pinned.

It does **not** close the edge, and the reason is measured rather than assumed:
this repository has no x86_64 instruction semantics. The conversions in
`mm0/statements/` are RV64 (Sail), and [PROOF_CHAIN.md](../PROOF_CHAIN.md)
already records that RV64 statements cannot certify AArch64 or x86_64 output. So
what PC08 can honestly deliver is the plan's own fallback — "an unavailable
decoder/model produces an explicit open edge, not a successful native profile" —
with the capture and the statement of record pinned so that nothing about the
fragment can change silently.

## The fragment

`examples/native-fragment/NativeFragment.aum`:

```austral
function add(x: Int64, y: Int64): Int64 is
    return (x + y);
end;

function main(): ExitCode is
    let r: Int64 := add(3, 4);
    return ExitSuccess();
end;
```

Compiled on the CPS path (`--use-cps-jit --target-type=c`), the CPS handed to
the bridge's own `cps2cell` (the same `compile_cps_to_clif` the JIT uses), and
the machine code read back out of the `AVCE` container. On this host
(`x86_64`) `add` is 22 bytes in 7 instructions:

| offset | bytes | instruction |
|---|---|---|
| 0 | `55` | `push rbp` |
| 1 | `48 89 e5` | `mov rbp, rsp` |
| 4 | `49 b8 …` | `movabs r8, @Austral.Pervasive::trappingAdd` (Abs8 reloc) |
| 14 | `41 ff d0` | `call r8` |
| 17 | `48 89 ec` | `mov rsp, rbp` |
| 20 | `5d` | `pop rbp` |
| 21 | `c3` | `ret` |

The arithmetic is not emitted inline: the CPS body calls the imported
`Austral.Pervasive::trappingAdd`, and the captured code is the SysV shim around
that call — prologue, two argument registers, the relocated call, epilogue,
`ret` with the result in `rax`. `main` is captured as well (48 bytes, 9
instructions): it loads the immediates `3` and `4` into `edi`/`esi`, calls
`add`, allocates 8 bytes through `au_alloc`, stores the result and returns
`ExitSuccess`. Recording the caller is what makes the immediates and the
intra-capture call observable.

`mm0/proof_chain/native/add.capture` is that capture as committed text: arch,
the CPS digest it came from, the container digest, the CLIF shape `clif_dump`
measured for `add` (`call 2 1 i64,i64 -> i64`, `return 1 0 i64 -> none`), the
imports, and per function its bytes, relocations and decoded instructions.

## The decoder is a table, and it is closed

`scripts/native-edge` decodes those bytes with 13 x86_64 forms — the ones this
fragment actually contains — and refuses any byte it cannot name. That is the
point of the table's being closed: a new codegen form becomes a visible failure
("byte 0xNN at offset k is not a form in the table") instead of an unexamined
instruction inside a fragment described as captured. A relocation must land
exactly on an immediate of a matching width, so a moved patch site is a decode
error, not a footnote.

This is a decoder for the fragment, not a verified decoder: the table is read,
not proved, and `-- certificate: [gap]` says so.

## The stated edge

`mm0/proof_chain/native_add.mm0` is **generated** from the capture (so it cannot
drift from it) and states the obligation in the same vocabulary-only style as
`mm0/store_projection.mm0`:

```
term entry (x y: Val): St;          -- SysV in: rdi, rsi
term x86_add (s: St): St;           -- the decoded instructions, as one step
term value (s: St): Val;            -- rax at `ret`
term trapping_add (x y: Val): Val;  -- the CPS body's imported arithmetic
term native_add_edge (x y: Val): Prop;
```

`native_add_edge x y` is *intended* to mean `value (x86_add (entry x y)) =
trapping_add x y`; the equality is not stated, because MM0 has no built-in
equality and importing one would claim a theory this artifact has not earned.
There is no `axiom` and no `theorem` in the file, and the gate refuses either:
`mm0-rs` accepting it means "the obligation is well-formed", not "it holds".

## Controls

`scripts/native-edge selftest` (all eight run without the mm0 toolchain):

| control | refused by |
|---|---|
| a changed argument immediate (`mov edi, 3` → `5`) | the recorded instructions no longer match a fresh decode |
| a changed relocation target (`add` → `main`) | same |
| a flipped instruction byte (`55` → `56`) | the decoder's closed table |
| a truncated function body | the decoder cannot account for the last instruction |
| a different target architecture (`aarch64`) | the capture must be the host's x86_64 fragment |
| a hand-edited statement | the stated edge is generated from the capture |
| the edge relabelled `certificate: [pass]` | the edge must stay `[gap]` |
| an axiom added to the edge | the artifact may state, not assume |

The `check` mode additionally re-derives the whole capture from the fixture when
this host has `austral` + `cps2cell` (byte-identical, so a codegen change is a
finding), and replays the stated edge with `mm0-rs`/`mm0-c` when they are
present. Both are reported as `[gap]` when absent — never as passes.

## Reproduce

```sh
# pure checking (decoder + pin + controls), no compiler needed
python3 scripts/native-edge check
python3 scripts/native-edge selftest

# re-capture from the fixture (needs ./austral and cps2cell)
python3 scripts/native-edge capture

# replay the stated edge
MM0=$(nix build "git+file://$PWD#mm0" --no-link --print-out-paths)
PATH="$MM0/bin:$PATH" python3 scripts/native-edge check
```

## The x86_64 re-derivation is an explained gap, not a pass

`capture`/`check` re-derive the fragment by compiling it on the host. That needs
a host whose backend emits x86_64, and every route was checked on 2026-11-23:

| Route | State |
|---|---|
| this host | aarch64 (`uname -m`), so `cps2cell` emits `arch aarch64` and the closed x86_64 table refuses it; `check` probes the arch before decoding and reports `[gap] PC08: the capture was not re-derived here (this host compiles aarch64 …)` |
| a second host | none reachable; no x86_64 machine or runner is configured for this workspace |
| `rustup` cross | only `aarch64-unknown-linux-gnu` and `x86_64-linux-android` are installed — neither runs an x86_64 Linux binary here |
| emulation | `podman` is present but the re-derivation runs the *OCaml compiler* (`austral`, `cps2cell`), which would have to be built for x86_64 first; no such build exists |
| CI | `ubuntu-latest` is the only reachable x86_64 host: `.github/workflows/ci.yml` `test` job builds the tree's `austral` and `cps2cell`, builds the mm0 toolchain, and runs `python3 scripts/native-edge check` + `selftest`, so the re-derivation and the edge replay are measured there (this row used to say no job invoked the script) |

So the decode-and-pin half is a measured pass on this host, and the
byte-identical re-derivation is measured on the CI runner instead: the `test`
job in `.github/workflows/ci.yml` builds `austral` + `cps2cell` + the mm0
toolchain and runs `check` and `selftest`, which is the x86_64 half of this
gate. On this aarch64 host `check` reports the gap by name and never as a pass;
locally the full run still needs an x86_64 host running
`python3 scripts/native-edge check`.

## What this is not, and what would close it

- **Not a native profile.** The simulation obligation is open; the fragment is
  real, the decoder is a table, and the statement is unproved.
- **Not a second compiler.** `cps2cell` and `clif_dump` call the same
  `compile_cps_to_clif` the JIT calls; nothing here re-implements lowering.
- **Not the arithmetic.** The captured `add` calls the imported `trappingAdd`,
  so a proof of this fragment would still end at that external boundary — the
  same shape as PC07's `native_agrees`.
- **What would close it:** a reviewed x86_64 semantics (or a conversion of one,
  like the RV64 ones under `mm0/statements/`), a simulation from machine states
  to CPS states across the ABI, and either an inlined arithmetic form or a
  model of `trappingAdd`. Each of those is a separate card's worth of work, and
  none is faked here.
