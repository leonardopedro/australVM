//! Independent checker and reference evaluator for Austral GPU bundles.
//!
//! The default feature set is CPU-only: strict bundle parsing, canonical IR
//! digest reproduction, Naga validation of the emitted WGSL, and evaluation of
//! the validated KernelIR. `device` is reserved for wgpu dispatch (G09/G10).

pub mod artifact;
#[cfg(feature = "device")]
pub mod device;
pub mod reference;
pub mod run;
pub mod schema;
#[cfg(feature = "service")]
pub mod service;
pub mod validate;
pub mod wrapper;

#[cfg(test)]
mod tests;

pub use artifact::{load_bundle, LoadedBundle};
pub use schema::KernelBundle;
