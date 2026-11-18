//! `avm-exec` — the hermetic executor (spec §2 topology, Sprint 3).
//!
//! * [`cell_builder`] — serialize/verify relocatable `.avm` cells in the
//!   exact §3.4 layout (AVMC magic, drv hash, Cedar capability table,
//!   page-aligned text, relocation table, Ed25519 signature).
//! * [`runner`] — Cedar-gated, `unshare`-isolated spawning of builder
//!   processes (task 3.1): a request without the needed capability fails
//!   with a genuine `EPERM` *before* any process exists, and a spawned
//!   builder runs without a usable network namespace. Its live caller is
//!   `avm-pkg build --policy`, which judges a cell's capability table and
//!   spawns the compiler/`cps2cell` through it (PLAN_avm_pkg.md §4.13).
//! * [`linker`] — task 3.3: resolve a cell's relocations against provider
//!   cells *mapped by the ephemeral hot store*, by their real addresses.
//! * [`loader`] — gate a cell's own calls (`Execute`, then one `Call:<symbol>`
//!   per declared import) against its own capability table, link the image,
//!   and run it through the runner. Its live caller is `avm-pkg load`.
//!
//! Spec §6.3 for this workspace: `deny(unsafe_op_in_unsafe_fn)` everywhere
//! and no `unsafe` at all in `avm-cas`/`avm-eval`/`avm-exec`/`avm-cli` —
//! `hot_store` assembles cells in an anonymous mapping through `memmap2`'s
//! safe `map_anon`/`make_read_only` APIs, so there is no mmap `unsafe`
//! block left to sanction.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod cell_builder;
pub mod linker;
pub mod loader;
pub mod runner;

pub use cell_builder::{
    build_cell, parse_avce, AvceDoc, AvceFn, AvceReloc, BuildCell, CellError, CellView, SymDef,
};
pub use linker::{link, link_with, LinkedImage, Provider};
pub use loader::{load, render_image, run_image, LoadError, LoadedImage};
pub use runner::{Isolation, Runner, RunnerError, Sandbox};
