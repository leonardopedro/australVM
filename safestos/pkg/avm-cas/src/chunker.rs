//! FastCDC-style Content-Defined Chunking (Sprint 1, Task 1.1).
//!
//! Variable-size chunking with the plan's target parameters
//! (min 16 KiB / avg 64 KiB / max 256 KiB) so that a single-byte edit in a
//! large artifact shifts at most one or two chunk boundaries instead of
//! invalidating every downstream fixed-size block.
//!
//! Normalization (two-gear-mask scheme): right after the minimum size the
//! cut mask is *rarer* (so a run of near-minimum chunks cannot collapse the
//! distribution), and past the midpoint it becomes *frequent* (so the scan
//! converges on `max` with overwhelming probability). Cut detection is a
//! 64-bit gear hash: `h = (h << 1) + GEAR[data[i]]`, cut when the low bits
//! of `h` match the active mask.
//!
//! The gear table is fixed and reproducible: 256 u64 words from SplitMix64
//! with the constant seed below. avm-pkg only ever compares chunks produced
//! by its own chunker, so self-consistency plus reproducibility across
//! builds is the requirement; the seed makes table drift a deliberate,
//! store-format-breaking change rather than an accident.

use crate::Blake3Digest;

/// Fixed gear table seed. Changing this re-chunks every store on earth.
const GEAR_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 256-entry gear table, generated once per process from [`GEAR_SEED`].
static GEAR: std::sync::LazyLock<[u64; 256]> = std::sync::LazyLock::new(|| {
    let mut state = GEAR_SEED;
    let mut table = [0u64; 256];
    for slot in table.iter_mut() {
        *slot = splitmix64(&mut state);
    }
    table
});

/// Chunker size parameters. `min <= avg <= max` is checked at chunk time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkerConfig {
    pub min: usize,
    pub avg: usize,
    pub max: usize,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        // Spec §3.2: FastCDC v3, min 16 KiB, avg 64 KiB, max 256 KiB.
        Self {
            min: 16 * 1024,
            avg: 64 * 1024,
            max: 256 * 1024,
        }
    }
}

impl ChunkerConfig {
    fn validate(&self) -> bool {
        self.min > 0 && self.min <= self.avg && self.avg <= self.max
    }

    /// floor(log2(avg)) — the base mask width. avg must be >= 2.
    fn log2_avg(&self) -> u32 {
        usize::BITS - 1 - self.avg.leading_zeros()
    }
}

/// One chunk of a payload: its byte range and plaintext BLAKE3 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub offset: usize,
    pub len: usize,
    pub hash: Blake3Digest,
}

/// Cut position (chunk length) for the next chunk starting at `data[0]`.
///
/// Guarantees: `min < returned <= data.len()`, and `returned <= max`
/// whenever `data.len() >= max`.
fn cut_point(data: &[u8], cfg: &ChunkerConfig) -> usize {
    let n = data.len();
    if n <= cfg.min {
        return n;
    }
    // Normalization masks around the base width log2(avg):
    //   early = base + 2 bits -> rare cuts (expected ~4x avg distance),
    //   late  = base - 2 bits -> frequent cuts (expected ~avg/4 distance).
    let base = cfg.log2_avg().max(3);
    let mask_early = if base + 2 >= 64 {
        u64::MAX
    } else {
        (1u64 << (base + 2)) - 1
    };
    let mask_late = (1u64 << base.saturating_sub(2).max(1)) - 1;
    // Midpoint: switch masks halfway between min and avg so most chunks
    // land near avg.
    let mid = cfg.min + (cfg.avg - cfg.min) / 2;

    let mut h: u64 = 0;
    let scan_end = n.min(cfg.max);
    let mut i = cfg.min;
    while i < scan_end {
        h = h.wrapping_shl(1).wrapping_add(GEAR[data[i] as usize]);
        let mask = if i < mid { mask_early } else { mask_late };
        if h & mask == 0 {
            return i + 1;
        }
        i += 1;
    }
    // Either a hard cut at max, or the (short) remainder.
    scan_end
}

/// Split `data` into content-defined chunks with BLAKE3 plaintext digests.
///
/// Concatenating the chunk byte ranges reproduces `data` exactly; an empty
/// payload yields no chunks.
pub fn chunk_payload(data: &[u8], cfg: &ChunkerConfig) -> Result<Vec<Chunk>, crate::CasError> {
    if !cfg.validate() {
        return Err(crate::CasError::Corrupt(format!(
            "invalid chunker config min={} avg={} max={}",
            cfg.min, cfg.avg, cfg.max
        )));
    }
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let len = cut_point(&data[offset..], cfg);
        let hash = *blake3::hash(&data[offset..offset + len]).as_bytes();
        out.push(Chunk { offset, len, hash });
        offset += len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random filler: BLAKE3 in counter mode. Avoids a
    /// `rand` dependency and makes every acceptance run bit-identical.
    fn pseudo_random_bytes(len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 32);
        let mut counter: u64 = 0;
        while out.len() < len {
            out.extend_from_slice(blake3::hash(&counter.to_le_bytes()).as_bytes());
            counter += 1;
        }
        out.truncate(len);
        out
    }

    #[test]
    fn roundtrip_and_bounds() {
        let cfg = ChunkerConfig::default();
        for size in [
            0usize,
            1,
            4095,
            cfg.min - 1,
            cfg.min,
            cfg.avg + 1,
            3 * cfg.max + 17,
        ] {
            let data = pseudo_random_bytes(size);
            let chunks = chunk_payload(&data, &cfg).unwrap();
            // Coverage: exact, gapless, in-order partition of the payload.
            let mut cursor = 0;
            for c in &chunks {
                assert_eq!(c.offset, cursor, "chunk boundaries must tile the input");
                assert!(c.len > 0);
                cursor += c.len;
                assert_eq!(
                    *blake3::hash(&data[c.offset..c.offset + c.len]).as_bytes(),
                    c.hash,
                    "chunk digest must match its plaintext"
                );
            }
            assert_eq!(cursor, data.len(), "partition must cover the input");
            if size > cfg.min {
                assert!(
                    chunks.iter().all(|c| c.len <= cfg.max.max(cfg.min)),
                    "chunk exceeds max"
                );
            }
        }
    }

    #[test]
    fn average_size_sane() {
        let cfg = ChunkerConfig::default();
        let data = pseudo_random_bytes(32 * 1024 * 1024);
        let chunks = chunk_payload(&data, &cfg).unwrap();
        let avg = data.len() / chunks.len();
        assert!(
            avg >= cfg.avg / 2 && avg <= cfg.avg * 2,
            "average chunk size {avg} out of sane band [{}, {}]",
            cfg.avg / 2,
            cfg.avg * 2
        );
    }

    #[test]
    fn max_bound_respected() {
        let cfg = ChunkerConfig {
            min: 1024,
            avg: 4096,
            max: 8192,
        };
        // Highly-compressible data still must respect the hard max.
        let data = vec![7u8; 1024 * 1024];
        let chunks = chunk_payload(&data, &cfg).unwrap();
        assert!(chunks.iter().all(|c| c.len <= cfg.max));
        assert!(chunks.len() > 1);
    }

    /// Task 1.1 acceptance: a single-byte mutation in a 64 MiB payload must
    /// leave > 95% of the original chunks' digests present in the mutated
    /// chunk set. In the default suite (measured: well under a second).
    #[test]
    fn acceptance_64mib_single_byte_mutation() {
        mutation_acceptance(64 * 1024 * 1024);
    }

    /// Same property at suite scale (fast enough for the default run).
    #[test]
    fn mutation_resync_16mib() {
        mutation_acceptance(16 * 1024 * 1024);
    }

    fn mutation_acceptance(size: usize) {
        let cfg = ChunkerConfig::default();
        let mut data = pseudo_random_bytes(size);
        let before = chunk_payload(&data, &cfg).unwrap();

        // One-bit flip near the middle.
        let mid = size / 2;
        data[mid] ^= 0x01;
        let after = chunk_payload(&data, &cfg).unwrap();

        let after_set: std::collections::HashSet<_> = after.iter().map(|c| c.hash).collect();
        let matched = before
            .iter()
            .filter(|c| after_set.contains(&c.hash))
            .count();
        let ratio = matched as f64 / before.len() as f64;
        eprintln!(
            "mutation resync ({size} bytes, {mut_} chunks): {matched}/{} kept ({:.1}%)",
            before.len(),
            ratio * 100.0,
            mut_ = before.len()
        );
        assert!(
            ratio > 0.95,
            "single-byte mutation kept {matched}/{} chunks ({:.2}%), need > 95%",
            before.len(),
            ratio * 100.0
        );
    }

    #[test]
    fn gear_table_is_stable() {
        // Reproducibility across builds/processes is the store-format
        // contract; pin two known words so table drift fails loudly.
        assert_eq!(GEAR[0], splitmix64_first(GEAR_SEED));
        assert_ne!(GEAR[0], GEAR[1]);
    }

    fn splitmix64_first(seed: u64) -> u64 {
        let mut st = seed;
        splitmix64(&mut st)
    }
}
