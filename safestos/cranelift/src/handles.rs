//! U13 — the host-managed typed handle table for owned Rust values.
//!
//! An owned Rust value must stay in the provider that allocated it: a module
//! never sees the value, only a handle, and every use goes back through the
//! provider's table. This module is that table, hosted by the bridge because a
//! generated `__new`/`__dispose` wrapper is compiled with
//! `austral_cranelift_bridge` as its only dependency.
//!
//! A handle is an opaque `(owner, type, provider generation, slot, generation)`
//! tuple — deliberately not an integer, so no safe interface can fabricate one
//! from a number. Four independent checks answer four different mistakes:
//!
//! - **owner**: a handle issued by another registry is not a handle here, even
//!   if the slot index and generation happen to match. A provider that serves
//!   several object types gets one registry per type, so the owner tag is also
//!   what keeps one type's handle from resolving in another's table — the two
//!   words a module carries have no room for a type tag and no need for one.
//! - **type**: a handle minted for another object type is refused, so a stale
//!   numeric slot cannot be reinterpreted as a different type. This is the
//!   in-process check; a wrapper's words are checked by the owner tag.
//! - **provider generation**: a handle kept across a provider reload belongs to
//!   the old generation and is refused instead of resolving into the new table.
//! - **slot generation**: freeing a slot bumps its generation, so a handle to
//!   the value that used to live there is stale after reuse. The bump is
//!   `checked`: a slot whose generation would wrap is *retired* (never reused)
//!   rather than allowed to hand out an equal generation again.
//!
//! Destruction is `take`/`destroy`: the value is moved out of the slot and
//! dropped exactly once, and the slot is freed in the same step. A second
//! `destroy` on the same handle is `Stale`, so a destructor cannot run twice.
//!
//! **Borrows (U14).** `borrow`/`borrow_mut` mint a [`Borrow`] token and
//! `release` discharges it. Any number of immutable claims may be live at once
//! (a group borrow); a mutable claim excludes every other claim, and any claim
//! excludes `get_mut`, `take` and `destroy` on its slot. A `Borrow` is neither
//! `Clone` nor `Copy`, so one claim has one token, and `release` refuses a
//! token this registry does not hold — the runtime half of the borrowed-view
//! contract. The other half is the wrapper: `__with` holds the table for the
//! callback, so no wrapper can invalidate a borrow while it runs.
//!
//! **The wire form.** A handle crosses the ABI as two 64-bit words, packed
//! without loss because every component is 32 bits:
//!
//! ```text
//! word 0 = owner << 32 | slot
//! word 1 = provider generation << 32 | slot generation
//! ```
//!
//! The pair `(0, 0)` is the null handle — no owner, no slot — and is refused as
//! `Malformed`; a real registry is tagged from 1, so no live handle can encode
//! to it. Because the words carry no type, the type is the wrapper's: the
//! generator numbers the registries in one adapter crate from 1, so a handle
//! for one object type has an owner tag no other registry in that crate holds.

use std::ffi::c_void;
use std::fmt;

/// Version of the handle contract. Bumped when slot reuse, generation, the word
/// encoding or ownership accounting changes, so a wrapper generated against one
/// version is never linked against another.
pub const HANDLE_SCHEMA: u32 = 1;

/// The status word every generated wrapper returns, kept separate from the
/// payload it writes through its out slot. `0` done, `1` refused by the host
/// (a bad, stale or foreign word, a null out, a null callback) — no provider
/// code ran — and `2` the provider panicked and the wrapper caught it.
/// `i64::MIN`, `-1` and every other value stay domain results; none of them is
/// a status.
pub const STATUS_OK: i64 = 0;
pub const STATUS_REFUSED: i64 = 1;
pub const STATUS_PANIC: i64 = 2;

/// A checked status: host callers decode the word instead of comparing it, so
/// a value the ABI does not define is reported rather than read as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Refused,
    Panic,
}

impl Status {
    pub const fn word(self) -> i64 {
        match self {
            Status::Ok => STATUS_OK,
            Status::Refused => STATUS_REFUSED,
            Status::Panic => STATUS_PANIC,
        }
    }

    /// The one decoder: every wrapper's return is one of the three words, and
    /// anything else is an error carrying the value it saw.
    pub fn from_word(word: i64) -> Result<Status, i64> {
        match word {
            STATUS_OK => Ok(Status::Ok),
            STATUS_REFUSED => Ok(Status::Refused),
            STATUS_PANIC => Ok(Status::Panic),
            other => Err(other),
        }
    }
}

/// The registry a handle belongs to. Issued by the wrapper generator (one tag
/// per registry, counted from 1 inside one adapter crate), never by a module: a
/// module-supplied tag could name another type's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner(u32);

impl Owner {
    /// The generator's seam. Tag 0 is the null handle's owner and must not be
    /// used for a live registry.
    pub const fn issued(tag: u32) -> Self {
        Owner(tag)
    }

    /// For diagnostics and for the wrapper's own bookkeeping; not an interface
    /// a module may take a value from.
    pub const fn tag(self) -> u32 {
        self.0
    }
}

/// The object type a registry holds. Compared by name, so two registries built
/// from the same descriptor agree and a handle from a different descriptor is
/// `ForeignType` rather than a silent reinterpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeTag(&'static str);

impl TypeTag {
    pub const fn named(name: &'static str) -> Self {
        TypeTag(name)
    }

    pub const fn name(self) -> &'static str {
        self.0
    }
}

/// An opaque reference to one live value in one provider's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    owner: Owner,
    kind: TypeTag,
    provider: u32,
    index: u32,
    generation: u32,
}

impl Handle {
    /// The type this handle was minted for.
    pub const fn kind(self) -> TypeTag {
        self.kind
    }

    /// The slot this handle names, for diagnostics only: it is not a stable
    /// identity and it is reused after the handle goes stale.
    pub const fn slot(self) -> u32 {
        self.index
    }

    /// The provider generation this handle was minted in.
    pub const fn provider(self) -> u32 {
        self.provider
    }

    /// The two words a module carries: `owner << 32 | slot` and
    /// `provider generation << 32 | slot generation`. Every component is 32
    /// bits, so nothing is truncated and nothing has to be re-derived.
    pub const fn to_words(self) -> (u64, u64) {
        (
            ((self.owner.0 as u64) << 32) | self.index as u64,
            ((self.provider as u64) << 32) | self.generation as u64,
        )
    }
}

/// A live claim on one value, minted by [`HandleRegistry::borrow`] or
/// [`HandleRegistry::borrow_mut`] and discharged by
/// [`HandleRegistry::release`]. Deliberately neither `Clone` nor `Copy`: one
/// token per claim, so a double release is refused by identity instead of
/// silently releasing a claim someone else still holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Borrow {
    id: u64,
    owner: Owner,
    index: u32,
    generation: u32,
    mutable: bool,
}

impl Borrow {
    /// Whether this claim excludes every other claim and all mutation.
    pub const fn is_mutable(&self) -> bool {
        self.mutable
    }

    /// The slot this claim protects, for diagnostics only.
    pub const fn slot(&self) -> u32 {
        self.index
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleError {
    /// The handle was issued by a different registry.
    ForeignOwner { expected: u32, found: u32 },
    /// The handle was minted for a different object type.
    ForeignType {
        expected: &'static str,
        found: &'static str,
    },
    /// The handle belongs to a provider generation that is no longer loaded.
    StaleProvider { expected: u32, found: u32 },
    /// The slot is free, out of range, or the slot generation moved on.
    Stale { slot: u32, generation: u32 },
    /// The two words are the null handle: no owner and no slot.
    Malformed { lo: u64, hi: u64 },
    /// A borrow is live on the slot, so the value cannot be mutated or
    /// destroyed. `mutable` names the claim that blocked it: a mutable claim
    /// excludes every operation, an immutable one still excludes mutation.
    Borrowed {
        slot: u32,
        generation: u32,
        mutable: bool,
    },
    /// A release names a claim this registry is not holding: it was already
    /// released, or it belongs to another registry.
    UnknownBorrow { id: u64 },
    /// The table has no slot indices left.
    Exhausted { slots: u32 },
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandleError::ForeignOwner { expected, found } => write!(
                f,
                "handle belongs to registry {found}, this registry is {expected}; a \
                 handle from another provider (or another object type) is not a \
                 fresh one"
            ),
            HandleError::ForeignType { expected, found } => write!(
                f,
                "handle is for `{found}`, this table holds `{expected}`; an owned \
                 value is only usable through the type that allocated it"
            ),
            HandleError::StaleProvider { expected, found } => write!(
                f,
                "handle was issued by provider generation {found}, the live one is \
                 {expected}; the value it named died with the old provider"
            ),
            HandleError::Stale { slot, generation } => write!(
                f,
                "handle is stale: slot {slot} generation {generation} is no longer \
                 live (it was destroyed, or the slot was reused)"
            ),
            HandleError::Malformed { lo, hi } => write!(
                f,
                "handle words {lo:#x}/{hi:#x} are the null handle (no owner, no \
                 slot); a module cannot pass \"no value\" where a live one is due"
            ),
            HandleError::Borrowed {
                slot,
                generation,
                mutable,
            } => {
                let held = if *mutable {
                    "a mutable borrow"
                } else {
                    "an immutable borrow"
                };
                write!(
                    f,
                    "handle slot {slot} generation {generation} is still borrowed: \
                     {held} is live, so the value cannot be mutated or destroyed \
                     until it is released"
                )
            }
            HandleError::UnknownBorrow { id } => write!(
                f,
                "borrow token {id} is not live in this registry (released already, \
                 or issued by another registry)"
            ),
            HandleError::Exhausted { slots } => {
                write!(f, "the handle table has exhausted its {slots} slot indices")
            }
        }
    }
}

impl std::error::Error for HandleError {}

enum State<T> {
    Live(T),
    Free,
}

/// One outstanding claim. Kept beside the slots rather than inside them so a
/// released token can be recognised (and refused) by id alone.
struct Claim {
    id: u64,
    index: u32,
    mutable: bool,
}

struct Slot<T> {
    generation: u32,
    state: State<T>,
}

/// One registry's handle table. Holds at most one object type: a registry is
/// built from the descriptor of the type it serves, so "wrong type" is a
/// property of the handle, not a downcast the caller has to remember.
pub struct HandleRegistry<T> {
    owner: Owner,
    kind: TypeTag,
    provider: u32,
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    claims: Vec<Claim>,
    next_claim: u64,
    retired: u32,
    issued: u64,
    released: u64,
    allocated: u64,
    freed: u64,
}

impl<T> HandleRegistry<T> {
    /// Constant so a generated wrapper can hold one in a `static Mutex`: the
    /// empty table has no allocation until the first `insert`.
    pub const fn new(owner: Owner, kind: TypeTag, provider_generation: u32) -> Self {
        HandleRegistry {
            owner,
            kind,
            provider: provider_generation,
            slots: Vec::new(),
            free: Vec::new(),
            claims: Vec::new(),
            // Claim ids start at 1: `(0, 0)` is the module's "no view" pair,
            // so no live token may encode to it.
            next_claim: 1,
            retired: 0,
            issued: 0,
            released: 0,
            allocated: 0,
            freed: 0,
        }
    }

    /// Store a value and mint its handle. A freed slot is reused with its
    /// generation already bumped, so any handle to the previous occupant stays
    /// stale.
    pub fn insert(&mut self, value: T) -> Result<Handle, HandleError> {
        self.issued += 1;
        let index = match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                debug_assert!(matches!(slot.state, State::Free));
                slot.state = State::Live(value);
                index
            }
            None => {
                let index = u32::try_from(self.slots.len())
                    .map_err(|_| HandleError::Exhausted { slots: u32::MAX })?;
                self.slots.push(Slot {
                    generation: 0,
                    state: State::Live(value),
                });
                index
            }
        };
        self.allocated += 1;
        let slot = &self.slots[index as usize];
        Ok(Handle {
            owner: self.owner,
            kind: self.kind,
            provider: self.provider,
            index,
            generation: slot.generation,
        })
    }

    pub fn get(&self, handle: Handle) -> Result<&T, HandleError> {
        let index = self.resolve(handle)?;
        self.refuse_if_mutably_claimed(handle)?;
        match &self.slots[index].state {
            State::Live(value) => Ok(value),
            State::Free => Err(HandleError::Stale {
                slot: handle.index,
                generation: handle.generation,
            }),
        }
    }

    pub fn get_mut(&mut self, handle: Handle) -> Result<&mut T, HandleError> {
        let index = self.resolve(handle)?;
        self.refuse_if_claimed(handle)?;
        match &mut self.slots[index].state {
            State::Live(value) => Ok(value),
            State::Free => Err(HandleError::Stale {
                slot: handle.index,
                generation: handle.generation,
            }),
        }
    }

    /// Move the value out and free the slot. The caller owns the value from
    /// here (dropping it is the destruction); the handle is stale afterwards.
    /// The value is *moved out*, not freed by this table, so it is not counted
    /// as a free.
    pub fn take(&mut self, handle: Handle) -> Result<T, HandleError> {
        let index = self.resolve(handle)?;
        self.refuse_if_claimed(handle)?;
        let slot = &mut self.slots[index];
        let value = match std::mem::replace(&mut slot.state, State::Free) {
            State::Live(value) => value,
            State::Free => {
                return Err(HandleError::Stale {
                    slot: handle.index,
                    generation: handle.generation,
                })
            }
        };
        // Retire a slot that cannot bump its generation: reusing it would hand
        // out a generation equal to one already given away.
        match slot.generation.checked_add(1) {
            Some(next) => {
                slot.generation = next;
                self.free.push(handle.index);
            }
            None => self.retired += 1,
        }
        self.released += 1;
        Ok(value)
    }

    /// Drop the value in place and free the slot: the destruction seam. A
    /// second `destroy` on the same handle is `Stale`, so the destructor runs
    /// exactly once, and only a successful destroy counts as a `freed` value.
    pub fn destroy(&mut self, handle: Handle) -> Result<(), HandleError> {
        drop(self.take(handle)?);
        self.freed += 1;
        Ok(())
    }

    /// Claim an immutable view of a live value. Immutable claims coexist, so
    /// several readers may hold the same value; a mutable claim excludes them
    /// and they exclude it.
    pub fn borrow(&mut self, handle: Handle) -> Result<Borrow, HandleError> {
        self.resolve(handle)?;
        self.refuse_if_mutably_claimed(handle)?;
        Ok(self.claim(handle, false))
    }

    /// Claim exclusive mutation of a live value. Refused while any other claim
    /// is live on the slot, so two handles (or a copied handle) never yield
    /// simultaneous mutable references.
    pub fn borrow_mut(&mut self, handle: Handle) -> Result<Borrow, HandleError> {
        self.resolve(handle)?;
        self.refuse_if_claimed(handle)?;
        Ok(self.claim(handle, true))
    }

    /// The word-pair seams the generated wrappers and host callers use, so a
    /// module never has to rebuild a `Handle` to take a view.
    pub fn borrow_words(&mut self, lo: u64, hi: u64) -> Result<Borrow, HandleError> {
        let handle = self.handle_from_words(lo, hi)?;
        self.borrow(handle)
    }

    pub fn borrow_mut_words(&mut self, lo: u64, hi: u64) -> Result<Borrow, HandleError> {
        let handle = self.handle_from_words(lo, hi)?;
        self.borrow_mut(handle)
    }

    /// Read through a view token, so a caller does not have to hold the handle
    /// beside the claim it already proves.
    pub fn get_borrowed(&self, borrow: &Borrow) -> Result<&T, HandleError> {
        let index = self.resolve_claim(borrow, false)?;
        match &self.slots[index].state {
            State::Live(value) => Ok(value),
            State::Free => Err(HandleError::Stale {
                slot: borrow.index,
                generation: borrow.generation,
            }),
        }
    }

    /// Mutate through a mutable view token: the only path to `&mut T` while a
    /// claim exists, and it requires the exclusive claim. A different handle's
    /// `get_mut` is still refused, so a copied handle cannot alias this one.
    pub fn get_mut_borrowed(&mut self, borrow: &Borrow) -> Result<&mut T, HandleError> {
        let index = self.resolve_claim(borrow, true)?;
        match &mut self.slots[index].state {
            State::Live(value) => Ok(value),
            State::Free => Err(HandleError::Stale {
                slot: borrow.index,
                generation: borrow.generation,
            }),
        }
    }

    /// Mint an immutable view and encode it as the two words a module holds.
    /// `(0, 0)` is never a valid token: owner tag 0 is reserved for the null
    /// handle, so a caller that skipped the status word cannot release claim 0
    /// of some registry by accident.
    pub fn borrow_to_words(&mut self, lo: u64, hi: u64) -> Result<(u64, u64), HandleError> {
        let borrow = self.borrow_words(lo, hi)?;
        Ok((borrow.id, u64::from(self.owner.tag())))
    }

    /// Mint an exclusive view and encode it as the two words a module holds.
    /// The claim excludes every other claim on the slot, so the module's write
    /// cannot overlap a reader or another writer.
    pub fn borrow_mut_to_words(&mut self, lo: u64, hi: u64) -> Result<(u64, u64), HandleError> {
        let borrow = self.borrow_mut_words(lo, hi)?;
        Ok((borrow.id, u64::from(self.owner.tag())))
    }

    /// The mutation a mutable token exists for. An immutable token is refused
    /// (`Borrowed`), so a reader cannot write through the view it holds.
    pub fn get_words_view_mut(&mut self, w0: u64, w1: u64) -> Result<&mut T, HandleError> {
        let index = {
            let claim = self.view_from_words(w0, w1)?;
            if !claim.mutable {
                return Err(HandleError::Borrowed {
                    slot: claim.index,
                    generation: self.slots[claim.index as usize].generation,
                    mutable: false,
                });
            }
            claim.index as usize
        };
        let slot = &mut self.slots[index];
        match &mut slot.state {
            State::Live(value) => Ok(value),
            State::Free => Err(HandleError::Stale {
                slot: index as u32,
                generation: slot.generation,
            }),
        }
    }

    /// Read the value a view token names. The token is checked against the
    /// live claims, so a released view — or one from another build — reads
    /// nothing.
    pub fn get_words_view(&self, w0: u64, w1: u64) -> Result<&T, HandleError> {
        let claim = self.view_from_words(w0, w1)?;
        let slot = &self.slots[claim.index as usize];
        match &slot.state {
            State::Live(value) => Ok(value),
            State::Free => Err(HandleError::Stale {
                slot: claim.index,
                generation: slot.generation,
            }),
        }
    }

    /// Discharge a held view by its words: the token names a live claim by id
    /// and the owner tag says which registry may consume it.
    pub fn release_words(&mut self, w0: u64, w1: u64) -> Result<(), HandleError> {
        if w0 == 0 || w1 != u64::from(self.owner.tag()) {
            return Err(HandleError::UnknownBorrow { id: w0 });
        }
        match self.claims.iter().position(|claim| claim.id == w0) {
            Some(position) => {
                self.claims.remove(position);
                Ok(())
            }
            None => Err(HandleError::UnknownBorrow { id: w0 }),
        }
    }

    /// The claim a token names, if it is live here. The id is the identity and
    /// the owner tag is the registry check, so a token cannot be replayed into
    /// another build's table even if claim numbering happens to agree.
    fn view_from_words(&self, w0: u64, w1: u64) -> Result<&Claim, HandleError> {
        if w0 == 0 || w1 != u64::from(self.owner.tag()) {
            return Err(HandleError::UnknownBorrow { id: w0 });
        }
        self.claims
            .iter()
            .find(|claim| claim.id == w0)
            .ok_or(HandleError::UnknownBorrow { id: w0 })
    }

    /// Destroy every live value and free its slot: the generation-replacement
    /// seam. A provider reload replaces this table, so the values it holds must
    /// run their destructors *here* instead of being stranded for the life of
    /// the process. A live borrow refuses the whole drain, because the value a
    /// module can still read must not be freed under it; the reload then fails
    /// rather than leaking or pulling a value out from under a view.
    ///
    /// Returns the number of values destroyed. Slot generations advance exactly
    /// as `take` advances them, so a handle the module kept is stale afterwards.
    pub fn drain(&mut self) -> Result<u64, HandleError> {
        if let Some(claim) = self.claims.first() {
            return Err(HandleError::Borrowed {
                slot: claim.index,
                generation: self.slots[claim.index as usize].generation,
                mutable: claim.mutable,
            });
        }
        let mut drained = 0;
        for index in 0..self.slots.len() {
            let slot = &mut self.slots[index];
            if !matches!(slot.state, State::Live(_)) {
                continue;
            }
            // Assignment drops the value, which is the provider's destructor.
            slot.state = State::Free;
            match slot.generation.checked_add(1) {
                Some(next) => {
                    slot.generation = next;
                    self.free.push(index as u32);
                }
                None => self.retired += 1,
            }
            self.released += 1;
            self.freed += 1;
            drained += 1;
        }
        Ok(drained)
    }

    /// Discharge a claim. A token this registry did not issue, or one already
    /// released, is refused rather than counted down twice.
    pub fn release(&mut self, borrow: Borrow) -> Result<(), HandleError> {
        if borrow.owner != self.owner {
            return Err(HandleError::UnknownBorrow { id: borrow.id });
        }
        let live = self.claims.iter().position(|claim| {
            claim.id == borrow.id
                && claim.index == borrow.index
                && claim.mutable == borrow.mutable
                && self.slots[borrow.index as usize].generation == borrow.generation
        });
        match live {
            Some(position) => {
                self.claims.remove(position);
                Ok(())
            }
            None => Err(HandleError::UnknownBorrow { id: borrow.id }),
        }
    }

    fn claim(&mut self, handle: Handle, mutable: bool) -> Borrow {
        let id = self.next_claim;
        self.next_claim += 1;
        self.claims.push(Claim {
            id,
            index: handle.index,
            mutable,
        });
        Borrow {
            id,
            owner: self.owner,
            index: handle.index,
            generation: handle.generation,
            mutable,
        }
    }

    fn resolve_claim(&self, borrow: &Borrow, mutable: bool) -> Result<usize, HandleError> {
        if borrow.owner != self.owner || (mutable && !borrow.mutable) {
            return Err(HandleError::UnknownBorrow { id: borrow.id });
        }
        let live = self.claims.iter().any(|claim| {
            claim.id == borrow.id
                && claim.index == borrow.index
                && claim.mutable == borrow.mutable
                && self
                    .slots
                    .get(borrow.index as usize)
                    .is_some_and(|slot| slot.generation == borrow.generation)
        });
        if !live {
            return Err(HandleError::UnknownBorrow { id: borrow.id });
        }
        Ok(borrow.index as usize)
    }

    fn refuse_if_claimed(&self, handle: Handle) -> Result<(), HandleError> {
        match self.claims.iter().find(|claim| claim.index == handle.index) {
            Some(claim) => Err(HandleError::Borrowed {
                slot: handle.index,
                generation: handle.generation,
                mutable: claim.mutable,
            }),
            None => Ok(()),
        }
    }

    fn refuse_if_mutably_claimed(&self, handle: Handle) -> Result<(), HandleError> {
        match self
            .claims
            .iter()
            .find(|claim| claim.index == handle.index && claim.mutable)
        {
            Some(_) => Err(HandleError::Borrowed {
                slot: handle.index,
                generation: handle.generation,
                mutable: true,
            }),
            None => Ok(()),
        }
    }

    /// Claims currently held on this registry (not yet released).
    pub fn claims(&self) -> usize {
        self.claims.len()
    }

    /// The in-process handle a pair of module words names. The words carry no
    /// type, so the type is this table's; a pair minted for another type's
    /// registry is refused by the owner tag.
    pub fn handle_from_words(&self, lo: u64, hi: u64) -> Result<Handle, HandleError> {
        if lo == 0 && hi == 0 {
            return Err(HandleError::Malformed { lo, hi });
        }
        Ok(Handle {
            owner: Owner((lo >> 32) as u32),
            kind: self.kind,
            provider: (hi >> 32) as u32,
            index: lo as u32,
            generation: hi as u32,
        })
    }

    pub fn get_words(&self, lo: u64, hi: u64) -> Result<&T, HandleError> {
        let handle = self.handle_from_words(lo, hi)?;
        self.get(handle)
    }

    /// Destroy the value a module's word pair names. The wrapper's seam: a
    /// null pair, a foreign registry, a previous provider generation and a
    /// stale slot are all refusals, never a destructor call.
    pub fn destroy_words(&mut self, lo: u64, hi: u64) -> Result<(), HandleError> {
        let handle = self.handle_from_words(lo, hi)?;
        self.destroy(handle)
    }

    fn resolve(&self, handle: Handle) -> Result<usize, HandleError> {
        if handle.owner != self.owner {
            return Err(HandleError::ForeignOwner {
                expected: self.owner.tag(),
                found: handle.owner.tag(),
            });
        }
        if handle.kind != self.kind {
            return Err(HandleError::ForeignType {
                expected: self.kind.name(),
                found: handle.kind.name(),
            });
        }
        if handle.provider != self.provider {
            return Err(HandleError::StaleProvider {
                expected: self.provider,
                found: handle.provider,
            });
        }
        let slot = self
            .slots
            .get(handle.index as usize)
            .ok_or(HandleError::Stale {
                slot: handle.index,
                generation: handle.generation,
            })?;
        if slot.generation != handle.generation {
            return Err(HandleError::Stale {
                slot: handle.index,
                generation: handle.generation,
            });
        }
        Ok(handle.index as usize)
    }

    /// Values currently stored (issued minus released, whatever their slot).
    pub fn live(&self) -> u64 {
        self.issued - self.released
    }

    /// Handles minted so far, including those since destroyed.
    pub fn issued(&self) -> u64 {
        self.issued
    }

    /// Values moved out or destroyed so far: the count `live()` is derived
    /// from, kept separately so a test can compare allocation and release
    /// counts.
    pub fn released(&self) -> u64 {
        self.released
    }

    /// Values this registry stored: one per `insert`. Independent of `issued`
    /// only in that it is the allocation count, never decremented or reused.
    pub fn allocated(&self) -> u64 {
        self.allocated
    }

    /// Values this registry destroyed in place (the destructor ran). A value
    /// left through `take` is the caller's, so it is released but not freed
    /// here — which is what lets a test hold `allocated`, `freed` and the
    /// provider's own drop counter against each other.
    pub fn freed(&self) -> u64 {
        self.freed
    }

    /// Slots whose generation was exhausted and can never be reused.
    pub fn retired(&self) -> u32 {
        self.retired
    }
}

/// One owned value the provider allocated, seen from the module side as a
/// handle. The pointer is only ever touched through the registry (under the
/// wrapper's mutex), and the provider's destructor is what frees it, so
/// dropping this value calls the provider's `free`, exactly once, and never
/// the host allocator's.
pub struct ProviderValue {
    raw: *mut c_void,
    destroy: unsafe extern "C" fn(*mut c_void),
}

impl ProviderValue {
    /// Wrap a provider allocation. `destroy` is the provider's own destructor
    /// symbol: the host never frees a provider's memory.
    pub const fn new(raw: *mut c_void, destroy: unsafe extern "C" fn(*mut c_void)) -> Self {
        ProviderValue { raw, destroy }
    }

    /// The provider's pointer, for a generated call that hands it back to the
    /// provider. Not an interface a module can take a value from.
    pub const fn as_ptr(&self) -> *mut c_void {
        self.raw
    }
}

impl Drop for ProviderValue {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { (self.destroy)(self.raw) }
        }
    }
}

impl fmt::Debug for ProviderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProviderValue({:p})", self.raw)
    }
}

// SAFETY: the pointer is only dereferenced by the provider's destructor, which
// the table calls while it holds the wrapper's mutex, so no two threads touch
// one value at once; the destructor is a plain code pointer.
unsafe impl Send for ProviderValue {}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> HandleRegistry<u32> {
        HandleRegistry::new(Owner::issued(7), TypeTag::named("Counter"), 1)
    }

    #[test]
    fn a_slot_whose_generation_would_wrap_is_retired_not_reused() {
        let mut registry = registry();
        let first = registry.insert(1).unwrap();
        registry.destroy(first).unwrap();
        // Pretend the slot has already handed out every generation it has.
        registry.slots[0].generation = u32::MAX;

        let handle = registry.insert(2).unwrap();
        assert_eq!(handle.slot(), 0, "the free slot is reused");
        assert_eq!(registry.take(handle).unwrap(), 2);
        assert_eq!(registry.retired(), 1);
        assert_eq!(registry.live(), 0);

        // The retired slot is not handed out again: the next value gets a new
        // slot, so no generation is ever issued twice.
        let next = registry.insert(3).unwrap();
        assert_ne!(next.slot(), handle.slot());
        assert_eq!(registry.get(next), Ok(&3));
        assert!(matches!(
            registry.get(handle),
            Err(HandleError::Stale { .. })
        ));
    }

    #[test]
    fn the_word_pair_carries_every_component_without_loss() {
        let mut registry = HandleRegistry::new(Owner::issued(0xdead_beef), TypeTag::named("W"), 9);
        let handle = registry.insert(1).unwrap();
        let (lo, hi) = handle.to_words();
        assert_eq!(lo >> 32, 0xdead_beef);
        assert_eq!(lo as u32, handle.slot());
        assert_eq!(hi >> 32, 9);
        assert_eq!(hi as u32, 0);

        // Round-tripping through the words resolves the same value.
        assert_eq!(registry.get_words(lo, hi), Ok(&1));
        // The null pair is refused as malformed, never as slot 0.
        assert_eq!(
            registry.get_words(0, 0),
            Err(HandleError::Malformed { lo: 0, hi: 0 })
        );
    }

    #[test]
    fn a_status_word_is_checked_and_domain_edges_are_not_statuses() {
        assert_eq!(Status::from_word(0), Ok(Status::Ok));
        assert_eq!(Status::from_word(1), Ok(Status::Refused));
        assert_eq!(Status::from_word(2), Ok(Status::Panic));
        assert_eq!(Status::Ok.word(), STATUS_OK);
        assert_eq!(Status::Panic.word(), STATUS_PANIC);
        // A legitimate domain result — including the edges the wrappers must
        // not confuse with a failure — is not a status.
        for edge in [-1, i64::MIN, i64::MAX, 3] {
            assert_eq!(Status::from_word(edge), Err(edge));
        }
    }

    #[test]
    fn a_live_borrow_blocks_mutation_and_destruction_until_released() {
        let mut registry = registry();
        let handle = registry.insert(1).unwrap();
        let view = registry.borrow(handle).unwrap();
        assert!(!view.is_mutable());
        assert_eq!(registry.claims(), 1);

        // Reading is what a view is for; every write and the destruction are
        // refused while it is live, so the value a view names cannot change or
        // disappear under it.
        assert_eq!(registry.get(handle), Ok(&1));
        assert_eq!(registry.get_borrowed(&view), Ok(&1));
        assert_eq!(
            registry.get_mut(handle),
            Err(HandleError::Borrowed {
                slot: handle.slot(),
                generation: handle.to_words().1 as u32,
                mutable: false,
            })
        );
        assert!(matches!(
            registry.take(handle),
            Err(HandleError::Borrowed { mutable: false, .. })
        ));
        assert!(matches!(
            registry.destroy(handle),
            Err(HandleError::Borrowed { mutable: false, .. })
        ));
        assert_eq!(registry.live(), 1);

        registry.release(view).unwrap();
        assert_eq!(registry.claims(), 0);
        registry.destroy(handle).unwrap();
        assert_eq!(registry.live(), 0);
    }

    #[test]
    fn a_group_borrow_coexists_and_a_mutable_claim_excludes_everything() {
        let mut registry = registry();
        let handle = registry.insert(1).unwrap();
        let first = registry.borrow(handle).unwrap();
        let second = registry.borrow(handle).unwrap();
        assert_eq!(registry.claims(), 2, "immutable claims form a group");

        // No exclusive claim while any reader is live.
        assert!(matches!(
            registry.borrow_mut(handle),
            Err(HandleError::Borrowed { mutable: false, .. })
        ));
        registry.release(first).unwrap();
        registry.release(second).unwrap();

        // A mutable claim now succeeds and excludes readers as well, so no
        // `&T` can overlap it. The token is the mutation path; the handle's
        // own `get_mut` stays refused, so a copied handle cannot alias it.
        let edit = registry.borrow_mut(handle).unwrap();
        assert!(edit.is_mutable());
        *registry.get_mut_borrowed(&edit).unwrap() = 2;
        assert!(matches!(
            registry.get_mut(handle),
            Err(HandleError::Borrowed { mutable: true, .. })
        ));
        assert!(matches!(
            registry.borrow(handle),
            Err(HandleError::Borrowed { mutable: true, .. })
        ));
        assert!(matches!(
            registry.get(handle),
            Err(HandleError::Borrowed { mutable: true, .. })
        ));
        registry.release(edit).unwrap();
        assert_eq!(registry.get(handle), Ok(&2));
        assert_eq!(registry.claims(), 0);

        // A token is not a capability once it is released, and an immutable
        // token cannot be used as a mutation path.
        let view = registry.borrow(handle).unwrap();
        assert!(registry.get_mut_borrowed(&view).is_err());
        registry.release(view).unwrap();
    }

    #[test]
    fn replacing_a_child_while_its_view_is_live_is_refused() {
        // The Chest/Gem shape: a view of the child is taken and the boxed child
        // is then replaced. The replacement cannot destroy the value the view
        // names, so the old view never reads freed memory.
        let mut registry = registry();
        let child = registry.insert(7).unwrap();
        let view = registry.borrow(child).unwrap();
        assert_eq!(registry.get(child), Ok(&7));
        assert!(matches!(
            registry.destroy(child),
            Err(HandleError::Borrowed { .. })
        ));

        // After the view is discharged the replacement is allowed, and the old
        // handle cannot name it: the slot generation moved on.
        registry.release(view).unwrap();
        registry.destroy(child).unwrap();
        let replacement = registry.insert(9).unwrap();
        assert_eq!(replacement.slot(), child.slot());
        assert!(matches!(
            registry.get(child),
            Err(HandleError::Stale { .. })
        ));
        assert_eq!(registry.get(replacement), Ok(&9));
    }

    #[test]
    fn a_token_from_another_registry_is_refused() {
        let mut here = registry();
        let mut elsewhere = HandleRegistry::new(Owner::issued(8), TypeTag::named("Counter"), 1);
        let handle = elsewhere.insert(4).unwrap();
        let view = elsewhere.borrow(handle).unwrap();

        // A token names its claim by id, and an id is only meaningful in the
        // registry that issued it: releasing it here cannot touch this table.
        assert!(matches!(
            here.release(view),
            Err(HandleError::UnknownBorrow { .. })
        ));
        assert_eq!(here.claims(), 0);
    }

    #[test]
    fn a_word_pair_is_enough_to_take_a_view() {
        let mut registry = registry();
        let handle = registry.insert(5).unwrap();
        let (lo, hi) = handle.to_words();
        let view = registry.borrow_words(lo, hi).unwrap();
        assert_eq!(
            registry.borrow_mut_words(lo, hi),
            Err(HandleError::Borrowed {
                slot: handle.slot(),
                generation: hi as u32,
                mutable: false,
            })
        );
        registry.release(view).unwrap();
        let edit = registry.borrow_mut_words(lo, hi).unwrap();
        registry.release(edit).unwrap();
        assert_eq!(registry.get_words(lo, hi), Ok(&5));
    }

    #[test]
    fn a_held_view_is_a_token_the_module_reads_and_releases() {
        let mut registry = registry();
        let handle = registry.insert(5).unwrap();
        let (lo, hi) = handle.to_words();

        let (w0, w1) = registry.borrow_to_words(lo, hi).unwrap();
        assert_eq!(w1, 7, "the token names the registry that may consume it");
        assert_eq!(registry.get_words_view(w0, w1), Ok(&5));
        // The view blocks destruction while it is held, exactly like a handle
        // borrow, so the value cannot be freed under the module's token.
        assert!(matches!(
            registry.destroy_words(lo, hi),
            Err(HandleError::Borrowed { .. })
        ));

        registry.release_words(w0, w1).unwrap();
        assert_eq!(registry.claims(), 0);
        // The token is dead: neither reading nor releasing it again works.
        assert!(matches!(
            registry.get_words_view(w0, w1),
            Err(HandleError::UnknownBorrow { .. })
        ));
        assert!(matches!(
            registry.release_words(w0, w1),
            Err(HandleError::UnknownBorrow { .. })
        ));
        registry.destroy_words(lo, hi).unwrap();
    }

    #[test]
    fn a_view_token_from_another_registry_is_refused() {
        let mut here = registry();
        let mut elsewhere = HandleRegistry::new(Owner::issued(8), TypeTag::named("Counter"), 1);
        let handle = elsewhere.insert(4).unwrap();
        let (lo, hi) = handle.to_words();
        let (w0, w1) = elsewhere.borrow_to_words(lo, hi).unwrap();

        assert!(matches!(
            here.get_words_view(w0, w1),
            Err(HandleError::UnknownBorrow { .. })
        ));
        assert!(matches!(
            here.release_words(w0, w1),
            Err(HandleError::UnknownBorrow { .. })
        ));
        // The owner's own table still holds and resolves the claim.
        assert_eq!(elsewhere.get_words_view(w0, w1), Ok(&4));
        elsewhere.release_words(w0, w1).unwrap();
    }

    #[test]
    fn a_mutable_token_is_the_only_word_form_that_writes() {
        let mut registry = registry();
        let handle = registry.insert(5).unwrap();
        let (lo, hi) = handle.to_words();

        let (r0, r1) = registry.borrow_to_words(lo, hi).unwrap();
        // A reader's token cannot become a writer's: the claim is immutable.
        assert!(matches!(
            registry.get_words_view_mut(r0, r1),
            Err(HandleError::Borrowed { mutable: false, .. })
        ));
        registry.release_words(r0, r1).unwrap();

        let (w0, w1) = registry.borrow_mut_to_words(lo, hi).unwrap();
        *registry.get_words_view_mut(w0, w1).unwrap() = 6;
        assert_eq!(registry.get_words_view(w0, w1), Ok(&6));
        // The exclusive token still blocks a second claim until it is released.
        assert!(matches!(
            registry.borrow_to_words(lo, hi),
            Err(HandleError::Borrowed { mutable: true, .. })
        ));
        registry.release_words(w0, w1).unwrap();
        assert_eq!(registry.get_words(lo, hi), Ok(&6));
    }

    #[test]
    fn the_zero_token_is_never_a_view() {
        let mut registry = registry();
        assert!(matches!(
            registry.get_words_view(0, 0),
            Err(HandleError::UnknownBorrow { .. })
        ));
        assert!(matches!(
            registry.release_words(0, 7),
            Err(HandleError::UnknownBorrow { .. })
        ));
    }

    #[test]
    fn draining_destroys_live_values_and_refuses_a_held_borrow() {
        let mut registry = registry();
        let a = registry.insert(1).unwrap();
        let b = registry.insert(2).unwrap();
        let (lo, hi) = a.to_words();
        let view = registry.borrow(a).unwrap();

        // A value a module can still read must not be freed under the view:
        // the whole drain is refused, not partly applied.
        assert!(matches!(
            registry.drain(),
            Err(HandleError::Borrowed { mutable: false, .. })
        ));
        assert_eq!(registry.live(), 2);
        assert_eq!(registry.freed(), 0);

        registry.release(view).unwrap();
        assert_eq!(registry.drain(), Ok(2));
        assert_eq!(registry.live(), 0);
        assert_eq!(registry.freed(), 2, "draining is destruction, not release");
        // The values are gone and so are the handles that named them.
        assert!(matches!(registry.get(a), Err(HandleError::Stale { .. })));
        assert!(matches!(registry.get(b), Err(HandleError::Stale { .. })));
        assert!(matches!(
            registry.get_words(lo, hi),
            Err(HandleError::Stale { .. })
        ));
        // Drained slots are free for the next values, with a bumped generation.
        let next = registry.insert(3).unwrap();
        assert_eq!(registry.get(next), Ok(&3));
        assert_eq!(registry.drain(), Ok(1));
        assert_eq!(registry.live(), 0);
    }

    #[test]
    fn allocation_and_free_counters_separate_destroy_from_take() {
        let mut registry = registry();
        let a = registry.insert(1).unwrap();
        let b = registry.insert(2).unwrap();
        let c = registry.insert(3).unwrap();
        registry.destroy(a).unwrap();
        assert_eq!(registry.take(b), Ok(2));
        assert_eq!(registry.allocated(), 3);
        assert_eq!(registry.freed(), 1, "only `destroy` frees in the table");
        assert_eq!(registry.released(), 2);
        assert_eq!(registry.live(), 1);
        registry.destroy(c).unwrap();
        assert_eq!(registry.freed(), 2);
        assert_eq!(registry.live(), 0);
    }
}
