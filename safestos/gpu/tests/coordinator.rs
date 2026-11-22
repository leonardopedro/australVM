//! G23a: bounded placement across independent devices, with mock devices.
//!
//! Nothing here runs a kernel. The devices are mock workers whose job closures
//! the test holds or releases, which is what makes "the second device progresses
//! while the first is held" an observation instead of a timing hope. The mock
//! identity strings are labelled as mock so a reader cannot mistake this file
//! for a hardware record; G23b replaces them with a selected adapter.

#![cfg(feature = "service")]

use avm_gpu::coordinator::{Budget, Coordinator, Identity, Outcome};
use avm_gpu::service::{BUSY, COMPLETE, DEVICE_LOST, LIMIT_REFUSED, PENDING, STALE, UNKNOWN_HANDLE};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn mock_identity(name: &str) -> Identity {
    Identity {
        backend: "mock".to_string(),
        name: name.to_string(),
        // Deliberately not a physical classification: a mock device is never a
        // hardware record, and `is_physical` must say so.
        device_type: "mock".to_string(),
    }
}

fn budget(max_in_flight: usize, max_bytes: usize) -> Budget {
    Budget {
        max_in_flight,
        max_bytes,
    }
}

/// Wait for `job` to leave PENDING, bounded so a stuck coordinator fails the
/// test instead of hanging it.
fn wait_terminal(coordinator: &Arc<Coordinator>, context: i64, job: i64) -> i64 {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = coordinator.poll(context, job);
        if state != PENDING {
            return state;
        }
        assert!(Instant::now() < deadline, "job {job} never became terminal");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_mock_device_is_not_a_physical_device() {
    assert!(!mock_identity("mock-gpu").is_physical());
    let physical = Identity {
        backend: "vulkan".to_string(),
        name: "reported adapter".to_string(),
        device_type: "physical".to_string(),
    };
    assert!(physical.is_physical());
}

#[test]
fn the_second_device_completes_while_the_first_is_held() {
    let coordinator = Coordinator::new();
    let first = coordinator.add_device(mock_identity("mock-a"), budget(4, 4096));
    let second = coordinator.add_device(mock_identity("mock-b"), budget(4, 4096));
    assert_eq!(coordinator.device_count(), 2);

    let first_context = 0x100;
    let second_context = 0x200;
    let first_generation = coordinator.open_context(first_context, first);
    let second_generation = coordinator.open_context(second_context, second);
    assert!(first_generation > 0 && second_generation > 0);

    let (release, held) = channel::<()>();
    let held_job = coordinator.submit(first_context, first_generation, 64, move || {
        // The driver wait lives in the worker, not in the coordinator: this
        // thread is blocked inside the closure until the test releases it.
        held.recv().expect("release the held job");
        Outcome::Complete
    });
    assert!(held_job > 0);
    assert_eq!(
        coordinator.poll(first_context, held_job),
        PENDING,
        "the held job must not have completed yet"
    );

    // The other device's worker is independent of the blocked one.
    let free_job = coordinator
        .submit(second_context, second_generation, 64, || Outcome::Complete);
    assert!(free_job > 0 && free_job != held_job);
    assert_eq!(wait_terminal(&coordinator, second_context, free_job), COMPLETE);
    assert_eq!(
        coordinator.poll(first_context, held_job),
        PENDING,
        "the free device's progress must not have finished the held job"
    );

    release.send(()).unwrap();
    assert_eq!(wait_terminal(&coordinator, first_context, held_job), COMPLETE);
    assert_eq!(coordinator.result(second_context, free_job), 0);
    assert_eq!(coordinator.result(first_context, held_job), 0);
    assert_eq!(coordinator.teardown(first), 0);
    assert_eq!(coordinator.teardown(second), 0);
}

#[test]
fn an_over_budget_submission_is_refused_before_it_runs() {
    let coordinator = Coordinator::new();
    let device = coordinator.add_device(mock_identity("mock-budget"), budget(1, 64));
    let context = 7;
    let generation = coordinator.open_context(context, device);

    let (release, held) = channel::<()>();
    let first = coordinator.submit(context, generation, 16, move || {
        held.recv().expect("release");
        Outcome::Complete
    });
    assert!(first > 0);
    assert_eq!(coordinator.reserved(device), Some((1, 16)));

    // The queue limit is one job: a second is refused with BUSY and never runs.
    let queued = coordinator.submit(context, generation, 16, || Outcome::Complete);
    assert_eq!(queued, BUSY);
    assert_eq!(coordinator.reserved(device), Some((1, 16)));

    release.send(()).unwrap();
    assert_eq!(wait_terminal(&coordinator, context, first), COMPLETE);
    // Capacity comes back on the terminal outcome, so the same budget admits
    // again.
    assert_eq!(coordinator.reserved(device), Some((0, 0)));
    let second = coordinator.submit(context, generation, 16, || Outcome::Complete);
    assert!(second > 0);

    // The byte limit is independent of the queue limit: with a bigger queue but
    // 64 bytes, a 48-byte job followed by a 32-byte job is refused on bytes.
    let bytes_device = coordinator.add_device(mock_identity("mock-bytes"), budget(4, 64));
    let bytes_context = 8;
    let bytes_generation = coordinator.open_context(bytes_context, bytes_device);
    let (release_bytes, held_bytes) = channel::<()>();
    let wide = coordinator.submit(bytes_context, bytes_generation, 48, move || {
        held_bytes.recv().expect("release");
        Outcome::Complete
    });
    assert!(wide > 0);
    let narrow = coordinator.submit(bytes_context, bytes_generation, 32, || Outcome::Complete);
    assert_eq!(narrow, LIMIT_REFUSED);
    assert_eq!(coordinator.reserved(bytes_device), Some((1, 48)));
    release_bytes.send(()).unwrap();
    assert_eq!(wait_terminal(&coordinator, bytes_context, wide), COMPLETE);
    assert_eq!(coordinator.reserved(bytes_device), Some((0, 0)));
    assert_eq!(coordinator.teardown(device), 0);
    assert_eq!(coordinator.teardown(bytes_device), 0);
}

#[test]
fn receipts_are_bound_to_their_context_and_generation() {
    let coordinator = Coordinator::new();
    let device = coordinator.add_device(mock_identity("mock-bind"), budget(4, 4096));
    let context = 11;
    let other = 12;
    let generation = coordinator.open_context(context, device);
    let other_generation = coordinator.open_context(other, device);

    let job = coordinator.submit(context, generation, 8, || Outcome::Complete);
    assert!(job > 0);
    assert_eq!(wait_terminal(&coordinator, context, job), COMPLETE);

    // A receipt from another context is not visible through this one.
    assert_eq!(coordinator.poll(other, job), UNKNOWN_HANDLE);
    assert_eq!(coordinator.result(other, job), UNKNOWN_HANDLE);
    // The other context's own generation is live but is not this job's.
    assert_eq!(coordinator.poll(other, job), UNKNOWN_HANDLE);
    let _ = other_generation;

    // Re-opening the handle starts a new generation: the old token is stale even
    // though the handle number is the same.
    let reopened = coordinator.open_context(context, device);
    assert_ne!(reopened, generation);
    assert_eq!(coordinator.poll(context, job), STALE);
    assert_eq!(coordinator.result(context, job), STALE);
    assert_eq!(
        coordinator.submit(context, generation, 8, || Outcome::Complete),
        STALE
    );

    // A fresh job on the new generation is readable, exactly once.
    let fresh = coordinator.submit(context, reopened, 8, || Outcome::Complete);
    assert!(fresh > 0);
    assert_eq!(wait_terminal(&coordinator, context, fresh), COMPLETE);
    assert_eq!(coordinator.result(context, fresh), 0);
    assert_eq!(coordinator.result(context, fresh), STALE);
    assert_eq!(coordinator.teardown(device), 0);
}

#[test]
fn a_lost_workers_reservation_is_charged_until_teardown() {
    let coordinator = Coordinator::new();
    let device = coordinator.add_device(mock_identity("mock-lost"), budget(4, 4096));
    let context = 21;
    let generation = coordinator.open_context(context, device);

    // The mock device dies inside the job: the worker thread panics, so its
    // publication never runs. The panic message is expected output; the test's
    // subject is what the coordinator does with the reservation.
    let job = coordinator.submit(context, generation, 32, || panic!("mock device lost"));
    assert!(job > 0);
    let deadline = Instant::now() + Duration::from_secs(20);
    while coordinator.is_lost(device) != Some(true) {
        assert!(Instant::now() < deadline, "the mock worker never exited");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(coordinator.reserved(device), Some((1, 32)));

    // The job never reached a terminal outcome, and the lost worker's budget is
    // still charged: the device is not silently reused.
    assert_eq!(coordinator.poll(context, job), PENDING);
    assert_eq!(coordinator.reserved(device), Some((1, 32)));
    assert_eq!(
        coordinator.submit(context, generation, 8, || Outcome::Complete),
        DEVICE_LOST
    );
    assert_eq!(coordinator.reserved(device), Some((1, 32)));

    // Teardown is the release point: the budget goes, and the still-pending job
    // is reported as a device loss rather than as a completion.
    assert_eq!(coordinator.teardown(device), 0);
    assert_eq!(coordinator.reserved(device), Some((0, 0)));
    assert_eq!(coordinator.poll(context, job), DEVICE_LOST);
    assert_eq!(coordinator.result(context, job), DEVICE_LOST);
}

#[test]
fn every_terminal_outcome_returns_the_capacity() {
    let coordinator = Coordinator::new();
    let device = coordinator.add_device(mock_identity("mock-terminal"), budget(4, 128));
    let context = 31;
    let generation = coordinator.open_context(context, device);

    let completed = coordinator.submit(context, generation, 16, || Outcome::Complete);
    let cancelled = coordinator.submit(context, generation, 16, || Outcome::Cancelled);
    let failed = coordinator.submit(context, generation, 16, || Outcome::Failed(DEVICE_LOST));
    assert!(completed > 0 && cancelled > 0 && failed > 0);
    assert_eq!(wait_terminal(&coordinator, context, completed), COMPLETE);
    assert_eq!(
        wait_terminal(&coordinator, context, cancelled),
        avm_gpu::service::CANCELLED
    );
    assert_eq!(wait_terminal(&coordinator, context, failed), DEVICE_LOST);
    assert_eq!(coordinator.reserved(device), Some((0, 0)));

    // A cancelled job has no receipt, and a failed job reports its own code.
    assert_eq!(coordinator.result(context, cancelled), STALE);
    assert_eq!(coordinator.result(context, failed), DEVICE_LOST);

    // The full budget is available again.
    let reused = coordinator.submit(context, generation, 128, || Outcome::Complete);
    assert!(reused > 0);
    assert_eq!(wait_terminal(&coordinator, context, reused), COMPLETE);
    assert_eq!(coordinator.teardown(device), 0);
}

#[test]
fn a_missing_device_or_context_is_unknown() {
    let coordinator = Coordinator::new();
    assert_eq!(coordinator.open_context(1, 0), UNKNOWN_HANDLE);
    assert_eq!(
        coordinator.submit(1, 1, 8, || Outcome::Complete),
        UNKNOWN_HANDLE
    );
    assert_eq!(coordinator.identity(0), None);
    assert_eq!(coordinator.reserved(0), None);
    assert_eq!(coordinator.teardown(0), UNKNOWN_HANDLE);
    assert_eq!(Coordinator::new().device_count(), 0);
}
