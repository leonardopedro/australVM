/-
  mm0/toy_witness/ToyWitness.lean — the toy memory's axioms, checked by Lean.

  WHY A SECOND CHECKER. `scripts/toy-witness` already runs the witness from
  `mm0/store_projection_toy.mm0`'s header as Rust and enumerates the model. This
  file is the same witness as a *Lean* definition, with the axiom checks phrased
  as `Bool` computations and discharged by `decide` (kernel reduction) — so the
  verdict does not rest on trusting my reading of an `if` in another language, and
  the negations of the axioms the toy rejected are theorems here rather than
  printed counterexample strings. The two are deliberately independent: same
  model, two checkers, different failure modes.

  WHAT IT DOES NOT DO. It is a finite model check, not a proof over an arbitrary
  atom type: atoms are `Fin 4` and memories are lists of at most two cells. An
  axiom instance over more atoms than that is the same equality pattern renamed,
  which is an argument, not something this file establishes. Where the witness
  argument actually stops is recorded at the bottom, with the failures proved.

  Run: `lean mm0/toy_witness/ToyWitness.lean` (the gate runs it in the `lean`
  shell and reports a `[gap]` elsewhere). No Mathlib, no dependencies.
-/

-- The finite quantifiers below are long `List.all` chains, which the default
-- recursion depth is too shallow for; the kernel reduction itself is cheap.
set_option maxRecDepth 200000

namespace ToyWitness

/- ── The concrete interpreter: a list of at most two (address, value) cells ── -/

/-- Four atoms. `0` is `unset`, as in the Rust checker; a value is an atom. -/
abbrev Atom := Fin 4

/-- The whole atom universe, for the finite quantifiers below. -/
def atoms : List Atom := List.finRange 4

/-- A cell: address, value. -/
abbrev Cell := Atom × Atom

/-- A memory is a list of cells, newest first, of at most two of them. -/
abbrev Mem := List Cell

def unset : Atom := 0

/-- `put m a v`: the new cell first, any cell already at `a` dropped, the result
truncated to two — the header's `(a,v) :: take 1 (m minus a)`, written so that it
also applies to a memory that is already longer than two cells. -/
def put (m : Mem) (a v : Atom) : Mem :=
  ((a, v) :: m.filter (fun c => c.1 ≠ a)).take 2

/-- `get m a`: the value of the first cell at `a`, else `unset`. -/
def get (m : Mem) (a : Atom) : Atom :=
  (m.find? (fun c => c.1 = a)).map Prod.snd |>.getD unset

/-- `held m a`: some cell is at `a`. -/
def held (m : Mem) (a : Atom) : Bool := m.any (fun c => c.1 = a)

/-- `notfull m`: fewer than two cells. -/
def notfull (m : Mem) : Bool := m.length < 2

/-- `narrow4 x`: narrowing to four bytes. A truncation that *loses* information
(`x % 2`), so idempotence is not satisfied by a function that does nothing. -/
def narrow4 (x : Atom) : Atom :=
  ⟨x.val % 2, Nat.lt_of_lt_of_le (Nat.mod_lt _ (by decide)) (by decide)⟩

/-- Every memory the quantifiers range over: the empty one, all one-cell ones,
and all two-cell ones. Two-cell memories include shapes `put` never builds (both
cells at one address); they are still values of `Mem` the axioms quantify over. -/
def cellsAll : List Cell := atoms.flatMap fun a => atoms.map fun v => (a, v)

def allMems : List Mem :=
  [[]] ++ cellsAll.map (fun c => [c])
    ++ cellsAll.flatMap fun c1 => cellsAll.map fun c2 => [c1, c2]

/-- `Bool`-valued `∀` over the atom universe. -/
def allAtoms (p : Atom → Bool) : Bool := atoms.all p

/-- The three-way distinctness the capacity axioms assume. -/
def distinct3 (x y z : Atom) : Bool := x ≠ y && x ≠ z && y ≠ z

/- ── The seventeen axioms, each discharged by the kernel ─────────────────────
   `decide` reduces the `Bool` computation in the kernel, so the proof term is a
   `rfl`-shaped certificate rather than a trusted execution. The two capacity
   axioms quantify over ~10⁶ instances and use `native_decide` instead, which
   trusts the compiled evaluator (its proof term mentions `Lean.ofReduceBool`);
   the comment on each says which. -/

theorem ax_veq_refl : allAtoms (fun x => x = x) = true := by decide

theorem ax_veq_trans :
    allAtoms fun x => allAtoms fun y => allAtoms fun z =>
      if x = y && y = z then x = z else true := by decide

theorem ax_vne_sym :
    allAtoms fun x => allAtoms fun y => if x ≠ y then y ≠ x else true := by decide

theorem ax_narrow4_cong :
    allAtoms fun x => allAtoms fun y => if x = y then narrow4 x = narrow4 y else true := by
  decide

theorem ax_narrow4_idem : allAtoms (fun x => narrow4 (narrow4 x) = narrow4 x) = true := by
  decide

theorem ax_get_init : allAtoms (fun a => get [] a = unset) = true := by decide

theorem ax_init_notfull : notfull [] = true := by decide

theorem ax_put_init_notfull :
    allAtoms fun a => allAtoms fun v => notfull (put [] a v) = true := by decide

theorem ax_at_put :
    allMems.all fun m => allAtoms fun a => allAtoms fun v => get (put m a v) a = v := by
  decide

theorem ax_put_held :
    allMems.all fun m => allAtoms fun a => allAtoms fun v => held (put m a v) a = true := by
  decide

theorem ax_put_held_other :
    allMems.all fun m => allAtoms fun a => allAtoms fun b => allAtoms fun v =>
      if a ≠ b && held m b && notfull m then held (put m a v) b else true := by
  decide

theorem ax_at_put_held_other :
    allMems.all fun m => allAtoms fun x => allAtoms fun y => allAtoms fun w =>
      if x ≠ y && held m x then get (put m x w) y = get m y else true := by
  decide

theorem ax_at_put_held_keeps :
    allMems.all fun m => allAtoms fun x => allAtoms fun y => allAtoms fun w =>
      if x ≠ y && held m y && held m x then held (put m x w) y else true := by
  decide

theorem ax_at_put_held_no_new :
    allMems.all fun m => allAtoms fun x => allAtoms fun y => allAtoms fun w =>
      if x ≠ y && held (put m x w) y then held m y else true := by
  decide

theorem ax_at_put_other :
    allMems.all fun m => allAtoms fun a => allAtoms fun b => allAtoms fun v =>
      if a ≠ b && notfull m then get (put m a v) b = get m b else true := by
  decide

/-- The capacity axioms quantify over six atoms and every memory, so the kernel
would take far too long to reduce them; `native_decide` compiles the same
decision procedure and checks the result. -/
theorem ax_at_drop :
    allMems.all fun m => allAtoms fun x => allAtoms fun y => allAtoms fun z =>
      allAtoms fun vx => allAtoms fun vy => allAtoms fun vz =>
        if distinct3 x y z then get (put (put (put m x vx) y vy) z vz) x = unset else true := by
  native_decide

theorem ax_at_keep_prev :
    allMems.all fun m => allAtoms fun x => allAtoms fun y => allAtoms fun z =>
      allAtoms fun vx => allAtoms fun vy => allAtoms fun vz =>
        if distinct3 x y z then get (put (put (put m x vx) y vy) z vz) y = vy else true := by
  native_decide

/- ── Where the witness argument breaks ───────────────────────────────────────
   Each of the four statements below is *false* in this model, and proved false
   here. They are not hypothetical: `put_notfull` was carried by an earlier
   revision of the toy, the two unguarded preservation axioms are what the
   `notfull`/`held` guards exist to exclude, and "narrowing is the identity" is
   what the width-4 theorems would be if `narrow4` did nothing. -/

/-- An earlier revision's closure axiom: `notfull` survives any write. False the
moment the write takes the second cell — which is exactly what a capacity-2
memory does, and what `at_drop` requires. -/
theorem put_notfull_breaks :
    ¬ (∀ (m : Mem) (a v : Atom), notfull m = true → notfull (put m a v) = true) := by
  intro h
  -- one cell in, two cells out
  have : notfull (put [(0, 0)] 1 0) = true := h [(0, 0)] 1 0 (by decide)
  exact absurd this (by decide)

/-- `at_put_other` without `notfull`: a write to a fresh address in a full memory
evicts, so the other cell's content is not preserved. -/
theorem at_put_other_unguarded_breaks :
    ¬ (∀ (m : Mem) (a b v : Atom), a ≠ b → get (put m a v) b = get m b) := by
  intro h
  have : get (put [(0, 0), (1, 1)] 2 0) 1 = get [(0, 0), (1, 1)] 1 := h _ 2 1 0 (by decide)
  exact absurd this (by decide)

/-- `put_held_other` without `notfull`: the same write loses the held cell. -/
theorem put_held_other_unguarded_breaks :
    ¬ (∀ (m : Mem) (a b v : Atom), a ≠ b → held m b = true → held (put m a v) b = true) := by
  intro h
  have : held (put [(0, 0), (1, 0)] 2 0) 1 = true := h _ 2 1 0 (by decide) (by decide)
  exact absurd this (by decide)

/-- "Narrowing to four bytes is the identity" — false, and if it held the width-4
theorems would say nothing: a value that fits in four bytes is not every value. -/
theorem narrow4_is_identity_breaks : ¬ (∀ x : Atom, narrow4 x = x) := by
  intro h
  have : narrow4 2 = 2 := h 2
  exact absurd this (by decide)

/- ── The argument's other limit: a degenerate model of the same axioms ───────
   `veq` is uninterpreted, and the toy's axioms only demand reflexivity and
   transitivity. So the *universal* relation is also a model: every `veq`
   conclusion is trivially true, and the axioms still hold. That means
   consistency alone is cheap — it does not say the axioms constrain `get`/`put`
   in any way. What carries the toy's content is the *instance* proofs (which
   rest on `at_put`, `at_drop`, …) and this witness, not satisfiability. -/

def veqUniversal (_ _ : Atom) : Prop := True

theorem veq_universal_satisfies_trans :
    ∀ x y z : Atom, veqUniversal x y → veqUniversal y z → veqUniversal x z :=
  fun _ _ _ _ _ => trivial

theorem veq_universal_satisfies_at_put :
    ∀ (m : Mem) (a v : Atom), veqUniversal (get (put m a v) a) v :=
  fun _ _ _ => trivial

/- ── Summary the gate reads ─────────────────────────────────────────────────
   Printed at elaboration so `verify-invariants` can compare it with the axiom
   list in `mm0/store_projection_toy.mm0` and fail when a new axiom has no Lean
   statement here. -/

def checkedAxioms : List String :=
  ["veq_refl", "veq_trans", "vne_sym", "narrow4_cong", "narrow4_idem", "get_init",
   "init_notfull", "put_init_notfull", "at_put", "put_held", "put_held_other",
   "at_put_held_other", "at_put_held_keeps", "at_put_held_no_new", "at_put_other",
   "at_drop", "at_keep_prev"]

def brokenCandidates : List String :=
  ["put_notfull", "at_put_other_unguarded", "put_held_other_unguarded",
   "narrow4_is_identity"]

#eval do
  IO.println s!"lean-witness: {checkedAxioms.length} axiom(s) hold in the list model, \
{brokenCandidates.length} rejected candidate(s) proved false"
  IO.println s!"lean-witness-axioms: {String.intercalate "," checkedAxioms}"
  IO.println s!"lean-witness-broken: {String.intercalate "," brokenCandidates}"

end ToyWitness
