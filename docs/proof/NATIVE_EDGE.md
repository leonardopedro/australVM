# PC08 — captured native fragments, and the edges that stay open

[CHAIN_GATE.md](CHAIN_GATE.md) ends PC07's chain at a device value. This page is
the first native layer under it: two real host fragments — one whose arithmetic
is an imported call, one whose arithmetic is inlined — captured from the same
compilation the JIT runs, decoded, and pinned.

It does **not** close the edge, and the reason is measured rather than assumed:
this repository has no x86_64 instruction semantics. The conversions in
`mm0/statements/` are RV64 (Sail), and [PROOF_CHAIN.md](../PROOF_CHAIN.md)
already records that RV64 statements cannot certify AArch64 or x86_64 output. So
what PC08 can honestly deliver is the plan's own fallback — "an unavailable
decoder/model produces an explicit open edge, not a successful native profile" —
with the capture and the statement of record pinned so that nothing about the
fragment can change silently.

## The first fragment: the arithmetic is a call

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

## The second fragment: the arithmetic inlined

`add`'s arithmetic is a call, so its machine code says nothing about `+`. That
is why `examples/native-fragment/ArithFragment.aum` exists: `Int64` subtraction
and multiplication are lowered by `Compiler_cps.trapping_arith_op` to the
bridge's `sub`/`mul` opcodes (0x06/0x18), which `emit_trapping_arith` turns into
the ALU instruction plus an overflow branch:

| offset | bytes | instruction |
|---|---|---|
| 0 | `55` / `48 89 e5` / `48 83 ec 10` | prologue, 16 bytes of frame |
| 8 | `4c 89 24 24` | save `r12` |
| 12 | `49 89 fc` | `mov r12, rdi` |
| 15 | `4c 2b e6` | **`sub r12, rsi`** |
| 18 | `41 0f 90 c0` / `45 84 c0` | `seto r8b`; `test r8b, r8b` |
| 25 | `0f 85 …` | `jne` → the trap path |
| 31 | `4c 89 e0` / `e9 …` | `mov rax, r12`; `jmp` → epilogue |
| 39 | `49 b9 …` / `41 ff d1` | `movabs r9, @trappingSubtract`; `call r9` |

`mul` is the same shape with `mov rax, rdi` / **`imul rsi`** / `mov r12, rax`
and a `trappingMultiply` fallback. So the overflow case really does leave the
fragment: the inlined op is guarded and only the trap falls back to the imported
method. `main` calls `sub(9, 4)` then `mul(a, 3)`, so the immediates `9`, `4`,
`3` and both intra-capture calls are recorded as well.

`mm0/proof_chain/native/arith.capture` and `native_arith.mm0` are the same two
artifacts as for `add`, with `native_sub_edge` as the stated obligation. Its
value is that a future instruction semantics has actual ALU instructions and a
real flag use to interpret, instead of one shim around an external call.

## The decoder is a table, and it is closed

`scripts/native-edge` decodes each fragment's bytes with a table of exactly the
x86_64 forms that fragment contains (13 for `add`, 28 for `arith`) and refuses
any byte it cannot name. That is the
point of the table's being closed: a new codegen form becomes a visible failure
("byte 0xNN at offset k is not a form in the table") instead of an unexamined
instruction inside a fragment described as captured. A relocation must land
exactly on an immediate of a matching width, so a moved patch site is a decode
error, not a footnote.

This is a decoder for the fragment, not a verified decoder: the table is read,
not proved, and `-- certificate: [gap]` says so.

## The stated edge

`mm0/proof_chain/native_add.mm0` and `native_arith.mm0` are **generated** from
their captures (so they cannot drift from them) and state the obligation in the
same vocabulary-only style as `mm0/store_projection.mm0`:

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

## The cited arithmetic model (and its limits)

`mm0/proof_chain/native/X86Arith.lean` is the model PC08 was missing, scoped to
what the `arith` fragment actually executes. Its header cites the Intel SDM by
section text — `SUB` (Vol. 2B: `DEST := DEST - SRC`, OF on signed overflow),
`IMUL` (Vol. 2B, two-operand: product truncated, OF when the full product does
not fit), `SETcc`/`SETO` — and then proves, on a toy machine state (registers
hold mathematical `Int`s, not bits):

| theorem | what it says |
|---|---|
| `sub_in_range_value` | the inline sequence returns `x - y` when representable |
| `sub_in_range_flag_clear` | `seto` is false exactly then, so `jne` is not taken |
| `guarded_sub_value` | the guarded path yields `some (x - y)` in range |
| `mul_in_range_value` / `mul_in_range_flag_clear` | the same for `imul` |
| `sub_out_of_range_flag_set` | `-2^63 - 1` sets OF: the guard is load-bearing |
| `guarded_sub_overflows_to_the_trap` | out of range, the fragment leaves for `trappingSubtract` (`none`) |
| `mul_out_of_range_flag_set` | `2^62 * 4` sets OF |

Seventeen theorems in all, no `sorry`, no `axiom`, no oracle. Nine more cover
the rest of the model: the `imul` truncation (`imul_low_is_truncation`,
`imul_low_agrees_when_it_fits`, `imul_truncation_witness`,
`imul_truncation_differs`) and the SysV frame as an abstract discipline
(`push_pop_restores_rbp`, `push_pop_restores_stack`, `pop_keeps_value`) — the
last two being the fragment's edge from entry to `ret`, arithmetic and frame
included (`run_fragment_in_range`, `run_fragment_overflows`). `scripts/native-edge check`
runs `lean` on the file (falling back to the flake's store path, so it does not
need the devshell that builds the Sail models), pins its sha256 in both stated
edges, and refuses a capture whose ALU form has no definition in `MODELED` — a
new instruction cannot appear without semantics. The `[gap]` is therefore
narrower and named: the arithmetic and the frame discipline are proved, stack
addresses and bytes, the ABI's memory and the trap call are not, so the
fragment's edge is still open.

## Controls

`scripts/native-edge selftest` runs ten controls per fragment (20 in all, none
of which needs the mm0 toolchain):

| control | refused by |
|---|---|
| a changed argument immediate (`mov edi, 3` → `5`) | the recorded instructions no longer match a fresh decode |
| a changed relocation target (`add` → `main`) | same |
| a flipped instruction byte (`55` → `57`) | the decoder's closed table |
| a truncated function body | the decoder cannot account for the last instruction |
| a different target architecture (`aarch64`) | the capture must be the host's x86_64 fragment |
| a hand-edited statement | the stated edge is generated from the capture |
| the edge relabelled `certificate: [pass]` | the edge must stay `[gap]` |
| an axiom added to the edge | the artifact may state, not assume |
| a **reviewed x86_64 model appearing** (`$X86_MODEL`, `mm0/statements/x86_*.mm0`) | a stated edge with a model beside it must become a proof |
| the cited model's bytes changed | the pin in the edge is stale |
| a capture whose ALU form is absent from `MODELED` | checked positively in the selftest, refused in `check` |

The last one is the hinge this page would otherwise lack: `MODEL_SOURCES` names
where the missing semantics would appear, so the day one does, `check` fails
instead of the `[gap]` quietly continuing to pass.

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

## What this is not, and what would close it

- **Not a native profile.** The simulation obligation is open; the fragments are
  real, the decoder is a table, and the statements are unproved.
- **Not a second compiler.** `cps2cell` and `clif_dump` call the same
  `compile_cps_to_clif` the JIT calls; nothing here re-implements lowering.
- **Not the arithmetic, in `add`.** That fragment calls the imported
  `trappingAdd`, so its proof would still end at an external boundary. `arith`
  removes that for the non-overflow case and leaves one split to argue: the
  trap path is the imported method.
- **What would close the rest:** the same Lean model extended from its abstract
  frame to real stack addresses and bytes and to the `call`/`ret` ABI (and a
  bitvector word type), plus a model of `trappingSubtract`. The arithmetic core
  and the frame discipline themselves are no longer guessed: they are
  `X86Arith.lean`.
- **What would close it:** a reviewed x86_64 semantics (or a conversion of one,
  like the RV64 ones under `mm0/statements/`), a simulation from machine states
  to CPS states across the ABI, and a model of `trappingAdd` for the trap path.
  The smallest first conversion is the arithmetic fragment's ALU set:
  `sub r64, r64`, `imul r64` and `seto` — after that the `arith` edge is
  provable for in-range arguments without touching the trap. No reviewed
  x86_64 source exists here or in the sibling `../Bend` checkout (which carries
  a C/Metal/CUDA emitter, not an ISA model), so this page converts nothing and
  says so. Each of these is a separate card's worth of work, and none is faked
  here.
