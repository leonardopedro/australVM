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
pub const STATUS_STALE_RESOURCE: i64 = -15;

/// A request outbox is a bounded local queue; retrying forever must not grow
/// it without limit. The cap is far above the D16/D17 vectors.
const MAX_OUTBOX: usize = 64;

/// Completed/disposed request ids are remembered until their resource
/// incarnation ends, so a replay cannot name a second action. Keep that
/// tombstone set bounded; a full set fails closed with `STATUS_CAPACITY`.
const MAX_REQUEST_IDS: usize = 1 << 16;

/// Released resources keep their wire values so adoption can distinguish a
/// stale transfer from an unknown one. Bound that cache; under pressure stale
/// wires are evicted first, and a cache full of live wires fails closed.
const MAX_WIRES: usize = 1 << 16;

/// One D17 authority verb. The FFI functions below are thin wrappers over
/// this enum so a live transport can carry the same operation to another
/// process without changing the Austral surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Reset,
    Introduce {
        resource: i64,
        deployment: i64,
        owner: i64,
        epoch: i64,
    },
    Retire {
        resource: i64,
        new_epoch: i64,
    },
    IssueToken {
        resource: i64,
        owner: i64,
        deployment: i64,
        epoch: i64,
    },
    TokenWire {
        token: i64,
    },
    AdoptWire {
        wire: i64,
    },
    OpenRequest {
        token: i64,
        request_id: i64,
        digest: i64,
    },
    Submit {
        request: i64,
    },
    Retry {
        request: i64,
    },
    Wait {
        request: i64,
    },
    CancelWait {
        request: i64,
    },
    ReleaseRequest {
        request: i64,
    },
    Respond {
        request_id: i64,
        digest: i64,
        accepted: i64,
    },
    ReceiptCode {
        receipt: i64,
    },
    ReceiptRelease {
        receipt: i64,
    },
    Authorize {
        token: i64,
        resource: i64,
    },
    ReleaseToken {
        token: i64,
    },
    ReleaseResource {
        resource: i64,
    },
}

/// The D17 authority as reached from the FFI surface. The default is the
/// in-process authority; a harness may install a transport to make the same
/// verbs travel to an authority in another process. The local handles remain
/// opaque words either way.
pub trait Transport: Send {
    fn call(&mut self, operation: Operation) -> i64;
}

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
    /// Distinguishes successive introductions of the same resource id. A
    /// token or request from a released incarnation must never authorize
    /// against the next one, even when every caller-supplied field matches.
    incarnation: i64,
}

#[derive(Debug, Clone)]
struct Token {
    resource: i64,
    deployment: i64,
    owner: i64,
    epoch: i64,
    incarnation: i64,
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
    incarnation: i64,
    fence: i64,
    state: RequestState,
    attempts: u32,
    outbox: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy)]
struct Receipt {
    code: i64,
    resource: i64,
    epoch: i64,
    incarnation: i64,
}

#[derive(Debug, Clone, Copy)]
struct RequestIdTombstone {
    resource: i64,
    incarnation: i64,
    epoch: i64,
}

/// The authority's state. A fresh one is created for each process; tests build
/// their own instead of sharing the FFI global.
pub struct Authority {
    next_handle: i64,
    next_incarnation: i64,
    resources: BTreeMap<i64, Resource>,
    tokens: BTreeMap<i64, Token>,
    requests: BTreeMap<i64, Request>,
    receipts: BTreeMap<i64, Receipt>,
    /// request id -> the resource incarnation it names. One id names one
    /// action while that incarnation is live, including after the local
    /// request object was waited on or disposed. The id is global because
    /// `respond` carries only the request id and digest.
    request_ids: BTreeMap<i64, RequestIdTombstone>,
    /// wire id -> the token fields it carries and whether it was adopted.
    wires: BTreeMap<i64, (Token, bool)>,
    /// Test seam for the capacity refusal; production authorities use
    /// `MAX_REQUEST_IDS`.
    request_id_limit: usize,
}

impl Default for Authority {
    fn default() -> Self {
        Self {
            next_handle: 0,
            next_incarnation: 0,
            resources: BTreeMap::new(),
            tokens: BTreeMap::new(),
            requests: BTreeMap::new(),
            receipts: BTreeMap::new(),
            request_ids: BTreeMap::new(),
            wires: BTreeMap::new(),
            request_id_limit: MAX_REQUEST_IDS,
        }
    }
}

impl Authority {
    /// Build an authority whose request-id capacity can be reached cheaply by
    /// the live transport harness. The production constructor is `default`.
    #[doc(hidden)]
    pub fn with_request_id_limit(limit: usize) -> Self {
        Self {
            request_id_limit: limit.max(1),
            ..Self::default()
        }
    }

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
        let Some(incarnation) = self.next_incarnation.checked_add(1) else {
            return STATUS_OVERFLOW;
        };
        self.next_incarnation = incarnation;
        self.resources.insert(
            resource,
            Resource {
                deployment,
                owner,
                epoch,
                generation: 0,
                incarnation,
            },
        );
        resource
    }

    pub fn retire(&mut self, resource: i64, new_epoch: i64) -> i64 {
        if !valid_identity(new_epoch) {
            return STATUS_INVALID;
        }
        let (old_epoch, incarnation) = {
            let Some(entry) = self.resources.get_mut(&resource) else {
                return STATUS_NO_RESOURCE;
            };
            if new_epoch <= entry.epoch {
                return STATUS_RETIRED_EPOCH;
            }
            let old_epoch = entry.epoch;
            entry.epoch = new_epoch;
            entry.generation = 0;
            (old_epoch, entry.incarnation)
        };
        // Retiring an epoch ends every handle, receipt and id tombstone that
        // belonged to it. The resource id itself stays live for the new epoch.
        self.tokens
            .retain(|_, token| !(token.resource == resource && token.epoch == old_epoch));
        self.requests
            .retain(|_, request| !(request.resource == resource && request.epoch == old_epoch));
        self.receipts
            .retain(|_, receipt| !(receipt.resource == resource && receipt.epoch == old_epoch));
        self.wires
            .retain(|_, (token, _)| !(token.resource == resource && token.epoch == old_epoch));
        self.request_ids.retain(|_, tombstone| {
            !(tombstone.resource == resource
                && tombstone.incarnation == incarnation
                && tombstone.epoch == old_epoch)
        });
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
        let incarnation = entry.incarnation;
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
                incarnation,
            },
        );
        handle
    }

    fn token_status(&self, value: &Token) -> i64 {
        let Some(entry) = self.resources.get(&value.resource) else {
            return STATUS_NO_RESOURCE;
        };
        if entry.owner != value.owner {
            return STATUS_WRONG_OWNER;
        }
        if entry.deployment != value.deployment {
            return STATUS_WRONG_DEPLOYMENT;
        }
        if entry.epoch != value.epoch {
            return STATUS_RETIRED_EPOCH;
        }
        if entry.incarnation != value.incarnation {
            return STATUS_STALE_RESOURCE;
        }
        STATUS_OK
    }

    /// Serialize a token to its wire identity *and consume the local handle*.
    /// A serialized authority is therefore transferable, not copyable. A token
    /// that no longer names a live incarnation is refused before it becomes a
    /// wire value.
    pub fn token_wire(&mut self, token: i64) -> i64 {
        let Some(value) = self.tokens.get(&token) else {
            return STATUS_NO_TOKEN;
        };
        let status = self.token_status(value);
        if status != STATUS_OK {
            return status;
        }
        let status = self.make_wire_room();
        if status != STATUS_OK {
            return status;
        }
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

    /// Keep the wire cache bounded without losing the stale-transfer
    /// distinction while there is room. Only wires for a released incarnation
    /// or a retired epoch are evictable; live wires are not silently dropped.
    fn make_wire_room(&mut self) -> i64 {
        if self.wires.len() < MAX_WIRES {
            return STATUS_OK;
        }
        self.wires.retain(|_, (token, _)| {
            self.resources.get(&token.resource).is_some_and(|resource| {
                resource.epoch == token.epoch && resource.incarnation == token.incarnation
            })
        });
        if self.wires.len() < MAX_WIRES {
            STATUS_OK
        } else {
            STATUS_CAPACITY
        }
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
        let status = self.token_status(&token);
        if status != STATUS_OK {
            return status;
        }
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
        let incarnation = resource.incarnation;
        if self.request_ids.contains_key(&request_id) {
            return STATUS_DUPLICATE;
        }
        if self.request_ids.len() >= self.request_id_limit {
            return STATUS_CAPACITY;
        }
        let handle = self.mint();
        if handle < 0 {
            return handle;
        }
        self.request_ids.insert(
            request_id,
            RequestIdTombstone {
                resource: value.resource,
                incarnation,
                epoch: value.epoch,
            },
        );
        self.requests.insert(
            handle,
            Request {
                request_id,
                digest,
                resource: value.resource,
                deployment: value.deployment,
                owner: value.owner,
                epoch: value.epoch,
                incarnation: value.incarnation,
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
        if resource.incarnation != value.incarnation {
            return STATUS_STALE_RESOURCE;
        }
        if resource.epoch != value.epoch {
            return STATUS_RETIRED_EPOCH;
        }
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
        let Some(resource) = self.resources.get(&value.resource) else {
            return STATUS_NO_RESOURCE;
        };
        if resource.incarnation != value.incarnation {
            return STATUS_STALE_RESOURCE;
        }
        if resource.epoch != value.epoch {
            return STATUS_RETIRED_EPOCH;
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
        let Some(resource) = self.resources.get(&value.resource) else {
            return STATUS_NO_RESOURCE;
        };
        if resource.incarnation != value.incarnation {
            return STATUS_STALE_RESOURCE;
        }
        if resource.epoch != value.epoch {
            return STATUS_RETIRED_EPOCH;
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
        if entry.incarnation != self.requests[&handle].incarnation {
            return STATUS_STALE_RESOURCE;
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
            let Some(resource) = self.resources.get(&value.resource) else {
                return STATUS_NO_RESOURCE;
            };
            if resource.incarnation != value.incarnation {
                return STATUS_STALE_RESOURCE;
            }
            if resource.epoch != value.epoch {
                return STATUS_RETIRED_EPOCH;
            }
            return 0;
        }
        let receipt = Receipt {
            code: if value.state == RequestState::Applied {
                RESULT_APPLIED
            } else {
                RESULT_REJECTED
            },
            resource: value.resource,
            epoch: value.epoch,
            incarnation: value.incarnation,
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
        if entry.incarnation != value.incarnation {
            return STATUS_STALE_RESOURCE;
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
        let Some(entry) = self.resources.remove(&resource) else {
            return STATUS_NO_RESOURCE;
        };
        // Live local handles for this incarnation die with the resource. A
        // wire token may still be adopted (and is then refused as stale by its
        // recorded incarnation), but it can never answer for the next one.
        self.tokens
            .retain(|_, token| token.incarnation != entry.incarnation);
        self.requests
            .retain(|_, request| request.incarnation != entry.incarnation);
        self.receipts
            .retain(|_, receipt| receipt.incarnation != entry.incarnation);
        self.request_ids
            .retain(|_, tombstone| tombstone.incarnation != entry.incarnation);
        STATUS_OK
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

fn transport() -> &'static Mutex<Option<Box<dyn Transport>>> {
    static TRANSPORT: OnceLock<Mutex<Option<Box<dyn Transport>>>> = OnceLock::new();
    TRANSPORT.get_or_init(|| Mutex::new(None))
}

/// Install (or clear) the transport used by the FFI verbs. The harness owns
/// the transport lifetime; clearing it returns the FFI surface to the local
/// authority.
pub fn set_transport(transport_value: Option<Box<dyn Transport>>) {
    let mut slot = transport()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *slot = transport_value;
}

fn with<R>(f: impl FnOnce(&mut Authority) -> R) -> R {
    let mut guard = authority()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// Apply an operation to this process's authority. The server side of a
/// transport harness uses the same function, so both paths share validation.
pub fn apply(authority: &mut Authority, operation: Operation) -> i64 {
    match operation {
        Operation::Reset => {
            let limit = authority.request_id_limit;
            *authority = Authority::default();
            authority.request_id_limit = limit;
            0
        }
        Operation::Introduce {
            resource,
            deployment,
            owner,
            epoch,
        } => authority.introduce(resource, deployment, owner, epoch),
        Operation::Retire {
            resource,
            new_epoch,
        } => authority.retire(resource, new_epoch),
        Operation::IssueToken {
            resource,
            owner,
            deployment,
            epoch,
        } => authority.issue_token(resource, owner, deployment, epoch),
        Operation::TokenWire { token } => authority.token_wire(token),
        Operation::AdoptWire { wire } => authority.adopt_wire(wire),
        Operation::OpenRequest {
            token,
            request_id,
            digest,
        } => authority.open_request(token, request_id, digest),
        Operation::Submit { request } => authority.submit(request),
        Operation::Retry { request } => authority.retry(request),
        Operation::Wait { request } => authority.wait(request),
        Operation::CancelWait { request } => authority.cancel_wait(request),
        Operation::ReleaseRequest { request } => authority.release_request(request),
        Operation::Respond {
            request_id,
            digest,
            accepted,
        } => authority.respond(request_id, digest, accepted),
        Operation::ReceiptCode { receipt } => authority.receipt_code(receipt),
        Operation::ReceiptRelease { receipt } => authority.receipt_release(receipt),
        Operation::Authorize { token, resource } => authority.authorize(token, resource),
        Operation::ReleaseToken { token } => authority.release_token(token),
        Operation::ReleaseResource { resource } => authority.release_resource(resource),
    }
}

fn dispatch(operation: Operation) -> i64 {
    {
        let mut slot = transport()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(active) = slot.as_mut() {
            return active.call(operation);
        }
    }
    with(|authority| apply(authority, operation))
}

#[no_mangle]
pub extern "C" fn au_dist_reset() -> i64 {
    dispatch(Operation::Reset)
}

#[no_mangle]
pub extern "C" fn au_dist_introduce(resource: i64, deployment: i64, owner: i64, epoch: i64) -> i64 {
    dispatch(Operation::Introduce {
        resource,
        deployment,
        owner,
        epoch,
    })
}

#[no_mangle]
pub extern "C" fn au_dist_retire(resource: i64, new_epoch: i64) -> i64 {
    dispatch(Operation::Retire {
        resource,
        new_epoch,
    })
}

#[no_mangle]
pub extern "C" fn au_dist_issue_token(
    resource: i64,
    owner: i64,
    deployment: i64,
    epoch: i64,
) -> i64 {
    dispatch(Operation::IssueToken {
        resource,
        owner,
        deployment,
        epoch,
    })
}

#[no_mangle]
pub extern "C" fn au_dist_token_wire(token: i64) -> i64 {
    dispatch(Operation::TokenWire { token })
}

#[no_mangle]
pub extern "C" fn au_dist_adopt_wire(wire: i64) -> i64 {
    dispatch(Operation::AdoptWire { wire })
}

#[no_mangle]
pub extern "C" fn au_dist_open_request(token: i64, request_id: i64, digest: i64) -> i64 {
    dispatch(Operation::OpenRequest {
        token,
        request_id,
        digest,
    })
}

#[no_mangle]
pub extern "C" fn au_dist_submit(request: i64) -> i64 {
    dispatch(Operation::Submit { request })
}

#[no_mangle]
pub extern "C" fn au_dist_retry(request: i64) -> i64 {
    dispatch(Operation::Retry { request })
}

#[no_mangle]
pub extern "C" fn au_dist_wait(request: i64) -> i64 {
    dispatch(Operation::Wait { request })
}

#[no_mangle]
pub extern "C" fn au_dist_cancel_wait(request: i64) -> i64 {
    dispatch(Operation::CancelWait { request })
}

#[no_mangle]
pub extern "C" fn au_dist_release_request(request: i64) -> i64 {
    dispatch(Operation::ReleaseRequest { request })
}

#[no_mangle]
pub extern "C" fn au_dist_respond(request_id: i64, digest: i64, accepted: i64) -> i64 {
    dispatch(Operation::Respond {
        request_id,
        digest,
        accepted,
    })
}

#[no_mangle]
pub extern "C" fn au_dist_receipt_code(receipt: i64) -> i64 {
    dispatch(Operation::ReceiptCode { receipt })
}

#[no_mangle]
pub extern "C" fn au_dist_receipt_release(receipt: i64) -> i64 {
    dispatch(Operation::ReceiptRelease { receipt })
}

#[no_mangle]
pub extern "C" fn au_dist_authorize(token: i64, resource: i64) -> i64 {
    dispatch(Operation::Authorize { token, resource })
}

#[no_mangle]
pub extern "C" fn au_dist_release_token(token: i64) -> i64 {
    dispatch(Operation::ReleaseToken { token })
}

#[no_mangle]
pub extern "C" fn au_dist_release_resource(resource: i64) -> i64 {
    dispatch(Operation::ReleaseResource { resource })
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
        let wire_token = a.issue_token(101, 303, 202, 1);
        let wire = a.token_wire(wire_token);
        assert!(wire > 0);
        let serializable = a.issue_token(101, 303, 202, 1);
        let token = a.issue_token(101, 303, 202, 1);
        assert!(token > 0);
        assert_eq!(a.retire(101, 2), STATUS_OK);
        assert_eq!(a.issue_token(101, 303, 202, 1), STATUS_RETIRED_EPOCH);
        assert_eq!(a.authorize(token, 101), STATUS_NO_TOKEN);
        assert_eq!(a.token_wire(serializable), STATUS_NO_TOKEN);
        assert_eq!(a.adopt_wire(wire), STATUS_NO_TOKEN);
    }

    #[test]
    fn replayed_wire_token_cannot_create_a_second_owner() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let wire = a.token_wire(token);
        assert!(wire > 0);
        assert_eq!(a.token_wire(token), STATUS_NO_TOKEN);
        let adopted = a.adopt_wire(wire);
        assert!(adopted > 0);
        assert_eq!(a.adopt_wire(wire), STATUS_REPLAY);
        assert_eq!(a.authorize(adopted, 101), STATUS_OK);
        assert_eq!(a.authorize(adopted, 101), STATUS_NO_TOKEN);
        // The original handle was consumed by serialization.
        assert_eq!(a.authorize(token, 101), STATUS_NO_TOKEN);
        assert_eq!(a.release_resource(101), STATUS_OK);
        assert_eq!(a.release_resource(101), STATUS_NO_RESOURCE);
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

        let mut c = Authority::default();
        c.next_incarnation = i64::MAX;
        assert_eq!(c.introduce(101, 202, 303, 1), STATUS_OVERFLOW);
        assert!(!c.resources.contains_key(&101));
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

    #[test]
    fn a_reintroduced_resource_cannot_revive_a_stale_token_or_wire() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let wire = a.token_wire(token);
        assert!(wire > 0);

        assert_eq!(a.release_resource(101), STATUS_OK);
        assert_eq!(a.introduce(101, 202, 303, 1), 101);

        assert_eq!(a.adopt_wire(wire), STATUS_STALE_RESOURCE);
        // Adoption does not consume a stale wire: retrying the same bytes is
        // still refused, never converted into a current token.
        assert_eq!(a.adopt_wire(wire), STATUS_STALE_RESOURCE);

        let fresh = a.issue_token(101, 303, 202, 1);
        assert!(fresh > 0);
        assert_eq!(a.authorize(fresh, 101), STATUS_OK);
    }

    #[test]
    fn a_reintroduced_resource_cannot_answer_a_stale_pending_request() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert_eq!(a.submit(request), STATUS_OK);

        assert_eq!(a.release_resource(101), STATUS_OK);
        assert_eq!(a.introduce(101, 202, 303, 1), 101);

        assert_eq!(a.respond(7, 11, 1), STATUS_NO_REQUEST);
        assert_eq!(a.wait(request), STATUS_NO_REQUEST);
        assert_eq!(a.retry(request), STATUS_NO_REQUEST);

        // A fresh request with the same id is legal only because the old
        // resource incarnation was released; it cannot inherit the old one.
        let token = a.issue_token(101, 303, 202, 1);
        let fresh = a.open_request(token, 7, 11);
        assert!(fresh > 0);
        assert_eq!(a.submit(fresh), STATUS_OK);
        assert_eq!(a.respond(7, 11, 1), STATUS_OK);
    }

    #[test]
    fn a_response_cannot_be_replayed_or_applied_twice() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.respond(7, 11, 1), STATUS_OK);
        assert_eq!(a.respond(7, 11, 1), STATUS_ALREADY_SENT);
        let receipt = a.wait(request);
        assert!(receipt > 0);
        assert_eq!(a.receipt_code(receipt), RESULT_APPLIED);
        assert_eq!(a.respond(7, 11, 1), STATUS_NO_REQUEST);
        assert_eq!(a.wait(request), STATUS_NO_REQUEST);
    }

    #[test]
    fn releasing_a_resource_drops_its_receipts_and_request_ids() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.respond(7, 11, 1), STATUS_OK);
        let receipt = a.wait(request);
        assert_eq!(a.receipt_code(receipt), RESULT_APPLIED);
        assert!(!a.request_ids.is_empty());

        assert_eq!(a.release_resource(101), STATUS_OK);
        assert_eq!(a.receipt_code(receipt), STATUS_NO_REQUEST);
        assert!(a.request_ids.is_empty());
    }

    #[test]
    fn stale_wires_are_evicted_before_the_wire_cache_refuses() {
        let mut a = authority_with_resource();
        let stale = Token {
            resource: 999,
            deployment: 202,
            owner: 303,
            epoch: 1,
            incarnation: 1,
        };
        for wire in 1..=MAX_WIRES as i64 {
            a.wires.insert(wire, (stale.clone(), false));
        }

        let token = a.issue_token(101, 303, 202, 1);
        let wire = a.token_wire(token);
        assert!(wire > 0);
        assert_eq!(a.wires.len(), 1, "stale wires should be reclaimed");
    }

    #[test]
    fn a_rejected_response_receipt_lives_until_the_resource_is_released() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.respond(7, 11, 0), STATUS_OK);
        let receipt = a.wait(request);
        assert!(receipt > 0);
        assert_eq!(a.receipt_code(receipt), RESULT_REJECTED);
        assert_eq!(a.release_resource(101), STATUS_OK);
        assert_eq!(a.receipt_code(receipt), STATUS_NO_REQUEST);
    }

    #[test]
    fn cancellation_alone_mints_no_receipt_and_keeps_the_request_terminal() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.cancel_wait(request), STATUS_OK);
        assert_eq!(
            a.wait(request),
            0,
            "cancellation leaves the request pending"
        );
        assert!(a.receipts.is_empty());

        assert_eq!(a.respond(7, 11, 0), STATUS_OK);
        let receipt = a.wait(request);
        assert!(receipt > 0);
        assert_eq!(a.receipt_code(receipt), RESULT_REJECTED);
        assert_eq!(a.receipt_release(receipt), STATUS_OK);
        assert_eq!(a.receipt_code(receipt), STATUS_NO_REQUEST);
    }

    #[test]
    fn a_full_live_wire_cache_fails_closed_without_consuming_the_token() {
        let mut a = authority_with_resource();
        let live = Token {
            resource: 101,
            deployment: 202,
            owner: 303,
            epoch: 1,
            incarnation: 1,
        };
        for wire in 1..=MAX_WIRES as i64 {
            a.wires.insert(wire, (live.clone(), false));
        }

        let token = a.issue_token(101, 303, 202, 1);
        assert_eq!(a.token_wire(token), STATUS_CAPACITY);
        assert_eq!(a.authorize(token, 101), STATUS_OK);
    }

    #[test]
    fn request_id_tombstones_are_bounded_and_release_clears_them() {
        let mut a = authority_with_resource();
        for id in 1..=MAX_REQUEST_IDS as i64 {
            a.request_ids.insert(
                id,
                RequestIdTombstone {
                    resource: 101,
                    incarnation: 1,
                    epoch: 1,
                },
            );
        }
        let token = a.issue_token(101, 303, 202, 1);
        assert_eq!(
            a.open_request(token, MAX_REQUEST_IDS as i64 + 1, 11),
            STATUS_CAPACITY
        );

        assert_eq!(a.release_resource(101), STATUS_OK);
        assert!(a.request_ids.is_empty());
        assert_eq!(a.introduce(101, 202, 303, 1), 101);
        let token = a.issue_token(101, 303, 202, 1);
        assert!(a.open_request(token, 7, 11) > 0);
    }

    #[test]
    fn retiring_an_epoch_drops_old_handles_and_frees_request_ids() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let unsent = a.open_request(token, 7, 11);
        assert!(unsent > 0);

        let token = a.issue_token(101, 303, 202, 1);
        let sent = a.open_request(token, 8, 12);
        assert_eq!(a.submit(sent), STATUS_OK);

        let token = a.issue_token(101, 303, 202, 1);
        let finished = a.open_request(token, 9, 13);
        assert_eq!(a.submit(finished), STATUS_OK);
        assert_eq!(a.respond(9, 13, 1), STATUS_OK);
        let receipt = a.wait(finished);
        assert_eq!(a.receipt_code(receipt), RESULT_APPLIED);

        assert_eq!(a.retire(101, 2), STATUS_OK);
        assert_eq!(a.receipt_code(receipt), STATUS_NO_REQUEST);
        assert_eq!(a.submit(unsent), STATUS_NO_REQUEST);
        assert_eq!(a.retry(sent), STATUS_NO_REQUEST);
        assert_eq!(a.cancel_wait(sent), STATUS_NO_REQUEST);
        assert_eq!(a.wait(sent), STATUS_NO_REQUEST);
        assert_eq!(a.respond(8, 12, 1), STATUS_NO_REQUEST);

        // The new epoch starts with no ids or handles from the old one.
        let token = a.issue_token(101, 303, 202, 2);
        assert!(token > 0);
        assert!(a.open_request(token, 7, 11) > 0);
    }

    #[test]
    fn a_request_id_names_one_action_while_its_incarnation_lives() {
        let mut a = authority_with_resource();
        let token = a.issue_token(101, 303, 202, 1);
        let request = a.open_request(token, 7, 11);
        assert!(request > 0);

        let duplicate = a.issue_token(101, 303, 202, 1);
        assert_eq!(a.open_request(duplicate, 7, 11), STATUS_DUPLICATE);

        assert_eq!(a.submit(request), STATUS_OK);
        assert_eq!(a.respond(7, 11, 1), STATUS_OK);
        let receipt = a.wait(request);
        assert!(receipt > 0);

        // The local request is gone, but a replay of its id is still the same
        // action, not a new one.
        let replay = a.issue_token(101, 303, 202, 1);
        assert_eq!(a.open_request(replay, 7, 11), STATUS_DUPLICATE);
    }

    #[test]
    fn a_request_id_cannot_be_active_on_two_resources_at_once() {
        let mut a = authority_with_resource();
        assert_eq!(a.introduce(102, 202, 303, 1), 102);
        let first = a.issue_token(101, 303, 202, 1);
        assert!(a.open_request(first, 7, 11) > 0);
        let second = a.issue_token(102, 303, 202, 1);
        assert_eq!(a.open_request(second, 7, 11), STATUS_DUPLICATE);
    }
}
