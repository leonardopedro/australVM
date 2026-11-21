//! U13 (bounded start) acceptance: an owned value is used only through the
//! handle that its own provider issued, and destruction is exact.
//!
//! Every test is a different mistake the registry has to refuse, and each one
//! would otherwise be a use-after-free, a cross-type reinterpretation or a
//! double destructor in generated code. The drop counter is the oracle for
//! destruction: it is independent of the registry's own release accounting, so
//! the two must agree.

use std::cell::RefCell;
use std::rc::Rc;

use umbrella::handles::{Handle, HandleError, HandleRegistry, Owner, TypeTag, HANDLE_SCHEMA};

/// A value that counts its own destructions.
#[derive(Debug)]
struct Tracked {
    id: u32,
    drops: Rc<RefCell<Vec<u32>>>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.drops.borrow_mut().push(self.id);
    }
}

fn drops() -> Rc<RefCell<Vec<u32>>> {
    Rc::new(RefCell::new(Vec::new()))
}

fn tracked(id: u32, drops: &Rc<RefCell<Vec<u32>>>) -> Tracked {
    Tracked {
        id,
        drops: Rc::clone(drops),
    }
}

fn registry() -> HandleRegistry<Tracked> {
    HandleRegistry::new(Owner::issued(11), TypeTag::named("Tracked"), 1)
}

#[test]
fn the_handle_schema_is_versioned() {
    assert_eq!(HANDLE_SCHEMA, 1);
}

#[test]
fn a_stored_value_is_readable_through_its_own_handle() {
    let drops = drops();
    let mut registry = registry();
    let handle = registry.insert(tracked(1, &drops)).expect("insert");

    assert_eq!(registry.get(handle).expect("get").id, 1);
    registry.get_mut(handle).expect("get_mut").id = 2;
    assert_eq!(registry.get(handle).expect("get").id, 2);
    assert_eq!(registry.live(), 1);

    registry.destroy(handle).expect("destroy");
    assert_eq!(&*drops.borrow(), &[2]);
    assert_eq!(registry.live(), 0);
    assert_eq!(registry.released(), 1);
}

#[test]
fn destroying_a_handle_twice_does_not_drop_the_value_twice() {
    let drops = drops();
    let mut registry = registry();
    let handle = registry.insert(tracked(1, &drops)).expect("insert");

    registry.destroy(handle).expect("first destroy");
    let err = registry.destroy(handle).expect_err("second destroy");
    assert!(matches!(err, HandleError::Stale { .. }), "{err}");

    assert_eq!(&*drops.borrow(), &[1], "the destructor ran exactly once");
    assert_eq!(registry.released(), 1, "the release count did not double");
}

#[test]
fn a_reused_slot_does_not_resurrect_the_destroyed_value() {
    let drops = drops();
    let mut registry = registry();
    let old = registry.insert(tracked(1, &drops)).expect("insert");
    registry.destroy(old).expect("destroy");

    let new = registry.insert(tracked(2, &drops)).expect("insert");
    assert_eq!(new.slot(), old.slot(), "the freed slot is reused");
    assert_ne!(new, old, "but the handle is a different one");

    let err = registry.get(old).expect_err("stale handle");
    assert!(matches!(err, HandleError::Stale { .. }), "{err}");
    assert_eq!(registry.get(new).expect("new handle").id, 2);
    assert_eq!(&*drops.borrow(), &[1]);

    registry.destroy(new).expect("destroy");
    assert_eq!(&*drops.borrow(), &[1, 2]);
    assert_eq!(registry.issued(), 2);
    assert_eq!(registry.released(), 2);
}

#[test]
fn a_handle_from_another_provider_instance_is_refused() {
    let drops = drops();
    let mut mine = HandleRegistry::<Tracked>::new(Owner::issued(1), TypeTag::named("Tracked"), 1);
    let mut theirs = HandleRegistry::<Tracked>::new(Owner::issued(2), TypeTag::named("Tracked"), 1);

    let handle: Handle = theirs.insert(tracked(9, &drops)).expect("insert");
    let err = mine.get(handle).expect_err("foreign handle");
    match err {
        HandleError::ForeignOwner { expected, found } => {
            assert_eq!((expected, found), (1, 2));
        }
        other => panic!("expected ForeignOwner, got {other}"),
    }

    // Same slot index and generation in this table is still not this handle.
    let local = mine.insert(tracked(1, &drops)).expect("insert");
    assert_eq!(local.slot(), handle.slot());
    assert_ne!(local, handle);
    assert_eq!(mine.get(local).expect("local").id, 1);
}

#[test]
fn a_handle_for_another_type_is_refused() {
    let numbers = HandleRegistry::<u32>::new(Owner::issued(1), TypeTag::named("Counter"), 1);
    let mut strings = HandleRegistry::<u32>::new(Owner::issued(1), TypeTag::named("Label"), 1);

    let handle = strings.insert(7).expect("insert");
    let err = numbers.get(handle).expect_err("wrong type");
    match err {
        HandleError::ForeignType { expected, found } => {
            assert_eq!((expected, found), ("Counter", "Label"));
        }
        other => panic!("expected ForeignType, got {other}"),
    }
}

#[test]
fn a_handle_from_a_previous_provider_generation_is_refused() {
    let drops = drops();
    let mut old = HandleRegistry::<Tracked>::new(Owner::issued(1), TypeTag::named("Tracked"), 1);
    let handle = old.insert(tracked(1, &drops)).expect("insert");

    // The provider reloaded: same instance tag, same type, next generation.
    let mut new = HandleRegistry::<Tracked>::new(Owner::issued(1), TypeTag::named("Tracked"), 2);
    let err = new.get(handle).expect_err("stale provider");
    match err {
        HandleError::StaleProvider { expected, found } => assert_eq!((expected, found), (2, 1)),
        other => panic!("expected StaleProvider, got {other}"),
    }

    let fresh = new.insert(tracked(2, &drops)).expect("insert");
    assert_eq!(fresh.slot(), handle.slot());
    assert_ne!(fresh, handle);
    assert_eq!(new.get(fresh).expect("fresh").id, 2);
}

#[test]
fn a_word_pair_round_trips_and_the_null_pair_is_refused() {
    let mut registry = HandleRegistry::<u32>::new(Owner::issued(3), TypeTag::named("W"), 4);
    let handle = registry.insert(7).expect("insert");
    let (lo, hi) = handle.to_words();

    // Every component is 32 bits, so both words are the pair's fields verbatim.
    assert_eq!((lo >> 32, lo as u32), (3, handle.slot()));
    assert_eq!((hi >> 32, hi as u32), (4, 0));
    assert_eq!(*registry.get_words(lo, hi).expect("words"), 7);

    assert_eq!(
        registry.get_words(0, 0),
        Err(HandleError::Malformed { lo: 0, hi: 0 }),
        "the null pair is refused rather than read as slot 0"
    );
    assert_eq!(registry.get_words(0, hi), Err(HandleError::ForeignOwner { expected: 3, found: 0 }));
    assert_eq!(
        registry.get_words(lo, hi + (1 << 32)),
        Err(HandleError::StaleProvider {
            expected: 4,
            found: 5
        })
    );
}

#[test]
fn allocation_and_free_counters_stand_apart_from_the_drop_oracle() {
    let drops = drops();
    let mut registry = registry();
    let destroyed = registry.insert(tracked(1, &drops)).expect("insert");
    let moved = registry.insert(tracked(2, &drops)).expect("insert");

    registry.destroy(destroyed).expect("destroy");
    let value = registry.take(moved).expect("take");

    assert_eq!(registry.allocated(), 2, "two values were stored");
    assert_eq!(registry.freed(), 1, "only `destroy` frees in the table");
    assert_eq!(registry.released(), 2, "both slots are free");
    assert_eq!(registry.live(), 0);
    assert_eq!(&*drops.borrow(), &[1], "a moved value is the caller's to drop");

    drop(value);
    assert_eq!(&*drops.borrow(), &[1, 2], "the oracle agrees with `freed` + the moved drop");
}

#[test]
fn take_moves_the_value_out_and_leaves_the_handle_stale() {
    let drops = drops();
    let mut registry = registry();
    let handle = registry.insert(tracked(5, &drops)).expect("insert");

    let value = registry.take(handle).expect("take");
    assert_eq!(value.id, 5);
    assert!(drops.borrow().is_empty(), "moving does not drop");
    drop(value);
    assert_eq!(&*drops.borrow(), &[5]);

    let err = registry.take(handle).expect_err("second take");
    assert!(matches!(err, HandleError::Stale { .. }), "{err}");
    assert_eq!(&*drops.borrow(), &[5], "the value was not dropped again");
}
