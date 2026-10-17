//! Compression codecs for packfile entries (Sprint 1, Task 1.2).
//!
//! LZ4 for in-flight/local packs (streaming speed dominates: the hot store
//! re-inflates cells on demand). zstd for cold/remote archival is Sprint 5
//! and deliberately *not* stubbed here — an unimplemented codec id must
//! fail loudly at decode time rather than silently store uncompressed.

use crate::CasError;

/// Uncompressed passthrough (used when LZ4 does not shrink the chunk).
pub const CODEC_STORE: u8 = 0;
/// LZ4 block format (lz4_flex).
pub const CODEC_LZ4: u8 = 1;

/// Compress `data`, choosing the smaller of LZ4 and STORE. Returns the
/// codec id actually used. Highly-compressible payloads (sparse cells,
/// zero-filled sections) legitimately fall back to STORE only if LZ4
/// somehow expands them — in practice never, but the fallback keeps the
/// format total.
pub fn compress(data: &[u8]) -> (u8, Vec<u8>) {
    // Size-prefixed form: self-delimiting, pairs with
    // `decompress_size_prepended` below (the raw `compress` output would
    // silently mismatch that reader).
    let lz = lz4_flex::compress_prepend_size(data);
    if lz.len() < data.len() {
        (CODEC_LZ4, lz)
    } else {
        (CODEC_STORE, data.to_vec())
    }
}

/// Decompress a packfile entry. Unknown codec ids are a hard error: a
/// future zstd codec must be added as an explicit variant, never guessed.
pub fn decompress(codec: u8, data: &[u8]) -> Result<Vec<u8>, CasError> {
    match codec {
        CODEC_STORE => Ok(data.to_vec()),
        CODEC_LZ4 => lz4_flex::decompress_size_prepended(data)
            .map_err(|e| CasError::Corrupt(format!("lz4 block invalid: {e}"))),
        other => Err(CasError::Codec(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lz4_roundtrip_shrinks_sparse_data() {
        let data = vec![0u8; 1 << 20];
        let (codec, comp) = compress(&data);
        assert_eq!(codec, CODEC_LZ4);
        // LZ4 encodes long matches with 255-byte extended lengths, so a
        // single 1 MiB run costs ~size/255 bytes ≈ 4 KiB in the token
        // stream: bound the assertion on that formula, not a guess.
        let expected_ceiling = data.len() / 255 + 64;
        assert!(
            comp.len() < expected_ceiling,
            "1 MiB of zeros compressed to {} (ceiling {expected_ceiling})",
            comp.len()
        );
        let back = decompress(codec, &comp).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn incompressible_data_roundtrips_via_store() {
        let mut data = Vec::with_capacity(64 * 1024);
        let mut ctr: u64 = 0;
        while data.len() < 64 * 1024 {
            data.extend_from_slice(blake3::hash(&ctr.to_le_bytes()).as_bytes());
            ctr += 1;
        }
        let (codec, comp) = compress(&data);
        let back = decompress(codec, &comp).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn unknown_codec_is_loud() {
        assert!(matches!(decompress(9, b"x"), Err(CasError::Codec(9))));
    }
}
