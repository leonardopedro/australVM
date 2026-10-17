//! `avm-exec` — the hermetic executor (spec §2 topology, Sprint 3).
//!
//! * [`cell_builder`] — serialize/verify relocatable `.avm` cells in the
//!   exact §3.4 layout (AVMC magic, drv hash, Cedar capability table,
//!   page-aligned text, relocation table, Ed25519 signature).
//! * [`runner`] — Cedar-gated, `unshare`-isolated spawning of builder
//!   processes (task 3.1): a request without the needed capability fails
//!   with a genuine `EPERM` *before* any process exists, and a spawned
//!   builder runs without a usable network namespace.
//! * [`linker`] — task 3.3: resolve a cell's relocations against provider
//!   cells *mapped by the ephemeral hot store*, by their real addresses.
//!
//! Spec §6.3 for this workspace: `deny(unsafe_op_in_unsafe_fn)` everywhere
//! and no `unsafe` at all in `avm-cas`/`avm-eval`/`avm-exec`/`avm-cli`
//! beyond `hot_store`'s sanctioned mmap call.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod cell_builder;
pub mod linker;
pub mod runner;

pub use cell_builder::{
    build_cell, parse_avce, AvceDoc, AvceFn, AvceReloc, BuildCell, CellError, CellView, SymDef,
};
pub use linker::{link, LinkedImage, Provider};
pub use runner::{Isolation, Runner, RunnerError, Sandbox};
