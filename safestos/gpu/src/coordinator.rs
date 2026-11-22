//! G23a: bounded placement of independent jobs across devices.
//!
//! A coordinator owns **one reusable worker thread per device** and an explicit
//! budget per device: how many jobs may be outstanding, and how many bytes they
//! may reserve. Capacity is reserved when a job is admitted and returned when
//! that job reaches a terminal outcome, so a slow job on one device cannot
//! occupy another device's worker, and an over-budget submission is refused
//! before any work starts.
//!
//! Identities are bound to their context. A context is opened on one device and
//! carries a generation; a submission and a receipt both name that generation,
//! and a receipt is only readable from the context that submitted it. Re-opening
//! a handle starts a new generation, so a stale token cannot read a newer job's
//! placement.
//!
//! Driver waits stay in the worker's closure, never in the coordinator: `submit`
//! only reserves and queues, `poll` is a non-blocking observation, and `teardown`
//! is the one call that joins a device's worker. G23a exercises that with mock
//! devices; G23b replaces the mock identity with a selected adapter. A software
//! device is never reported as physical ([`Identity::is_physical`]), which is how
//! one-device availability stays honest.

use crate::service::{BUSY, CANCELLED, COMPLETE, DEVICE_LOST, LIMIT_REFUSED, PENDING, STALE, UNKNOWN_HANDLE};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

/// A device's recorded identity. `device_type` is the adapter's classification
/// (`physical`, `integrated-gpu`, `cpu`, …) as the backend reported it; the
/// coordinator never guesses it from OS device nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub backend: String,
    pub name: String,
    pub device_type: String,
}

impl Identity {
    /// Only an adapter the backend classified as physical may back a
    /// required-hardware claim. A CPU or software adapter is useful for a CPU
    /// lane and is never a GPU execution record.
    pub fn is_physical(&self) -> bool {
        self.device_type == "physical" || self.device_type == "integrated-gpu"
            || self.device_type == "discrete-gpu"
    }
}

/// Per-device admission limits. Both are enforced before work is queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_in_flight: usize,
    pub max_bytes: usize,
}

/// How a submitted job ended. The worker's closure returns exactly one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    Cancelled,
    Failed(i64),
}

type Work = (i64, Box<dyn FnOnce() -> Outcome + Send>);

struct Submission {
    context: i64,
    generation: i64,
    device: usize,
    bytes: usize,
    /// `None` until the worker publishes the terminal event.
    state: Option<Outcome>,
    result_taken: bool,
}

struct Slot {
    identity: Identity,
    budget: Budget,
    used_bytes: usize,
    in_flight: usize,
    lost: bool,
    /// Dropping this closes the worker's queue; teardown does it to end the loop.
    queue: Option<Sender<Work>>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Inner {
    slots: Vec<Slot>,
    submissions: HashMap<i64, Submission>,
    /// Context handle → (device, generation).
    contexts: HashMap<i64, (usize, i64)>,
    next_job: i64,
    next_generation: i64,
}

/// Bounded placement across independently budgeted devices.
pub struct Coordinator {
    inner: Mutex<Inner>,
}

impl Default for Coordinator {
    fn default() -> Self {
        Coordinator {
            inner: Mutex::new(Inner {
                next_job: 1,
                next_generation: 1,
                ..Inner::default()
            }),
        }
    }
}

impl Coordinator {
    pub fn new() -> Arc<Coordinator> {
        Arc::new(Coordinator::default())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // The lock is never held while a job's closure runs or while a worker is
        // joined, so a panicking job cannot poison it.
        self.inner.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// Add a device and start its one reusable worker. Returns the device index
    /// placement uses.
    pub fn add_device(self: &Arc<Self>, identity: Identity, budget: Budget) -> usize {
        let (queue, jobs) = channel::<Work>();
        let coordinator = Arc::clone(self);
        let worker = std::thread::spawn(move || {
            while let Ok((job, run)) = jobs.recv() {
                let outcome = run();
                coordinator.publish(job, outcome);
            }
        });
        let mut inner = self.lock();
        inner.slots.push(Slot {
            identity,
            budget,
            used_bytes: 0,
            in_flight: 0,
            lost: false,
            queue: Some(queue),
            worker: Some(worker),
        });
        inner.slots.len() - 1
    }

    pub fn device_count(&self) -> usize {
        self.lock().slots.len()
    }

    pub fn identity(&self, device: usize) -> Option<Identity> {
        self.lock().slots.get(device).map(|s| s.identity.clone())
    }

    /// Whether the device's worker has been lost: a job panicked its worker, or
    /// teardown has run. A lost device admits nothing until teardown releases
    /// what it reserved.
    pub fn is_lost(&self, device: usize) -> Option<bool> {
        // Read-only on purpose: an audit must not be what changes the state it
        // reports, or a test could observe its own observation. Admission is
        // what marks a finished worker lost.
        let inner = self.lock();
        let slot = inner.slots.get(device)?;
        Some(
            slot.lost
                || slot
                    .worker
                    .as_ref()
                    .map(|worker| worker.is_finished())
                    .unwrap_or(true),
        )
    }

    /// `(in_flight, reserved_bytes)` for a device. The coordinate an operator
    /// audits: a lost worker's reservation stays visible here until teardown.
    pub fn reserved(&self, device: usize) -> Option<(usize, usize)> {
        self.lock()
            .slots
            .get(device)
            .map(|s| (s.in_flight, s.used_bytes))
    }

    /// Bind a context handle to a device and return its generation. Re-opening
    /// the same handle is the only way to make an older token stale.
    pub fn open_context(&self, context: i64, device: usize) -> i64 {
        let mut inner = self.lock();
        if device >= inner.slots.len() {
            return UNKNOWN_HANDLE;
        }
        let generation = inner.next_generation;
        inner.next_generation += 1;
        inner.contexts.insert(context, (device, generation));
        generation
    }

    /// Reserve capacity on the context's device and queue `run`. Returns the job
    /// identity, or a negative refusal: `UNKNOWN_HANDLE` (no such context or
    /// device), `STALE` (generation), `DEVICE_LOST`, `BUSY` (queue full) or
    /// `LIMIT_REFUSED` (bytes).
    pub fn submit(
        self: &Arc<Self>,
        context: i64,
        generation: i64,
        bytes: usize,
        run: impl FnOnce() -> Outcome + Send + 'static,
    ) -> i64 {
        let mut inner = self.lock();
        let (device, current) = match inner.contexts.get(&context) {
            Some(bound) => *bound,
            None => return UNKNOWN_HANDLE,
        };
        if current != generation {
            return STALE;
        }
        let (budget, used, in_flight, lost, finished) = {
            let slot = match inner.slots.get(device) {
                Some(slot) => slot,
                None => return UNKNOWN_HANDLE,
            };
            (
                slot.budget,
                slot.used_bytes,
                slot.in_flight,
                slot.lost,
                slot.worker
                    .as_ref()
                    .map(|worker| worker.is_finished())
                    .unwrap_or(true),
            )
        };
        if lost || finished {
            // The worker thread is gone (a panicking job, or a backend worker
            // that exited). Its reservation stays charged until teardown, and
            // nothing new is admitted here: pretending otherwise would report a
            // device that cannot run work as live.
            if let Some(slot) = inner.slots.get_mut(device) {
                slot.lost = true;
            }
            return DEVICE_LOST;
        }
        if in_flight >= budget.max_in_flight {
            return BUSY;
        }
        if used.saturating_add(bytes) > budget.max_bytes {
            return LIMIT_REFUSED;
        }
        let job = inner.next_job;
        inner.next_job += 1;
        inner.submissions.insert(
            job,
            Submission {
                context,
                generation,
                device,
                bytes,
                state: None,
                result_taken: false,
            },
        );
        let queued = {
            let slot = &mut inner.slots[device];
            slot.in_flight += 1;
            slot.used_bytes += bytes;
            match slot.queue.as_ref() {
                Some(queue) => queue.send((job, Box::new(run))).is_ok(),
                None => false,
            }
        };
        if !queued {
            let slot = &mut inner.slots[device];
            slot.in_flight -= 1;
            slot.used_bytes -= bytes;
            slot.lost = true;
            inner.submissions.remove(&job);
            return DEVICE_LOST;
        }
        job
    }

    /// The worker's publication point: the first terminal outcome owns the job
    /// and returns its capacity; a later or reordered one is ignored.
    fn publish(&self, job: i64, outcome: Outcome) {
        let mut inner = self.lock();
        let (device, bytes) = match inner.submissions.get_mut(&job) {
            Some(submission) => {
                if submission.state.is_some() {
                    return;
                }
                submission.state = Some(outcome);
                (submission.device, submission.bytes)
            }
            None => return,
        };
        if let Some(slot) = inner.slots.get_mut(device) {
            slot.in_flight = slot.in_flight.saturating_sub(1);
            slot.used_bytes = slot.used_bytes.saturating_sub(bytes);
        }
    }

    /// Bind `job` to `context` and its current generation, or say why not.
    fn bound<'a>(
        &self,
        inner: &'a Inner,
        context: i64,
        job: i64,
    ) -> Result<&'a Submission, i64> {
        let submission = inner.submissions.get(&job).ok_or(UNKNOWN_HANDLE)?;
        if submission.context != context {
            return Err(UNKNOWN_HANDLE);
        }
        match inner.contexts.get(&context) {
            Some((_, generation)) if *generation == submission.generation => Ok(submission),
            // A handle that was re-opened (or closed) belongs to another
            // generation: the old token cannot observe the new placement.
            _ => Err(STALE),
        }
    }

    /// Observe a job without consuming it.
    pub fn poll(&self, context: i64, job: i64) -> i64 {
        let inner = self.lock();
        let submission = match self.bound(&inner, context, job) {
            Ok(submission) => submission,
            Err(code) => return code,
        };
        match submission.state {
            None => PENDING,
            Some(Outcome::Complete) => COMPLETE,
            Some(Outcome::Cancelled) => CANCELLED,
            Some(Outcome::Failed(code)) => code,
        }
    }

    /// Consume a completed receipt once. Returns 0 on the claim, or a refusal:
    /// `BUSY` (not terminal), `STALE` (already claimed, or cancelled/failed
    /// generation), the job's own failure code.
    pub fn result(&self, context: i64, job: i64) -> i64 {
        let mut inner = self.lock();
        {
            let submission = match self.bound(&inner, context, job) {
                Ok(submission) => submission,
                Err(code) => return code,
            };
            match submission.state {
                None => return BUSY,
                Some(Outcome::Cancelled) => return STALE,
                Some(Outcome::Failed(code)) => return code,
                Some(Outcome::Complete) => {}
            }
            if submission.result_taken {
                return STALE;
            }
        }
        match inner.submissions.get_mut(&job) {
            Some(submission) => submission.result_taken = true,
            None => return UNKNOWN_HANDLE,
        }
        0
    }

    /// Stop admitting on a device, end its worker and release what it reserved.
    ///
    /// This is the only call that joins a worker, and it is the point at which a
    /// lost worker's budget stops being charged. A job still pending here is
    /// reported as `DEVICE_LOST`; one that had already completed keeps its
    /// completed receipt.
    pub fn teardown(self: &Arc<Self>, device: usize) -> i64 {
        let (queue, worker) = {
            let mut inner = self.lock();
            let slot = match inner.slots.get_mut(device) {
                Some(slot) => slot,
                None => return UNKNOWN_HANDLE,
            };
            slot.lost = true;
            (slot.queue.take(), slot.worker.take())
        };
        // Closing the queue ends the worker's `recv` loop; joining then covers
        // whatever the last job was doing.
        drop(queue);
        if let Some(worker) = worker {
            let _ = worker.join();
        }
        let mut inner = self.lock();
        if let Some(slot) = inner.slots.get_mut(device) {
            slot.in_flight = 0;
            slot.used_bytes = 0;
        }
        for submission in inner.submissions.values_mut() {
            if submission.device == device && submission.state.is_none() {
                submission.state = Some(Outcome::Failed(DEVICE_LOST));
            }
        }
        0
    }
}
