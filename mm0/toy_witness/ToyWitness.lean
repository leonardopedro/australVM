/-
  mm0/toy_witness/ToyWitness.lean — the toy memory's axioms, proved in Lean.

  WHY A SECOND CHECKER. `scripts/toy-witness` checks the same axioms in Rust by
  enumerating a finite model on a fixed atom set. This file is the same model as
  a *Lean* definition, but it does not enumerate: each of the toy's axioms is a
  theorem about an **arbitrary atom type** `α` with decidable equality (and a
  default atom for `unset`), proved by induction on the list that models a
  memory. So there is no atom-domain threshold here at all — an instance over any
  number of atoms is an instance of the same theorem, and no `decide` over a
  finite atom set stands in for a proof. The negations of the axioms the toy
  rejected are also theorems here, exhibited at a concrete small domain (a
  counterexample needs only one), rather than counterexample strings printed by a
  program I wrote. The two checkers are deliberately independent: same model, and
  they fail differently.

  THE CARRIER IS IN THE TYPE. The artifact's `Mem` is an abstract sort; a memory
  in this model is a list of cells, and `Cap` (a list of at most three cells,
  which is what `put` builds) is the interpretation of that sort. Keeping the
  capacity in the type rather than in a hypothesis on each axiom is what makes
  the statements below read exactly as they do in `store_projection_toy.mm0` —
  two of the axioms are *false* on lists of four cells, and saying so by
  restricting the carrier is honest about where the model's universe ends.

  WHAT IT DOES NOT DO. `narrow4` needs a numeric domain, so that section is at
  `Fin n` for every `n ≥ 2` (narrowing to two values needs at least two atoms);
  two is the smallest possible hypothesis there, not a scale chosen to make a
  check cheap. The Rust witness is still what covers the degenerate memory shapes
  and the larger atom sweep; this file is the proof, that one is the test.

  Run: `lean mm0/toy_witness/ToyWitness.lean` (the gate runs it in the `lean`
  shell and reports a `[gap]` elsewhere). No Mathlib, no dependencies.
-/

-- The model's operations take their instances as section variables, so a few
-- statements that do not mention an operation still carry them; that is the
-- price of writing every axiom the same way, and not a warning worth chasing.
set_option linter.unusedSectionVars false

namespace ToyWitness

/- ── The model: cells, memories, and the carrier of at most three ──────────── -/

/-- A cell: address, value. -/
abbrev Cell (α : Type) := α × α

/-- A memory is a list of cells, newest first. -/
abbrev Mem (α : Type) := List (Cell α)

/-- The interpretation of the artifact's `Mem`: the memories of at most three
cells. `put` never builds anything else, and the two axioms that would otherwise
be false on a longer list (`at_put_held_other`, `at_put_held_keeps`) are exactly
where a longer list breaks the truncation. -/
def Cap (α : Type) : Type := { m : Mem α // m.length ≤ 3 }

/-- The `unset` atom: the domain's default. -/
def unset (α : Type) [Inhabited α] : α := default

/- ── List-level model, which the proofs work on ───────────────────────────── -/

/-- `drop a m`: `m` with every cell at `a` removed (the `filter` in the header's
`put`, written as a structural recursion so the inductions are on the list). -/
def drop {α : Type} [DecidableEq α] (a : α) : Mem α → Mem α
  | [] => []
  | c :: t => if c.1 = a then drop a t else c :: drop a t

/-- The header's `put`: the new cell first, any cell already at `a` dropped, the
result truncated to three. -/
def putL {α : Type} [DecidableEq α] (m : Mem α) (a v : α) : Mem α :=
  ((a, v) :: drop a m).take 3

/-- The header's `get`: the value of the first cell at `a`, else `unset`. -/
def getL {α : Type} [DecidableEq α] [Inhabited α] (m : Mem α) (a : α) : α :=
  match m with
  | [] => unset α
  | c :: t => if c.1 = a then c.2 else getL t a

/-- The header's `held`: some cell is at `a`. -/
def heldL {α : Type} (m : Mem α) (a : α) : Prop :=
  match m with
  | [] => False
  | c :: t => c.1 = a ∨ heldL t a

/-- The header's `notfull`: fewer than three cells. -/
-- `abbrev` (with `notfull` below) so `decide` unfolds the predicate all the
-- way to the `Nat` comparison instead of stopping at a `def` head.
abbrev notfullL {α : Type} (m : Mem α) : Prop := m.length < 3

@[simp] theorem getL_nil {α : Type} [DecidableEq α] [Inhabited α] (a : α) :
    getL ([] : Mem α) a = unset α := rfl

@[simp] theorem getL_cons {α : Type} [DecidableEq α] [Inhabited α]
    (c : Cell α) (t : Mem α) (a : α) :
    getL (c :: t) a = if c.1 = a then c.2 else getL t a := rfl

@[simp] theorem heldL_nil {α : Type} (a : α) : ¬ heldL ([] : Mem α) a := fun h => h

@[simp] theorem heldL_cons {α : Type} (c : Cell α) (t : Mem α) (a : α) :
    heldL (c :: t) a ↔ (c.1 = a ∨ heldL t a) := Iff.rfl

@[simp] theorem take3_cons {α : Type} (c : Cell α) (l : Mem α) :
    (c :: l).take 3 = c :: l.take 2 := rfl

@[simp] theorem take2_cons {α : Type} (c : Cell α) (l : Mem α) :
    (c :: l).take 2 = c :: l.take 1 := rfl

@[simp] theorem take1_cons {α : Type} (c : Cell α) (l : Mem α) :
    (c :: l).take 1 = [c] := rfl

theorem length_drop_le {α : Type} [DecidableEq α] (a : α) (m : Mem α) :
    (drop a m).length ≤ m.length := by
  induction m with
  | nil => simp [drop]
  | cons c t ih =>
    by_cases hc : c.1 = a
    · have := ih
      simp [drop, hc]
      omega
    · have := ih
      simp [drop, hc]
      omega

/-- Dropping a held address strictly shortens the memory: that is what makes the
capacity-3 hypotheses below give a two-cell memory to truncate. -/
theorem length_drop_lt_of_held {α : Type} [DecidableEq α] (a : α) (m : Mem α)
    (h : heldL m a) : (drop a m).length < m.length := by
  induction m with
  | nil => simp at h
  | cons c t ih =>
    by_cases hc : c.1 = a
    · have := length_drop_le a t
      simp [drop, hc]
      omega
    · have ht : heldL t a := h.resolve_left hc
      have := ih ht
      simp [drop, hc]
      omega

theorem getL_drop_of_ne {α : Type} [DecidableEq α] [Inhabited α] (a b : α)
    (hab : a ≠ b) (m : Mem α) : getL (drop a m) b = getL m b := by
  induction m with
  | nil => simp [drop]
  | cons c t ih =>
    by_cases hc : c.1 = a
    · have hcb : c.1 ≠ b := by rw [hc]; exact hab
      simp only [drop, if_pos hc, getL_cons, if_neg hcb]
      exact ih
    · simp only [drop, if_neg hc, getL_cons]
      by_cases hcb : c.1 = b
      · rw [if_pos hcb, if_pos hcb]
      · rw [if_neg hcb, if_neg hcb]
        exact ih

theorem heldL_drop_of_ne {α : Type} [DecidableEq α] (a b : α) (hab : a ≠ b)
    (m : Mem α) : heldL (drop a m) b ↔ heldL m b := by
  induction m with
  | nil => simp [drop]
  | cons c t ih =>
    by_cases hc : c.1 = a
    · have hcb : c.1 ≠ b := by rw [hc]; exact hab
      simp only [drop, if_pos hc, heldL_cons]
      exact ⟨fun h => Or.inr (ih.mp h), fun h => ih.mpr (h.resolve_left hcb)⟩
    · simp only [drop, if_neg hc, heldL_cons]
      by_cases hcb : c.1 = b
      · simp [hcb]
      · simp [hcb, ih]

theorem heldL_exists {α : Type} (m : Mem α) (a : α) :
    heldL m a ↔ ∃ c ∈ m, c.1 = a := by
  induction m with
  | nil => simp [heldL]
  | cons c t ih =>
    simp only [heldL, ih, List.mem_cons]
    constructor
    · rintro (h | ⟨c', hc', hc'a⟩)
      · exact ⟨c, Or.inl rfl, h⟩
      · exact ⟨c', Or.inr hc', hc'a⟩
    · intro h
      obtain ⟨c', hmem, hval⟩ := h
      rcases hmem with hc' | hc'
      · exact Or.inl (hc' ▸ hval)
      · exact Or.inr ⟨c', hc', hval⟩

/-- Taking a prefix cannot introduce a held address: the truncated memory holds
only what the whole one did. -/
theorem heldL_of_heldL_take {α : Type} (n : Nat) (m : Mem α) (b : α)
    (h : heldL (m.take n) b) : heldL m b := by
  obtain ⟨c, hc, hcb⟩ := (heldL_exists _ _).mp h
  exact (heldL_exists _ _).mpr ⟨c, (List.take_sublist n m).subset hc, hcb⟩

/-- Four distinct stores, computed: the list is exactly the three latest cells,
newest first, and the earliest address is gone. Capacity 3 is what makes the
fourth store evict exactly one cell. This is the whole of `at_drop` and
`at_keep_prev`, stated as a list equality so both read off it. -/
theorem putL_four {α : Type} [DecidableEq α] (m : Mem α)
    (x y z w vx vy vz vw : α)
    (hxy : x ≠ y) (hxz : x ≠ z) (hxw : x ≠ w)
    (hyz : y ≠ z) (hyw : y ≠ w) (hzw : z ≠ w) :
    putL (putL (putL (putL m x vx) y vy) z vz) w vw = [(w, vw), (z, vz), (y, vy)] := by
  simp [putL, drop, hxy, hxz, hxw, hyz, hyw, hzw]

/- ── The same operations on the carrier ──────────────────────────────────── -/

section Carrier
variable {α : Type} [DecidableEq α] [Inhabited α]

/-- The empty memory. -/
def init : Cap α := ⟨[], by simp⟩

/-- A store: `put` on a memory of at most three cells is again one, because the
result is a truncation to three. -/
def put (m : Cap α) (a v : α) : Cap α := ⟨putL m.val a v, List.length_take_le 3 _⟩

def get (m : Cap α) (a : α) : α := getL m.val a

def held (m : Cap α) (a : α) : Prop := heldL m.val a

-- `abbrev`, not `def`: `decide` must be able to unfold it to the `Nat`
-- comparison. A bespoke `Decidable (notfull m)` instance is not found by Lean
-- 4.33's search when `m` is a concrete subtype term, and an `inferInstanceAs`
-- body for it reduces to a stuck `Decidable` even when applied explicitly; the
-- abbreviation reaches `Nat`'s decision procedure, which computes.
abbrev notfull (m : Cap α) : Prop := notfullL m.val

/-- The second width-8 store, with the *other* refresh order: a write to a held
address keeps the value the cell already has. The model is the identity —
nothing readable moves, which is exactly what its two laws say, and why a
definition with this ordering cannot make a refresh visible. -/
def keep (m : Cap α) (_a _v : α) : Cap α := m

/-- The carrier is where the capacity lives, so this is free. -/
theorem cap_length (m : Cap α) : m.val.length ≤ 3 := m.2

theorem cap_notfull_iff (m : Cap α) : notfull m ↔ m.val.length ≤ 2 := by
  simp [notfull, notfullL]
  omega

/- ── The toy's axioms, over an arbitrary atom domain ──────────────────────── -/

theorem ax_veq_refl (x : α) : x = x := rfl

theorem ax_veq_trans {x y z : α} (h1 : x = y) (h2 : y = z) : x = z := h1.trans h2

theorem ax_vne_sym {x y : α} (h : x ≠ y) : y ≠ x := fun hyx => h hyx.symm

theorem ax_get_init (a : α) : get (init : Cap α) a = unset α := rfl

theorem ax_init_notfull : notfull (init : Cap α) := by
  simp [notfull, notfullL, init]

theorem ax_put_init_notfull (a v : α) : notfull (put (init : Cap α) a v) := by
  have hval : (put (init : Cap α) a v).val = [(a, v)] := rfl
  simp only [notfull, notfullL, hval]
  simp

/-- Two stores into a fresh memory still leave a free cell: the counting step the
third store needs, and the statement that would be false at capacity two. -/
theorem ax_put2_init_notfull (a b va vb : α) :
    notfull (put (put (init : Cap α) a va) b vb) := by
  have h1 : (put (init : Cap α) a va).val = [(a, va)] := rfl
  have h2 : (put (put (init : Cap α) a va) b vb).val
      = ((b, vb) :: drop b [(a, va)]).take 3 := by
    rw [show (put (put (init : Cap α) a va) b vb).val
        = putL (put (init : Cap α) a va).val b vb from rfl, h1]
    rfl
  have hdrop : (drop b [(a, va)]).length ≤ 1 := by
    by_cases hb : a = b
    · simp [drop, hb]
    · simp [drop, hb]
  have hlen2 : ((b, vb) :: drop b [(a, va)]).length ≤ 2 := by
    simp only [List.length_cons]
    omega
  have hlen : (((b, vb) :: drop b [(a, va)]).take 3).length ≤ 2 := by
    rw [List.take_of_length_le (by omega : ((b, vb) :: drop b [(a, va)]).length ≤ 3)]
    exact hlen2
  simp only [notfull, notfullL, h2]
  omega

theorem ax_at_put (m : Cap α) (a v : α) : get (put m a v) a = v := by
  simp [get, put, putL]

theorem ax_put_held (m : Cap α) (a v : α) : held (put m a v) a := by
  simp [held, put, putL]

theorem ax_put_held_other (m : Cap α) (a b v : α) (h : a ≠ b) (k : held m b)
    (f : notfull m) : held (put m a v) b := by
  have hlen : (drop a m.val).length ≤ 2 := by
    have := length_drop_le a m.val
    have := (cap_notfull_iff m).mp f
    omega
  have hb : heldL (drop a m.val) b := (heldL_drop_of_ne a b h m.val).mpr k
  show heldL (putL m.val a v) b
  have hp : putL m.val a v = (a, v) :: drop a m.val := by
    rw [show putL m.val a v = (a, v) :: (drop a m.val).take 2 from rfl,
      List.take_of_length_le hlen]
  rw [hp, heldL_cons]
  exact Or.inr hb

theorem ax_at_put_held_other (m : Cap α) (x y w : α) (h : x ≠ y) (k : held m x) :
    get (put m x w) y = get m y := by
  have hlen : (drop x m.val).length ≤ 2 := by
    have := length_drop_lt_of_held x m.val k
    have := cap_length m
    omega
  show getL (putL m.val x w) y = getL m.val y
  have hp : putL m.val x w = (x, w) :: drop x m.val := by
    rw [show putL m.val x w = (x, w) :: (drop x m.val).take 2 from rfl,
      List.take_of_length_le hlen]
  rw [hp, getL_cons, if_neg h, getL_drop_of_ne x y h m.val]

theorem ax_at_put_held_keeps (m : Cap α) (x y w : α) (h : x ≠ y) (j : held m y)
    (k : held m x) : held (put m x w) y := by
  have hlen : (drop x m.val).length ≤ 2 := by
    have := length_drop_lt_of_held x m.val k
    have := cap_length m
    omega
  have hy : heldL (drop x m.val) y := (heldL_drop_of_ne x y h m.val).mpr j
  show heldL (putL m.val x w) y
  have hp : putL m.val x w = (x, w) :: drop x m.val := by
    rw [show putL m.val x w = (x, w) :: (drop x m.val).take 2 from rfl,
      List.take_of_length_le hlen]
  rw [hp, heldL_cons]
  exact Or.inr hy

theorem ax_at_put_held_no_new (m : Cap α) (x y w : α) (h : x ≠ y)
    (j : held (put m x w) y) : held m y := by
  have h1 : heldL ((x, w) :: (drop x m.val).take 2) y := j
  have h2 : heldL ((drop x m.val).take 2) y := by
    simpa [heldL, h] using h1
  exact (heldL_drop_of_ne x y h m.val).mp (heldL_of_heldL_take 2 (drop x m.val) y h2)

theorem ax_at_put_other (m : Cap α) (a b v : α) (h : a ≠ b) (k : notfull m) :
    get (put m a v) b = get m b := by
  have hlen : (drop a m.val).length ≤ 2 := by
    have := length_drop_le a m.val
    have := (cap_notfull_iff m).mp k
    omega
  show getL (putL m.val a v) b = getL m.val b
  have hp : putL m.val a v = (a, v) :: drop a m.val := by
    rw [show putL m.val a v = (a, v) :: (drop a m.val).take 2 from rfl,
      List.take_of_length_le hlen]
  rw [hp, getL_cons, if_neg h, getL_drop_of_ne a b h m.val]

theorem ax_at_drop (m : Cap α) (x y z w vx vy vz vw : α)
    (hxy : x ≠ y) (hxz : x ≠ z) (hxw : x ≠ w)
    (hyz : y ≠ z) (hyw : y ≠ w) (hzw : z ≠ w) :
    get (put (put (put (put m x vx) y vy) z vz) w vw) x = unset α := by
  show getL (putL (putL (putL (putL m.val x vx) y vy) z vz) w vw) x = unset α
  rw [putL_four m.val x y z w vx vy vz vw hxy hxz hxw hyz hyw hzw]
  simp [getL, unset, Ne.symm hxw, Ne.symm hxz, Ne.symm hxy]

theorem ax_at_keep_prev (m : Cap α) (x y z w vx vy vz vw : α)
    (hxy : x ≠ y) (hxz : x ≠ z) (hxw : x ≠ w)
    (hyz : y ≠ z) (hyw : y ≠ w) (hzw : z ≠ w) :
    get (put (put (put (put m x vx) y vy) z vz) w vw) y = vy := by
  show getL (putL (putL (putL (putL m.val x vx) y vy) z vz) w vw) y = vy
  rw [putL_four m.val x y z w vx vy vz vw hxy hxz hxw hyz hyw hzw]
  simp [getL, Ne.symm hyw, Ne.symm hyz]

theorem ax_keep_at_put_held (m : Cap α) (x w : α) (_k : held m x) :
    get (keep m x w) x = get m x := rfl

theorem ax_keep_at_put_other (m : Cap α) (x y w : α) (_h : x ≠ y) :
    get (keep m x w) y = get m y := rfl

end Carrier

/- ── The toy's 28 theorems, over the same carrier ───────────────────────────
   The artifact's *theorems*, not its axioms. `store_projection_toy.mm1` proves
   each of them from the axioms by `veq` reasoning; here the proofs are chains of
   the same axiom instances, which makes them a checked statement that the twenty
   axioms are enough for what the artifact claims — if one were missing, the
   theorem that needs it would not close. Names are kept identical to the
   artifact's so the gate can compare the two lists.

   The width-4 theorems take the narrowing as a parameter with idempotence as its
   only law: congruence needs no hypothesis here because `veq` is equality in
   this model, which is exactly what `narrow4_cong` supplies when `veq` is
   abstract. The `Narrow` section above exhibits a lossy `narrow4` satisfying it
   at every `Fin n` with `n ≥ 2`, so the hypothesis is not vacuous. -/

namespace Toy

variable {α : Type} [DecidableEq α] [Inhabited α]

/-- The emitted `store.i64`: the same `put` as the model side. In the real
projection these are different definitions; the toy's licence is that they are
the same term. -/
def jit_store_i64 (m : Cap α) (a v : α) : Cap α := put m a v
def isa_store_w8 (m : Cap α) (a v : α) : Cap α := put m a v
def jit_load_i64 (m : Cap α) (a : α) : α := get m a
def isa_load_w8 (m : Cap α) (a : α) : α := get m a

/-- The same four one width down: the store narrows on the way in, the load on
the way out. -/
def jit_store_i32 (nr : α → α) (m : Cap α) (a v : α) : Cap α := put m a (nr v)
def isa_store_w4 (nr : α → α) (m : Cap α) (a v : α) : Cap α := put m a (nr v)
def jit_load_i32 (nr : α → α) (m : Cap α) (a : α) : α := nr (get m a)
def isa_load_w4 (nr : α → α) (m : Cap α) (a : α) : α := nr (get m a)

/-- The second width-8 store, under its own name. -/
def isa_store_w8_keep (m : Cap α) (a v : α) : Cap α := keep m a v

theorem toy_init_unset (a : α) : get (init : Cap α) a = unset α := ax_get_init a

theorem toy_jit_store_visible (m : Cap α) (a v : α) :
    get (jit_store_i64 m a v) a = v := ax_at_put m a v

theorem toy_isa_store_visible (m : Cap α) (a v : α) :
    get (isa_store_w8 m a v) a = v := ax_at_put m a v

/-- Both sides are the same write, so the projection is two instances of
`at_put`. That is the toy's by-construction licence, stated as a proof rather
than as a comment. -/
theorem toy_store_projection (m : Cap α) (a v : α) :
    get (jit_store_i64 m a v) a = get (isa_store_w8 m a v) a :=
  (ax_at_put m a v).trans (ax_at_put m a v).symm

theorem toy_load_projection (m : Cap α) (a : α) :
    jit_load_i64 m a = isa_load_w8 m a := rfl

theorem toy_jit_store_load_other (m : Cap α) (a b v : α) (h : a ≠ b)
    (k : notfull m) : get (jit_store_i64 m a v) b = get m b :=
  ax_at_put_other m a b v h k

theorem toy_isa_store_load_other (m : Cap α) (a b v : α) (h : a ≠ b)
    (k : notfull m) : get (isa_store_w8 m a v) b = get m b :=
  ax_at_put_other m a b v h k

/-- Two distinct stores into a fresh memory: the second write finds a free cell,
so `at_put_other` applies — with `put_init_notfull`, which is the axiom that
exists for exactly this step. -/
theorem toy_jit_two_cells_preserved (a b va vb : α) (h : a ≠ b) :
    get (jit_store_i64 (jit_store_i64 init a va) b vb) a = va :=
  (ax_at_put_other (put init a va) b a vb h.symm (ax_put_init_notfull a va)).trans
    (ax_at_put init a va)

theorem toy_isa_two_cells_preserved (a b va vb : α) (h : a ≠ b) :
    get (isa_store_w8 (isa_store_w8 init a va) b vb) a = va :=
  (ax_at_put_other (put init a va) b a vb h.symm (ax_put_init_notfull a va)).trans
    (ax_at_put init a va)

/-- After writing `a` and then `b`, the address `a` is still held — a write to a
fresh address into a memory with a free cell keeps what was there, which is
`put_held_other` itself. The refresh theorems need it to know the third write is
a refresh at all; without it the model could not tell a refresh from an append. -/
theorem held_after_second_store (a b va vb : α) (h : a ≠ b) :
    held (put (put init a va) b vb) a :=
  ax_put_held_other (put init a va) b a vb h.symm (ax_put_held init a va)
    (ax_put_init_notfull a va)

/-- A refresh keeps the other cell: after writing `a` then `b`, `a` is still held
(that is `put_held`, a write is held at the address it wrote), so the third write
is a refresh at a held address and `at_put_held_other` leaves `b` alone — and the
value at `b` is whatever the second write put there. -/
theorem toy_jit_refresh_keeps_other_cell (a b va vb va2 : α) (h : a ≠ b) :
    get (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb) a va2) b = vb :=
  (ax_at_put_held_other (put (put init a va) b vb) a b va2 h
    (held_after_second_store a b va vb h)).trans
    (ax_at_put (put init a va) b vb)

theorem toy_isa_refresh_keeps_other_cell (a b va vb va2 : α) (h : a ≠ b) :
    get (isa_store_w8 (isa_store_w8 (isa_store_w8 init a va) b vb) a va2) b = vb :=
  (ax_at_put_held_other (put (put init a va) b vb) a b va2 h
    (held_after_second_store a b va vb h)).trans
    (ax_at_put (put init a va) b vb)

/-- The read half of the refresh case, and the asymmetry that is its content:
**no** hypothesis, where its companion needs `a ≠ b` — what a write reads back at
its own address does not depend on capacity or on what else is held. -/
theorem toy_jit_refresh_visible (a b va vb va2 : α) :
    get (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb) a va2) a = va2 :=
  ax_at_put _ a va2

theorem toy_isa_refresh_visible (a b va vb va2 : α) :
    get (isa_store_w8 (isa_store_w8 (isa_store_w8 init a va) b vb) a va2) a = va2 :=
  ax_at_put _ a va2

theorem toy_jit_refresh_keeps_held_cells (a b va vb va2 : α) (h : a ≠ b) :
    held (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb) a va2) b :=
  ax_put_held_other (put (put init a va) b vb) a b va2 h
    (ax_put_held (put init a va) b vb) (ax_put2_init_notfull a b va vb)

theorem toy_jit_refresh_no_new_held (a b va vb va2 : α) (h : a ≠ b)
    (k : held (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb) a va2) b) :
    held (jit_store_i64 (jit_store_i64 init a va) b vb) b :=
  ax_at_put_held_no_new (put (put init a va) b vb) a b va2 h k

/-- Repetition at one address cannot evict: the second refresh is again a refresh
at a held address, and the memory it starts from is the first refresh's. -/
theorem toy_jit_repeated_refresh_never_evicts (a b va vb va2 va3 : α) (h : a ≠ b) :
    get (jit_store_i64 (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb)
      a va2) a va3) b = vb :=
  (ax_at_put_held_other (put (put (put init a va) b vb) a va2) a b va3 h
    (ax_put_held (put (put init a va) b vb) a va2)).trans
    (toy_jit_refresh_keeps_other_cell a b va vb va2 h)

/-- Depth is a parameter: with a third cell the third distinct store evicts
nothing. -/
theorem toy_jit_third_store_kept (a b c va vb vc : α) (hab : a ≠ b) (hac : a ≠ c) :
    get (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb) c vc) a = va :=
  (ax_at_put_other (put (put init a va) b vb) c a vc hac.symm
    (ax_put2_init_notfull a b va vb)).trans
    (toy_jit_two_cells_preserved a b va vb hab)

theorem toy_jit_fourth_store_drops_oldest (a b c d va vb vc vd : α)
    (hab : a ≠ b) (hac : a ≠ c) (had : a ≠ d)
    (hbc : b ≠ c) (hbd : b ≠ d) (hcd : c ≠ d) :
    get (jit_store_i64 (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb)
      c vc) d vd) a = unset α :=
  ax_at_drop init a b c d va vb vc vd hab hac had hbc hbd hcd

/-- Eviction continues one store at a time: the same `at_drop` fired with a
one-store memory, which is why the axiom is stated for an arbitrary `m`. -/
theorem toy_jit_fifth_store_drops_next (a b c d e va vb vc vd ve : α)
    (hbc : b ≠ c) (hbd : b ≠ d) (hbe : b ≠ e)
    (hcd : c ≠ d) (hce : c ≠ e) (hde : d ≠ e) :
    get (jit_store_i64 (jit_store_i64 (jit_store_i64 (jit_store_i64
      (jit_store_i64 init a va) b vb) c vc) d vd) e ve) b = unset α :=
  ax_at_drop (put init a va) b c d e vb vc vd ve hbc hbd hbe hcd hce hde

theorem toy_jit_recent_cells_readable (a b c d va vb vc vd : α)
    (hab : a ≠ b) (hac : a ≠ c) (had : a ≠ d)
    (hbc : b ≠ c) (hbd : b ≠ d) (hcd : c ≠ d) :
    get (jit_store_i64 (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb)
      c vc) d vd) b = vb :=
  ax_at_keep_prev init a b c d va vb vc vd hab hac had hbc hbd hcd

theorem toy_fourth_store_projection (a b c d va vb vc vd : α) :
    get (jit_store_i64 (jit_store_i64 (jit_store_i64 (jit_store_i64 init a va) b vb)
      c vc) d vd) c
    = get (isa_store_w8 (isa_store_w8 (isa_store_w8 (isa_store_w8 init a va) b vb)
      c vc) d vd) c := rfl

theorem toy_jit_store32_visible (nr : α → α) (m : Cap α) (a v : α) :
    get (jit_store_i32 nr m a v) a = nr v := ax_at_put m a (nr v)

theorem toy_store32_projection (nr : α → α) (m : Cap α) (a v : α) :
    get (jit_store_i32 nr m a v) a = get (isa_store_w4 nr m a v) a :=
  (ax_at_put m a (nr v)).trans (ax_at_put m a (nr v)).symm

theorem toy_load32_projection (nr : α → α) (m : Cap α) (a : α) :
    jit_load_i32 nr m a = isa_load_w4 nr m a := rfl

/-- A width-4 read of a cell written at width 8 returns the narrowed value:
`at_put` gives the value, and the load's narrowing is applied to both sides. -/
theorem toy_width4_load_after_wide_store (nr : α → α) (m : Cap α) (a v : α) :
    jit_load_i32 nr (jit_store_i64 m a v) a = nr v :=
  congrArg nr (ax_at_put m a v)

/-- And the round trip: the stored value is already `nr v`, so narrowing again is
`nr (nr v)`, which idempotence collapses — the one place a width-4 theorem needs
a law of the narrowing beyond congruence. -/
theorem toy_width4_store_load_roundtrip (nr : α → α)
    (hnr : ∀ x, nr (nr x) = nr x) (m : Cap α) (a v : α) :
    jit_load_i32 nr (jit_store_i32 nr m a v) a = nr v :=
  (congrArg nr (ax_at_put m a (nr v))).trans (hnr v)

theorem toy_keep_store_invisible (m : Cap α) (a v : α) (_k : held m a) :
    get (isa_store_w8_keep m a v) a = get m a := rfl

/-- Where the projection fails, on the same trace the refresh theorems use: this
store reads `va`, the value from before the refresh, where the JIT side reads
`va2`. -/
theorem toy_keep_refresh_misses_the_write (a b va vb va2 : α) (_h : a ≠ b) :
    get (isa_store_w8_keep (jit_store_i64 (jit_store_i64 init a va) b vb) a va2) a = va :=
  (ax_keep_at_put_held (put (put init a va) b vb) a va2
    (held_after_second_store a b va vb _h)).trans
    (toy_jit_two_cells_preserved a b va vb _h)

/-- The 28 names, in the order the artifact declares them. -/
def checkedTheorems : List String :=
  ["toy_init_unset", "toy_jit_store_visible", "toy_isa_store_visible",
   "toy_store_projection", "toy_load_projection", "toy_jit_store_load_other",
   "toy_isa_store_load_other", "toy_jit_two_cells_preserved",
   "toy_isa_two_cells_preserved", "toy_jit_refresh_keeps_other_cell",
   "toy_isa_refresh_keeps_other_cell", "toy_jit_refresh_visible",
   "toy_isa_refresh_visible", "toy_jit_refresh_keeps_held_cells",
   "toy_jit_refresh_no_new_held", "toy_jit_repeated_refresh_never_evicts",
   "toy_jit_third_store_kept", "toy_jit_fourth_store_drops_oldest",
   "toy_jit_fifth_store_drops_next", "toy_jit_recent_cells_readable",
   "toy_fourth_store_projection", "toy_jit_store32_visible",
   "toy_store32_projection", "toy_load32_projection",
   "toy_width4_load_after_wide_store", "toy_width4_store_load_roundtrip",
   "toy_keep_store_invisible", "toy_keep_refresh_misses_the_write"]

end Toy

/- ── `narrow4`, for every atom domain with at least two atoms ─────────────── -/

section Narrow

/-- Narrowing to four bytes: a truncation that *loses* information (`x % 2`), so
idempotence is not satisfied by a function that does nothing. Requires only that
the domain has two values to narrow to — the smallest possible hypothesis. -/
def narrow4 {n : Nat} (hn : 2 ≤ n) (x : Fin n) : Fin n :=
  ⟨x.val % 2, Nat.lt_of_lt_of_le (Nat.mod_lt _ (by omega)) hn⟩

theorem ax_narrow4_cong {n : Nat} (hn : 2 ≤ n) {x y : Fin n} (h : x = y) :
    narrow4 hn x = narrow4 hn y := by rw [h]

theorem ax_narrow4_idem {n : Nat} (hn : 2 ≤ n) (x : Fin n) :
    narrow4 hn (narrow4 hn x) = narrow4 hn x := by
  ext
  simp only [narrow4]
  rw [Nat.mod_eq_of_lt (Nat.mod_lt _ (by omega))]

end Narrow

/- ── Where the witness argument breaks ──────────────────────────────────────
   Each statement below is *false* in this model, and proved false here. A
   refutation needs one atom domain, so this section fixes the smallest that
   carries four distinct atoms; the axioms above need no fixed domain at all. -/

section Broken

abbrev Atom4 := Fin 4

-- `notfull` is an abbreviation for a length bound, so `decide` unfolds it to
-- the `Nat` comparison and uses `Nat`'s decision procedure; `held` is a
-- recursion, so the refutations below compute it instead of deciding it.

/-- An earlier revision's closure axiom: `notfull` survives any write. False the
moment the write takes the third cell — which is exactly what a capacity-3 memory
does, and what `at_drop` requires. Its negation is the axiom
`put2_init_notfull`, which says where the closure does hold. -/
theorem put_notfull_breaks :
    ¬ (∀ (m : Cap Atom4) (a v : Atom4), notfull m → notfull (put m a v)) := by
  intro h
  have hm : notfull (⟨[(0, 0), (1, 1)], by decide⟩ : Cap Atom4) := by decide
  have h3 := h ⟨[(0, 0), (1, 1)], by decide⟩ 2 0 hm
  have hlen : (put (⟨[(0, 0), (1, 1)], by decide⟩ : Cap Atom4) 2 0).val.length = 3 := rfl
  show False
  have : ¬ notfull (put (⟨[(0, 0), (1, 1)], by decide⟩ : Cap Atom4) 2 0) := by
    show ¬ ((put (⟨[(0, 0), (1, 1)], by decide⟩ : Cap Atom4) 2 0).val.length < 3)
    omega
  exact this h3

/-- `at_put_other` without `notfull`: a write to a fresh address in a full memory
evicts, so the other cell's content is not preserved. -/
theorem at_put_other_unguarded_breaks :
    ¬ (∀ (m : Cap Atom4) (a b v : Atom4), a ≠ b → get (put m a v) b = get m b) := by
  intro h
  have h3 : get (put (⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ : Cap Atom4) 3 0) 2
      = get (⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ : Cap Atom4) 2 :=
    h ⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ 3 2 0 (by decide)
  exact absurd h3 (by decide)

/-- `put_held_other` without `notfull`: the same write loses the held cell. -/
theorem put_held_other_unguarded_breaks :
    ¬ (∀ (m : Cap Atom4) (a b v : Atom4), a ≠ b → held m b → held (put m a v) b) := by
  intro h
  have hk : held (⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ : Cap Atom4) 2 := by
    simp [held, heldL]
  have h3 : held (put (⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ : Cap Atom4) 3 0) 2 :=
    h ⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ 3 2 0 (by decide) hk
  have hv : (put (⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ : Cap Atom4) 3 0).val
      = [((3 : Atom4), (0 : Atom4)), (0, 0), (1, 1)] := rfl
  have hnot : ¬ held (put (⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ : Cap Atom4) 3 0) 2 := by
    show ¬ heldL (put (⟨[(0, 0), (1, 1), (2, 2)], by decide⟩ : Cap Atom4) 3 0).val 2
    rw [hv]
    simp [heldL]
  exact hnot h3

/-- The eviction axiom the two-cell revision carried: after *three* distinct
stores the earliest address reads `unset`. False with a third cell, which is the
whole point of adding it — a checker that still accepted it would be modelling
the old memory. -/
theorem at_drop_at_two_cells_breaks :
    ¬ (∀ (m : Cap Atom4) (x y z vx vy vz : Atom4), x ≠ y → x ≠ z → y ≠ z →
        get (put (put (put m x vx) y vy) z vz) x = unset Atom4) := by
  intro h
  have h3 : get (put (put (put (init : Cap Atom4) 0 1) 1 1) 2 1) 0 = unset Atom4 :=
    h (init : Cap Atom4) 0 1 2 1 1 1 (by decide) (by decide) (by decide)
  exact absurd h3 (by decide)

/-- "The second width-8 store is visible at the address it wrote" — the
projection instance the other four theorems would need. False under the other
refresh order, and false here: the store keeps the old value, which is where the
projection breaks. -/
theorem keep_store_visible_breaks :
    ¬ (∀ (m : Cap Atom4) (a v : Atom4), get (keep m a v) a = v) := by
  intro h
  have h3 : get (keep (init : Cap Atom4) 0 1) 0 = 1 := h (init : Cap Atom4) 0 1
  exact absurd h3 (by decide)

/-- "Narrowing to four bytes is the identity" — false, and if it held the width-4
theorems would say nothing: a value that fits in four bytes is not every value. -/
theorem narrow4_is_identity_breaks : ¬ (∀ x : Fin 3, narrow4 (by decide) x = x) := by
  intro h
  have h3 : narrow4 (by decide : 2 ≤ 3) 2 = 2 := h 2
  exact absurd h3 (by decide)

end Broken

/- ── The argument's other limit: a degenerate model of the same axioms ───────
   `veq` is uninterpreted, and the toy's axioms only demand reflexivity and
   transitivity. So the *universal* relation is also a model: every `veq`
   conclusion is trivially true, and the axioms still hold. That means
   consistency alone is cheap — it does not say the axioms constrain `get`/`put`
   in any way. What carries the toy's content is the *instance* proofs (which
   rest on `at_put`, `at_drop`, …) and this witness, not satisfiability. -/

section Degenerate

variable {α : Type} [DecidableEq α] [Inhabited α]

def veqUniversal (_ _ : α) : Prop := True

theorem veq_universal_satisfies_trans (x y z : α) :
    veqUniversal x y → veqUniversal y z → veqUniversal x z :=
  fun _ _ => trivial

theorem veq_universal_satisfies_at_put (m : Cap α) (a v : α) :
    veqUniversal (get (put m a v) a) v := trivial

end Degenerate


/- ── The trust base, printed for the gate ────────────────────────────────────
   `#print axioms` reports what each declaration actually rests on. The point is
   not decoration: `sorryAx` would appear here, and so would any principle a
   later edit introduced. No axiom of the artifact appears as an axiom — each is a
   theorem here — and the proofs are structural, so what is left is Lean's standard
   foundation and nothing else: `propext`, `Classical.choice`, `Quot.sound`,
   pulled in by `simp`/`omega` rather than by the statements (the `veq_*` and
   `keep_at_put_*` proofs rest on nothing at all). The gate requires all 48 lines
   (the toy's 28 theorems plus the 20 axiom-statement theorems) to name a subset of
   those three, and to be 48 of them, so a `sorry` that still elaborated would be
   red rather than quiet. This is the audit shape `hydro` uses in
   `hydro_lean/Hydro/AxCheck.lean`, with the same allowed set. -/

/- The artifact's axioms, as theorems: -/
#print axioms ax_veq_refl
#print axioms ax_veq_trans
#print axioms ax_vne_sym
#print axioms ax_get_init
#print axioms ax_init_notfull
#print axioms ax_put_init_notfull
#print axioms ax_put2_init_notfull
#print axioms ax_at_put
#print axioms ax_put_held
#print axioms ax_put_held_other
#print axioms ax_at_put_held_other
#print axioms ax_at_put_held_keeps
#print axioms ax_at_put_held_no_new
#print axioms ax_at_put_other
#print axioms ax_at_drop
#print axioms ax_at_keep_prev
#print axioms ax_keep_at_put_held
#print axioms ax_keep_at_put_other
#print axioms ax_narrow4_cong
#print axioms ax_narrow4_idem

/- The artifact's theorems: -/
#print axioms Toy.toy_init_unset
#print axioms Toy.toy_jit_store_visible
#print axioms Toy.toy_isa_store_visible
#print axioms Toy.toy_store_projection
#print axioms Toy.toy_load_projection
#print axioms Toy.toy_jit_store_load_other
#print axioms Toy.toy_isa_store_load_other
#print axioms Toy.toy_jit_two_cells_preserved
#print axioms Toy.toy_isa_two_cells_preserved
#print axioms Toy.toy_jit_refresh_keeps_other_cell
#print axioms Toy.toy_isa_refresh_keeps_other_cell
#print axioms Toy.toy_jit_refresh_visible
#print axioms Toy.toy_isa_refresh_visible
#print axioms Toy.toy_jit_refresh_keeps_held_cells
#print axioms Toy.toy_jit_refresh_no_new_held
#print axioms Toy.toy_jit_repeated_refresh_never_evicts
#print axioms Toy.toy_jit_third_store_kept
#print axioms Toy.toy_jit_fourth_store_drops_oldest
#print axioms Toy.toy_jit_fifth_store_drops_next
#print axioms Toy.toy_jit_recent_cells_readable
#print axioms Toy.toy_fourth_store_projection
#print axioms Toy.toy_jit_store32_visible
#print axioms Toy.toy_store32_projection
#print axioms Toy.toy_load32_projection
#print axioms Toy.toy_width4_load_after_wide_store
#print axioms Toy.toy_width4_store_load_roundtrip
#print axioms Toy.toy_keep_store_invisible
#print axioms Toy.toy_keep_refresh_misses_the_write

/- ── Summary the gate reads ─────────────────────────────────────────────────
   Printed at elaboration so `verify-invariants` can compare it with the axiom
   list in `mm0/store_projection_toy.mm0` and fail when a new axiom has no Lean
   statement here. -/

def checkedAxioms : List String :=
  ["veq_refl", "veq_trans", "vne_sym", "narrow4_cong", "narrow4_idem", "get_init",
   "init_notfull", "put_init_notfull", "at_put", "put_held", "put_held_other",
   "at_put_held_other", "at_put_held_keeps", "at_put_held_no_new", "at_put_other",
   "at_drop", "at_keep_prev", "keep_at_put_held", "keep_at_put_other",
   "put2_init_notfull"]

def brokenCandidates : List String :=
  ["put_notfull", "at_put_other_unguarded", "put_held_other_unguarded",
   "at_drop_at_two_cells", "keep_store_visible", "narrow4_is_identity"]

#eval do
  IO.println ("lean-witness: " ++ toString checkedAxioms.length ++
    " axiom(s) proved for an arbitrary atom domain, " ++
    toString brokenCandidates.length ++ " rejected candidate(s) proved false")
  IO.println ("lean-witness-axioms: " ++ String.intercalate "," checkedAxioms)
  IO.println ("lean-witness-broken: " ++ String.intercalate "," brokenCandidates)
  IO.println ("lean-witness-theorems: " ++ String.intercalate "," Toy.checkedTheorems)

end ToyWitness
