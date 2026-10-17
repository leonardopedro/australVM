//! `avm-cas` — the avm-pkg dual-tier storage engine (PLAN_avm_pkg.md §S1).
//!
//! * **Cold store** ([`engine::ChunkStore`]): append-only packfiles of
//!   LZ4-compressed, content-defined chunks, addressed by unkeyed BLAKE3 of
//!   the *plaintext* chunk data, with Ed25519-signed manifests.
//! * **Hot store** ([`hot_store::HotStore`]): an ephemeral, bounded,
//!   mmap-backed page cache for active cells. Eviction is zero-cost: clean
//!   mapping pages are dropped with no writeback, because every cell is
//!   reproducibly re-constructible from the cold store.
//!
//! Engineering directives honored here (spec §6): the whole subsystem is
//! `#![deny(unsafe_op_in_unsafe_fn)]`, and in fact contains *no* `unsafe`
//! blocks at all — `memmap2`'s safe wrappers cover every `mmap` call, and
//! hashing/serialization are pure safe Rust. All identity digests are
//! computed over explicitly sorted or fixed-order encodings; nothing is
//! ever hashed from an unordered container's iteration order.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod chunker;
pub mod compression;
pub mod crypto;
pub mod engine;
pub mod hot_store;

pub use chunker::{chunk_payload, Chunk, ChunkerConfig};
pub use engine::{ChunkStore, Digest, Manifest, GcReport};
pub use hot_store::{HotStats, HotStore};

use std::fmt;

/// 32-byte BLAKE3 digest, the store's universal address type.
pub type Blake3Digest = [u8; 32];

/// Unified error type for the storage subsystem. Hand-rolled (no deps) so
/// the crate stays inside the offline-cached dependency set.
#[derive(Debug)]
pub enum CasError {
    Io(std::io::Error),
    NotFound(String),
    Corrupt(String),
    Signature(String),
    Capacity(String),
    Codec(u8),
}

impl fmt::Display for CasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CasError::Io(e) => write!(f, "i/o error: {e}"),
            CasError::NotFound(what) => write!(f, "not found: {what}"),
            CasError::Corrupt(what) => write!(f, "corrupt store data: {what}"),
            CasError::Signature(what) => write!(f, "signature check failed: {what}"),
            CasError::Capacity(what) => write!(f, "capacity error: {what}"),
            CasError::Codec(c) => write!(f, "unknown compression codec id {c}"),
        }
    }
}

impl std::error::Error for CasError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CasError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CasError {
    fn from(e: std::io::Error) -> Self {
        CasError::Io(e)
    }
}
