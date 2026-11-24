//! D16 differential reference for `RequestState.aui`/`.aum`.
//!
//! This is intentionally a separate implementation, not a translation of the
//! generated C. `scripts/check-request-state` compiles both this file and the
//! Austral module, runs the same fixed vectors, and diffs stdout. The compiler
//! remains a trust boundary: this checks correspondence of the emitted
//! transition, not compiler correctness.
//!
//! Build/run:
//!   rustc --edition 2021 examples/distributed/RequestStateReference.rs -o /tmp/rs
//!   /tmp/rs

#[derive(Clone, Copy)]
enum State {
    Idle,
    Pending { held_request: i64, held_digest: i64 },
    Applied { held_request: i64, held_digest: i64 },
    Rejected { held_request: i64, held_digest: i64 },
}

#[derive(Clone, Copy)]
enum Event {
    Submit { request: i64, digest: i64 },
    MatchingResponse { request: i64, digest: i64 },
    Retry,
    CancelWait,
}

struct Transition {
    next: State,
    code: i32,
}

fn step(state: State, event: Event) -> Transition {
    match state {
        State::Idle => match event {
            Event::Submit { request, digest } => Transition {
                next: State::Pending {
                    held_request: request,
                    held_digest: digest,
                },
                code: 1,
            },
            _ => Transition {
                next: State::Idle,
                code: 4,
            },
        },
        State::Pending {
            held_request,
            held_digest,
        } => match event {
            Event::Submit { .. } => Transition {
                next: State::Pending {
                    held_request,
                    held_digest,
                },
                code: 4,
            },
            Event::MatchingResponse { request, digest } => {
                if request != held_request {
                    Transition {
                        next: State::Pending {
                            held_request,
                            held_digest,
                        },
                        code: 4,
                    }
                } else if digest != held_digest {
                    Transition {
                        next: State::Rejected {
                            held_request,
                            held_digest,
                        },
                        code: 3,
                    }
                } else {
                    Transition {
                        next: State::Applied {
                            held_request,
                            held_digest,
                        },
                        code: 2,
                    }
                }
            }
            Event::Retry | Event::CancelWait => Transition {
                next: State::Pending {
                    held_request,
                    held_digest,
                },
                code: 1,
            },
        },
        State::Applied {
            held_request,
            held_digest,
        } => match event {
            Event::Submit { .. } => Transition {
                next: State::Applied {
                    held_request,
                    held_digest,
                },
                code: 4,
            },
            _ => Transition {
                next: State::Applied {
                    held_request,
                    held_digest,
                },
                code: 2,
            },
        },
        State::Rejected {
            held_request,
            held_digest,
        } => Transition {
            next: State::Rejected {
                held_request,
                held_digest,
            },
            code: 3,
        },
    }
}

fn tag(state: State) -> i32 {
    match state {
        State::Idle => 0,
        State::Pending { .. } => 1,
        State::Applied { .. } => 2,
        State::Rejected { .. } => 3,
    }
}

fn report(label: &str, transition: Transition) {
    println!(
        "{} state={} code={}",
        label,
        tag(transition.next),
        transition.code
    );
}

fn main() {
    let pending = State::Pending {
        held_request: 7,
        held_digest: 11,
    };
    let applied = State::Applied {
        held_request: 7,
        held_digest: 11,
    };
    let rejected = State::Rejected {
        held_request: 7,
        held_digest: 11,
    };
    let bound = State::Pending {
        held_request: i64::MAX,
        held_digest: i64::MAX,
    };
    // A restarted process rebuilds this from durable identity and digest only.
    let restored = State::Pending {
        held_request: 7,
        held_digest: 11,
    };

    report(
        "idle-submit",
        step(
            State::Idle,
            Event::Submit {
                request: 7,
                digest: 11,
            },
        ),
    );
    report("pending-retry", step(pending, Event::Retry));
    report("pending-cancel-wait", step(pending, Event::CancelWait));
    report(
        "pending-wrong-request",
        step(
            pending,
            Event::MatchingResponse {
                request: 8,
                digest: 11,
            },
        ),
    );
    report(
        "pending-wrong-digest",
        step(
            pending,
            Event::MatchingResponse {
                request: 7,
                digest: 12,
            },
        ),
    );
    report(
        "pending-matching",
        step(
            pending,
            Event::MatchingResponse {
                request: 7,
                digest: 11,
            },
        ),
    );
    report(
        "restart-resume",
        step(
            restored,
            Event::MatchingResponse {
                request: 7,
                digest: 11,
            },
        ),
    );
    report(
        "applied-repeat",
        step(
            applied,
            Event::MatchingResponse {
                request: 7,
                digest: 11,
            },
        ),
    );
    report(
        "applied-submit",
        step(
            applied,
            Event::Submit {
                request: 9,
                digest: 13,
            },
        ),
    );
    report(
        "rejected-matching",
        step(
            rejected,
            Event::MatchingResponse {
                request: 7,
                digest: 11,
            },
        ),
    );
    report(
        "idle-response",
        step(
            State::Idle,
            Event::MatchingResponse {
                request: 7,
                digest: 11,
            },
        ),
    );
    report(
        "bound-matching",
        step(
            bound,
            Event::MatchingResponse {
                request: i64::MAX,
                digest: i64::MAX,
            },
        ),
    );
}
