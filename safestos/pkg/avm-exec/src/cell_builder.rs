//! The `.avm` cell builder (spec §3.4, task 3.2): parse the bridge's
//! `AVCE` container, serialize the relocatable `.avm` layout, verify it.
//!
//! Layout exactly as the spec's offset table:
//!
//! ```text
//! Offset    Size      Field
//! 0x00      4 bytes   Magic 0x41564D43 ('AVMC', little-endian numeric)
//! 0x04      2 bytes   Version 0x0001
//! 0x06      2 bytes   Target architecture (1 = x86_64, 2 = aarch64)
//! 0x08     32 bytes   Derivation input hash (BLAKE3)
//! 0x28      4 bytes   Capability table length N
//! 0x2C      N bytes   Cedar capability table (JSON)
//! 0x2C+N    4 bytes   Text-section length M (page-aligned to 0x1000)
//! …         M bytes   Machine code: function bodies concatenated and
//!                     zero-padded to the page boundary
//! …         K bytes   "Constant pool & relocation table":
//!                       u32 symdef_count, each { name, u64 text_off, u32 len }
//!                       u32 import_count,  each { name }
//!                       u32 reloc_count,   each { u64 off, i64 addend,
//!                                                  kind, symbol }
//! …        64 bytes   Ed25519 signature over every preceding byte
//! ```
//!
//! The spec names the K region but leaves its internals to the
//! implementation; the grammar above is this implementation's design,
//! recorded in PLAN_avm_pkg.md §4. Function entries are symbol *names*
//! (symdefs), matching invariant 5: cells link by name at load time.
//!
//! Relocation offsets are cell-text-relative: an `AVCE` reloc offset is
//! relative to its own function's body, and `build_cell` shifts it by the
//! function's text offset while laying the bodies out.

use ed25519_dalek::{Signer, VerifyingKey};
use std::fmt;

pub const AVMC_MAGIC: [u8; 4] = [0x41, 0x56, 0x4D, 0x43]; // read as LE u32 = 0x41564D43
pub const AVMC_VERSION: u16 = 1;
pub const PAGE: usize = 4096;
pub const SIG_LEN: usize = 64;

pub const ARCH_X86_64: u16 = 1;
pub const ARCH_AARCH64: u16 = 2;

/// Everything that can go wrong parsing, building, or linking a cell.
#[derive(Debug)]
pub enum CellError {
    Parse(String),
    Arch(String),
    Sig(String),
    Link(String),
    Io(std::io::Error),
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CellError::Parse(m) => write!(f, "cell parse: {m}"),
            CellError::Arch(m) => write!(f, "cell arch: {m}"),
            CellError::Sig(m) => write!(f, "cell signature: {m}"),
            CellError::Link(m) => write!(f, "cell link: {m}"),
            CellError::Io(e) => write!(f, "cell i/o: {e}"),
        }
    }
}

impl std::error::Error for CellError {}

impl From<std::io::Error> for CellError {
    fn from(e: std::io::Error) -> Self {
        CellError::Io(e)
    }
}

// ── AVCE container (bridge → cell builder wire format) ──────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvceReloc {
    pub offset: u64,
    pub addend: i64,
    pub kind: String,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvceFn {
    pub name: String,
    pub body: Vec<u8>,
    pub relocs: Vec<AvceReloc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvceDoc {
    pub arch: String,
    pub functions: Vec<AvceFn>,
    pub imports: Vec<String>,
}

/// Bounds-checked AVCE reader; every read is validated, and the parser
/// demands the cursor consume the container *exactly* (trailing bytes are
/// corruption, not slack).
struct R<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> R<'a> {
    fn new(b: &'a [u8]) -> Self {
        R { b, i: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], CellError> {
        let end = self
            .i
            .checked_add(n)
            .ok_or_else(|| CellError::Parse("length overflow".into()))?;
        if end > self.b.len() {
            return Err(CellError::Parse(format!(
                "needs {n} bytes at offset {}, only {} remain",
                self.i,
                self.b.len() - self.i
            )));
        }
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, CellError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, CellError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CellError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64v(&mut self) -> Result<i64, CellError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes_str(&mut self) -> Result<String, CellError> {
        let n = self.u32()? as usize;
        let raw = self.take(n)?;
        String::from_utf8(raw.to_vec())
            .map_err(|_| CellError::Parse("utf-8 in string field".into()))
    }
    fn done(&self) -> bool {
        self.i == self.b.len()
    }
}

/// Parse the `AVCE` container a `cps2cell` run produced.
pub fn parse_avce(bytes: &[u8]) -> Result<AvceDoc, CellError> {
    let mut r = R::new(bytes);
    let magic = r.u32()?;
    if magic != 0x4156_4345 {
        return Err(CellError::Parse(format!("bad AVCE magic {magic:#010x}")));
    }
    let version = r.u16()?;
    if version != 1 {
        return Err(CellError::Parse(format!(
            "AVCE version {version} unsupported"
        )));
    }
    let arch = r.bytes_str()?;
    let nfuncs = r.u32()? as usize;
    if nfuncs == 0 {
        return Err(CellError::Parse("container defines no functions".into()));
    }
    let mut functions = Vec::with_capacity(nfuncs.min(1 << 20));
    for _ in 0..nfuncs {
        let name = r.bytes_str()?;
        let blen = r.u32()? as usize;
        let body = r.take(blen)?.to_vec();
        if body.is_empty() {
            return Err(CellError::Parse(format!("function {name} has empty body")));
        }
        let nrel = r.u32()? as usize;
        if nrel > 1 << 20 {
            return Err(CellError::Parse("relocation count absurd".into()));
        }
        let mut relocs = Vec::with_capacity(nrel);
        for _ in 0..nrel {
            let offset = r.u64()?;
            let addend = r.i64v()?;
            let kind = r.bytes_str()?;
            let symbol = r.bytes_str()?;
            if offset as usize >= body.len() {
                return Err(CellError::Parse(format!(
                    "reloc offset {offset} outside function {name} ({} bytes)",
                    body.len()
                )));
            }
            relocs.push(AvceReloc {
                offset,
                addend,
                kind,
                symbol,
            });
        }
        functions.push(AvceFn { name, body, relocs });
    }
    let nimports = r.u32()? as usize;
    let mut imports = Vec::with_capacity(nimports.min(1 << 20));
    for _ in 0..nimports {
        imports.push(r.bytes_str()?);
    }
    if !r.done() {
        return Err(CellError::Parse(format!(
            "{} trailing bytes after AVCE body",
            bytes.len() - r.i
        )));
    }
    Ok(AvceDoc {
        arch,
        functions,
        imports,
    })
}

// ── .avm view (parse / verify) ──────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymDef {
    pub name: String,
    pub offset: u64,
    pub len: u32,
}

/// A parsed `.avm` cell borrowing the underlying bytes.
#[derive(Debug, Clone)]
pub struct CellView<'a> {
    pub version: u16,
    pub arch: u16,
    pub drv_hash: [u8; 32],
    pub capability_table: serde_json::Value,
    pub text: &'a [u8],
    pub symdefs: Vec<SymDef>,
    pub imports: Vec<String>,
    pub relocs: Vec<AvceReloc>,
    pub signature: [u8; SIG_LEN],
    /// All bytes covered by the signature (everything before the trailer).
    pub signed: &'a [u8],
}

fn arch_code(name: &str) -> Result<u16, CellError> {
    match name {
        "x86_64" | "x86-64" => Ok(ARCH_X86_64),
        "aarch64" | "arm64" => Ok(ARCH_AARCH64),
        other => Err(CellError::Arch(format!(
            "cannot put arch {other:?} into a cell header (known: x86_64, aarch64)"
        ))),
    }
}

fn arch_name(code: u16) -> Result<&'static str, CellError> {
    match code {
        ARCH_X86_64 => Ok("x86_64"),
        ARCH_AARCH64 => Ok("aarch64"),
        c => Err(CellError::Arch(format!("unknown arch code {c}"))),
    }
}

impl<'a> CellView<'a> {
    /// Full structural parse: header, capability table, text, K region,
    /// signature trailer — consumed exactly, checked at every step.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, CellError> {
        if bytes.len() < 0x2C + SIG_LEN + 4 {
            return Err(CellError::Parse("cell far too short".into()));
        }
        let mut r = R::new(bytes);
        let magic = r.take(4)?;
        if magic != AVMC_MAGIC {
            return Err(CellError::Parse(format!("bad AVMC magic {magic:?}")));
        }
        let version = r.u16()?;
        if version != AVMC_VERSION {
            return Err(CellError::Parse(format!("version {version} unsupported")));
        }
        let arch = r.u16()?;
        arch_name(arch)?; // unknown arch codes are a hard error
        let drv_hash: [u8; 32] = r.take(32)?.try_into().unwrap();
        let cap_len = r.u32()? as usize;
        if cap_len > 16 * 1024 * 1024 {
            return Err(CellError::Parse("capability table > 16 MiB".into()));
        }
        let cap_bytes = r.take(cap_len)?;
        let capability_table: serde_json::Value = serde_json::from_slice(cap_bytes)
            .map_err(|e| CellError::Parse(format!("capability table is not valid JSON: {e}")))?;
        let m = r.u32()? as usize;
        if m == 0 || !m.is_multiple_of(PAGE) {
            return Err(CellError::Parse(format!(
                "text length {m} must be a positive page multiple"
            )));
        }
        if m > 1 << 30 {
            return Err(CellError::Parse("text > 1 GiB".into()));
        }
        let text = r.take(m)?;

        // K region: everything up to the 64-byte signature trailer.
        let k_end = bytes.len() - SIG_LEN;
        if r.i > k_end {
            return Err(CellError::Parse("cell truncated before signature".into()));
        }
        let nsym = r.u32()? as usize;
        if nsym > 1 << 20 {
            return Err(CellError::Parse("symdef count absurd".into()));
        }
        let mut symdefs = Vec::with_capacity(nsym.min(4096));
        for _ in 0..nsym {
            let name = r.bytes_str()?;
            let offset = r.u64()?;
            let len = r.u32()?;
            if offset
                .checked_add(u64::from(len))
                .is_none_or(|e| e > m as u64)
            {
                return Err(CellError::Parse(format!(
                    "symdef {name} runs past the text section"
                )));
            }
            symdefs.push(SymDef { name, offset, len });
        }
        let nimp = r.u32()? as usize;
        if nimp > 1 << 20 {
            return Err(CellError::Parse("import count absurd".into()));
        }
        let mut imports = Vec::with_capacity(nimp.min(4096));
        for _ in 0..nimp {
            imports.push(r.bytes_str()?);
        }
        let nrel = r.u32()? as usize;
        if nrel > 1 << 20 {
            return Err(CellError::Parse("reloc count absurd".into()));
        }
        let mut relocs = Vec::with_capacity(nrel.min(4096));
        for _ in 0..nrel {
            let offset = r.u64()?;
            let addend = r.i64v()?;
            let kind = r.bytes_str()?;
            let symbol = r.bytes_str()?;
            if offset as usize >= m {
                return Err(CellError::Parse(format!(
                    "reloc offset {offset} outside text ({m} bytes)"
                )));
            }
            relocs.push(AvceReloc {
                offset,
                addend,
                kind,
                symbol,
            });
        }
        if r.i != k_end {
            return Err(CellError::Parse(format!(
                "K region did not end at the signature: cursor {} vs {k_end}",
                r.i
            )));
        }
        let signed = &bytes[..k_end];
        let signature: [u8; SIG_LEN] = bytes[k_end..].try_into().unwrap();
        Ok(CellView {
            version,
            arch,
            drv_hash,
            capability_table,
            text,
            symdefs,
            imports,
            relocs,
            signature,
            signed,
        })
    }

    pub fn arch_str(&self) -> Result<&'static str, CellError> {
        arch_name(self.arch)
    }

    pub fn symdef(&self, name: &str) -> Option<&SymDef> {
        self.symdefs.iter().find(|s| s.name == name)
    }

    /// Real code length (furthest symdef end); falls back to the page-
    /// padded text length for the degenerate no-symdef cell.
    pub fn code_len(&self) -> usize {
        self.symdefs
            .iter()
            .map(|s| (s.offset + u64::from(s.len)) as usize)
            .max()
            .unwrap_or(self.text.len())
    }

    /// Verify the trailing Ed25519 signature against a builder key.
    /// Fail-closed: an unsigned or mis-signed cell is refused.
    pub fn verify_signature(&self, verifying_key: &[u8; 32]) -> Result<(), CellError> {
        let vk = VerifyingKey::from_bytes(verifying_key)
            .map_err(|e| CellError::Sig(format!("bad verifying key: {e}")))?;
        use ed25519_dalek::Signature;
        let sig = Signature::from_bytes(&self.signature);
        vk.verify_strict(self.signed, &sig)
            .map_err(|e| CellError::Sig(format!("signature check failed: {e}")))
    }
}

// ── build ───────────────────────────────────────────────────────────

/// Everything needed to serialize one `.avm` cell.
pub struct BuildCell<'a> {
    pub drv_hash: [u8; 32],
    /// Cedar capability table: must serialize to a JSON array or object.
    pub capability_table: &'a serde_json::Value,
    pub avce: &'a AvceDoc,
    /// Builder's Ed25519 seed; the 64-byte signature goes last.
    pub signing_seed: &'a [u8; 32],
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(&(s.len() as u32).to_le_bytes());
    b.extend_from_slice(s.as_bytes());
}

/// Serialize the §3.4 layout. Page-aligns the concatenated text, shifts
/// each `AVCE` reloc to a cell-text offset, and signs every preceding
/// byte with the builder key.
pub fn build_cell(spec: &BuildCell<'_>) -> Result<Vec<u8>, CellError> {
    if spec.avce.functions.is_empty() {
        return Err(CellError::Parse("no functions to put in the cell".into()));
    }
    let arch = arch_code(&spec.avce.arch)?;
    let cap_bytes = serde_json::to_vec(spec.capability_table)
        .map_err(|e| CellError::Parse(format!("capability table not serializable: {e}")))?;
    if cap_bytes.len() > 16 * 1024 * 1024 {
        return Err(CellError::Parse("capability table > 16 MiB".into()));
    }

    // Layout the text: bodies concatenated, offsets tracked for symdefs
    // and for shifting function-relative relocs to cell-relative.
    let mut text: Vec<u8> = Vec::new();
    let mut symdefs: Vec<SymDef> = Vec::new();
    let mut relocs: Vec<AvceReloc> = Vec::new();
    for f in &spec.avce.functions {
        let off = text.len();
        text.extend_from_slice(&f.body);
        symdefs.push(SymDef {
            name: f.name.clone(),
            offset: off as u64,
            len: f.body.len() as u32,
        });
        for r in &f.relocs {
            let shifted = r.offset + off as u64;
            if shifted as usize >= text.len() {
                return Err(CellError::Parse(format!(
                    "reloc offset {shifted} outside laid-out text"
                )));
            }
            relocs.push(AvceReloc {
                offset: shifted,
                addend: r.addend,
                kind: r.kind.clone(),
                symbol: r.symbol.clone(),
            });
        }
    }
    let real = text.len();
    let m = real
        .checked_add(PAGE - 1)
        .map(|v| v / PAGE * PAGE)
        .ok_or_else(|| CellError::Parse("text length overflow".into()))?;
    if m == 0 {
        return Err(CellError::Parse("empty text".into()));
    }
    text.resize(m, 0);

    let mut b: Vec<u8> = Vec::with_capacity(m + cap_bytes.len() + 4096);
    b.extend_from_slice(&AVMC_MAGIC);
    b.extend_from_slice(&AVMC_VERSION.to_le_bytes());
    b.extend_from_slice(&arch.to_le_bytes());
    b.extend_from_slice(&spec.drv_hash);
    b.extend_from_slice(&(cap_bytes.len() as u32).to_le_bytes());
    b.extend_from_slice(&cap_bytes);
    b.extend_from_slice(&(m as u32).to_le_bytes());
    b.extend_from_slice(&text);

    // K region.
    b.extend_from_slice(&(symdefs.len() as u32).to_le_bytes());
    for s in &symdefs {
        put_str(&mut b, &s.name);
        b.extend_from_slice(&s.offset.to_le_bytes());
        b.extend_from_slice(&s.len.to_le_bytes());
    }
    b.extend_from_slice(&(spec.avce.imports.len() as u32).to_le_bytes());
    for name in &spec.avce.imports {
        put_str(&mut b, name);
    }
    b.extend_from_slice(&(relocs.len() as u32).to_le_bytes());
    for r in &relocs {
        b.extend_from_slice(&r.offset.to_le_bytes());
        b.extend_from_slice(&r.addend.to_le_bytes());
        put_str(&mut b, &r.kind);
        put_str(&mut b, &r.symbol);
    }

    // Signature trailer: Ed25519 over everything written so far.
    let key = ed25519_dalek::SigningKey::from_bytes(spec.signing_seed);
    let sig = key.sign(&b).to_bytes();
    b.extend_from_slice(&sig);
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn avce_x86_single() -> AvceDoc {
        // `mov eax, 7; ret`
        AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "run".into(),
                body: vec![0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3],
                relocs: vec![],
            }],
            imports: vec![],
        }
    }

    fn cap_table() -> serde_json::Value {
        // Cedar entities-JSON shape — the runner parses exactly this.
        serde_json::json!([
            { "uid": { "type": "NetworkCap", "id": "net-1" }, "attrs": {}, "parents": [] },
            { "uid": { "type": "ExecCap", "id": "exec-1" }, "attrs": {}, "parents": [] }
        ])
    }

    #[test]
    fn avce_roundtrip_and_bounds() {
        let doc = avce_x86_single();
        // Encode (mirror of capture.rs encode_avce) then parse.
        let mut b = Vec::new();
        b.extend_from_slice(&0x4156_4345u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        put_str(&mut b, &doc.arch);
        b.extend_from_slice(&(doc.functions.len() as u32).to_le_bytes());
        for f in &doc.functions {
            put_str(&mut b, &f.name);
            b.extend_from_slice(&(f.body.len() as u32).to_le_bytes());
            b.extend_from_slice(&f.body);
            b.extend_from_slice(&(f.relocs.len() as u32).to_le_bytes());
            for r in &f.relocs {
                b.extend_from_slice(&r.offset.to_le_bytes());
                b.extend_from_slice(&r.addend.to_le_bytes());
                put_str(&mut b, &r.kind);
                put_str(&mut b, &r.symbol);
            }
        }
        b.extend_from_slice(&(doc.imports.len() as u32).to_le_bytes());
        let parsed = parse_avce(&b).unwrap();
        assert_eq!(&parsed, &doc, "byte-exact round trip");

        // Truncation and trailing garbage are both hard errors.
        for cut in [0, 3, 12, 27, b.len() - 1] {
            assert!(parse_avce(&b[..cut]).is_err(), "cut at {cut} must fail");
        }
        let mut longer = b.clone();
        longer.push(0);
        assert!(parse_avce(&longer).is_err(), "trailing byte must fail");
    }

    #[test]
    fn build_and_verify_cell_layout() {
        let doc = avce_x86_single();
        let seed = [7u8; 32];
        let drv = [9u8; 32];
        let cell = build_cell(&BuildCell {
            drv_hash: drv,
            capability_table: &cap_table(),
            avce: &doc,
            signing_seed: &seed,
        })
        .unwrap();

        // Exact spec offsets: magic, version, arch.
        assert_eq!(&cell[0..4], &AVMC_MAGIC, "0x00 magic");
        assert_eq!(
            u16::from_le_bytes(cell[4..6].try_into().unwrap()),
            1,
            "0x04 version"
        );
        assert_eq!(
            u16::from_le_bytes(cell[6..8].try_into().unwrap()),
            ARCH_X86_64,
            "0x06 arch"
        );
        assert_eq!(&cell[8..40], &drv, "0x08 drv hash");
        let cap_len = u32::from_le_bytes(cell[40..44].try_into().unwrap()) as usize;
        assert_eq!(
            &cell[44..44 + cap_len],
            serde_json::to_vec(&cap_table()).unwrap().as_slice(),
            "0x2C capability table"
        );
        let m_off = 44 + cap_len;
        let m = u32::from_le_bytes(cell[m_off..m_off + 4].try_into().unwrap()) as usize;
        assert_eq!(m % PAGE, 0, "text page-aligned");
        assert!(m >= 6, "text holds the body");
        let text_start = m_off + 4;
        assert_eq!(
            &cell[text_start..text_start + 6],
            &[0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3],
            "body at text start"
        );
        assert_eq!(
            cell[text_start + 6..text_start + m],
            vec![0u8; m - 6],
            "pad to page is zero"
        );
        assert!(
            cell.len() >= text_start + m + SIG_LEN,
            "cell holds text and the 64-byte signature trailer"
        );

        let view = CellView::parse(&cell).unwrap();
        assert_eq!(view.drv_hash, drv);
        assert_eq!(view.arch_str().unwrap(), "x86_64");
        assert_eq!(view.capability_table, cap_table());
        assert_eq!(view.code_len(), 6);
        assert_eq!(view.symdef("run").unwrap().offset, 0);
        view.verify_signature(
            &ed25519_dalek::SigningKey::from_bytes(&seed)
                .verifying_key()
                .to_bytes(),
        )
        .unwrap();

        // Tampering anywhere in the signed range must fail verification.
        let mut bad = cell.clone();
        bad[40] ^= 1; // flip a capability-length byte
                      // (structure may now fail to parse — either outcome is a refusal)
        match CellView::parse(&bad) {
            Err(_) => {}
            Ok(v) => assert!(
                v.verify_signature(
                    &ed25519_dalek::SigningKey::from_bytes(&seed)
                        .verifying_key()
                        .to_bytes()
                )
                .is_err(),
                "tampered cell must not verify"
            ),
        }
        // Wrong key must fail.
        let other = ed25519_dalek::SigningKey::from_bytes(&[8u8; 32])
            .verifying_key()
            .to_bytes();
        assert!(view.verify_signature(&other).is_err(), "wrong key refused");

        // Truncated / corrupted structures are refused by the parser.
        for cut in [10, 41, 45, cell.len() - 65] {
            assert!(
                CellView::parse(&cell[..cut]).is_err(),
                "cut {cut} must fail"
            );
        }
    }

    #[test]
    fn parse_rejects_unknown_arch() {
        let doc = avce_x86_single();
        let seed = [1u8; 32];
        let mut cell = build_cell(&BuildCell {
            drv_hash: [0u8; 32],
            capability_table: &serde_json::json!([]),
            avce: &doc,
            signing_seed: &seed,
        })
        .unwrap();
        // Corrupt arch to a bogus code and re-sign (so only arch is wrong).
        cell[6] = 0x09;
        cell[7] = 0x00;
        let sig_off = cell.len() - SIG_LEN;
        let sig = {
            let key = ed25519_dalek::SigningKey::from_bytes(&seed);
            key.sign(&cell[..sig_off]).to_bytes()
        };
        cell[sig_off..].copy_from_slice(&sig);
        let err = CellView::parse(&cell).unwrap_err();
        assert!(
            matches!(err, CellError::Arch(_)),
            "unknown arch refused: {err}"
        );
    }
}
