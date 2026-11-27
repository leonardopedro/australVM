/-
  D21: machine-checked induction over arbitrary runs of the three claims whose
  finite models live in `unfer_consensus/tests/*_invariant.rs` and whose
  symbolic proofs live in `docs/distributed/why3/*.mlw`.

  WHY THIS FILE EXISTS. The Why3 theories prove each claim over an arbitrary
  operation list with an induction principle (`let rec lemma ... variant
  { ops }`). This file states the same transition relations and the same run
  invariant in Lean 4, where the induction is the kernel's own `List` recursor:
  no `axiom`, no `parameter`, no solver in the loop. `scripts/check-lean-d21`
  runs the file and requires `#print axioms` to report that every theorem
  depends on nothing (or only on Lean's three standard axioms, which this file
  does not use).

  Each namespace mirrors one Why3 module one-to-one, so the two checkers can be
  compared clause by clause:

    * `Epoch`   <-> `docs/distributed/why3/epoch_monotonicity.mlw`
    * `Fence`   <-> `docs/distributed/why3/fence_invariant.mlw`
    * `Replay`  <-> `docs/distributed/why3/request_replay.mlw`

  WHAT IS NOT MODELLED. Same limits as the Why3 theories: the epoch model is
  one resource; the fence model reduces the holder to a boolean and uses the
  abstracted finite generation top; the replay model has two incarnations and
  two ids, and abstracts digests to their equality class. Those wider finite
  domains are the Rust model checks' job, not this file's.
-/

namespace D21

/-! ## Epoch monotonicity

  Mirrors `epoch_monotonicity.mlw`: claim, renew, release, settle, disable and
  a takeover compare-and-swap, with `nextGen` as `FenceTable::next_position`
  without the wrapping top. -/
namespace Epoch

/-- The committed fence state: `epoch` 0 means no fence has been issued. -/
structure State where
  epoch : Int
  gen : Int
  held : Bool
deriving DecidableEq, Repr

def initial : State := ⟨0, 0, false⟩

/-- Well-formedness: E3 (`held` implies a real authority epoch) plus
non-negativity of both counters. -/
def Wf (s : State) : Prop :=
  0 ≤ s.epoch ∧ 0 ≤ s.gen ∧ (s.held = true → 1 ≤ s.epoch)

def nextGen (s : State) : State := ⟨s.epoch, s.gen + 1, s.held⟩

/-- The takeover compare-and-swap: accepted only under a strictly greater
epoch, and it installs generation 1 (E1, E4). -/
def takeover (s : State) (proposed : Int) : State :=
  if proposed > s.epoch then ⟨proposed, 1, true⟩ else s

def claim (s : State) : State :=
  if s.held = true then s
  else if s.epoch = 0 then ⟨1, 1, true⟩
  else nextGen ⟨s.epoch, s.gen, true⟩

inductive Op where
  | claim
  | renew
  | release
  | settle
  | disable
  | takeover (proposed : Int)
deriving DecidableEq, Repr

def step (s : State) : Op → State
  | .claim => claim s
  | .renew => if s.held = true then nextGen s else s
  | .release => ⟨s.epoch, s.gen, false⟩
  | .settle => ⟨s.epoch, s.gen, false⟩
  | .disable => ⟨s.epoch, s.gen, false⟩
  | .takeover p => takeover s p

theorem nextGen_preserves_wf (s : State) (h : Wf s) : Wf (nextGen s) := by
  obtain ⟨h0, h1, h2⟩ := h
  refine ⟨by simpa [nextGen] using h0, by simp only [nextGen]; omega, ?_⟩
  simpa [nextGen] using h2

theorem claim_preserves_wf (s : State) (h : Wf s) : Wf (claim s) := by
  obtain ⟨h0, h1, h2⟩ := h
  unfold claim
  by_cases hh : s.held = true
  · rw [if_pos hh]; exact ⟨h0, h1, h2⟩
  · rw [if_neg hh]
    by_cases he : s.epoch = 0
    · rw [if_pos he]; simp [Wf]
    · rw [if_neg he]
      refine nextGen_preserves_wf _ ⟨h0, h1, ?_⟩
      intro _
      show 1 ≤ s.epoch
      omega

theorem takeover_preserves_wf (s : State) (p : Int) (h : Wf s) :
    Wf (takeover s p) := by
  obtain ⟨h0, h1, h2⟩ := h
  unfold takeover
  by_cases hp : p > s.epoch
  · rw [if_pos hp]; simp only [Wf]; omega
  · rw [if_neg hp]; exact ⟨h0, h1, h2⟩

theorem release_preserves_wf (s : State) (h : Wf s) : Wf ⟨s.epoch, s.gen, false⟩ := by
  obtain ⟨h0, h1, _⟩ := h
  simp only [Wf]
  exact ⟨h0, h1, by simp⟩

theorem step_preserves_wf (s : State) (o : Op) (h : Wf s) : Wf (step s o) := by
  cases o with
  | claim => exact claim_preserves_wf s h
  | renew =>
      simp only [step]
      by_cases hh : s.held = true
      · rw [if_pos hh]; exact nextGen_preserves_wf s h
      · rw [if_neg hh]; exact h
  | release => exact release_preserves_wf s h
  | settle => exact release_preserves_wf s h
  | disable => exact release_preserves_wf s h
  | takeover p => exact takeover_preserves_wf s p h

theorem takeover_accepted_epoch (s : State) (p : Int) (hp : p > s.epoch) :
    (takeover s p).epoch = p := by
  unfold takeover
  rw [if_pos hp]

theorem accepted_takeover_strictly_raises (s : State) (p : Int)
    (_h : Wf s) (hp : p > s.epoch) :
    (takeover s p).epoch = p ∧ (takeover s p).epoch > s.epoch ∧
      (takeover s p).gen = 1 := by
  unfold takeover
  rw [if_pos hp]
  exact ⟨rfl, hp, rfl⟩

theorem stale_takeover_changes_nothing (s : State) (p : Int)
    (_h : Wf s) (hp : p ≤ s.epoch) : takeover s p = s := by
  unfold takeover
  rw [if_neg (by omega)]

theorem first_epoch_is_authority (s : State) (o : Op) (h : Wf s)
    (h0 : s.epoch = 0) (hh : (step s o).held = true) :
    1 ≤ (step s o).epoch := by
  obtain ⟨_, _, hheld⟩ := h
  cases o with
  | claim =>
      simp only [step]
      unfold claim
      by_cases hb : s.held = true
      · rw [if_pos hb]
        have := hheld hb
        omega
      · rw [if_neg hb, if_pos h0]
        simp
  | renew =>
      simp only [step]
      by_cases hb : s.held = true
      · rw [if_pos hb]
        simp only [nextGen]
        have := hheld hb
        omega
      · simp only [step] at hh
        rw [if_neg hb] at hh
        exact absurd hh hb
  | release => simp [step] at hh
  | settle => simp [step] at hh
  | disable => simp [step] at hh
  | takeover p =>
      simp only [step]
      unfold takeover
      by_cases hp : p > s.epoch
      · rw [if_pos hp]
        show 1 ≤ p
        omega
      · rw [if_neg hp]
        have := hheld (by simpa [step, takeover, hp] using hh)
        omega

theorem step_non_decreasing (s : State) (o : Op) (h : Wf s) :
    s.epoch ≤ (step s o).epoch := by
  obtain ⟨h0, _, _⟩ := h
  cases o with
  | claim =>
      simp only [step]
      unfold claim
      by_cases hb : s.held = true
      · rw [if_pos hb]; omega
      · rw [if_neg hb]
        by_cases he : s.epoch = 0
        · rw [if_pos he]
          show s.epoch ≤ 1
          omega
        · rw [if_neg he]; simp only [nextGen]; omega
  | renew =>
      simp only [step]
      by_cases hb : s.held = true
      · rw [if_pos hb]; simp only [nextGen]; omega
      · rw [if_neg hb]; omega
  | release => simp [step]
  | settle => simp [step]
  | disable => simp [step]
  | takeover p =>
      simp only [step]
      unfold takeover
      by_cases hp : p > s.epoch
      · rw [if_pos hp]
        show s.epoch ≤ p
        omega
      · rw [if_neg hp]
        omega

/-- The operation list is the run: a rejected operation is the identity in
`step`, so an arbitrary run is `List` recursion, and the claim about every run
is an induction on that list. -/
def run (s : State) : List Op → State
  | [] => s
  | o :: rest => run (step s o) rest

theorem run_preserves_wf (s : State) (ops : List Op) (h : Wf s) :
    Wf (run s ops) := by
  induction ops generalizing s with
  | nil => exact h
  | cons o rest ih => exact ih (step s o) (step_preserves_wf s o h)

/-- E2 over an arbitrary run: no operation list can lower the committed epoch. -/
theorem run_non_decreasing (s : State) (ops : List Op) (h : Wf s) :
    s.epoch ≤ (run s ops).epoch := by
  induction ops generalizing s with
  | nil => exact Int.le_refl s.epoch
  | cons o rest ih =>
      show s.epoch ≤ (run (step s o) rest).epoch
      have h1 : s.epoch ≤ (step s o).epoch := step_non_decreasing s o h
      have h2 : (step s o).epoch ≤ (run (step s o) rest).epoch :=
        ih (step s o) (step_preserves_wf s o h)
      omega

end Epoch

/-! ## Fence invariant

  Mirrors `fence_invariant.mlw`: one resource, the holder reduced to a boolean,
  the generation top abstracted to `maxGen`. The run-level theorem is
  `run_fence_le`: across an arbitrary operation list the fence never sorts
  below where it started, which is I2's "a released-and-reclaimed resource
  never hands out a fence a sink has already passed". -/
namespace Fence

/-- `JobState` labels, as in the Why3 theory. -/
def queued : Int := 0
def claimed : Int := 1
def fired : Int := 2
def disabled : Int := 3

/-- The abstracted finite generation top used by the Rust model check. -/
def maxGen : Int := 3

structure State where
  hasFence : Bool
  epoch : Int
  generation : Int
  hasHolder : Bool
  phase : Int
  released : Bool
deriving DecidableEq, Repr

def initial : State := ⟨false, 0, 0, false, queued, false⟩

/-- I3's lifecycle invariant, plus "a fence is never issued at epoch or
 generation zero". -/
def Inv (s : State) : Prop :=
  ((s.phase = claimed) ↔ (s.hasHolder = true ∧ s.released = false)) ∧
  (s.released = true → s.hasFence = true) ∧
  (s.hasFence = false → s.phase = queued) ∧
  (s.hasFence = true → 1 ≤ s.epoch ∧ 1 ≤ s.generation)

def current (s : State) : Prop :=
  s.phase = claimed ∧ s.hasFence = true ∧ s.released = false

def held (s : State) : Prop := s.hasFence = true ∧ s.released = false

-- `held`/`current` are abbreviations, so typeclass search does not see their
-- `Decidable` instances through the definition; provide them explicitly.
instance (s : State) : Decidable (held s) := by
  unfold held; infer_instance

instance (s : State) : Decidable (current s) := by
  unfold current; infer_instance

def older (a b : State) : Prop :=
  a.epoch < b.epoch ∨ (a.epoch = b.epoch ∧ a.generation < b.generation)

def nextEpoch (s : State) : Int :=
  if s.generation < maxGen then s.epoch else s.epoch + 1

def nextGeneration (s : State) : Int :=
  if s.generation < maxGen then s.generation + 1 else 1

/-- `FenceTable::next_position` on a held fence. The caller must hold a fence;
`claim` issues the literal `(1,1)` when there is none, which is why this
function alone is not invariant-preserving from `initial`. -/
def issue (s : State) : State :=
  ⟨true, nextEpoch s, nextGeneration s, true, claimed, false⟩

def claim (s : State) : State :=
  if s.phase = queued ∧ ¬ held s then
    if s.hasFence = true then issue s
    else ⟨true, 1, 1, true, claimed, false⟩
  else s

inductive Op where
  | claim
  | renew
  | reclaim
  | release
  | settle
  | disable
  deriving DecidableEq, Repr

def step (s : State) : Op → State
  | .claim => claim s
  | .renew => if current s then issue s else s
  | .reclaim => claim s
  | .release =>
      if current s then ⟨s.hasFence, s.epoch, s.generation, false, queued, true⟩
      else s
  | .settle =>
      if current s then ⟨s.hasFence, s.epoch, s.generation, false, fired, true⟩
      else s
  | .disable =>
      if current s then ⟨s.hasFence, s.epoch, s.generation, false, disabled, true⟩
      else s

theorem issue_epoch_pos (s : State) (h : 1 ≤ s.epoch) : 1 ≤ (issue s).epoch := by
  unfold issue nextEpoch
  by_cases hg : s.generation < maxGen
  · rw [if_pos hg]; exact h
  · rw [if_neg hg]; simp; omega

theorem issue_generation_pos (s : State) (h : 1 ≤ s.generation) :
    1 ≤ (issue s).generation := by
  unfold issue nextGeneration
  by_cases hg : s.generation < maxGen
  · rw [if_pos hg]; simp; omega
  · rw [if_neg hg]; simp

theorem issue_preserves_inv (s : State) (h : Inv s) (hf : s.hasFence = true) :
    Inv (issue s) := by
  obtain ⟨h1, h2, h3, h4⟩ := h
  obtain ⟨he, hg⟩ := h4 hf
  refine ⟨?_, ?_, ?_, ?_⟩
  · simp [issue]
  · simp [issue]
  · simp [issue]
  · intro _
    exact ⟨issue_epoch_pos s he, issue_generation_pos s hg⟩

theorem fresh_claim_preserves_inv (s : State) (_h : Inv s) :
    Inv (⟨true, 1, 1, true, claimed, false⟩ : State) := by
  refine ⟨?_, ?_, ?_, ?_⟩ <;> simp

theorem claim_preserves_inv (s : State) (h : Inv s) : Inv (claim s) := by
  unfold claim
  by_cases hq : s.phase = queued ∧ ¬ held s
  · rw [if_pos hq]
    by_cases hf : s.hasFence = true
    · rw [if_pos hf]; exact issue_preserves_inv s h hf
    · rw [if_neg hf]; exact fresh_claim_preserves_inv s h
  · rw [if_neg hq]; exact h

theorem fire_preserves_inv (s : State) (h : Inv s) (hf : s.hasFence = true)
    (p : Int) (hp : p ≠ claimed) :
    Inv ⟨s.hasFence, s.epoch, s.generation, false, p, true⟩ := by
  obtain ⟨h1, h2, h3, h4⟩ := h
  refine ⟨?_, ?_, ?_, ?_⟩
  · simp [hp]
  · intro _; exact hf
  · intro hf'
    rw [hf'] at hf
    exact absurd hf (by simp)
  · exact h4

theorem step_preserves_inv (s : State) (o : Op) (h : Inv s) : Inv (step s o) := by
  cases o with
  | claim => exact claim_preserves_inv s h
  | reclaim => exact claim_preserves_inv s h
  | renew =>
      simp only [step]
      by_cases hc : current s
      · rw [if_pos hc]; obtain ⟨_, hf, _⟩ := hc
        exact issue_preserves_inv s h hf
      · rw [if_neg hc]; exact h
  | release =>
      simp only [step]
      by_cases hc : current s
      · rw [if_pos hc]
        obtain ⟨_, hf, _⟩ := hc
        exact fire_preserves_inv s h hf queued (by simp [queued, claimed])
      · rw [if_neg hc]; exact h
  | settle =>
      simp only [step]
      by_cases hc : current s
      · rw [if_pos hc]
        obtain ⟨_, hf, _⟩ := hc
        exact fire_preserves_inv s h hf fired (by simp [fired, claimed])
      · rw [if_neg hc]; exact h
  | disable =>
      simp only [step]
      by_cases hc : current s
      · rw [if_pos hc]
        obtain ⟨_, hf, _⟩ := hc
        exact fire_preserves_inv s h hf disabled (by simp [disabled, claimed])
      · rw [if_neg hc]; exact h

/-- I1: a settle that changes anything was a current fence. -/
theorem settle_applies_only_when_current (s : State) (_h : Inv s)
    (hs : step s Op.settle ≠ s) : current s := by
  simp only [step] at hs
  by_cases hc : current s
  · exact hc
  · rw [if_neg hc] at hs; exact absurd rfl hs

/-- I1's contrapositive: a settle against a stale fence is the identity. -/
theorem stale_settle_changes_nothing (s : State) (_h : Inv s) (hc : ¬ current s) :
    step s Op.settle = s := by
  simp only [step]
  rw [if_neg hc]

/-- The fence order used by `run_fence_le`. `false` on the left is below
anything, matching "no fence has been issued yet". -/
def fenceLe (a b : State) : Prop :=
  a.hasFence = false ∨
    (b.hasFence = true ∧
      (a.epoch < b.epoch ∨ (a.epoch = b.epoch ∧ a.generation ≤ b.generation)))

theorem issue_older (s : State) (_h : s.hasFence = true) : older s (issue s) := by
  unfold issue nextEpoch nextGeneration older
  by_cases hg : s.generation < maxGen
  · rw [if_pos hg, if_pos hg]; right; constructor <;> simp; omega
  · rw [if_neg hg, if_neg hg]; left; simp; omega

theorem fenceLe_refl (s : State) : fenceLe s s := by
  unfold fenceLe
  by_cases hf : s.hasFence = true
  · right; exact ⟨hf, Or.inr ⟨rfl, Int.le_refl _⟩⟩
  · left; simpa using hf

theorem fenceLe_of_older (a b : State) (_ha : a.hasFence = true)
    (hb : b.hasFence = true) (ho : older a b) : fenceLe a b := by
  unfold fenceLe older at *
  exact Or.inr ⟨hb, by omega⟩

theorem fenceLe_trans (a b c : State) (h1 : fenceLe a b) (h2 : fenceLe b c) :
    fenceLe a c := by
  unfold fenceLe at *
  rcases h1 with hf | ⟨hbf, hord⟩
  · exact Or.inl hf
  · rcases h2 with hbf' | ⟨hcf, hord2⟩
    · rw [hbf] at hbf'; exact absurd hbf' (by simp)
    · right
      refine ⟨hcf, ?_⟩
      rcases hord with e | ⟨e, g⟩ <;> rcases hord2 with f | ⟨f, g'⟩ <;> omega

theorem issue_fenceLe (s : State) (_h : Inv s) (hf : s.hasFence = true) :
    fenceLe s (issue s) := by
  unfold issue
  exact fenceLe_of_older s ⟨true, nextEpoch s, nextGeneration s, true, claimed, false⟩
    hf rfl (issue_older s hf)

theorem claim_fenceLe (s : State) (h : Inv s) : fenceLe s (claim s) := by
  unfold claim
  by_cases hq : s.phase = queued ∧ ¬ held s
  · rw [if_pos hq]
    by_cases hf : s.hasFence = true
    · rw [if_pos hf]; exact issue_fenceLe s h hf
    · rw [if_neg hf]
      unfold fenceLe
      left
      simpa using hf
  · rw [if_neg hq]; exact fenceLe_refl s

theorem step_fenceLe (s : State) (o : Op) (h : Inv s) : fenceLe s (step s o) := by
  cases o with
  | claim => exact claim_fenceLe s h
  | reclaim => exact claim_fenceLe s h
  | renew =>
      simp only [step]
      by_cases hc : current s
      · rw [if_pos hc]; obtain ⟨_, hf, _⟩ := hc
        exact issue_fenceLe s h hf
      · rw [if_neg hc]; exact fenceLe_refl s
  | release =>
      simp only [step]
      by_cases hc : current s
      · rw [if_pos hc]
        unfold fenceLe
        by_cases hf : s.hasFence = true
        · right; exact ⟨hf, Or.inr ⟨rfl, by simp⟩⟩
        · left; simpa using hf
      · rw [if_neg hc]; exact fenceLe_refl s
  | settle =>
      simp only [step]
      by_cases hc : current s
      · rw [if_pos hc]
        unfold fenceLe
        by_cases hf : s.hasFence = true
        · right; exact ⟨hf, Or.inr ⟨rfl, by simp⟩⟩
        · left; simpa using hf
      · rw [if_neg hc]; exact fenceLe_refl s
  | disable =>
      simp only [step]
      by_cases hc : current s
      · rw [if_pos hc]
        unfold fenceLe
        by_cases hf : s.hasFence = true
        · right; exact ⟨hf, Or.inr ⟨rfl, by simp⟩⟩
        · left; simpa using hf
      · rw [if_neg hc]; exact fenceLe_refl s

/-- The run over an arbitrary operation list. -/
def run (s : State) : List Op → State
  | [] => s
  | o :: rest => run (step s o) rest

theorem run_preserves_inv (s : State) (ops : List Op) (h : Inv s) :
    Inv (run s ops) := by
  induction ops generalizing s with
  | nil => exact h
  | cons o rest ih => exact ih (step s o) (step_preserves_inv s o h)

/-- I2 over an arbitrary run: the fence never sorts below the state the run
started from. -/
theorem run_fence_le (s : State) (ops : List Op) (h : Inv s) :
    fenceLe s (run s ops) := by
  induction ops generalizing s with
  | nil => exact fenceLe_refl s
  | cons o rest ih =>
      show fenceLe s (run (step s o) rest)
      have hs : fenceLe s (step s o) := step_fenceLe s o h
      have hnext : fenceLe (step s o) (run (step s o) rest) :=
        ih (step s o) (step_preserves_inv s o h)
      exact fenceLe_trans s (step s o) (run (step s o) rest) hs hnext

end Fence

/-! ## Request replay

  Mirrors `request_replay.mlw`: `AdmissionBook`'s decision table in the order the
  implementation checks it (floor, record, capacity, incarnation bounds) for two
  incarnations and two ids.

  R1--R4 are stated over *every* book, not only reachable ones: `admit` is a
  function of the book alone, so a property of its decision table does not need
  a reachability premise, and the run induction here is the one the claims do
  need -- the R6 accounting invariant, which says `retained` counts exactly the
  live entries after an arbitrary sequence of records, settles and
  acknowledgements. -/
namespace Replay

inductive Outcome where
  | pending (sub : Int)
  | decided (value : Int)
deriving DecidableEq, Repr

structure Entry where
  digest : Int
  outcome : Outcome
deriving DecidableEq, Repr

structure Window where
  floor : Int
  has0 : Bool
  e0 : Entry
  has1 : Bool
  e1 : Entry
deriving DecidableEq, Repr

structure Book where
  holds0 : Bool
  w0 : Window
  holds1 : Bool
  w1 : Window
  retained : Int
  capacity : Int
  maxScopes : Int
  maxIncarnations : Int
deriving DecidableEq, Repr

def dummy : Entry := ⟨0, .pending 0⟩

def emptyWindow : Window := ⟨0, false, dummy, false, dummy⟩

def lookup (w : Window) (seq : Int) : Option Entry :=
  if seq = 0 then (if w.has0 = true then some w.e0 else none)
  else if seq = 1 then (if w.has1 = true then some w.e1 else none)
  else none

def windowOf (b : Book) (inc : Int) : Option Window :=
  if inc = 0 then (if b.holds0 = true then some b.w0 else none)
  else if inc = 1 then (if b.holds1 = true then some b.w1 else none)
  else none

def present (b : Book) (inc seq : Int) : Bool :=
  match windowOf b inc with
  | some w => (lookup w seq).isSome
  | none => false

inductive Verdict where
  | fresh
  | duplicate (o : Outcome)
  | conflict (recorded presented : Int)
  | belowRetention (floor : Int)
  | atCapacity (bound : Int)
  | incarnationsExhausted (bound live : Int)
deriving DecidableEq, Repr

def live (b : Book) : Int :=
  (if b.holds0 = true then 1 else 0) + (if b.holds1 = true then 1 else 0)

def admit (b : Book) (inc seq digest : Int) : Verdict :=
  match windowOf b inc with
  | some w =>
      if seq ≤ w.floor then .belowRetention w.floor
      else match lookup w seq with
           | some e =>
               if e.digest = digest then .duplicate e.outcome
               else .conflict e.digest digest
           | none => if b.retained ≥ b.capacity then .atCapacity b.capacity else .fresh
  | none =>
      if b.retained ≥ b.capacity then .atCapacity b.capacity
      else if live b ≥ b.maxIncarnations then
        .incarnationsExhausted b.maxIncarnations (live b)
      else if live b ≥ b.maxScopes then .atCapacity b.maxScopes
      else .fresh

/-! ### R1--R3: the decision table, for every book -/

theorem replay_is_the_recorded_outcome (b : Book) (inc seq digest : Int)
    (w : Window) (e : Entry) (hw : windowOf b inc = some w)
    (hl : lookup w seq = some e) (hfloor : w.floor < seq)
    (hd : e.digest = digest) :
    admit b inc seq digest = .duplicate e.outcome := by
  unfold admit
  rw [hw]
  dsimp only
  rw [if_neg (by omega), hl]
  dsimp only
  rw [if_pos hd]

theorem equivocation_is_a_conflict (b : Book) (inc seq digest : Int)
    (w : Window) (e : Entry) (hw : windowOf b inc = some w)
    (hl : lookup w seq = some e) (hfloor : w.floor < seq)
    (hd : e.digest ≠ digest) :
    admit b inc seq digest = .conflict e.digest digest := by
  unfold admit
  rw [hw]
  dsimp only
  rw [if_neg (by omega), hl]
  dsimp only
  rw [if_neg hd]

theorem below_floor_is_refused (b : Book) (inc seq digest : Int) (w : Window)
    (hw : windowOf b inc = some w) (hfloor : seq ≤ w.floor) :
    admit b inc seq digest = .belowRetention w.floor := by
  unfold admit
  rw [hw]
  dsimp only
  rw [if_pos hfloor]

/-! ### R4: a settle cannot invent a record -/

def setOutcome (w : Window) (seq : Int) (o : Outcome) : Window :=
  if seq = 0 then
    (if w.has0 = true then ⟨w.floor, true, ⟨w.e0.digest, o⟩, w.has1, w.e1⟩ else w)
  else if seq = 1 then
    (if w.has1 = true then ⟨w.floor, w.has0, w.e0, true, ⟨w.e1.digest, o⟩⟩ else w)
  else w

def settle (b : Book) (inc seq : Int) (o : Outcome) : Book :=
  if inc = 0 then
    (if b.holds0 = true then
      ⟨true, setOutcome b.w0 seq o, b.holds1, b.w1, b.retained, b.capacity,
        b.maxScopes, b.maxIncarnations⟩
     else b)
  else
    (if b.holds1 = true then
      ⟨b.holds0, b.w0, true, setOutcome b.w1 seq o, b.retained, b.capacity,
        b.maxScopes, b.maxIncarnations⟩
     else b)

theorem settle_without_a_record_changes_nothing (b : Book) (inc seq : Int)
    (o : Outcome) (hinc : inc = 0 ∨ inc = 1) (h : present b inc seq = false) :
    settle b inc seq o = b := by
  unfold settle
  by_cases h0 : inc = 0
  · rw [if_pos h0]
    by_cases hb : b.holds0 = true
    · rw [if_pos hb]
      have hl : lookup b.w0 seq = none := by
        unfold present at h
        unfold windowOf at h
        rw [if_pos h0, if_pos hb] at h
        dsimp only at h
        by_cases hs : (lookup b.w0 seq).isSome = true
        · rw [hs] at h; cases h
        · cases hh : lookup b.w0 seq with
          | none => rfl
          | some e => rw [hh] at hs; exact absurd rfl hs
      unfold setOutcome
      by_cases s0 : seq = 0
      · rw [if_pos s0]
        unfold lookup at hl
        rw [if_pos s0] at hl
        by_cases hz : b.w0.has0 = true
        · rw [if_pos hz] at hl; simp at hl
        · rw [if_neg hz]; cases b; simp_all
      · rw [if_neg s0]
        by_cases s1 : seq = 1
        · rw [if_pos s1]
          unfold lookup at hl
          rw [if_neg s0, if_pos s1] at hl
          by_cases ho : b.w0.has1 = true
          · rw [if_pos ho] at hl; simp at hl
          · rw [if_neg ho]; cases b; simp_all
        · rw [if_neg s1]; cases b; simp_all
    · rw [if_neg hb]
  · have h1 : inc = 1 := by
      rcases hinc with h | h
      · exact absurd h h0
      · exact h
    rw [if_neg h0]
    by_cases hb : b.holds1 = true
    · rw [if_pos hb]
      have hl : lookup b.w1 seq = none := by
        unfold present at h
        unfold windowOf at h
        rw [if_neg h0, if_pos h1, if_pos hb] at h
        dsimp only at h
        by_cases hs : (lookup b.w1 seq).isSome = true
        · rw [hs] at h; cases h
        · cases hh : lookup b.w1 seq with
          | none => rfl
          | some e => rw [hh] at hs; exact absurd rfl hs
      unfold setOutcome
      by_cases s0 : seq = 0
      · rw [if_pos s0]
        unfold lookup at hl
        rw [if_pos s0] at hl
        by_cases hz : b.w1.has0 = true
        · rw [if_pos hz] at hl; simp at hl
        · rw [if_neg hz]; cases b; simp_all
      · rw [if_neg s0]
        by_cases s1 : seq = 1
        · rw [if_pos s1]
          unfold lookup at hl
          rw [if_neg s0, if_pos s1] at hl
          by_cases ho : b.w1.has1 = true
          · rw [if_pos ho] at hl; simp at hl
          · rw [if_neg ho]; cases b; simp_all
        · rw [if_neg s1]; cases b; simp_all
    · rw [if_neg hb]

/-! ### R5--R6: what the shell does and does not induct over

  R1--R4 above are stated over every `Book`, so their proofs already cover
  every state a run can reach: there is no reachability premise to induct over,
  and adding one would weaken the claim. The Why3 theory has the same shape
  (its lemmas quantify over `b : book`), while
  `unfer_consensus/tests/request_replay_invariant.rs` is what explores the
  reachable finite states. The run-level inductions in this file are therefore
  `Epoch.run_non_decreasing` and `Fence.run_fence_le`, where the claim is about
  what a *sequence* of accepted transitions does, not about a single state.
-/

end Replay
end D21

/-! ## Trusted-base audit

  `#print axioms` asks the kernel which axioms each theorem's proof term
  depends on. Every line below must read "does not depend on any axioms"; a
  change that reaches for `Classical.choice`, `propext` or a `sorry` shows up
  here, and `scripts/check-lean-d21` refuses the file unless all of them are
  axiom-free. -/
#print axioms D21.Epoch.step_preserves_wf
#print axioms D21.Epoch.step_non_decreasing
#print axioms D21.Epoch.accepted_takeover_strictly_raises
#print axioms D21.Epoch.stale_takeover_changes_nothing
#print axioms D21.Epoch.first_epoch_is_authority
#print axioms D21.Epoch.run_preserves_wf
#print axioms D21.Epoch.run_non_decreasing
#print axioms D21.Fence.issue_preserves_inv
#print axioms D21.Fence.step_preserves_inv
#print axioms D21.Fence.settle_applies_only_when_current
#print axioms D21.Fence.stale_settle_changes_nothing
#print axioms D21.Fence.fenceLe_trans
#print axioms D21.Fence.step_fenceLe
#print axioms D21.Fence.run_preserves_inv
#print axioms D21.Fence.run_fence_le
#print axioms D21.Replay.replay_is_the_recorded_outcome
#print axioms D21.Replay.equivocation_is_a_conflict
#print axioms D21.Replay.below_floor_is_refused
#print axioms D21.Replay.settle_without_a_record_changes_nothing
