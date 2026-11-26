//! D19: host-enforced markers for the Austral channel-contract wrapper.
//!
//! The Austral module can hold an opaque handle, but it cannot fabricate the
//! contract behind it. [`au_channel_acquire`] builds the consensus submission
//! contract in the host, and the two probe verbs answer only after the host has
//! obtained the corresponding marker from that contract. A forged or released
//! handle is refused rather than treated as a claim.
//!
//! This is deliberately small: the Austral surface exposes the contract that
//! the live submission path actually enforces (total order, exactly-once by
//! request identity, bounded by the durable journal). It does not expose the
//! other constructors yet, because no production boundary has adopted them.

use std::sync::{Mutex, OnceLock};

use unfer_protocol::channel_contracts::{consensus_live_submission_contract, ChannelContract};

/// A handle is an index into this process-local table; it is never serialized.
const MAX_HANDLES: usize = 1024;

const STATUS_INVALID: i64 = -1;
const STATUS_CAPACITY: i64 = -2;
const STATUS_CONTRACT_INVALID: i64 = -3;

#[derive(Default)]
struct Registry {
    slots: Vec<Option<ChannelContract>>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

/// Build the live submission contract and return an opaque handle.
///
/// `0` is never a valid handle. A full table returns [`STATUS_CAPACITY`]; a
/// contract that fails its own constructors returns
/// [`STATUS_CONTRACT_INVALID`] instead of a handle.
#[no_mangle]
pub extern "C" fn au_channel_acquire() -> i64 {
    let contract = match consensus_live_submission_contract() {
        Ok(contract) => contract,
        Err(_) => return STATUS_CONTRACT_INVALID,
    };
    let mut registry = match registry().lock() {
        Ok(registry) => registry,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(index) = registry.slots.iter().position(Option::is_none) {
        registry.slots[index] = Some(contract);
        return (index + 1) as i64;
    }
    if registry.slots.len() >= MAX_HANDLES {
        return STATUS_CAPACITY;
    }
    registry.slots.push(Some(contract));
    registry.slots.len() as i64
}

fn with_contract(handle: i64, probe: impl FnOnce(&ChannelContract) -> bool) -> i64 {
    if handle <= 0 {
        return STATUS_INVALID;
    }
    let registry = match registry().lock() {
        Ok(registry) => registry,
        Err(poisoned) => poisoned.into_inner(),
    };
    match registry.slots.get((handle - 1) as usize) {
        Some(Some(contract)) => i64::from(probe(contract)),
        _ => STATUS_INVALID,
    }
}

/// `1` when the contract's ordering scope covers its retry lifetime.
#[no_mangle]
pub extern "C" fn au_channel_ordered(handle: i64) -> i64 {
    with_contract(handle, |contract| contract.ordered_channel().is_ok())
}

/// `1` when the contract carries exactly-once delivery and its suppressor.
#[no_mangle]
pub extern "C" fn au_channel_exactly_once(handle: i64) -> i64 {
    with_contract(handle, |contract| contract.exactly_once_channel().is_ok())
}

/// Release the handle. A second release is refused.
#[no_mangle]
pub extern "C" fn au_channel_release(handle: i64) -> i64 {
    if handle <= 0 {
        return STATUS_INVALID;
    }
    let mut registry = match registry().lock() {
        Ok(registry) => registry,
        Err(poisoned) => poisoned.into_inner(),
    };
    match registry.slots.get_mut((handle - 1) as usize) {
        Some(slot @ Some(_)) => {
            *slot = None;
            0
        }
        _ => STATUS_INVALID,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_handle_proves_only_the_live_contract_markers() {
        let handle = au_channel_acquire();
        assert!(handle > 0, "the live contract is constructible");
        assert_eq!(au_channel_ordered(handle), 1);
        assert_eq!(au_channel_exactly_once(handle), 1);
        assert_eq!(au_channel_release(handle), 0);
        assert_eq!(au_channel_release(handle), STATUS_INVALID);
        assert_eq!(au_channel_ordered(handle), STATUS_INVALID);
    }

    #[test]
    fn forged_handles_are_refused() {
        assert_eq!(au_channel_ordered(0), STATUS_INVALID);
        assert_eq!(au_channel_ordered(-9), STATUS_INVALID);
        assert_eq!(au_channel_exactly_once(999_999), STATUS_INVALID);
    }
}
