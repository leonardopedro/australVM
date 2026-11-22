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
