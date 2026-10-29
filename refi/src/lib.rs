//! ReFi exchange web service library.
//!
//! `main.rs` is the deployment shell (secrets, CLI, listener); everything it
//! drives is here so the whole request path can be exercised without a socket:
//! `service::Service::handle` takes a transport-neutral request and returns a
//! response, and `engine::Engine` owns the exchange and the Austral policy
//! kernel.

pub mod accounts;
pub mod server;
pub mod content;
pub mod engine;
pub mod mm0;
pub mod proofs;
pub mod service;
pub mod site;
pub mod util;
