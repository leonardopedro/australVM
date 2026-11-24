# PC08 — captured native fragments, and the edges that stay open

[CHAIN_GATE.md](CHAIN_GATE.md) ends PC07's chain at a device value. This page is
the first native layer under it: two real host fragments — one whose arithmetic
is an imported call, one whose arithmetic is inlined — captured from the same
compilation the JIT runs, decoded, and pinned.

It does **not** close the edge. The only converted instruction models in
`mm0/statements/` are RV64 (Sail), and [PROOF_CHAIN.md](../PROOF_CHAIN.md)
already records that RV64 statements cannot certify AArch64 or x86_64 output.
What this repository does have is a scoped x86_64 model for the arithmetic
fragment's actual instruction set (`X86Arith.lean`: `SUB`, `IMUL`, `SETO`, the
stack pointer discipline and the `call`/`ret` wiring), so the gap is now a named
layer — the stack's byte contents and the trap callee's body — rather than
"no semantics". The capture and the statement of record stay pinned so that nothing
about the fragment can change silently, and the stated edge remains a `[gap]`
because the model is not a full ISA or ABI proof.

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

`mm0/proof_chain/native/X86Arith.lean` is the scoped model PC08 needed, covering
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
| `imul_low_is_truncation` / `imul_low_agrees_when_it_fits` | `imul`'s destination is the product modulo 2^64, and the product itself when it fits |
| `imul_truncation_witness` / `imul_truncation_differs` | the truncation is visible exactly where the guard fires |
| `push_pop_restores_rbp` / `push_pop_restores_stack` / `push_pop_restores_rsp` | the prologue's slot discipline restores the frame pointer, its slots and its stack address |
| `pop_keeps_value` | the epilogue's pop does not disturb `rax` |
| `saved_r12_roundtrip` | the callee-saved `r12` survives its store/restore pair |
| `call_ret_roundtrip` | `call` and `ret` move `rsp` by exactly one slot |
| `prologue_epilogue_balance` | opening and closing 16 bytes of frame storage leaves `rsp` where it entered |
| `run_fragment_in_range` | from the callee's entry to `ret`, the value returned is `x - y` |
| `run_fragment_restores_caller_stack` / `run_fragment_restores_rbp` | after `ret` the caller's `rsp` is one slot up (the popped return address) and its `rbp` is restored |
| `run_fragment_overflows` | out of range, the fragment leaves for the trap (`none` in the trap-free model) |
| `run_fragment_calls_the_trap` | out of range, the fallback is called with the original `rdi`/`rsi` and its result left in `rax` — with the callee a theorem parameter, not an axiom |

Twenty-four theorems in all, no `sorry`, no `axiom`, no oracle. `scripts/native-edge check`
runs `lean` on the file (falling back to the flake's store path, so it does not
need the devshell that builds the Sail models), pins its sha256 in both stated
edges, and refuses a capture whose ALU form has no definition in `MODELED` — a
new instruction cannot appear without semantics. The `[gap]` is therefore
narrower and named: the arithmetic, the stack-pointer discipline and the
`call`/`ret` wiring are proved; the stack's byte contents, the ABI's memory
model and the trap callee's body are not, so the fragment's edge is still open.

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

- **Not a native profile.** The simulation obligation is open; the fragments are
  real, the decoder is a table, and the statements are unproved.
- **Not a second compiler.** `cps2cell` and `clif_dump` call the same
  `compile_cps_to_clif` the JIT calls; nothing here re-implements lowering.
- **Not the arithmetic, in `add`.** That fragment calls the imported
  `trappingAdd`, so its proof would still end at an external boundary. `arith`
  removes that for the non-overflow case and leaves one split to argue: the
  trap path is the imported method.
- **What would close the rest:** the same Lean model extended from stack
  addresses to byte-addressed memory (so a saved slot's *value* is read back,
  not only its pointer), and a model of `trappingSubtract`'s body. The
  arithmetic core, the pointer discipline and the `call`/`ret` wiring are no
  longer guessed: they are `X86Arith.lean`.
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
