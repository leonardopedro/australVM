//! Independent checker and reference evaluator for Austral GPU bundles.
//!
//! The default feature set is CPU-only: strict bundle parsing, canonical IR
//! digest reproduction, Naga validation of the emitted WGSL, and evaluation of
//! the validated KernelIR. `device` is reserved for wgpu dispatch (G09/G10).

pub mod artifact;
#[cfg(feature = "device")]
pub mod device;
pub mod nvlaunch;
pub mod reduce;
pub mod reduce_artifact;
pub mod reduce_audit;
pub mod reduce_run;
pub mod reference;
pub mod run;
pub mod schema;
pub mod target;
#[cfg(feature = "service")]
pub mod coordinator;
#[cfg(feature = "service")]
pub mod service;
pub mod validate;
pub mod wgsl_reflect;
pub mod wrapper;

#[cfg(test)]
mod tests;

pub use artifact::{load_bundle, LoadedBundle};
pub use reduce_artifact::{load_reduce, LoadedReduction};
pub use schema::KernelBundle;
