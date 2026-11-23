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

/-! ## The SysV frame: prologue, epilogue and `ret`

The fragment is not only arithmetic: the capture opens with `push rbp` /
`mov rbp, rsp` / `sub rsp, 0x10`, saves `r12`, and closes with the epilogue
(`mov rax, r12`, restore `r12`, `mov rsp, rbp`, `pop rbp`, `ret`). Those are
modelled here as an abstract frame — a stack of saved slots, a frame pointer and
a stack pointer as levels, not addresses — which is what is needed to state the
edge *from entry to `ret`* rather than only "after the arithmetic".
-/

/-- A call frame: the registers, the saved slots (most recent first), and the
frame/stack pointers as abstract levels. No addresses and no memory contents:
this models the discipline, not the stack bytes. -/
structure Frame where
  regs : State
  rbp : Word
  rsp : Word
  stack : List Word

/-- The state at the fragment's entry, with an empty frame. (Named rather than
    namespaced as `Frame.entry` so it does not shadow the register-state
    `entry` it wraps.) -/
def entryFrame (x y : Word) : Frame :=
  { regs := entry x y, rbp := 0, rsp := 0, stack := [] }

/-- `push rbp`. -/
def pushRbp (f : Frame) : Frame := { f with stack := f.rbp :: f.stack }

/-- `pop rbp`. -/
def popRbp (f : Frame) : Frame :=
  match f.stack with
  | b :: rest => { f with rbp := b, stack := rest }
  | [] => f

/-- `sub rsp, 0x10`: one level of frame storage, abstractly. -/
def openFrame (f : Frame) : Frame := { f with rsp := f.rsp + 16 }

/-- `add rsp, 0x10` / `mov rsp, rbp`. -/
def closeFrame (f : Frame) : Frame := { f with rsp := f.rbp }

/-- The prologue's stack discipline restores the frame pointer exactly. -/
theorem push_pop_restores_rbp (f : Frame) : (popRbp (pushRbp f)).rbp = f.rbp := rfl

/-- ... and leaves the remaining saved slots untouched. -/
theorem push_pop_restores_stack (f : Frame) :
    (popRbp (pushRbp f)).stack = f.stack := rfl

/-- The epilogue's pop does not touch the return value. -/
theorem pop_keeps_value (f : Frame) : value (popRbp f).regs = value f.regs := by
  unfold popRbp value
  split <;> rfl

/-- The whole captured fragment, entry to `ret`: prologue, the guarded
arithmetic, and the epilogue. `none` is the `trappingSubtract` call on the
`jne` path, which this model deliberately leaves outside the fragment. -/
def runFragment (f : Frame) : Option Word :=
  let f1 := openFrame (pushRbp f)
  let s1 := movR12Rdi f1.regs
  let s2 := subR12Rsi s1
  if s2.of then none
  else
    let f2 := { f1 with regs := movRaxR12 s2 }
    let f3 := closeFrame f2
    some (value (popRbp f3).regs)

/-- **The fragment's edge, in range**: from the SysV entry to `ret`, the value
returned is the difference — prologue and epilogue included. -/
theorem run_fragment_in_range (x y : Word) (h : inRange (x - y)) :
    runFragment (entryFrame x y) = some (x - y) := by
  have hf : fits (x - y) = true := by
    dsimp [fits]
    exact decide_eq_true h
  simp [runFragment, entryFrame, openFrame, pushRbp, movR12Rdi, subR12Rsi,
        movRaxR12, closeFrame, popRbp, entry, value, hf]

/-- **The fragment's edge, out of range**: the same path reaches the `jne`, and
the value is produced by `trappingSubtract` — outside the fragment, so the model
says `none` rather than inventing it. -/
theorem run_fragment_overflows :
    runFragment (entryFrame (-twoPow63) 1) = none := by
  have h : ¬ inRange ((-twoPow63) - 1) := by
    unfold inRange twoPow63
    omega
  have hf : fits ((-twoPow63) - 1) = false := by
    dsimp [fits]
    exact decide_eq_false h
  simp [runFragment, entryFrame, openFrame, pushRbp, movR12Rdi, subR12Rsi,
        entry, hf]

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
  , "pop_keeps_value"
  , "run_fragment_in_range"
  , "run_fragment_overflows"
  ]

/-- The instruction forms this model covers, keyed exactly as the capture
    decodes them. A form in a capture but not here is an uncovered instruction,
    which `scripts/native-edge model` reports rather than ignores. -/
def MODELED : List String := ["sub", "imul", "seto"]

end X86Arith
