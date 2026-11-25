//! D17: the host authority behind the linear `Distributed` wrappers.
//!
//! The Austral module cannot forge a remote resource, an authority token or a
//! pending request: every constructor is a foreign call into this module, and
//! every handle is a process-local word. What crosses the wire is the identity
//! triple `(resource, deployment, epoch)` plus the request id and payload
//! digest; the local handle is never serialized.
//!
//! The authority mirrors the D14 fence and the D15 dispatch protocol closely
//! enough to be a test seam: a submitted request has a fence generation owned
//! by the authority, and its lifecycle is recorded as `planned`, `dispatching`
//! and then `acknowledged`. A response is accepted only for the request id and
//! digest that were submitted, and only under the current resource epoch.
//!
//! This is not the live quePaxa transport and not the durable outbox: those
//! are `unfer_consensus` and `unfer_ffi`'s job. It is the host half that makes
//! the wrappers executable, and the D17 evidence names that boundary.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

pub const STATUS_OK: i64 = 0;
pub const STATUS_REFUSED: i64 = -1;
pub const STATUS_DUPLICATE: i64 = -2;
pub const STATUS_NO_TOKEN: i64 = -3;
pub const STATUS_WRONG_OWNER: i64 = -4;
pub const STATUS_WRONG_DEPLOYMENT: i64 = -5;
pub const STATUS_RETIRED_EPOCH: i64 = -6;
pub const STATUS_REPLAY: i64 = -7;
pub const STATUS_NO_RESOURCE: i64 = -8;
pub const STATUS_NO_REQUEST: i64 = -9;
pub const STATUS_WRONG_DIGEST: i64 = -10;
pub const STATUS_ALREADY_SENT: i64 = -11;
pub const STATUS_INVALID: i64 = -12;
pub const STATUS_OVERFLOW: i64 = -13;
pub const STATUS_CAPACITY: i64 = -14;

/// A request outbox is a bounded local queue; retrying forever must not grow
/// it without limit. The cap is far above the D16/D17 vectors.
const MAX_OUTBOX: usize = 64;

/// Wire/authority identifiers are positive and below the i64 handle space.
/// Keeping `0` and negatives out is what makes a stray sentinel look like a
/// refusal instead of an authority or a resource.
fn valid_identity(value: i64) -> bool {
    value > 0
}

fn valid_digest(value: i64) -> bool {
    value >= 0
}

pub const RESULT_APPLIED: i64 = 2;
pub const RESULT_REJECTED: i64 = 3;

#[derive(Debug, Clone)]
struct Resource {
    deployment: i64,
    owner: i64,
    epoch: i64,
    generation: i64,
}

#[derive(Debug, Clone)]
struct Token {
    resource: i64,
    deployment: i64,
    owner: i64,
    epoch: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Pending,
    Applied,
    Rejected,
}

#[derive(Debug, Clone)]
struct Request {
    request_id: i64,
    digest: i64,
    resource: i64,
    deployment: i64,
    owner: i64,
    epoch: i64,
    fence: i64,
    state: RequestState,
    attempts: u32,
    outbox: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy)]
struct Receipt {
    code: i64,
}

/// The authority's state. A fresh one is created for each process; tests build
/// their own instead of sharing the FFI global.
#[derive(Default)]
pub struct Authority {
    next_handle: i64,
    resources: BTreeMap<i64, Resource>,
    tokens: BTreeMap<i64, Token>,
    requests: BTreeMap<i64, Request>,
    receipts: BTreeMap<i64, Receipt>,
    /// wire id -> the token fields it carries and whether it was adopted.
    wires: BTreeMap<i64, (Token, bool)>,
}

impl Authority {
    fn mint(&mut self) -> i64 {
        match self.next_handle.checked_add(1) {
            Some(next) => {
                self.next_handle = next;
                next
            }
            None => STATUS_OVERFLOW,
        }
    }

    pub fn introduce(&mut self, resource: i64, deployment: i64, owner: i64, epoch: i64) -> i64 {
        if !valid_identity(resource)
            || !valid_identity(deployment)
            || !valid_identity(owner)
            || !valid_identity(epoch)
        {
            return STATUS_INVALID;
        }
        if self.resources.contains_key(&resource) {
            return STATUS_DUPLICATE;
        }
        self.resources.insert(
            resource,
            Resource {
                deployment,
                owner,
                epoch,
                generation: 0,
            },
        );
        resource
    }

    pub fn retire(&mut self, resource: i64, new_epoch: i64) -> i64 {
        if !valid_identity(new_epoch) {
            return STATUS_INVALID;
        }
        let Some(entry) = self.resources.get_mut(&resource) else {
            return STATUS_NO_RESOURCE;
        };
        if new_epoch <= entry.epoch {
            return STATUS_RETIRED_EPOCH;
        }
        entry.epoch = new_epoch;
        entry.generation = 0;
        STATUS_OK
    }

    pub fn issue_token(&mut self, resource: i64, owner: i64, deployment: i64, epoch: i64) -> i64 {
        if !valid_identity(owner) || !valid_identity(deployment) || !valid_identity(epoch) {
            return STATUS_INVALID;
        }
        let Some(entry) = self.resources.get(&resource) else {
            return STATUS_NO_RESOURCE;
        };
        if entry.owner != owner {
            return STATUS_WRONG_OWNER;
        }
        if entry.deployment != deployment {
            return STATUS_WRONG_DEPLOYMENT;
        }
        if entry.epoch != epoch {
            return STATUS_RETIRED_EPOCH;
        }
        let handle = self.mint();
        if handle < 0 {
            return handle;
        }
        self.tokens.insert(
            handle,
            Token {
                resource,
                deployment,
                owner,
                epoch,
            },
        );
        handle
    }

    /// Serialize a token to its wire identity *and consume the local handle*.
    /// A serialized authority is therefore transferable, not copyable.
    pub fn token_wire(&mut self, token: i64) -> i64 {
        let Some(value) = self.tokens.remove(&token) else {
            return STATUS_NO_TOKEN;
        };
        let wire = self.mint();
        if wire < 0 {
            return wire;
        }
        self.wires.insert(wire, (value, false));
        wire
    }

    /// Adopt wire bytes exactly once. Replaying the same bytes is refused, so
    /// two processes cannot both turn one authority into an owner.
    pub fn adopt_wire(&mut self, wire: i64) -> i64 {
        let Some((token, adopted)) = self.wires.get(&wire) else {
            return STATUS_NO_TOKEN;
        };
        if *adopted {
            return STATUS_REPLAY;
        }
        let token = token.clone();
        let handle = self.mint();
        if handle < 0 {
            return handle;
        }
        if let Some((_, adopted)) = self.wires.get_mut(&wire) {
            *adopted = true;
        }
        self.tokens.insert(handle, token);
        handle
    }

    pub fn open_request(&mut self, token: i64, request_id: i64, digest: i64) -> i64 {
        if !valid_identity(request_id) || !valid_digest(digest) {
            return STATUS_INVALID;
        }
        let Some(value) = self.tokens.remove(&token) else {
            return STATUS_NO_TOKEN;
        };
        let Some(resource) = self.resources.get(&value.resource) else {
            return STATUS_NO_RESOURCE;
        };
        if resource.owner != value.owner {
            return STATUS_WRONG_OWNER;
        }
        if resource.deployment != value.deployment {
            return STATUS_WRONG_DEPLOYMENT;
        }
        if resource.epoch != value.epoch {
            return STATUS_RETIRED_EPOCH;
        }
        let handle = self.mint();
        if handle < 0 {
            return handle;
        }
        self.requests.insert(
            handle,
            Request {
                request_id,
                digest,
                resource: value.resource,
                deployment: value.deployment,
                owner: value.owner,
                epoch: value.epoch,
                fence: 0,
                state: RequestState::Pending,
                attempts: 0,
                outbox: Vec::new(),
            },
        );
        handle
    }

    pub fn submit(&mut self, request: i64) -> i64 {
        let Some(value) = self.requests.get_mut(&request) else {
            return STATUS_NO_REQUEST;
        };
        if value.state != RequestState::Pending {
            return STATUS_ALREADY_SENT;
        }
        if value.fence != 0 {
            return STATUS_ALREADY_SENT;
        }
        if value.outbox.len() + 2 > MAX_OUTBOX {
            return STATUS_CAPACITY;
        }
        let Some(resource) = self.resources.get_mut(&value.resource) else {
            return STATUS_NO_RESOURCE;
        };
        let Some(next_generation) = resource.generation.checked_add(1) else {
            return STATUS_OVERFLOW;
        };
        resource.generation = next_generation;
        value.fence = resource.generation;
        value.attempts = value.attempts.saturating_add(1);
        value.outbox.push("planned");
        value.outbox.push("dispatching");
        STATUS_OK
    }

    /// Retry consumes the local request and returns it with the *same* wire
    /// identity and the same fence. A new fence would be a new claim, which is
    /// a different operation.
    pub fn retry(&mut self, request: i64) -> i64 {
        let Some(value) = self.requests.get_mut(&request) else {
            return STATUS_NO_REQUEST;
        };
        if value.state != RequestState::Pending {
            return 4;
        }
        if value.outbox.len() >= MAX_OUTBOX {
            return STATUS_CAPACITY;
        }
        value.attempts = value.attempts.saturating_add(1);
        value.outbox.push("dispatching");
        1
    }

    pub fn cancel_wait(&mut self, request: i64) -> i64 {
        let Some(value) = self.requests.get_mut(&request) else {
            return STATUS_NO_REQUEST;
        };
        if value.outbox.len() >= MAX_OUTBOX {
            return STATUS_CAPACITY;
        }
        value.outbox.push("cancel_wait");
        // The authority still owns the request; cancellation is local.
        STATUS_OK
    }

    pub fn release_request(&mut self, request: i64) -> i64 {
        if self.requests.remove(&request).is_some() {
            STATUS_OK
        } else {
            STATUS_NO_REQUEST
        }
    }

    pub fn respond(&mut self, request_id: i64, digest: i64, accepted: i64) -> i64 {
        if !valid_identity(request_id) || !valid_digest(digest) {
            return STATUS_INVALID;
        }
        if accepted != 0 && accepted != 1 {
            return STATUS_INVALID;
        }
        let mut found = None;
        for (handle, value) in &self.requests {
            if value.request_id == request_id {
                found = Some(*handle);
                break;
            }
        }
        let Some(handle) = found else {
            return STATUS_NO_REQUEST;
        };
        let (resource, owner, deployment, epoch) = {
            let value = &self.requests[&handle];
            (value.resource, value.owner, value.deployment, value.epoch)
        };
        let Some(entry) = self.resources.get(&resource) else {
            return STATUS_NO_RESOURCE;
        };
        if entry.owner != owner {
            return STATUS_WRONG_OWNER;
        }
        if entry.deployment != deployment {
            return STATUS_WRONG_DEPLOYMENT;
        }
        if entry.epoch != epoch {
            return STATUS_RETIRED_EPOCH;
        }
        let Some(value) = self.requests.get_mut(&handle) else {
            return STATUS_NO_REQUEST;
        };
        if value.fence == 0 {
            return STATUS_ALREADY_SENT;
        }
        if value.state != RequestState::Pending {
            return STATUS_ALREADY_SENT;
        }
        if value.digest != digest {
            value.state = RequestState::Rejected;
            value.outbox.push("rejected");
            return STATUS_WRONG_DIGEST;
        }
        value.state = if accepted != 0 {
            value.outbox.push("acknowledged");
            RequestState::Applied
        } else {
            value.outbox.push("rejected");
            RequestState::Rejected
        };
        STATUS_OK
    }

    /// Wait returns `0` while pending, or a receipt handle for the terminal
    /// state. The request is consumed by a terminal wait.
    pub fn wait(&mut self, request: i64) -> i64 {
        let Some(value) = self.requests.get(&request) else {
            return STATUS_NO_REQUEST;
        };
        if value.state == RequestState::Pending {
            return 0;
        }
        let receipt = Receipt {
            code: if value.state == RequestState::Applied {
                RESULT_APPLIED
            } else {
                RESULT_REJECTED
            },
        };
        self.requests.remove(&request);
        let handle = self.mint();
        if handle < 0 {
            return handle;
        }
        self.receipts.insert(handle, receipt);
        handle
    }

    pub fn receipt_code(&self, receipt: i64) -> i64 {
        self.receipts
            .get(&receipt)
            .map(|value| value.code)
            .unwrap_or(STATUS_NO_REQUEST)
    }

    pub fn receipt_release(&mut self, receipt: i64) -> i64 {
        if self.receipts.remove(&receipt).is_some() {
            STATUS_OK
        } else {
            STATUS_NO_REQUEST
        }
    }

    pub fn authorize(&mut self, token: i64, resource: i64) -> i64 {
        let Some(value) = self.tokens.remove(&token) else {
            return STATUS_NO_TOKEN;
        };
        let Some(entry) = self.resources.get(&resource) else {
            return STATUS_NO_RESOURCE;
        };
        if value.resource != resource {
            return STATUS_NO_RESOURCE;
        }
        if entry.owner != value.owner {
            return STATUS_WRONG_OWNER;
        }
        if entry.deployment != value.deployment {
            return STATUS_WRONG_DEPLOYMENT;
        }
        if entry.epoch != value.epoch {
            return STATUS_RETIRED_EPOCH;
        }
        STATUS_OK
    }

    pub fn release_token(&mut self, token: i64) -> i64 {
        if self.tokens.remove(&token).is_some() {
            STATUS_OK
        } else {
            STATUS_NO_TOKEN
        }
    }

    pub fn release_resource(&mut self, resource: i64) -> i64 {
        if self.resources.remove(&resource).is_some() {
            STATUS_OK
        } else {
            STATUS_NO_RESOURCE
        }
    }

    #[cfg(test)]
    fn outbox(&self, request: i64) -> Vec<&'static str> {
        self.requests
            .get(&request)
            .map(|value| value.outbox.clone())
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn fence(&self, resource: i64) -> i64 {
        self.resources
            .get(&resource)
            .map(|value| value.generation)
            .unwrap_or(0)
    }
}

fn authority() -> &'static Mutex<Authority> {
    static AUTHORITY: OnceLock<Mutex<Authority>> = OnceLock::new();
    AUTHORITY.get_or_init(|| Mutex::new(Authority::default()))
}

fn with<R>(f: impl FnOnce(&mut Authority) -> R) -> R {
    let mut guard = authority()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

#[no_mangle]
pub extern "C" fn au_dist_reset() -> i64 {
    with(|a| {
        *a = Authority::default();
        0
    })
}

#[no_mangle]
pub extern "C" fn au_dist_introduce(resource: i64, deployment: i64, owner: i64, epoch: i64) -> i64 {
    with(|a| a.introduce(resource, deployment, owner, epoch))
}

#[no_mangle]
pub extern "C" fn au_dist_retire(resource: i64, new_epoch: i64) -> i64 {
    with(|a| a.retire(resource, new_epoch))
}

#[no_mangle]
pub extern "C" fn au_dist_issue_token(
    resource: i64,
    owner: i64,
    deployment: i64,
    epoch: i64,
) -> i64 {
    with(|a| a.issue_token(resource, owner, deployment, epoch))
}

#[no_mangle]
pub extern "C" fn au_dist_token_wire(token: i64) -> i64 {
    with(|a| a.token_wire(token))
}

#[no_mangle]
pub extern "C" fn au_dist_adopt_wire(wire: i64) -> i64 {
    with(|a| a.adopt_wire(wire))
}

#[no_mangle]
pub extern "C" fn au_dist_open_request(token: i64, request_id: i64, digest: i64) -> i64 {
    with(|a| a.open_request(token, request_id, digest))
}

#[no_mangle]
pub extern "C" fn au_dist_submit(request: i64) -> i64 {
    with(|a| a.submit(request))
}

#[no_mangle]
pub extern "C" fn au_dist_retry(request: i64) -> i64 {
    with(|a| a.retry(request))
}

#[no_mangle]
pub extern "C" fn au_dist_wait(request: i64) -> i64 {
    with(|a| a.wait(request))
}

#[no_mangle]
pub extern "C" fn au_dist_cancel_wait(request: i64) -> i64 {
    with(|a| a.cancel_wait(request))
}

#[no_mangle]
pub extern "C" fn au_dist_release_request(request: i64) -> i64 {
    with(|a| a.release_request(request))
}

#[no_mangle]
pub extern "C" fn au_dist_respond(request_id: i64, digest: i64, accepted: i64) -> i64 {
    with(|a| a.respond(request_id, digest, accepted))
}

#[no_mangle]
pub extern "C" fn au_dist_receipt_code(receipt: i64) -> i64 {
    with(|a| a.receipt_code(receipt))
}

#[no_mangle]
pub extern "C" fn au_dist_receipt_release(receipt: i64) -> i64 {
    with(|a| a.receipt_release(receipt))
}

#[no_mangle]
pub extern "C" fn au_dist_authorize(token: i64, resource: i64) -> i64 {
    with(|a| a.authorize(token, resource))
}

#[no_mangle]
pub extern "C" fn au_dist_release_token(token: i64) -> i64 {
    with(|a| a.release_token(token))
}

#[no_mangle]
pub extern "C" fn au_dist_release_resource(resource: i64) -> i64 {
    with(|a| a.release_resource(resource))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority_with_resource() -> Authority {
        let mut a = Authority::default();
        assert_eq!(a.introduce(101, 202, 303, 1), 101);
        a
    }

    #[test]
    fn wrong_owner_is_refused() {
        let mut a = authority_with_resource();
        assert_eq!(a.issue_token(101, 999, 202, 1), STATUS_WRONG_OWNER);
    }

    #[test]
    fn wrong_deployment_is_refused() {
        let mut a = authority_with_resource();
        assert_eq!(a.issue_token(101, 303, 999, 1), STATUS_WRONG_DEPLOYMENT);
    }

    #[test]
    fn retired_epoch_is_refused() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        assert!(token > 0);
        assert_eq!(a.retire(101, 2), STATUS_OK);
        assert_eq!(a.authorize(token, 101), STATUS_RETIRED_EPOCH);
    }

    #[test]
    fn replayed_wire_token_cannot_create_a_second_owner() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let wire = a.token_wire(token);
        assert!(wire > 0);
        let adopted = a.adopt_wire(wire);
        assert!(adopted > 0);
        assert_eq!(a.adopt_wire(wire), STATUS_REPLAY);
        assert_eq!(a.authorize(adopted, 101), STATUS_OK);
        // The original handle was consumed by serialization.
        assert_eq!(a.authorize(token, 101), STATUS_NO_TOKEN);
    }

    #[test]
    fn request_transport_uses_a_fence_and_the_dispatch_protocol() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert!(request > 0);
        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.fence(101), 1);
        assert_eq!(a.outbox(request), vec!["planned", "dispatching"]);

        assert_eq!(a.wait(request), 0);
        assert_eq!(a.respond(7, 11, 1), STATUS_OK);
        let receipt = a.wait(request);
        assert!(receipt > 0);
        assert_eq!(a.receipt_code(receipt), RESULT_APPLIED);
        assert_eq!(a.receipt_release(receipt), STATUS_OK);
    }

    #[test]
    fn a_mismatched_digest_rejects_and_preserves_the_held_digest() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.respond(7, 12, 1), STATUS_WRONG_DIGEST);
        let receipt = a.wait(request);
        assert!(receipt > 0);
        assert_eq!(a.receipt_code(receipt), RESULT_REJECTED);
    }

    #[test]
    fn forged_handles_are_refused() {
        let mut a = authority_with_resource();
        assert_eq!(a.authorize(9_999, 101), STATUS_NO_TOKEN);
        assert_eq!(a.open_request(9_999, 7, 11), STATUS_NO_TOKEN);
        assert_eq!(a.submit(9_999), STATUS_NO_REQUEST);
        assert_eq!(a.release_resource(9_999), STATUS_NO_RESOURCE);
    }

    #[test]
    fn non_positive_identities_and_digests_are_refused() {
        let mut a = Authority::default();
        assert_eq!(a.introduce(0, 202, 303, 1), STATUS_INVALID);
        assert_eq!(a.introduce(101, -1, 303, 1), STATUS_INVALID);
        assert_eq!(a.introduce(101, 202, 0, 1), STATUS_INVALID);
        assert_eq!(a.introduce(101, 202, 303, 0), STATUS_INVALID);

        assert_eq!(a.introduce(101, 202, 303, 1), 101);
        let token = a.issue_token(101, 303, 202, 1);
        assert!(token > 0);
        assert_eq!(a.open_request(token, 0, 11), STATUS_INVALID);
        assert_eq!(a.open_request(token, 7, -1), STATUS_INVALID);
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert!(request > 0);
        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.respond(7, 11, 2), STATUS_INVALID);
        assert_eq!(a.retire(101, 0), STATUS_INVALID);
    }

    #[test]
    fn bounded_counters_refuse_overflow_and_capacity() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert_eq!(a.submit(request), STATUS_OK);
        for _ in 0..(MAX_OUTBOX - 2) {
            assert_eq!(a.retry(request), 1);
        }
        assert_eq!(a.retry(request), STATUS_CAPACITY);
        assert_eq!(a.cancel_wait(request), STATUS_CAPACITY);

        let mut b = authority_with_resource();
        b.resources.get_mut(&101).unwrap().generation = i64::MAX;
        let token = b.issue_token(101, 303, 202, 1);
        let request = b.open_request(token, 7, 11);
        assert_eq!(b.submit(request), STATUS_OVERFLOW);
    }

    #[test]
    fn retry_reuses_the_identity_and_fence() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.retry(request), 1);
        assert_eq!(a.fence(101), 1);
        let outbox = a.outbox(request);
        assert_eq!(outbox, vec!["planned", "dispatching", "dispatching"]);
    }
}
