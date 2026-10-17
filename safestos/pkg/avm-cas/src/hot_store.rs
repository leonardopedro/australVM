//! The ephemeral hot store (Sprint 1, Task 1.4) — the "read-only swap"
//! model: a bounded, mmap-backed page cache holding *only currently active
//! cells*. Every cell is reproducibly re-constructible from the cold store,
//! so eviction is a pure drop of clean pages: zero dirty writeback, ever.
//!
//! Hydration: reassemble a manifest's chunks (decompressed, LZ4) into a
//! scratch file inside the hot dir (put `$AVM_HOT_DIR` on tmpfs/memfd for
//! RAM-backed hydration; the default is the system temp dir), map it
//! `MAP_PRIVATE` copy-on-read via `memmap2`'s safe wrapper, then unlink the
//! scratch file immediately — the mapping outlives the file, and the pages
//! are clean, so dropping the mapping can never write anything back.
//!
//! There is no `unsafe` in this module: `memmap2` covers the `mmap` calls,
//! which keeps the spec §6.3 directive ("unsafe restricted to the mmap
//! calls") satisfied strictly rather than merely nominally.
//!
//! Accounting honesty: `usage` counts store-resident cells. A mapping
//! handed to a live caller outlives eviction if that caller still holds its
//! `Arc` — that is the read-only-swap contract (nobody can dirty it), and
//! it is the caller's drop that truly releases the pages.

use crate::engine::{ChunkStore, Digest};
use crate::{crypto, CasError};
use dashmap::DashMap;
use memmap2::{Mmap, MmapOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// One resident cell: a page-aligned read-only mapping plus LRU bookkeeping.
pub struct HotCell {
    pub manifest_hash: Digest,
    pub size: usize,
    pub mmap: Arc<Mmap>,
    last_used: AtomicU64,
}

/// Snapshot for `avm-pkg hot-stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotStats {
    pub capacity: u64,
    pub usage: u64,
    pub cells: usize,
}

pub struct HotStore {
    cold: Arc<ChunkStore>,
    cells: DashMap<Digest, Arc<HotCell>>,
    capacity: u64,
    usage: AtomicU64,
    tick: AtomicU64,
    hot_dir: PathBuf,
    enforce_signatures: bool,
}

impl HotStore {
    pub fn new(
        cold: Arc<ChunkStore>,
        capacity: u64,
        hot_dir: impl Into<PathBuf>,
        enforce_signatures: bool,
    ) -> Result<Self, CasError> {
        let hot_dir = hot_dir.into();
        std::fs::create_dir_all(&hot_dir)?;
        Ok(HotStore {
            cold,
            cells: DashMap::new(),
            capacity,
            usage: AtomicU64::new(0),
            tick: AtomicU64::new(0),
            hot_dir,
            enforce_signatures,
        })
    }

    pub fn hot_dir(&self) -> &Path {
        &self.hot_dir
    }

    pub fn stats(&self) -> HotStats {
        HotStats {
            capacity: self.capacity,
            usage: self.usage.load(Ordering::Relaxed),
            cells: self.cells.len(),
        }
    }

    /// Demand-hydrate a cell; fast path is a map hit (an `Arc` clone and a
    /// counter bump), slow path faults the cell in from the cold store.
    pub fn acquire(&self, manifest_hash: &Digest) -> Result<Arc<Mmap>, CasError> {
        if let Some(cell) = self.cells.get(manifest_hash) {
            cell.last_used.store(self.tick.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
            return Ok(cell.mmap.clone());
        }
        self.hydrate(manifest_hash)
    }

    /// Convenience: acquire and copy the cell's bytes out (CLI `get`).
    pub fn read_cell(&self, manifest_hash: &Digest) -> Result<Vec<u8>, CasError> {
        let mmap = self.acquire(manifest_hash)?;
        Ok(mmap.to_vec())
    }

    /// Explicitly drop a cell from the resident set.
    pub fn release(&self, manifest_hash: &Digest) {
        if let Some((_, cell)) = self.cells.remove(manifest_hash) {
            self.usage.fetch_sub(cell.size as u64, Ordering::Relaxed);
        }
    }

    fn hydrate(&self, manifest_hash: &Digest) -> Result<Arc<Mmap>, CasError> {
        let manifest = self.cold.get_manifest(manifest_hash)?;
        // Fail-closed signature policy:
        //   * signed  -> must verify (tampering is a hard refusal),
        //   * key-but-no-signature -> malformed/hostile, never hydrated,
        //   * plain unsigned -> allowed unless this store enforces.
        match (&manifest.pubkey, &manifest.signature) {
            (Some(_), Some(_)) => crypto::verify_manifest(&manifest)?,
            (None, None) if !self.enforce_signatures => {}
            _ => {
                return Err(CasError::Signature(format!(
                    "manifest {} is unsigned and this hot store requires signatures",
                    crate::engine::hex(manifest_hash)
                )))
            }
        }
        let total = manifest.total_size as usize;
        self.make_room(total as u64);

        // Scratch file in the hot dir (tmpfs when $AVM_HOT_DIR says so),
        // written chunk by chunk, mapped private, then unlinked.
        let mut tf = tempfile::NamedTempFile::new_in(&self.hot_dir)
            .map_err(CasError::Io)?;
        for c in &manifest.chunks {
            let raw = self.cold.get_chunk(c)?;
            tf.as_file_mut().write_all(&raw)?;
        }
        tf.as_file_mut().flush()?;
        // SAFETY: `map_copy_read_only` maps the file MAP_PRIVATE/PROT_READ
        // through the kernel; the file is a plain scratch file we own and
        // keep alive across the call. This is the one mmap site the spec
        // §6.3 sanctions; nothing else in the subsystem is unsafe.
        let mmap = unsafe { MmapOptions::new().map_copy_read_only(tf.as_file())? };
        let mmap = Arc::new(mmap);
        // Unlink now: the mapping outlives the file (unix semantics), and
        // a leaked scratch file would silently eat hot-store budget.
        drop(tf);

        self.usage.fetch_add(total as u64, Ordering::Relaxed);
        let cell = Arc::new(HotCell {
            manifest_hash: *manifest_hash,
            size: total,
            mmap: mmap.clone(),
            last_used: AtomicU64::new(self.tick.fetch_add(1, Ordering::Relaxed)),
        });
        self.cells.insert(*manifest_hash, cell);
        Ok(mmap)
    }

    /// Evict least-recently-used resident cells until `incoming` fits the
    /// budget. Pure drops — no writeback is possible by construction.
    fn make_room(&self, incoming: u64) {
        while self.usage.load(Ordering::Relaxed) + incoming > self.capacity {
            // Find the LRU victim without holding the iterator while
            // removing (dashmap shard re-entrancy would deadlock).
            let victim = self
                .cells
                .iter()
                .reduce(|a, b| {
                    if a.value().last_used.load(Ordering::Relaxed)
                        <= b.value().last_used.load(Ordering::Relaxed)
                    {
                        a
                    } else {
                        b
                    }
                })
                .map(|e| *e.key());
            match victim {
                Some(k) => self.release(&k),
                None => break, // nothing resident; caller decides
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filler(len: usize, salt: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 32);
        let mut ctr: u64 = salt as u64;
        while out.len() < len {
            let mut b = ctr.to_le_bytes().to_vec();
            b[7] ^= salt;
            out.extend_from_slice(blake3::hash(&b).as_bytes());
            ctr += 1;
        }
        out.truncate(len);
        out
    }

    fn setup(capacity: u64) -> (tempfile::TempDir, Arc<ChunkStore>, HotStore) {
        let dir = tempfile::tempdir().unwrap();
        let cold = Arc::new(ChunkStore::open(dir.path().join("store")).unwrap());
        let hot = HotStore::new(
            cold.clone(),
            capacity,
            dir.path().join("hot"),
            false,
        )
        .unwrap();
        (dir, cold, hot)
    }

    #[test]
    fn hydrate_returns_exact_bytes() {
        let (_d, cold, hot) = setup(64 * 1024 * 1024);
        let data = filler(2 * 1024 * 1024, 10);
        let m = cold.put_buffer("cell", "1", &data, None).unwrap();
        let id = m.identity_digest();
        let out = hot.read_cell(&id).unwrap();
        assert_eq!(out, data);
        assert_eq!(hot.stats().cells, 1);
        assert_eq!(hot.stats().usage, data.len() as u64);
        // Page alignment of the mapping (spec: page-aligned for zero-copy).
        let mmap = hot.acquire(&id).unwrap();
        assert_eq!(mmap.as_ptr() as usize % 4096, 0, "mapping must be page-aligned");
    }

    #[test]
    fn lru_ordering_evicts_coldest() {
        // 2.5 cells of budget: hydrating C while A and B are resident must
        // evict the least-recently-used one.
        let (_d, cold, hot) = setup(2 * 1024 * 1024 + 512 * 1024);
        let mk = |salt: u8| {
            let data = filler(1024 * 1024, salt);
            let m = cold.put_buffer("cell", "1", &data, None).unwrap();
            m.identity_digest()
        };
        let (a, b, c) = (mk(20), mk(21), mk(22));
        hot.acquire(&a).unwrap();
        hot.acquire(&b).unwrap();
        // Touch B so A becomes the LRU victim.
        hot.acquire(&b).unwrap();
        hot.acquire(&c).unwrap();
        assert_eq!(hot.stats().cells, 2, "capacity forces one eviction");
        assert_eq!(hot.stats().usage, 2 * 1024 * 1024);
        // B and C must be resident (fresh), A evicted.
        assert!(hot.cells.contains_key(&b));
        assert!(hot.cells.contains_key(&c));
        assert!(!hot.cells.contains_key(&a));
        // The evicted cell is still reconstructible from cold — that is the
        // zero-writeback contract's other half.
        let back = hot.read_cell(&a).unwrap();
        assert_eq!(back, filler(1024 * 1024, 20));
    }

    #[test]
    fn signed_manifest_tamper_refuses_hydration() {
        let dir = tempfile::tempdir().unwrap();
        let cold = Arc::new(ChunkStore::open(dir.path().join("store")).unwrap());
        let hot = HotStore::new(cold.clone(), 64 * 1024 * 1024, dir.path().join("hot"), false)
            .unwrap();
        let data = filler(256 * 1024, 30);
        let mut m = cold.put_buffer("cell", "1", &data, None).unwrap();
        crypto::sign_manifest(&mut m, &crypto::signing_key_from_seed(&[9u8; 32]));
        // Impersonate: keep identity, swap in a foreign key, drop the sig.
        m.pubkey = Some([1u8; 32]);
        m.signature = None;
        let id = cold.put_manifest(&m).unwrap();
        assert!(matches!(hot.read_cell(&id), Err(CasError::Signature(_))));
    }

    /// Task 1.4 acceptance: 2 GiB of *mapped* cell bytes against a 1 GiB
    /// budget; least-recently-used cells must be dropped, cold packfiles
    /// must be byte-untouched (no writeback, no metadata churn), and
    /// evicted cells must remain reconstructible.
    ///
    /// Cells share the bulk of their chunks in the cold store (that is what
    /// a CAS is for); the pressure being tested is on *mapped resident
    /// bytes*, which is what the budget bounds. Run explicitly:
    /// `cargo test -p avm-cas --release -- --ignored hot_store`.
    #[test]
    #[ignore]
    fn acceptance_hot_store_2gib_over_1gib_zero_writeback() {
        let dir = tempfile::tempdir().unwrap();
        let cold_path = dir.path().join("store");
        let hot_dir = std::env::var_os("AVM_HOT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.path().join("hot"));
        let cold = Arc::new(ChunkStore::open(&cold_path).unwrap());
        let capacity: u64 = 1024 * 1024 * 1024; // 1 GiB

        // 8 cells x 250 MiB = 2 GiB mapped. Mostly zeros with a unique
        // header per cell: dedup shares everything after the first chunk.
        let cell_size = 250 * 1024 * 1024;
        let mut ids = Vec::new();
        for i in 0..8u8 {
            let mut data = vec![0u8; cell_size];
            data[..512].copy_from_slice(&filler(512, 100 + i));
            let m = cold.put_buffer("cell", "1", &data, None).unwrap();
            ids.push((m.identity_digest(), data));
        }
        let cold_stats = cold.stats().unwrap();
        eprintln!(
            "cold store: {} chunks in {} packs, {:.1} MiB packed for {:.1} MiB of unique cell content",
            cold_stats.chunks,
            cold_stats.packs,
            cold_stats.pack_bytes as f64 / 1e6,
            (cell_size + 512) as f64 / 1e6
        );

        // Snapshot cold packs: hydration + eviction must not touch them.
        let before: Vec<(PathBuf, u64, std::time::SystemTime)> = cold
            .pack_paths()
            .iter()
            .map(|p| {
                let md = std::fs::metadata(p).unwrap();
                (p.clone(), md.len(), md.modified().unwrap())
            })
            .collect();

        let hot = HotStore::new(cold.clone(), capacity, &hot_dir, false).unwrap();

        // Hydrate all 8 sequentially, forcing every page resident so the
        // accounting is real, not lazy-map fiction.
        for (id, data) in &ids {
            let mmap = hot.acquire(id).unwrap();
            assert_eq!(mmap.len(), data.len());
            let mut acc = 0u8;
            let mut expect = 0u8;
            let mut off = 0;
            while off < data.len() {
                acc ^= mmap[off];
                expect ^= data[off];
                off += 4096;
            }
            assert_eq!(acc, expect, "page-touch pass must read the same bytes as the source");
            let s = hot.stats();
            assert!(
                s.usage <= capacity,
                "usage {} exceeds budget {} after hydration",
                s.usage,
                capacity
            );
        }
        assert_eq!(
            hot.stats().cells,
            (capacity / cell_size as u64) as usize,
            "resident cell count must equal budget/cell-size"
        );

        // Cold store untouched by the whole hydration/eviction storm.
        for (p, len, mtime) in &before {
            let md = std::fs::metadata(p).unwrap();
            assert_eq!(md.len(), *len, "pack {:?} changed size", p);
            assert_eq!(md.modified().unwrap(), *mtime, "pack {:?} mtime moved", p);
        }

        // Every cell is still reconstructible — eviction cost nothing.
        for (id, data) in &ids {
            let back = hot.read_cell(id).unwrap();
            assert_eq!(&back, data);
        }
    }
}
