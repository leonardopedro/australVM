//! Library surface for `umbrella`.
//!
//! `main.rs` is the driver; the project schema and the policy preflight live
//! here so integration tests can exercise them without spawning the whole
//! pipeline (which needs the stock `austral` front-end).

pub mod adapters;
pub mod cache;
pub mod cargo;
pub mod cli;
pub mod frontend;
pub mod handles;
pub mod interface;
pub mod policy;
pub mod process;
pub mod project;
pub mod provider;
pub mod staging;
pub mod trust;
