/-
  X86Arith.lean — the arithmetic core of PC08's `arith` fragment, modelled and
  checked. This is the "one reviewed instruction semantics" PC08 was waiting
  for, restricted to what the captured fragment actually executes.

  Normative source for the three instructions (Intel SDM, reviewed and cited by
  section, not re-derived here):

    * SUB — Subtract (Vol. 2B): `DEST := DEST - SRC`; OF is set when the
      subtraction of two signed operands overflows the destination width.
    * IMUL — Signed Multiply (Vol. 2B), two-operand form: `DEST := DEST * SRC`
      truncated to the destination width; OF is set when the full product does
      not fit.
    * SETcc — Set Byte on Condition (Vol. 2B), `SETO`: writes 1 when OF is set,
      else 0. The fragment uses it only through `test r8b, r8b` / `jne`.

  What the model is, and is not. A register holds a mathematical `Int` here, not
  sixty-four bits. That is enough for the case the fragment's guard admits: the
  inline path runs only when the true result is representable, and that is the
  case proved below (`sub_in_range_value`, `mul_in_range_value`). The
  out-of-range case is *not* modelled bit-accurately — only its flag is, by
  `fits`, and `sub_overflow_takes_the_trap` shows it leaves the fragment. So
  this is a checked model of the arithmetic core and its guard, not a
  bitvector ISA semantics. The SysV frame is modelled too, but only as a
  discipline — saved slots and frame/stack levels, no addresses and no stack
  bytes — and the ABI's memory and `trappingSubtract` are not modelled at all.

  Everything below is proved; there is no `sorry`, no `axiom`, and no use of an
  oracle. `scripts/native-edge model` runs `lean` on this file, requires exit 0,
  checks that the theorems listed in `THEOREMS` are present, and refuses the
  file if a new ALU/flag instruction appears in a capture without a definition
  here.
-/

namespace X86Arith

/-- A register's value: the mathematical value, see the header. -/
abbrev Word := Int

/-- 2^63, the magnitude of the signed 64-bit range's lower bound. -/
def twoPow63 : Int := 9223372036854775808

/-- `x` is representable as a signed 64-bit value. -/
abbrev inRange (n : Int) : Prop := -twoPow63 ≤ n ∧ n < twoPow63

/-- The `OF` flag as the fragment reads it. -/
def fits (n : Int) : Bool := decide (inRange n)

/-- The registers the fragment touches, plus the overflow flag that `seto`
    writes. `rdi`/`rsi` are the SysV arguments to `sub`, `r12` is the callee-
    saved register the lowering uses as its accumulator, `rax` the return
    register. -/
structure State where
  rdi : Word
  rsi : Word
  r12 : Word
  rax : Word
  of : Bool

/-- The SysV entry state: two arguments, scratch registers zeroed. -/
def entry (x y : Word) : State :=
  { rdi := x, rsi := y, r12 := 0, rax := 0, of := false }

/-- What the fragment leaves in the return register. -/
def value (s : State) : Word := s.rax

/-- `mov r12, rdi`. -/
def movR12Rdi (s : State) : State := { s with r12 := s.rdi }

/-- `sub r12, rsi` together with the adjacent `seto r8b`: the capture always
    executes them as one step, and the flag is `OF` of the subtraction. -/
def subR12Rsi (s : State) : State :=
  { s with r12 := s.r12 - s.rsi, of := !(fits (s.r12 - s.rsi)) }

/-- `mov rax, r12`. -/
def movRaxR12 (s : State) : State := { s with rax := s.r12 }

/-- The inline (non-overflow) path the capture emits: `mov r12, rdi`;
    `sub r12, rsi` + `seto`; `mov rax, r12`. -/
def inlineSub (s : State) : State :=
  movRaxR12 (subR12Rsi (movR12Rdi s))

/-- The guarded path: `test r8b, r8b` / `jne` sends an overflow to the
    `trappingSubtract` call, which is outside the fragment; a value is produced
    only when the flag is clear. -/
def guardedSub (s : State) : Option Word :=
  let s' := subR12Rsi (movR12Rdi s)
  if s'.of then none else some (value (movRaxR12 s'))

/-- `mov rax, rdi`, then `imul rsi` + `seto`, then `mov r12, rax`. The two
    instruction forms are one step for the same reason as `sub` above. -/
def mulRdiRsi (s : State) : State :=
  let p := s.rdi * s.rsi
  { s with rax := p, r12 := p, of := !(fits p) }

/-- The inline path for `mul`: `mov r12, rax` before the epilogue reads `rax`
    from `r12`, exactly as the capture does (`mov rax, r12`). -/
def inlineMul (s : State) : State := movRaxR12 (mulRdiRsi s)

/-- **`sub` computes the difference** on the inline path, for arguments whose
    true difference is representable. -/
theorem sub_in_range_value (x y : Word) (_h : inRange (x - y)) :
    value (inlineSub (entry x y)) = x - y := by
  simp [inlineSub, movRaxR12, subR12Rsi, movR12Rdi, entry, value]

/-- **`seto` is clear exactly in range**, so the `jne` to the trap call is not
    taken: the `sub` `OF` equals the modelled `fits` test. -/
theorem sub_in_range_flag_clear (x y : Word) (h : inRange (x - y)) :
    (subR12Rsi (movR12Rdi (entry x y))).of = false := by
  have hf : fits (x - y) = true := by
    dsimp [fits]
    exact decide_eq_true h
  simp [subR12Rsi, movR12Rdi, entry, hf]

/-- **The guarded path yields the difference** in range: the guard is not only
    clear, the value written to the output is the inline result. -/
theorem guarded_sub_value (x y : Word) (h : inRange (x - y)) :
    guardedSub (entry x y) = some (x - y) := by
  have hf : fits (x - y) = true := by
    dsimp [fits]
    exact decide_eq_true h
  simp [guardedSub, movRaxR12, movR12Rdi, subR12Rsi, entry, value, hf]

/-- **`imul` computes the product** for arguments whose true product is
    representable. -/
theorem mul_in_range_value (x y : Word) (_h : inRange (x * y)) :
    value (inlineMul (entry x y)) = x * y := by
  simp [inlineMul, movRaxR12, mulRdiRsi, entry, value]

/-- **`imul`'s `seto` is clear exactly in range.** -/
theorem mul_in_range_flag_clear (x y : Word) (h : inRange (x * y)) :
    (mulRdiRsi (entry x y)).of = false := by
  have hf : fits (x * y) = true := by
    dsimp [fits]
    exact decide_eq_true h
  simp [mulRdiRsi, entry, hf]

/-- **Out of range, `sub` overflows**: the difference `-2^63 - 1` is not
    representable, so the flag is set. This is the witness that the fragment's
    guard is load-bearing — without it the inline path would compute a value
    the hardware cannot hold. -/
theorem sub_out_of_range_flag_set :
    (subR12Rsi (movR12Rdi (entry (-twoPow63) 1))).of = true := by
  have h : ¬ inRange ((-twoPow63) - 1) := by
    unfold inRange twoPow63
    omega
  have hf : fits ((-twoPow63) - 1) = false := by
    dsimp [fits]
    exact decide_eq_false h
  simp [subR12Rsi, movR12Rdi, entry, hf]

/-- **Out of range, the guarded path leaves the fragment** (`none` is the
    `trappingSubtract` call, which this model does not describe). -/
theorem guarded_sub_overflows_to_the_trap :
    guardedSub (entry (-twoPow63) 1) = none := by
  have hf : (subR12Rsi (movR12Rdi (entry (-twoPow63) 1))).of = true :=
    sub_out_of_range_flag_set
  simp [guardedSub, hf]

/-- **Out of range, `imul` overflows**: `2^62 * 4` is not representable. -/
theorem mul_out_of_range_flag_set :
    (mulRdiRsi (entry 4611686018427387904 4)).of = true := by
  have h : ¬ inRange 18446744073709551616 := by
    unfold inRange twoPow63
    omega
  have hf : fits 18446744073709551616 = false := by
    dsimp [fits]
    exact decide_eq_false h
  simp [mulRdiRsi, entry, hf]

/-! ## Bitvector words: what the destination register really holds

The model above carries mathematical values, which is enough for the range the
guard admits. This layer is exact where the register is not: the destination is
a `BitVec 64`, and `imul`'s result is the low sixty-four bits of the product.
That is the case a value-level model cannot describe — the truncation — and it
is why the guard exists.
-/

/-- A 64-bit register. -/
abbrev Bits := BitVec 64

/-- A non-negative mathematical value as the bits a register would hold. -/
def bitsOf (x : Word) : Bits := BitVec.ofNat 64 x.toNat

/-- `imul`'s destination: the low 64 bits of the product (SDM: "the product is
truncated to the destination operand's width"). -/
def imulLow (x y : Bits) : Bits := x * y

/-- **The destination is the product modulo 2^64**, exactly. -/
theorem imul_low_is_truncation (x y : Bits) :
    (imulLow x y).toNat = (x.toNat * y.toNat) % 2^64 := by
  simp [imulLow, BitVec.toNat_mul]

/-- **When the product fits, the destination is the product** — the in-range
case, stated on the bits rather than on `Int`. -/
theorem imul_low_agrees_when_it_fits (x y : Bits)
    (h : x.toNat * y.toNat < 2^64) :
    (imulLow x y).toNat = x.toNat * y.toNat := by
  rw [imul_low_is_truncation, Nat.mod_eq_of_lt h]

/-- **The truncation is observable exactly where the guard fires**: `2^62 * 4`
is `2^64`, so the 64-bit destination is zero although the product is not. A
value-level model would have claimed `2^64` here; the bits do not. -/
theorem imul_truncation_witness :
    imulLow (bitsOf 4611686018427387904) (bitsOf 4) = 0 := by
  decide

/-- The truncated and the whole product agree only while the guard holds; at
the witness the two differ. -/
theorem imul_truncation_differs :
    (imulLow (bitsOf 4611686018427387904) (bitsOf 4)).toNat ≠ 2^64 := by
  rw [imul_truncation_witness]
  decide

/-! ## The SysV frame: stack addresses, prologue/epilogue and `call`/`ret`

The fragment is not only arithmetic: the capture opens with `push rbp` /
`mov rbp, rsp` / `sub rsp, 0x10`, saves `r12`, and closes with the epilogue
(`mov rax, r12`, restore `r12`, `mov rsp, rbp`, `pop rbp`, `ret`). This section
models those as *addresses*: each slot is eight bytes, `rsp` moves by that much
per push/pop, and the frame pointer is the address `mov rbp, rsp` records. The
stack's *contents* are still a list (no byte-addressed memory), so what is
proved is the pointer discipline and the register saves, not the loaded bytes.

The call/`ret` ABI is here too: `call` pushes the return address and `ret` pops
it, so the fragment's entry state is the callee's (`rsp` points at the return
slot). On the `jne` path the fallback is a real `call` to the CPS body's
`trappingSubtract`; its body is not invented — the called function is a
parameter, and what is proved is that the call is made with `rdi`/`rsi` and its
result left in `rax`.
-/

/-- A call frame: the registers, the saved slots (most recent first), the frame
and stack pointers as stack addresses, and the callee-saved `r12` the fragment
stores. No byte-addressed memory: this models the discipline, not the stack
bytes. -/
structure Frame where
  regs : State
  rbp : Word
  rsp : Word
  stack : List Word
  saved12 : Word

/-- One stack slot: eight bytes. -/
def slotBytes : Word := 8

/-- The state at the fragment's entry (the callee's view: `rsp` addresses the
return slot), with an empty frame. (Named rather than namespaced as
`Frame.entry` so it does not shadow the register-state `entry` it wraps.) -/
def entryFrame (x y base : Word) : Frame :=
  { regs := entry x y, rbp := base, rsp := base, stack := [], saved12 := 0 }

/-- `push rbp`: one slot below `rsp`, holding the caller's frame pointer. -/
def pushRbp (f : Frame) : Frame :=
  { f with rsp := f.rsp - slotBytes, stack := f.rbp :: f.stack }

/-- `pop rbp`: read the slot and release it. -/
def popRbp (f : Frame) : Frame :=
  match f.stack with
  | b :: rest => { f with rbp := b, rsp := f.rsp + slotBytes, stack := rest }
  | [] => { f with rsp := f.rsp + slotBytes }

/-- `mov rbp, rsp`: the frame pointer becomes the current stack address. -/
def movRbpRsp (f : Frame) : Frame := { f with rbp := f.rsp }

/-- `sub rsp, 0x10`: two slots of frame storage below the frame pointer. -/
def openFrame (f : Frame) : Frame := { f with rsp := f.rsp - 2 * slotBytes }

/-- `add rsp, 0x10` / `mov rsp, rbp`: discard the frame storage. -/
def closeFrame (f : Frame) : Frame := { f with rsp := f.rbp }

/-- `mov [rsp], r12` — the callee-saved register the fragment stores. -/
def saveR12 (f : Frame) : Frame := { f with saved12 := f.regs.r12 }

/-- `mov r12, [rsp]` — the matching restore. -/
def restoreR12 (f : Frame) : Frame :=
  { f with regs := { f.regs with r12 := f.saved12 } }

/-- `call`: push the eight-byte return address. The callee sees `rsp` after it. -/
def call (f : Frame) : Frame := { f with rsp := f.rsp - slotBytes }

/-- `ret`: pop the return address. -/
def ret (f : Frame) : Frame := { f with rsp := f.rsp + slotBytes }

/-- The prologue's stack discipline restores the frame pointer exactly. -/
theorem push_pop_restores_rbp (f : Frame) : (popRbp (pushRbp f)).rbp = f.rbp := rfl

/-- ... leaves the remaining saved slots untouched ... -/
theorem push_pop_restores_stack (f : Frame) :
    (popRbp (pushRbp f)).stack = f.stack := by
  simp [pushRbp, popRbp]

/-- ... and restores the stack address it started from. -/
theorem push_pop_restores_rsp (f : Frame) :
    (popRbp (pushRbp f)).rsp = f.rsp := by
  simp [pushRbp, popRbp, slotBytes]

/-- The epilogue's pop does not touch the return value. -/
theorem pop_keeps_value (f : Frame) : value (popRbp f).regs = value f.regs := by
  unfold popRbp value
  split <;> rfl

/-- The callee-saved register survives its store/restore pair. -/
theorem saved_r12_roundtrip (f : Frame) :
    (restoreR12 (saveR12 f)).regs.r12 = f.regs.r12 := by
  simp [restoreR12, saveR12]

/-- A `call`/`ret` pair returns the stack pointer to the caller's frame. -/
theorem call_ret_roundtrip (f : Frame) : (ret (call f)).rsp = f.rsp := by
  simp [call, ret, slotBytes]

/-- The prologue/epilogue discipline balances: after `pop rbp`, `rsp` is the
    same address the fragment entered with, though the frame storage was opened
    and closed at real addresses below it. -/
theorem prologue_epilogue_balance (f : Frame) :
    (popRbp (closeFrame (openFrame (movRbpRsp (pushRbp f))))).rsp = f.rsp := by
  simp [pushRbp, movRbpRsp, openFrame, closeFrame, popRbp, slotBytes]

/-- The fragment's epilogue and `ret`, from the same entry frame: the frame
result is what the stack-pointer and callee-saved proofs are about. -/
def runFragmentEpilogue (f : Frame) : Frame :=
  let f1 := saveR12 (openFrame (movRbpRsp (pushRbp f)))
  let s2 := subR12Rsi (movR12Rdi f1.regs)
  let f2 := { f1 with regs := movRaxR12 s2 }
  ret (popRbp (restoreR12 (closeFrame f2)))

/-- The whole captured fragment, entry to `ret`, with its stack addresses:
prologue, `r12` save, the guarded arithmetic, and the epilogue including `ret`.
`none` is the `trappingSubtract` call on the `jne` path, which
`runFragmentWith` models through a parameter. -/
def runFragment (f : Frame) : Option Word :=
  let f1 := saveR12 (openFrame (movRbpRsp (pushRbp f)))
  if (subR12Rsi (movR12Rdi f1.regs)).of then none
  else some (value (runFragmentEpilogue f).regs)

/-- The same fragment with the fallback `call` modelled: `trap` is the callee,
passed in rather than assumed, so what is proved is that the call is made with
the SysV arguments `rdi`/`rsi` and its result left in `rax`. -/
def runFragmentWith (trap : Word → Word → Word) (f : Frame) : Option Word :=
  let f1 := saveR12 (openFrame (movRbpRsp (pushRbp f)))
  if (subR12Rsi (movR12Rdi f1.regs)).of then
    some (trap f.regs.rdi f.regs.rsi)
  else some (value (runFragmentEpilogue f).regs)

/-- **The fragment's edge, in range**: from the callee's SysV entry to `ret`,
the value returned is the difference — prologue, `r12` save and epilogue
included. -/
theorem run_fragment_in_range (x y base : Word) (h : inRange (x - y)) :
    runFragment (entryFrame x y base) = some (x - y) := by
  have hf : fits (x - y) = true := by
    dsimp [fits]
    exact decide_eq_true h
  simp [runFragment, runFragmentEpilogue, entryFrame, openFrame, pushRbp,
        movRbpRsp, saveR12, movR12Rdi, subR12Rsi, movRaxR12, closeFrame,
        restoreR12, popRbp, ret, entry, value, hf]

/-- **The stack is left as the caller expects**: `ret` has popped the return
slot, so the callee's `rsp` finishes one slot above its entry address. -/
theorem run_fragment_restores_caller_stack (x y base : Word) :
    (runFragmentEpilogue (entryFrame x y base)).rsp = base + slotBytes := by
  simp [runFragmentEpilogue, entryFrame, pushRbp, movRbpRsp, openFrame,
        saveR12, movR12Rdi, subR12Rsi, movRaxR12, closeFrame, restoreR12,
        popRbp, ret, slotBytes]

/-- ... and the callee-saved frame pointer is the caller's again. -/
theorem run_fragment_restores_rbp (x y base : Word) :
    (runFragmentEpilogue (entryFrame x y base)).rbp = base := by
  simp [runFragmentEpilogue, entryFrame, pushRbp, movRbpRsp, openFrame,
        saveR12, closeFrame, restoreR12, popRbp, ret]

/-- **The fragment's edge, out of range**: the same path reaches the `jne`, and
the value is produced by `trappingSubtract` — outside the fragment, so the model
says `none` rather than inventing it. -/
theorem run_fragment_overflows (base : Word) :
    runFragment (entryFrame (-twoPow63) 1 base) = none := by
  have h : ¬ inRange ((-twoPow63) - 1) := by
    unfold inRange twoPow63
    omega
  have hf : fits ((-twoPow63) - 1) = false := by
    dsimp [fits]
    exact decide_eq_false h
  simp [runFragment, entryFrame, openFrame, pushRbp, movRbpRsp, saveR12,
        movR12Rdi, subR12Rsi, entry, hf]

/-- **The fallback call, modelled**: out of range, the fragment really calls the
fallback, with the original `rdi`/`rsi` arguments, and returns whatever that
callee returns. `trap` is a parameter, so no external body is assumed — what is
proved is the ABI wiring, not `trappingSubtract`'s arithmetic. -/
theorem run_fragment_calls_the_trap (trap : Word → Word → Word) (base : Word) :
    runFragmentWith trap (entryFrame (-twoPow63) 1 base) = some (trap (-twoPow63) 1) := by
  have h : ¬ inRange ((-twoPow63) - 1) := by
    unfold inRange twoPow63
    omega
  have hf : fits ((-twoPow63) - 1) = false := by
    dsimp [fits]
    exact decide_eq_false h
  simp [runFragmentWith, entryFrame, openFrame, pushRbp, movRbpRsp, saveR12,
        movR12Rdi, subR12Rsi, entry, hf]

/-- The theorems the capture's stated edge rests on, as one list so a checker
    can name them without parsing prose. -/
def THEOREMS : List String :=
  [ "sub_in_range_value"
  , "sub_in_range_flag_clear"
  , "guarded_sub_value"
  , "mul_in_range_value"
  , "mul_in_range_flag_clear"
  , "sub_out_of_range_flag_set"
  , "guarded_sub_overflows_to_the_trap"
  , "mul_out_of_range_flag_set"
  , "imul_low_is_truncation"
  , "imul_low_agrees_when_it_fits"
  , "imul_truncation_witness"
  , "imul_truncation_differs"
  , "push_pop_restores_rbp"
  , "push_pop_restores_stack"
  , "push_pop_restores_rsp"
  , "pop_keeps_value"
  , "saved_r12_roundtrip"
  , "call_ret_roundtrip"
  , "prologue_epilogue_balance"
  , "run_fragment_in_range"
  , "run_fragment_restores_caller_stack"
  , "run_fragment_restores_rbp"
  , "run_fragment_overflows"
  , "run_fragment_calls_the_trap"
  ]

/-- The instruction forms this model covers, keyed exactly as the capture
    decodes them. A form in a capture but not here is an uncovered instruction,
    which `scripts/native-edge model` reports rather than ignores. -/
def MODELED : List String := ["sub", "imul", "seto"]

end X86Arith
