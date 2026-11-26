//! U16 (bounded start) — call-scoped synchronous callbacks behind checked
//! tokens.
//!
//! A callback crosses two boundaries at once: a native library may keep the
//! function pointer and its context, and the JIT code that handed it over can
//! be retired. This module is the registry that makes the host able to refuse a
//! call *before* it jumps into anything: every callback is registered inside a
//! scope, every call goes through [`CallbackScope::invoke`], and that method
//! checks the token, the scope's lifetime, the provider generation, the
//! registration thread and the reentrancy policy while it still holds only the
//! registry, never the callback's own code.
//!
//! Two lifetimes are modelled:
//!
//! - [`Lifetime::Call`] — the scope belongs to one host call. A token is
//!   one-shot: after its first invocation it is retired, so a library that
//!   retained the token and calls again is refused before the callback runs,
//!   even inside the same scope.
//! - [`Lifetime::Instance`] — the scope belongs to the loaded instance. A token
//!   may be invoked until it is retired explicitly or its generation is
//!   retired, which is what an instance replacement does.
//!
//! **No lock is held across callback code.** `invoke` copies the function
//! pointer and context out under the mutex and drops the guard before calling,
//! so a nested callback can enter the same scope (allowed by
//! [`Reentrancy::Nested`], refused by [`Reentrancy::Forbidden`]) instead of
//! deadlocking. The reentrancy policy, not a lock, is what refuses recursive
//! mutation of a value that a callback already holds.
//!
//! **Panics are statuses, not unwinds across the ABI.** The call is wrapped in
//! `catch_unwind`; [`ErrorPolicy::StatusWord`] reports the panic and leaves the
//! token usable, while [`ErrorPolicy::RetireOnPanic`] additionally retires it so
//! a provider that panics once cannot be called again through that token. A
//! `panic=abort` provider still aborts the process — that is a worker crash,
//! not a catchable status, exactly like the handle wrappers.
//!
//! This is the bounded start: synchronous, call-scoped callbacks. It does not
//! spawn, queue, cross threads on its own, or store a callback for a library to
//! call later; a host that needs that needs an instance-lifetime scope and a
//! retirement hook, which is the next card, not this one.

use std::ffi::c_void;
use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
/// A thread identity that does not initialise Rust's thread-local storage.
///
/// `std::thread::current()` registers a TLS destructor inside the loaded
/// cdylib; after `dlclose` that destructor points at unmapped code and faults
/// the process at teardown (measured: SIGSEGV at library drop, only when the
/// borrow wrapper ran). `pthread_self` is a plain libc call with no such
/// bookkeeping. On a non-unix host the key is `0`, so the affinity check is
/// inert there; the generated wrappers and fixtures are unix builds.
#[cfg(unix)]
fn thread_key() -> usize {
    unsafe { libc::pthread_self() as usize }
}

#[cfg(not(unix))]
fn thread_key() -> usize {
    0
}

/// Version of the callback contract. Bumped when token encoding, lifetime,
/// generation or reentrancy semantics change, so a wrapper generated against
/// one version is never linked against another.
pub const CALLBACK_SCHEMA: u32 = 1;

/// The callback ABI the generated `__with` wrapper hands the registry: the
/// provider's own pointer, and one word back. It matches the wrapper's exported
/// signature, so a module never passes a raw JIT pointer that the host would
/// call directly.
/// `C-unwind`: a callback built with `panic=unwind` must be able to unwind into
/// `invoke`, which catches it and reports a status. With a plain `C` pointer
/// type a panic aborts the process before the catch can see it.
pub type CallbackFn = unsafe extern "C-unwind" fn(*mut c_void) -> i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    /// One host call: a token is one-shot and the caller closes the scope.
    Call,
    /// One loaded instance: tokens may be invoked until retired explicitly or
    /// by generation retirement.
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadAffinity {
    /// Only the thread that opened the scope may invoke its callbacks.
    CallerThread,
    /// Any thread may invoke, for a scope whose context is safe to cross.
    AnyThread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reentrancy {
    /// A callback may not invoke a token of this scope while a call is active.
    Forbidden,
    /// A callback may invoke another token of this scope, to a finite depth
    /// chosen by the host's call graph.
    Nested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorPolicy {
    /// A caught panic is reported to the caller and the token stays usable.
    StatusWord,
    /// A caught panic is reported and the token is retired, so the provider is
    /// not called again after it panicked.
    RetireOnPanic,
}

/// What a callback scope promises, decided by the descriptor that opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackPolicy {
    pub lifetime: Lifetime,
    /// The provider generation this scope belongs to. Retiring the generation
    /// refuses every scope opened at or before it.
    pub generation: u32,
    pub thread: ThreadAffinity,
    pub reentrancy: Reentrancy,
    pub error: ErrorPolicy,
}

impl CallbackPolicy {
    /// The synchronous borrow a generated `__with` renders: one call on the
    /// caller's thread, no reentrancy, a panic is a status word.
    pub const fn call_scoped(generation: u32) -> Self {
        CallbackPolicy {
            lifetime: Lifetime::Call,
            generation,
            thread: ThreadAffinity::CallerThread,
            reentrancy: Reentrancy::Forbidden,
            error: ErrorPolicy::StatusWord,
        }
    }

    /// A callback the host keeps for the life of one instance.
    pub const fn instance(generation: u32) -> Self {
        CallbackPolicy {
            lifetime: Lifetime::Instance,
            generation,
            thread: ThreadAffinity::CallerThread,
            reentrancy: Reentrancy::Nested,
            error: ErrorPolicy::RetireOnPanic,
        }
    }
}

/// An opaque permission to invoke one callback in one scope. Minted only by
/// [`CallbackScope::register`]; a token from another scope, a retired one and
/// one from another generation are all refused by identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackToken {
    id: u64,
    generation: u32,
}

impl CallbackToken {
    /// For a host trampoline that carries the token as two words. Decoding is
    /// not a check: `invoke` still verifies the id, the scope and the
    /// generation, so a fabricated pair can only name a token that does not
    /// exist.
    pub const fn from_words(lo: u64, hi: u64) -> Self {
        CallbackToken {
            id: lo,
            generation: hi as u32,
        }
    }

    pub const fn words(self) -> (u64, u64) {
        (self.id, self.generation as u64)
    }

    pub const fn id(self) -> u64 {
        self.id
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackError {
    /// No callback with this id is registered in this scope.
    UnknownToken { id: u64 },
    /// The token was retired: already invoked (call lifetime), retired by the
    /// host, or retired after a panic.
    Retired { id: u64 },
    /// The token, or the scope, belongs to a retired provider generation.
    StaleGeneration { expected: u32, found: u32 },
    /// The scope is call-affine and this is not the registration thread.
    WrongThread,
    /// A reentrancy-forbidding scope was asked to re-enter itself.
    Reentrant { id: u64 },
    /// The scope is closed; no callback of it may run.
    ScopeClosed,
    /// A null callback was registered.
    NullCallback,
    /// The callback unwound and the policy turned that into a status.
    Panicked,
}

impl fmt::Display for CallbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallbackError::UnknownToken { id } => write!(
                f,
                "callback token {id} is not registered in this scope (it belongs to \
                 another scope, or was never issued)"
            ),
            CallbackError::Retired { id } => write!(
                f,
                "callback token {id} is retired: a call-lifetime token is one-shot, and \
                 a retired or panicked token cannot be invoked again"
            ),
            CallbackError::StaleGeneration { expected, found } => write!(
                f,
                "callback belongs to provider generation {found}, generation {expected} \
                 was retired; the JIT code that issued it is gone"
            ),
            CallbackError::WrongThread => write!(
                f,
                "callback scope is call-affine: it was opened on another thread and may \
                 only be invoked there"
            ),
            CallbackError::Reentrant { id } => write!(
                f,
                "callback token {id} would re-enter a scope whose policy forbids \
                 reentrancy while a callback is active"
            ),
            CallbackError::ScopeClosed => {
                write!(f, "callback scope is closed: no callback of it may run")
            }
            CallbackError::NullCallback => {
                write!(f, "a null callback cannot be registered")
            }
            CallbackError::Panicked => write!(
                f,
                "the callback panicked; the wrapper caught it before it crossed the ABI"
            ),
        }
    }
}

impl std::error::Error for CallbackError {}

struct Slot {
    id: u64,
    callback: CallbackFn,
    context: usize,
    retired: bool,
}

struct ScopeState {
    slots: Vec<Slot>,
    /// Calls currently between the token check and the return of the callback.
    active: usize,
    calls: u64,
    closed: bool,
}

/// One scope's callbacks. The mutex guards only the slot table; it is never
/// held while a callback runs.
pub struct CallbackScope {
    generation: u32,
    policy: CallbackPolicy,
    opened_on: usize,
    next_id: Arc<AtomicU64>,
    retired_through: Arc<AtomicU32>,
    state: Mutex<ScopeState>,
}

impl CallbackScope {
    pub fn policy(&self) -> CallbackPolicy {
        self.policy
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Register one callback for this scope. The context pointer is the
    /// caller's: a call-lifetime scope is closed before it returns, so a stack
    /// context is sound; an instance-lifetime scope's context must outlive the
    /// scope, and the host owns that decision, not this registry.
    pub fn register(
        &self,
        callback: CallbackFn,
        context: *mut c_void,
    ) -> Result<CallbackToken, CallbackError> {
        if callback as usize == 0 {
            return Err(CallbackError::NullCallback);
        }
        let mut state = self.lock();
        if state.closed {
            return Err(CallbackError::ScopeClosed);
        }
        if self.generation <= self.retired_through.load(Ordering::SeqCst) {
            return Err(CallbackError::StaleGeneration {
                expected: self.retired_through.load(Ordering::SeqCst),
                found: self.generation,
            });
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        state.slots.push(Slot {
            id,
            callback,
            context: context as usize,
            retired: false,
        });
        Ok(CallbackToken {
            id,
            generation: self.generation,
        })
    }

    /// The one call seam. Every check happens before the function pointer is
    /// used, and the lock is dropped before the callback runs.
    pub fn invoke(&self, token: CallbackToken) -> Result<i64, CallbackError> {
        if token.id == 0 {
            // The null token is "no callback", not a generation claim.
            return Err(CallbackError::UnknownToken { id: 0 });
        }
        let retired_through = self.retired_through.load(Ordering::SeqCst);
        if self.generation <= retired_through {
            return Err(CallbackError::StaleGeneration {
                expected: retired_through,
                found: self.generation,
            });
        }
        if token.generation != self.generation {
            return Err(CallbackError::StaleGeneration {
                expected: self.generation,
                found: token.generation,
            });
        }
        let (callback, context) = {
            let mut state = self.lock();
            if state.closed {
                return Err(CallbackError::ScopeClosed);
            }
            let index = match state.slots.iter().position(|slot| slot.id == token.id) {
                Some(index) => index,
                None => return Err(CallbackError::UnknownToken { id: token.id }),
            };
            if state.slots[index].retired {
                return Err(CallbackError::Retired { id: token.id });
            }
            if self.policy.thread == ThreadAffinity::CallerThread && thread_key() != self.opened_on
            {
                return Err(CallbackError::WrongThread);
            }
            if self.policy.reentrancy == Reentrancy::Forbidden && state.active > 0 {
                return Err(CallbackError::Reentrant { id: token.id });
            }
            state.active += 1;
            state.calls += 1;
            (
                state.slots[index].callback,
                state.slots[index].context as *mut c_void,
            )
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            callback(context)
        }));
        let mut state = self.lock();
        state.active -= 1;
        if self.policy.lifetime == Lifetime::Call {
            if let Some(slot) = state.slots.iter_mut().find(|slot| slot.id == token.id) {
                slot.retired = true;
            }
        }
        match outcome {
            Ok(value) => Ok(value),
            Err(_) => {
                if self.policy.error == ErrorPolicy::RetireOnPanic {
                    if let Some(slot) = state.slots.iter_mut().find(|slot| slot.id == token.id) {
                        slot.retired = true;
                    }
                }
                Err(CallbackError::Panicked)
            }
        }
    }

    /// Retire one token: a retained token reaches this check and is refused
    /// before the callback runs.
    pub fn retire(&self, token: CallbackToken) -> Result<(), CallbackError> {
        let mut state = self.lock();
        match state.slots.iter_mut().find(|slot| slot.id == token.id) {
            Some(slot) => {
                slot.retired = true;
                Ok(())
            }
            None => Err(CallbackError::UnknownToken { id: token.id }),
        }
    }

    /// End the scope. Every token of it is refused from here on, and the
    /// registered callbacks and their contexts are dropped.
    pub fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.slots.clear();
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// Registered callbacks not yet retired: the cleanup count a test compares
    /// against the callbacks a scope was given.
    pub fn live_callbacks(&self) -> usize {
        self.lock()
            .slots
            .iter()
            .filter(|slot| !slot.retired)
            .count()
    }

    /// Calls that reached the callback (each beginning a scope-active span).
    pub fn calls(&self) -> u64 {
        self.lock().calls
    }

    /// Calls inside a callback right now: 0 between calls, which is what makes
    /// a leaked depth observable.
    pub fn active(&self) -> usize {
        self.lock().active
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ScopeState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Drop for CallbackScope {
    fn drop(&mut self) {
        self.close();
    }
}

/// The process-wide callback bookkeeping: token and scope numbering, and the
/// one retirement watermark that outlives any single scope.
pub struct CallbackRegistry {
    next_scope: AtomicU64,
    next_id: Arc<AtomicU64>,
    retired_through: Arc<AtomicU32>,
}

impl Default for CallbackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CallbackRegistry {
    pub fn new() -> Self {
        CallbackRegistry {
            // Scope and token ids start at 1: `(0, 0)` is the null token, so no
            // live token encodes to it.
            next_scope: AtomicU64::new(1),
            next_id: Arc::new(AtomicU64::new(1)),
            retired_through: Arc::new(AtomicU32::new(0)),
        }
    }

    /// The one registry the generated wrappers share, so a token's identity is
    /// process-wide and a generation retirement reaches every scope.
    pub fn global() -> &'static CallbackRegistry {
        static REGISTRY: OnceLock<CallbackRegistry> = OnceLock::new();
        REGISTRY.get_or_init(CallbackRegistry::new)
    }

    pub fn open(&self, policy: CallbackPolicy) -> Result<CallbackScope, CallbackError> {
        let retired_through = self.retired_through.load(Ordering::SeqCst);
        if policy.generation <= retired_through {
            return Err(CallbackError::StaleGeneration {
                expected: retired_through,
                found: policy.generation,
            });
        }
        // Scope ids are bookkeeping, never an authority: tokens carry their
        // scope's identity through the slot table they are registered in.
        let _scope = self.next_scope.fetch_add(1, Ordering::SeqCst);
        Ok(CallbackScope {
            generation: policy.generation,
            policy,
            opened_on: thread_key(),
            next_id: Arc::clone(&self.next_id),
            retired_through: Arc::clone(&self.retired_through),
            state: Mutex::new(ScopeState {
                slots: Vec::new(),
                active: 0,
                calls: 0,
                closed: false,
            }),
        })
    }

    /// Retire every generation at or below `generation`. Called when the JIT
    /// code that issued the callbacks is replaced; every scope opened at or
    /// before it refuses before a jump.
    pub fn retire_generations_through(&self, generation: u32) {
        self.retired_through.fetch_max(generation, Ordering::SeqCst);
    }

    pub fn retired_through(&self) -> u32 {
        self.retired_through.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex as StdMutex;

    // Per-thread, because the test harness runs each test on its own thread:
    // a process-wide counter would make one test's calls visible to another.
    thread_local! {
        static CALLS: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
        static SEEN: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }

    fn calls() -> i64 {
        CALLS.with(|calls| calls.get())
    }

    fn bump() {
        CALLS.with(|calls| calls.set(calls.get() + 1));
    }

    fn seen() -> i64 {
        SEEN.with(|seen| seen.get())
    }

    unsafe extern "C-unwind" fn double(ptr: *mut c_void) -> i64 {
        bump();
        let value = *(ptr as *const i64);
        SEEN.with(|seen| seen.set(value));
        value * 2
    }

    unsafe extern "C-unwind" fn boom(_ptr: *mut c_void) -> i64 {
        bump();
        panic!("provider callback failed");
    }

    fn policy(lifetime: Lifetime) -> CallbackPolicy {
        CallbackPolicy {
            lifetime,
            generation: 1,
            thread: ThreadAffinity::CallerThread,
            reentrancy: Reentrancy::Forbidden,
            error: ErrorPolicy::StatusWord,
        }
    }

    fn reset() {
        CALLS.with(|calls| calls.set(0));
        SEEN.with(|seen| seen.set(0));
    }

    #[test]
    fn the_callback_schema_is_versioned() {
        assert_eq!(CALLBACK_SCHEMA, 1);
    }

    #[test]
    fn a_callback_matches_the_direct_oracle() {
        reset();
        let registry = CallbackRegistry::new();
        let scope = registry
            .open(policy(Lifetime::Call))
            .expect("open a call scope");
        let value = 21i64;
        let token = scope
            .register(double, &value as *const i64 as *mut c_void)
            .expect("register");

        assert_eq!(scope.invoke(token), Ok(42));
        assert_eq!(
            unsafe { double(&value as *const i64 as *mut c_void) },
            42,
            "the registry result is the direct call's result"
        );
        assert_eq!(calls(), 2, "one direct, one hosted");
        assert_eq!(seen(), 21, "the argument is the context");
        assert_eq!(scope.calls(), 1);
        assert_eq!(scope.active(), 0, "no call leaked its depth");
    }

    #[test]
    fn a_call_lifetime_token_is_one_shot_and_a_retained_call_is_refused() {
        reset();
        let registry = CallbackRegistry::new();
        let scope = registry
            .open(policy(Lifetime::Call))
            .expect("open a call scope");
        let value = 5i64;
        let token = scope
            .register(double, &value as *const i64 as *mut c_void)
            .expect("register");

        assert_eq!(scope.invoke(token), Ok(10));
        assert_eq!(
            scope.invoke(token),
            Err(CallbackError::Retired { id: token.id() }),
            "a retained token is checked before the callback runs"
        );
        assert_eq!(calls(), 1, "the second call did not jump");
        assert_eq!(
            scope.live_callbacks(),
            0,
            "the one-shot token retired itself"
        );
    }

    #[test]
    fn an_instance_token_survives_until_it_is_retired() {
        reset();
        let registry = CallbackRegistry::new();
        let scope = registry
            .open(policy(Lifetime::Instance))
            .expect("open an instance scope");
        let value = 7i64;
        let token = scope
            .register(double, &value as *const i64 as *mut c_void)
            .expect("register");

        assert_eq!(scope.invoke(token), Ok(14));
        assert_eq!(scope.invoke(token), Ok(14), "an instance token is reusable");
        scope.retire(token).expect("retire");
        assert_eq!(
            scope.invoke(token),
            Err(CallbackError::Retired { id: token.id() }),
            "an explicitly retired token is refused"
        );
        assert_eq!(calls(), 2);
    }

    #[test]
    fn a_token_from_another_scope_and_the_null_pair_are_refused() {
        reset();
        let registry = CallbackRegistry::new();
        let mine = registry
            .open(policy(Lifetime::Instance))
            .expect("open mine");
        let theirs = registry
            .open(policy(Lifetime::Instance))
            .expect("open theirs");
        let value = 3i64;
        let token = theirs
            .register(double, &value as *const i64 as *mut c_void)
            .expect("register in theirs");

        assert_eq!(
            mine.invoke(token),
            Err(CallbackError::UnknownToken { id: token.id() }),
            "a token is only valid in the scope that issued it"
        );
        assert_eq!(
            mine.invoke(CallbackToken::from_words(0, 0)),
            Err(CallbackError::UnknownToken { id: 0 }),
            "the null token names no callback"
        );
        assert_eq!(calls(), 0, "no refusal jumped");
    }

    #[test]
    fn a_retired_generation_refuses_before_the_callback() {
        reset();
        let registry = CallbackRegistry::new();
        let scope = registry.open(policy(Lifetime::Instance)).expect("open");
        let value = 9i64;
        let token = scope
            .register(double, &value as *const i64 as *mut c_void)
            .expect("register");

        registry.retire_generations_through(1);
        assert_eq!(
            scope.invoke(token),
            Err(CallbackError::StaleGeneration {
                expected: 1,
                found: 1
            }),
            "a retired generation is refused before the jump"
        );
        assert_eq!(calls(), 0);
        assert!(matches!(
            registry.open(policy(Lifetime::Instance)),
            Err(CallbackError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn a_call_affine_scope_refuses_another_thread() {
        reset();
        let registry = CallbackRegistry::new();
        let scope = registry.open(policy(Lifetime::Instance)).expect("open");
        let value = 4i64;
        let token = scope
            .register(double, &value as *const i64 as *mut c_void)
            .expect("register");

        let scope = &scope;
        let token_words = token.words();
        std::thread::scope(|s| {
            s.spawn(move || {
                let scope = scope;
                let token = CallbackToken::from_words(token_words.0, token_words.1);
                assert_eq!(scope.invoke(token), Err(CallbackError::WrongThread));
            });
        });
        assert_eq!(calls(), 0, "the hop did not jump");
        assert_eq!(scope.invoke(token), Ok(8), "the owning thread still may");
    }

    #[test]
    fn an_any_thread_scope_allows_the_hop() {
        reset();
        let registry = CallbackRegistry::new();
        let mut policy = policy(Lifetime::Instance);
        policy.thread = ThreadAffinity::AnyThread;
        let scope = registry.open(policy).expect("open");
        let value = 6i64;
        let token = scope
            .register(double, &value as *const i64 as *mut c_void)
            .expect("register");
        let scope = &scope;
        let token_words = token.words();
        std::thread::scope(|s| {
            s.spawn(move || {
                assert_eq!(
                    scope.invoke(CallbackToken::from_words(token_words.0, token_words.1)),
                    Ok(12)
                );
            });
        });
    }

    /// A context that lets a callback re-enter the scope it belongs to.
    struct Reentry {
        scope: *const CallbackScope,
        token: CallbackToken,
        nested_calls: AtomicUsize,
        nested_result: StdMutex<Option<Result<i64, CallbackError>>>,
    }

    unsafe extern "C-unwind" fn reenter(ptr: *mut c_void) -> i64 {
        let reentry = unsafe { &*(ptr as *const Reentry) };
        reentry.nested_calls.fetch_add(1, Ordering::SeqCst);
        let scope = unsafe { &*reentry.scope };
        let result = scope.invoke(reentry.token);
        let mut slot = reentry.nested_result.lock().expect("lock");
        *slot = Some(result);
        -1
    }

    #[test]
    fn a_forbidden_scope_refuses_reentrancy_without_holding_a_lock() {
        reset();
        let registry = CallbackRegistry::new();
        let scope = registry.open(policy(Lifetime::Call)).expect("open");
        let inner = 2i64;
        let token = scope
            .register(double, &inner as *const i64 as *mut c_void)
            .expect("register");
        let reentry = Reentry {
            scope: &scope,
            token,
            nested_calls: AtomicUsize::new(0),
            nested_result: StdMutex::new(None),
        };
        let outer = scope
            .register(reenter, &reentry as *const Reentry as *mut c_void)
            .expect("register outer");

        // If `invoke` held its lock across the callback this would deadlock.
        assert_eq!(scope.invoke(outer), Ok(-1));
        assert_eq!(reentry.nested_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *reentry.nested_result.lock().expect("lock"),
            Some(Err(CallbackError::Reentrant { id: token.id() })),
            "a forbidden scope refuses the nested call before it jumps"
        );
        assert_eq!(scope.active(), 0, "the depth unwound");
    }

    #[test]
    fn a_nested_scope_allows_a_valid_nested_callback() {
        reset();
        let registry = CallbackRegistry::new();
        let mut policy = policy(Lifetime::Instance);
        policy.reentrancy = Reentrancy::Nested;
        let scope = registry.open(policy).expect("open");
        let inner = 2i64;
        let token = scope
            .register(double, &inner as *const i64 as *mut c_void)
            .expect("register");
        let reentry = Reentry {
            scope: &scope,
            token,
            nested_calls: AtomicUsize::new(0),
            nested_result: StdMutex::new(None),
        };
        let outer = scope
            .register(reenter, &reentry as *const Reentry as *mut c_void)
            .expect("register outer");

        assert_eq!(scope.invoke(outer), Ok(-1));
        assert_eq!(
            *reentry.nested_result.lock().expect("lock"),
            Some(Ok(4)),
            "the nested callback sees its own context and returns its own word"
        );
        assert_eq!(scope.calls(), 2);
        assert_eq!(scope.active(), 0);
    }

    #[test]
    fn a_panic_is_a_status_and_the_policy_decides_the_token() {
        reset();
        let registry = CallbackRegistry::new();
        let scope = registry.open(policy(Lifetime::Instance)).expect("open");
        let flag = 0i64;
        let token = scope
            .register(boom, &flag as *const i64 as *mut c_void)
            .expect("register");

        assert_eq!(scope.invoke(token), Err(CallbackError::Panicked));
        assert_eq!(scope.active(), 0, "a panic does not leak the depth");
        assert_eq!(scope.live_callbacks(), 1, "StatusWord keeps the token");

        let mut retiring = policy(Lifetime::Instance);
        retiring.error = ErrorPolicy::RetireOnPanic;
        let scope = registry.open(retiring).expect("open");
        let token = scope
            .register(boom, &flag as *const i64 as *mut c_void)
            .expect("register");
        assert_eq!(scope.invoke(token), Err(CallbackError::Panicked));
        assert_eq!(
            scope.invoke(token),
            Err(CallbackError::Retired { id: token.id() }),
            "RetireOnPanic refuses the provider a second time"
        );
        assert_eq!(calls(), 2, "the retired call did not jump");
    }

    #[test]
    fn closing_a_scope_refuses_every_token_and_cleans_up() {
        reset();
        let registry = CallbackRegistry::new();
        let scope = registry.open(policy(Lifetime::Instance)).expect("open");
        let value = 1i64;
        let token = scope
            .register(double, &value as *const i64 as *mut c_void)
            .expect("register");

        scope.close();
        assert!(scope.is_closed());
        assert_eq!(
            scope.live_callbacks(),
            0,
            "closing drops every registration"
        );
        assert_eq!(
            scope.invoke(token),
            Err(CallbackError::ScopeClosed),
            "a closed scope refuses before the jump"
        );
        assert_eq!(calls(), 0);
    }
}
