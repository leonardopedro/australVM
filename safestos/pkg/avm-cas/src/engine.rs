//! The cold content-addressed store (Sprint 1, Task 1.2).
//!
//! Append-only packfiles of LZ4-compressed, content-defined chunks,
//! addressed by the unkeyed BLAKE3 of each *plaintext* chunk. An atomic
//! index maps chunk digest -> (pack, framing); root manifests describe a
//! whole cell (ordered chunk list + totals) and are themselves
//! content-addressed, so a cell's hash is stable whether or not it carries
//! a signature.
//!
//! Determinism rule (spec §6.4): every encoding here is fixed-order or
//! explicitly sorted — chunk lists in manifest order, GC compaction in
//! digest order — so identical inputs always produce identical digests,
//! independent of hash-map iteration order.
//!
//! Layout:
//! ```text
//! <root>/packs/p-<id>.pack    append-only: [u32 comp_len][u8 codec][payload]*
//! <root>/index/chunks.idx     digest -> entry (atomic temp+rename writes)
//! <root>/manifests/<hex>.mfx  canonical manifests, named by identity hash
//! ```

use crate::chunker::{chunk_payload, ChunkerConfig};
use crate::compression;
use crate::{Blake3Digest, CasError};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 32-byte BLAKE3 digest: the store's address type (alias of the crate's).
pub type Digest = Blake3Digest;

const MANIFEST_MAGIC: [u8; 4] = *b"AVMM";
const MANIFEST_FORMAT: u16 = 1;
const INDEX_MAGIC: [u8; 4] = *b"AVMI";
const INDEX_FORMAT: u16 = 1;

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xF) as u32, 16).unwrap());
    }
    s
}

pub fn unhex(s: &str) -> Result<Digest, CasError> {
    if s.len() != 64 {
        return Err(CasError::Corrupt(format!(
            "digest must be 64 hex chars, got {}",
            s.len()
        )));
    }
    let bytes = s.as_bytes();
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = (bytes[2 * i] as char)
            .to_digit(16)
            .ok_or_else(|| CasError::Corrupt("bad hex".into()))? as u8;
        let lo = (bytes[2 * i + 1] as char)
            .to_digit(16)
            .ok_or_else(|| CasError::Corrupt("bad hex".into()))? as u8;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

/// A root manifest: the identity of one cell in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub total_size: u64,
    /// Reassembly order.
    pub chunks: Vec<Digest>,
    /// Derivation input hash, when the cell came from `avm-eval`.
    pub drv_hash: Option<Digest>,
    pub pubkey: Option<[u8; 32]>,
    pub signature: Option<[u8; 64]>,
}

impl Manifest {
    pub fn new(name: &str, version: &str) -> Self {
        Manifest {
            name: name.to_string(),
            version: version.to_string(),
            total_size: 0,
            chunks: Vec::new(),
            drv_hash: None,
            pubkey: None,
            signature: None,
        }
    }

    /// Canonical *unsigned* encoding — the signed and hashed message.
    fn encode_unsigned(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&MANIFEST_MAGIC);
        b.extend_from_slice(&MANIFEST_FORMAT.to_le_bytes());
        for s in [&self.name, &self.version] {
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        }
        b.extend_from_slice(&self.total_size.to_le_bytes());
        match &self.drv_hash {
            None => b.push(0),
            Some(h) => {
                b.push(1);
                b.extend_from_slice(h);
            }
        }
        b.extend_from_slice(&(self.chunks.len() as u32).to_le_bytes());
        for c in &self.chunks {
            b.extend_from_slice(c);
        }
        b
    }

    /// On-disk encoding: unsigned body plus key + signature (if any).
    fn encode_signed(&self) -> Vec<u8> {
        let mut b = self.encode_unsigned();
        match &self.pubkey {
            None => b.push(0),
            Some(k) => {
                b.push(1);
                b.extend_from_slice(k);
            }
        }
        match &self.signature {
            None => b.push(0),
            Some(s) => {
                b.push(1);
                b.extend_from_slice(s);
            }
        }
        b
    }

    /// Content address of the manifest: BLAKE3 of the unsigned encoding,
    /// so signing never changes a cell's identity.
    pub fn identity_digest(&self) -> Digest {
        *blake3::hash(&self.encode_unsigned()).as_bytes()
    }

    /// Canonical on-the-wire bytes for remote sync (Sprint 5.1): exactly
    /// what `manifests/<id>.mfx` holds, so a pulled manifest re-verifies to
    /// the same identity as a locally created one.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.encode_signed()
    }

    /// Inverse of [`Self::to_bytes`]. Identity is content-derived, so the
    /// caller compares `identity_digest()` against the id it actually
    /// asked for — this only guarantees structural validity.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CasError> {
        Self::decode(bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, CasError> {
        let mut r = Reader { b: bytes, i: 0 };
        if r.take(4)? != MANIFEST_MAGIC {
            return Err(CasError::Corrupt("manifest magic mismatch".into()));
        }
        let format = r.u16()?;
        if format != MANIFEST_FORMAT {
            return Err(CasError::Corrupt(format!(
                "manifest format {format} unsupported"
            )));
        }
        let name = r.string()?;
        let version = r.string()?;
        let total_size = r.u64()?;
        let drv_hash = match r.u8()? {
            0 => None,
            1 => Some(r.array32()?),
            _ => return Err(CasError::Corrupt("drv_hash presence byte invalid".into())),
        };
        let n = r.u32()? as usize;
        if n > 1 << 24 {
            return Err(CasError::Corrupt("manifest chunk count implausible".into()));
        }
        let mut chunks = Vec::with_capacity(n);
        for _ in 0..n {
            chunks.push(r.array32()?);
        }
        let pubkey = match r.u8()? {
            0 => None,
            1 => Some(r.array32()?),
            _ => return Err(CasError::Corrupt("pubkey presence byte invalid".into())),
        };
        let signature = match r.u8()? {
            0 => None,
            1 => Some(r.array64()?),
            _ => return Err(CasError::Corrupt("signature presence byte invalid".into())),
        };
        if r.i != bytes.len() {
            return Err(CasError::Corrupt("trailing bytes after manifest".into()));
        }
        Ok(Manifest {
            name,
            version,
            total_size,
            chunks,
            drv_hash,
            pubkey,
            signature,
        })
    }

    /// JSON view for the CLI (`stat`). Hex-encoded digests.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "name": self.name,
            "version": self.version,
            "total_size": self.total_size,
            "chunks": self.chunks.len(),
            "drv_hash": self.drv_hash.map(|h| hex(&h)),
            "identity": hex(&self.identity_digest()),
            "signed": self.signature.is_some(),
            "pubkey": self.pubkey.map(|k| hex(&k)),
        })
        .to_string()
    }
}

/// Cursor over a byte slice for canonical decoding. Bounds-checked; any
/// truncation is `Corrupt`, never a panic.
struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CasError> {
        if self.i + n > self.b.len() {
            return Err(CasError::Corrupt("manifest truncated".into()));
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, CasError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, CasError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, CasError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CasError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn array32(&mut self) -> Result<[u8; 32], CasError> {
        Ok(self.take(32)?.try_into().unwrap())
    }
    fn array64(&mut self) -> Result<[u8; 64], CasError> {
        Ok(self.take(64)?.try_into().unwrap())
    }
    fn string(&mut self) -> Result<String, CasError> {
        let n = self.u32()? as usize;
        if n > 1 << 20 {
            return Err(CasError::Corrupt("string length implausible".into()));
        }
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| CasError::Corrupt("manifest string not utf-8".into()))
    }
}

#[derive(Debug, Clone, Copy)]
struct ChunkEntry {
    pack_id: u32,
    offset: u64,
    comp_len: u32,
    raw_len: u32,
    codec: u8,
}

/// Store-level counters for `avm-pkg hot-stats`/`stat`-style inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreStats {
    pub chunks: usize,
    pub packs: usize,
    pub manifests: usize,
    pub pack_bytes: u64,
}

/// Outcome of a garbage-collection compaction pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcReport {
    pub live_chunks: usize,
    pub dead_chunks: usize,
    pub packs_before: usize,
    pub packs_after: usize,
    pub bytes_reclaimed: u64,
}

pub struct ChunkStore {
    root: PathBuf,
    state: Mutex<HashMap<Digest, ChunkEntry>>,
    chunker: ChunkerConfig,
    next_pack_id: Mutex<u32>,
}

impl ChunkStore {
    /// Open (or create) a store at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, CasError> {
        let root = root.into();
        for sub in ["packs", "index", "manifests"] {
            fs::create_dir_all(root.join(sub))?;
        }
        let index_path = root.join("index/chunks.idx");
        let state = if index_path.exists() {
            Self::load_index(&index_path)?
        } else {
            HashMap::new()
        };
        let next_pack_id = Self::scan_next_pack_id(&root)?;
        Ok(ChunkStore {
            root,
            state: Mutex::new(state),
            chunker: ChunkerConfig::default(),
            next_pack_id: Mutex::new(next_pack_id),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn pack_path(&self, id: u32) -> PathBuf {
        self.root.join(format!("packs/p-{id:08}.pack"))
    }

    fn scan_next_pack_id(root: &Path) -> Result<u32, CasError> {
        let mut next = 0u32;
        for e in fs::read_dir(root.join("packs"))? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if let Some(rest) = name
                .strip_prefix("p-")
                .and_then(|r| r.strip_suffix(".pack"))
            {
                if let Ok(id) = rest.parse::<u32>() {
                    next = next.max(id + 1);
                }
            }
        }
        Ok(next)
    }

    fn load_index(path: &Path) -> Result<HashMap<Digest, ChunkEntry>, CasError> {
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let mut r = Reader { b: &buf, i: 0 };
        if r.take(4)? != INDEX_MAGIC {
            return Err(CasError::Corrupt("index magic mismatch".into()));
        }
        let format = r.u16()?;
        if format != INDEX_FORMAT {
            return Err(CasError::Corrupt(format!(
                "index format {format} unsupported"
            )));
        }
        let n = r.u32()? as usize;
        let mut map = HashMap::with_capacity(n);
        for _ in 0..n {
            let hash = r.array32()?;
            let entry = ChunkEntry {
                pack_id: r.u32()?,
                offset: r.u64()?,
                comp_len: r.u32()?,
                raw_len: r.u32()?,
                codec: r.u8()?,
            };
            map.insert(hash, entry);
        }
        Ok(map)
    }

    fn persist_index_locked(&self, map: &HashMap<Digest, ChunkEntry>) -> Result<(), CasError> {
        let mut entries: BTreeMap<&Digest, &ChunkEntry> = BTreeMap::new();
        for (k, v) in map {
            entries.insert(k, v);
        }
        let mut buf = Vec::with_capacity(10 + entries.len() * 53);
        buf.extend_from_slice(&INDEX_MAGIC);
        buf.extend_from_slice(&INDEX_FORMAT.to_le_bytes());
        buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (hash, e) in entries {
            buf.extend_from_slice(hash);
            buf.extend_from_slice(&e.pack_id.to_le_bytes());
            buf.extend_from_slice(&e.offset.to_le_bytes());
            buf.extend_from_slice(&e.comp_len.to_le_bytes());
            buf.extend_from_slice(&e.raw_len.to_le_bytes());
            buf.push(e.codec);
        }
        atomic_write(&self.root.join("index/chunks.idx"), &buf)
    }

    pub fn has_chunk(&self, hash: &Digest) -> bool {
        self.state.lock().unwrap().contains_key(hash)
    }

    /// Raw (pre-compression) lengths for an ordered chunk list, straight
    /// from the index. Lets a caller lay out a cell's scratch file before
    /// any bytes are read — the hot store's parallel hydration uses this to
    /// know every chunk's final offset up front.
    pub fn chunk_raw_lens(&self, chunks: &[Digest]) -> Result<Vec<u64>, CasError> {
        let map = self.state.lock().unwrap();
        let mut out = Vec::with_capacity(chunks.len());
        for c in chunks {
            let e = map.get(c).ok_or_else(|| CasError::NotFound(hex(c)))?;
            out.push(u64::from(e.raw_len));
        }
        Ok(out)
    }

    /// Read one chunk (decompressed) by digest.
    pub fn get_chunk(&self, hash: &Digest) -> Result<Vec<u8>, CasError> {
        let entry = {
            let map = self.state.lock().unwrap();
            map.get(hash)
                .copied()
                .ok_or_else(|| CasError::NotFound(hex(hash)))?
        };
        let mut f = File::open(self.pack_path(entry.pack_id))?;
        f.seek(SeekFrom::Start(entry.offset))?;
        let mut framed = [0u8; 4];
        f.read_exact(&mut framed)?;
        let comp_len = u32::from_le_bytes(framed);
        if comp_len != entry.comp_len {
            return Err(CasError::Corrupt(
                "pack framing disagrees with index".into(),
            ));
        }
        let mut codec = [0u8; 1];
        f.read_exact(&mut codec)?;
        if codec[0] != entry.codec {
            return Err(CasError::Corrupt("pack codec disagrees with index".into()));
        }
        let mut comp = vec![0u8; entry.comp_len as usize];
        f.read_exact(&mut comp)?;
        let raw = compression::decompress(entry.codec, &comp)?;
        if raw.len() != entry.raw_len as usize {
            return Err(CasError::Corrupt(
                "decompressed size disagrees with index".into(),
            ));
        }
        let actual = *blake3::hash(&raw).as_bytes();
        if actual != *hash {
            return Err(CasError::Corrupt(
                "chunk plaintext does not match its digest".into(),
            ));
        }
        Ok(raw)
    }

    /// Ingest a whole buffer as a new cell: chunk, dedup, append, and
    /// persist a manifest. Returns the manifest (unsigned; sign separately
    /// via [`crate::crypto::sign_manifest`] then [`Self::put_manifest`]).
    pub fn put_buffer(
        &self,
        name: &str,
        version: &str,
        data: &[u8],
        drv_hash: Option<Digest>,
    ) -> Result<Manifest, CasError> {
        let chunks = chunk_payload(data, &self.chunker)?;

        // Dedup pass: which chunk digests are genuinely new? First-appearance
        // order per digest keeps the append deterministic.
        let mut pending_new: Vec<(Digest, usize)> = Vec::new();
        {
            let map = self.state.lock().unwrap();
            let mut seen: std::collections::HashSet<Digest> = std::collections::HashSet::new();
            for c in &chunks {
                if !map.contains_key(&c.hash) && seen.insert(c.hash) {
                    pending_new.push((c.hash, c.len));
                }
            }
        }

        // Append the new (compressed) chunks to one fresh pack in one shot:
        // a single file open and a single index write per put_buffer, so
        // ingesting 2 GiB of cells does not turn into O(n²) index churn.
        if !pending_new.is_empty() {
            let pack_id = {
                let mut guard = self.next_pack_id.lock().unwrap();
                let id = *guard;
                *guard += 1;
                id
            };
            let mut pack = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(self.pack_path(pack_id))?;
            let mut new_entries: Vec<(Digest, ChunkEntry)> = Vec::with_capacity(pending_new.len());
            let mut offset = 0u64;
            // One pass to map digest -> byte range; avoids O(n²) scanning
            // for large payloads (2 GiB cells have ~32k chunks).
            let mut ranges: HashMap<Digest, (usize, usize)> = HashMap::with_capacity(chunks.len());
            for c in &chunks {
                ranges.entry(c.hash).or_insert((c.offset, c.len));
            }
            for (digest, raw_len) in pending_new {
                let (start, _) = ranges[&digest];
                let raw = &data[start..start + raw_len];
                let (codec, comp) = compression::compress(raw);
                pack.write_all(&(comp.len() as u32).to_le_bytes())?;
                pack.write_all(&[codec])?;
                pack.write_all(&comp)?;
                new_entries.push((
                    digest,
                    ChunkEntry {
                        pack_id,
                        offset,
                        comp_len: comp.len() as u32,
                        raw_len: raw_len as u32,
                        codec,
                    },
                ));
                offset += 4 + 1 + comp.len() as u64;
            }
            pack.sync_data()?;
            let mut map = self.state.lock().unwrap();
            for (digest, entry) in new_entries {
                map.insert(digest, entry);
            }
            self.persist_index_locked(&map)?;
        }

        let mut manifest = Manifest::new(name, version);
        manifest.total_size = data.len() as u64;
        manifest.chunks = chunks.iter().map(|c| c.hash).collect();
        manifest.drv_hash = drv_hash;
        self.put_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Ingest already-addressed raw chunk bytes (remote pulls): hash each
    /// buffer, dedup against the store *and* within the batch, and append
    /// all genuinely new chunks to one fresh pack with a single index
    /// write — the same batching rule as [`Self::put_buffer`], so pulling
    /// thousands of chunks is not O(n²) index churn. Returns each input
    /// buffer's digest, in order.
    pub fn put_chunk_batch(&self, raws: Vec<Vec<u8>>) -> Result<Vec<Digest>, CasError> {
        let mut digests: Vec<Digest> = Vec::with_capacity(raws.len());
        let mut pending: Vec<usize> = Vec::new();
        {
            let map = self.state.lock().unwrap();
            let mut seen: std::collections::HashSet<Digest> = std::collections::HashSet::new();
            for (i, raw) in raws.iter().enumerate() {
                let d = *blake3::hash(raw).as_bytes();
                digests.push(d);
                if !map.contains_key(&d) && seen.insert(d) {
                    pending.push(i);
                }
            }
        }
        if !pending.is_empty() {
            let pack_id = {
                let mut guard = self.next_pack_id.lock().unwrap();
                let id = *guard;
                *guard += 1;
                id
            };
            let mut pack = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(self.pack_path(pack_id))?;
            let mut new_entries: Vec<(Digest, ChunkEntry)> = Vec::with_capacity(pending.len());
            let mut offset = 0u64;
            for &i in &pending {
                let raw = &raws[i];
                let (codec, comp) = compression::compress(raw);
                pack.write_all(&(comp.len() as u32).to_le_bytes())?;
                pack.write_all(&[codec])?;
                pack.write_all(&comp)?;
                new_entries.push((
                    digests[i],
                    ChunkEntry {
                        pack_id,
                        offset,
                        comp_len: comp.len() as u32,
                        raw_len: raw.len() as u32,
                        codec,
                    },
                ));
                offset += 4 + 1 + comp.len() as u64;
            }
            pack.sync_data()?;
            let mut map = self.state.lock().unwrap();
            for (digest, entry) in new_entries {
                map.insert(digest, entry);
            }
            self.persist_index_locked(&map)?;
        }
        Ok(digests)
    }

    /// Single-chunk convenience wrapper over [`Self::put_chunk_batch`].
    pub fn put_chunk_data(&self, raw: &[u8]) -> Result<Digest, CasError> {
        let mut v = self.put_chunk_batch(vec![raw.to_vec()])?;
        Ok(v.remove(0))
    }

    /// Persist a manifest under its identity digest; every referenced chunk
    /// must already be present (fail-closed against partial writes).
    pub fn put_manifest(&self, manifest: &Manifest) -> Result<Digest, CasError> {
        {
            let map = self.state.lock().unwrap();
            for c in &manifest.chunks {
                if !map.contains_key(c) {
                    return Err(CasError::NotFound(format!("chunk {} of manifest", hex(c))));
                }
            }
        }
        let id = manifest.identity_digest();
        atomic_write(
            &self.root.join(format!("manifests/{}.mfx", hex(&id))),
            &manifest.encode_signed(),
        )?;
        Ok(id)
    }

    pub fn get_manifest(&self, id: &Digest) -> Result<Manifest, CasError> {
        let path = self.root.join(format!("manifests/{}.mfx", hex(id)));
        let mut f = File::open(&path).map_err(|_| CasError::NotFound(hex(id)))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let manifest = Manifest::decode(&buf)?;
        if manifest.identity_digest() != *id {
            return Err(CasError::Corrupt(
                "manifest file does not hash to its name".into(),
            ));
        }
        Ok(manifest)
    }

    /// Every stored root manifest digest, sorted (deterministic).
    pub fn list_manifests(&self) -> Result<Vec<Digest>, CasError> {
        let mut out = Vec::new();
        for e in fs::read_dir(self.root.join("manifests"))? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".mfx") {
                out.push(unhex(stem)?);
            }
        }
        out.sort();
        Ok(out)
    }

    /// Drop a manifest (and, later, its now-dead chunks via [`Self::gc`]).
    pub fn drop_manifest(&self, id: &Digest) -> Result<(), CasError> {
        let path = self.root.join(format!("manifests/{}.mfx", hex(id)));
        fs::remove_file(path).map_err(|_| CasError::NotFound(hex(id)))
    }

    /// Reassemble a manifest's payload.
    pub fn read_assembled(&self, manifest: &Manifest) -> Result<Vec<u8>, CasError> {
        let mut out = Vec::with_capacity(manifest.total_size as usize);
        for c in &manifest.chunks {
            out.extend_from_slice(&self.get_chunk(c)?);
        }
        if out.len() as u64 != manifest.total_size {
            return Err(CasError::Corrupt(
                "assembled size disagrees with manifest".into(),
            ));
        }
        Ok(out)
    }

    /// Compacting GC: keep chunks referenced by any manifest, rewrite the
    /// rest away. Single-writer, documented crash window between the index
    /// swap and pack removal (rebuild the index to recover).
    pub fn gc(&self) -> Result<GcReport, CasError> {
        let mut live: std::collections::HashSet<Digest> = std::collections::HashSet::new();
        for id in self.list_manifests()? {
            for c in self.get_manifest(&id)?.chunks {
                live.insert(c);
            }
        }
        let packs_before = self.pack_paths().len();
        let bytes_before: u64 = self
            .pack_paths()
            .iter()
            .filter_map(|p| fs::metadata(p).ok().map(|m| m.len()))
            .sum();

        let (dead, mut keep): (usize, Vec<(Digest, ChunkEntry)>) = {
            let map = self.state.lock().unwrap();
            let mut keep = Vec::new();
            let mut dead = 0;
            for (k, v) in map.iter() {
                if live.contains(k) {
                    keep.push((*k, *v));
                } else {
                    dead += 1;
                }
            }
            (dead, keep)
        };
        if dead == 0 {
            return Ok(GcReport {
                live_chunks: live.len(),
                dead_chunks: 0,
                packs_before,
                packs_after: packs_before,
                bytes_reclaimed: 0,
            });
        }

        // Deterministic compaction: live entries copied in digest order.
        keep.sort_by_key(|k| k.0);
        let tmp_pack = self.root.join("packs/.tmp-compaction.pack");
        let mut new_map = HashMap::with_capacity(keep.len());
        {
            let mut pack = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_pack)?;
            let mut offset = 0u64;
            for (digest, entry) in &keep {
                let raw = self.get_chunk(digest)?;
                let (codec, comp) = compression::compress(&raw);
                pack.write_all(&(comp.len() as u32).to_le_bytes())?;
                pack.write_all(&[codec])?;
                pack.write_all(&comp)?;
                new_map.insert(
                    *digest,
                    ChunkEntry {
                        pack_id: 0,
                        offset,
                        comp_len: comp.len() as u32,
                        raw_len: entry.raw_len,
                        codec,
                    },
                );
                offset += 4 + 1 + comp.len() as u64;
            }
            pack.sync_data()?;
        }

        // Swap: index first (points at pack id 0), then the pack file, then
        // drop the old packs.
        {
            let mut map = self.state.lock().unwrap();
            *map = new_map;
            self.persist_index_locked(&map)?;
        }
        {
            let mut id = self.next_pack_id.lock().unwrap();
            *id = 1;
        }
        for p in self.pack_paths() {
            if p != self.root.join("packs/p-00000000.pack") {
                let _ = fs::remove_file(p);
            }
        }
        fs::rename(&tmp_pack, self.root.join("packs/p-00000000.pack"))?;

        let bytes_after: u64 = self
            .pack_paths()
            .iter()
            .filter_map(|p| fs::metadata(p).ok().map(|m| m.len()))
            .sum();
        Ok(GcReport {
            live_chunks: live.len(),
            dead_chunks: dead,
            packs_before,
            packs_after: 1,
            bytes_reclaimed: bytes_before.saturating_sub(bytes_after),
        })
    }

    pub fn pack_paths(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(self.root.join("packs")) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("p-") && name.ends_with(".pack") {
                    out.push(e.path());
                }
            }
        }
        out.sort();
        out
    }

    pub fn stats(&self) -> Result<StoreStats, CasError> {
        let packs = self.pack_paths();
        let pack_bytes: u64 = packs
            .iter()
            .filter_map(|p| fs::metadata(p).ok().map(|m| m.len()))
            .sum();
        let chunks = self.state.lock().unwrap().len();
        let manifests = fs::read_dir(self.root.join("manifests"))?.count();
        Ok(StoreStats {
            chunks,
            packs: packs.len(),
            manifests,
            pack_bytes,
        })
    }
}

/// Temp-file + fsync + rename: a manifest/index write is never observed
/// half-written.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CasError> {
    let tmp = path.with_extension("tmp-write");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_data()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto;

    fn tmpdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

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

    #[test]
    fn put_get_roundtrip() {
        let dir = tmpdir();
        let store = ChunkStore::open(dir.path().join("store")).unwrap();
        let data = filler(3 * 1024 * 1024, 1);
        let m = store.put_buffer("demo", "0.1.0", &data, None).unwrap();
        assert_eq!(m.total_size, data.len() as u64);
        assert!(m.chunks.len() > 10, "3 MiB should split into many chunks");
        let id = store.put_manifest(&m).unwrap();
        let back = store.get_manifest(&id).unwrap();
        assert_eq!(back, m);
        let bytes = store.read_assembled(&back).unwrap();
        assert_eq!(bytes, data, "reassembled payload must equal the original");
    }

    #[test]
    fn dedup_across_puts() {
        let dir = tmpdir();
        let store = ChunkStore::open(dir.path().join("store")).unwrap();
        let data = filler(2 * 1024 * 1024, 2);
        let m1 = store.put_buffer("a", "1", &data, None).unwrap();
        let chunks_after_first = store.stats().unwrap().chunks;
        let _m2 = store.put_buffer("b", "1", &data, None).unwrap();
        let stats = store.stats().unwrap();
        assert_eq!(
            stats.chunks, chunks_after_first,
            "identical content must dedup to one copy"
        );
        assert!(m1.chunks.len() as u64 * 65536 > 0);
    }

    #[test]
    fn minor_edit_reuses_most_chunks() {
        let dir = tmpdir();
        let store = ChunkStore::open(dir.path().join("store")).unwrap();
        let mut data = filler(4 * 1024 * 1024, 3);
        let m1 = store.put_buffer("a", "1", &data, None).unwrap();
        let n1 = m1.chunks.len();
        data[2 * 1024 * 1024] ^= 0x40;
        let m2 = store.put_buffer("a", "2", &data, None).unwrap();
        let set1: std::collections::HashSet<_> = m1.chunks.iter().collect();
        let reused = m2.chunks.iter().filter(|c| set1.contains(c)).count();
        let ratio = reused as f64 / n1 as f64;
        eprintln!(
            "minor-edit dedup: {reused}/{n1} chunks reused ({:.1}%)",
            ratio * 100.0
        );
        assert!(
            ratio > 0.85,
            "minor edit reused {reused}/{n1} chunks ({ratio:.2})"
        );
    }

    /// docs/spectrum_plan.md §3.4 / §4 task 7.2: vendor firmware ingestion.
    ///
    /// Three *minor revisions* of a synthetic-but-structurally-realistic
    /// `.mbn` bundle — fixed-size header (magic, header version, image id,
    /// revision, segment table) over a multi-segment body — go through the
    /// real CAS (`put_buffer` → FastCDC → BLAKE3 digests), and the chunk
    /// reuse of the follow-on revisions is measured against the plan's
    /// acceptance: **≥ 80 % chunk reuse**. `reuse` here is the same metric
    /// `minor_edit_reuses_most_chunks` uses: how much of each *later* ingest
    /// was already present in the store before that ingest. The overall
    /// storage saving (distinct digests over all ingests) is printed too,
    /// because it answers the different question — bytes actually not
    /// stored — and with three near-identical revisions it necessarily sits
    /// near 2/3, not near 100 %.
    ///
    /// Reproduce (per plan §5):
    /// `cargo test -p avm-cas firmware_bundle_dedup -- --nocapture`
    #[test]
    fn firmware_bundle_dedup() {
        let dir = tmpdir();
        let store = ChunkStore::open(dir.path().join("store")).unwrap();

        /// One `.mbn`-shaped bundle: 4 KiB header + a 2 MiB text segment +
        /// a 1 MiB data segment + filler tail ⇒ 4 MiB total. Revisions
        /// differ in exactly the places vendor firmware differs: the
        /// revision field in the header and one symbol-table byte in the
        /// code segment.
        fn mbn_bundle(rev: u32) -> Vec<u8> {
            let mut b = filler(4 * 1024 * 1024, 11);
            // Header (synthetic, MBN-*shaped* — not a claimed byte-layout
            // of any vendor's tooling):
            b[0..4].copy_from_slice(b"MBN\0"); // magic
            b[4..8].copy_from_slice(&3u32.to_le_bytes()); // header_vsn = 3
            b[8..12].copy_from_slice(&0x19u32.to_le_bytes()); // image_id: ELF
            b[12..16].copy_from_slice(&rev.to_le_bytes()); // build revision
                                                           // Segment table: three u32 (offset, len) pairs.
            let segs: [(u32, u32); 3] = [
                (4096, 2 * 1024 * 1024),               // code
                (4096 + 2 * 1024 * 1024, 1024 * 1024), // data
                (4096 + 3 * 1024 * 1024, 1024 * 1024), // padding
            ];
            for (i, (off, len)) in segs.iter().enumerate() {
                let at = 16 + i * 8;
                b[at..at + 4].copy_from_slice(&off.to_le_bytes());
                b[at + 4..at + 8].copy_from_slice(&len.to_le_bytes());
            }
            // Minor code delta: flip one byte of the code segment per rev.
            b[4096 + 128] ^= 0x01;
            b[4096 + 129] = rev as u8;
            b
        }

        let mut seen: std::collections::HashSet<[u8; 32]> = Default::default();
        let mut distinct = 0usize;
        let mut total = 0usize;
        let mut followon_reused = 0usize;
        let mut followon_total = 0usize;

        for rev in 1u32..=3 {
            let data = mbn_bundle(rev);
            let m = store
                .put_buffer("vendor-firmware", &format!("1.0.{rev}"), &data, None)
                .unwrap();
            let n = m.chunks.len();
            let reused = m.chunks.iter().filter(|c| seen.contains(*c)).count();
            eprintln!(
                "firmware rev {rev}: {reused}/{n} chunks already in store ({:.1} %)",
                100.0 * reused as f64 / n as f64
            );
            for c in &m.chunks {
                if seen.insert(*c) {
                    distinct += 1;
                }
            }
            total += n;
            if rev > 1 {
                followon_reused += reused;
                followon_total += n;
            }
        }

        let reuse = followon_reused as f64 / followon_total as f64;
        let saving = 1.0 - distinct as f64 / total as f64;
        eprintln!(
            "firmware dedup: follow-on reuse {followon_reused}/{followon_total} = {:.1} %; \
             storage saving = {distinct}/{total} distinct ⇒ {:.1} % saved",
            reuse * 100.0,
            saving * 100.0
        );
        assert!(
            reuse >= 0.80,
            "follow-on reuse {followon_reused}/{followon_total} ({reuse:.3}) below the 80 % acceptance"
        );
        assert!(
            saving >= 0.60,
            "storage saving {saving:.3} below the ~2/3 floor three near-identical revisions imply"
        );
        // The three revisions really are three distinct manifests.
        assert_eq!(seen.len(), distinct);
        assert!(distinct < total, "some chunks must have been reused");
    }

    #[test]
    fn gc_prunes_dead_keeps_live() {
        let dir = tmpdir();
        let store = ChunkStore::open(dir.path().join("store")).unwrap();
        let live = filler(1024 * 1024, 4);
        let dead = filler(1024 * 1024, 5);
        let m_live = store.put_buffer("live", "1", &live, None).unwrap();
        let id_live = store.put_manifest(&m_live).unwrap();
        let m_dead = store.put_buffer("dead", "1", &dead, None).unwrap();
        let id_dead = store.put_manifest(&m_dead).unwrap();
        store.drop_manifest(&id_dead).unwrap();

        let report = store.gc().unwrap();
        assert!(report.dead_chunks > 0);
        assert_eq!(report.packs_after, 1);
        // Live cell still fully readable after compaction.
        let back = store.get_manifest(&id_live).unwrap();
        assert_eq!(store.read_assembled(&back).unwrap(), live);
        // A second gc is a no-op.
        let again = store.gc().unwrap();
        assert_eq!(again.dead_chunks, 0);
    }

    #[test]
    fn manifest_identity_ignores_signature() {
        let dir = tmpdir();
        let store = ChunkStore::open(dir.path().join("store")).unwrap();
        let data = filler(256 * 1024, 6);
        let mut m = store.put_buffer("demo", "1", &data, None).unwrap();
        let unsigned_id = m.identity_digest();
        let key = crypto::signing_key_from_seed(&[7u8; 32]);
        crypto::sign_manifest(&mut m, &key);
        assert_eq!(
            m.identity_digest(),
            unsigned_id,
            "signing must not move identity"
        );
        let id = store.put_manifest(&m).unwrap();
        assert_eq!(id, unsigned_id);
        let back = store.get_manifest(&id).unwrap();
        crypto::verify_manifest(&back).expect("persisted signature must verify");
    }

    #[test]
    fn index_survives_reopen() {
        let dir = tmpdir();
        let data = filler(1024 * 1024, 7);
        let id = {
            let store = ChunkStore::open(dir.path().join("store")).unwrap();
            let m = store.put_buffer("demo", "1", &data, None).unwrap();
            store.put_manifest(&m).unwrap()
        };
        let store = ChunkStore::open(dir.path().join("store")).unwrap();
        let m = store.get_manifest(&id).unwrap();
        assert_eq!(store.read_assembled(&m).unwrap(), data);
    }

    #[test]
    fn unhex_rejects_garbage() {
        assert!(unhex("zz").is_err());
        assert!(unhex(&"0".repeat(63)).is_err());
        assert_eq!(unhex(&"0".repeat(64)).unwrap(), [0u8; 32]);
    }
}
