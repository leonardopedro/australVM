//! PC10: the GPU virtual/native boundary, made an enforceable policy.
//!
//! Every artifact the GPU branch publishes so far is *virtual*: WGSL is a
//! source language, PTX is a portable ISA the driver translates further, and
//! neither is the bytes the device executes. The proof chain must not let a
//! virtual artifact stand in for a native one, so the boundary is a policy with
//! a total order ([`Tier`]) and a refusal at every step below it:
//!
//! * [`Tier`] — `wgsl < spirv < ptx < native`. A certificate names the tier of
//!   the bytes it is about; a `native` policy is satisfied only by a `native`
//!   certificate.
//! * [`BoundaryRecord`] — what one artifact claims: its tier, the digest of the
//!   bytes at that tier, the ABI it was checked against, and (for native) the
//!   virtual subject it was derived from plus the device and blob facts.
//! * [`BoundaryRecord::verify`] — refuses a record below the policy, a native
//!   record whose bytes are absent (an *uncaptured* blob), bytes whose digest is
//!   not the recorded one, and a blob whose container no longer matches what was
//!   identified.
//!
//! What `identify` is: an ELF container parse — class, endianness, `e_machine`,
//! and the section table with digests — plus the byte count of the `.text.*`
//! sections. That is *identification* of the actual native bytes, not a
//! semantics proof: it says this blob is a CUDA executable with this much SASS,
//! never what the SASS computes. A disassembly listing is text about bytes; it
//! is not the instruction semantics, and this module does not pretend otherwise
//! ([`LIMITS`]).

use serde::{Deserialize, Serialize};

/// The `e_machine` value CUDA executables carry.
pub const EM_CUDA: u16 = 190;

/// What this boundary does and does not establish, quoted in outputs so a
/// reader never has to infer it.
pub const LIMITS: &str = "identifies the native container (ELF64/EM_CUDA, .text.* byte count, digests); \
it does not decode SASS and does not relate it to the PTX operand, memory or ABI semantics";

/// How concrete an artifact is. Ordered: a higher tier is the only thing that
/// satisfies a policy at that tier or below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// WGSL source text (the G09/G10 emission path).
    Wgsl,
    /// SPIR-V words, where a backend produces them.
    Spirv,
    /// PTX text: a virtual ISA the driver still translates.
    Ptx,
    /// An executable container the driver loaded for this device.
    Native,
}

impl Tier {
    pub fn parse(text: &str) -> Option<Tier> {
        match text {
            "wgsl" => Some(Tier::Wgsl),
            "spirv" => Some(Tier::Spirv),
            "ptx" => Some(Tier::Ptx),
            "native" => Some(Tier::Native),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Wgsl => "wgsl",
            Tier::Spirv => "spirv",
            Tier::Ptx => "ptx",
            Tier::Native => "native",
        }
    }

    pub fn is_virtual(self) -> bool {
        self != Tier::Native
    }
}

/// The ABI the record was checked against: the entry, the parameter count and
/// the launch geometry. Two artifacts with different values here are different
/// subjects even when their bytes agree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Abi {
    pub entry: String,
    pub params: usize,
    pub extent: usize,
    pub workgroup: usize,
    pub block: usize,
    pub grid: usize,
}

/// The device the native blob was produced for and loaded by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFacts {
    pub name: String,
    pub capability: String,
    pub chip: String,
    pub driver_version: i32,
}

/// One section of the native container, with its own digest so a mutation
/// inside `.text` is visible without disassembling anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub name: String,
    pub size: usize,
    pub sha256: String,
}

/// What the bytes are, as far as the container says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub file: String,
    pub size: usize,
    /// `elf64-little` today; the field exists so a different container is a
    /// recorded fact rather than an unstated assumption.
    pub format: String,
    /// `cuda` for `EM_CUDA`; identification refuses anything else.
    pub machine: String,
    /// Bytes of all `.text.*` sections: the SASS this blob will execute.
    pub sass_bytes: usize,
    pub sections: Vec<Section>,
}

/// One artifact's claim about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundaryRecord {
    pub schema: String,
    pub entry: String,
    pub tier: Tier,
    /// Digest of the bytes at `tier`: the PTX text, or the native blob.
    pub artifact_sha256: String,
    /// For a native record: the virtual subject it was linked from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ptx_sha256: Option<String>,
    /// For a virtual record: why no native artifact exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_gap: Option<String>,
    pub abi: Abi,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<Blob>,
    pub limits: String,
}

pub const SCHEMA: &str = "austral.gpu.boundary.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The artifact is less concrete than the policy requires.
    BelowPolicy { required: Tier, found: Tier },
    /// A native claim with no bytes to identify.
    Uncaptured { reason: String },
    /// The bytes are not the recorded ones.
    Identity { recorded: String, got: String },
    /// The bytes are not a native container this module can identify.
    Unidentifiable { reason: String },
    /// The container agrees on identity but disagrees on what it holds.
    BlobDrift { field: String, recorded: String, got: String },
    /// A native record must carry the virtual subject it was linked from.
    NoSubject,
    UnknownTier { text: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::BelowPolicy { required, found } => write!(
                f,
                "a {} artifact cannot satisfy a {} policy: {} is a virtual ISA, and only \
                 bytes the device executes satisfy a native requirement",
                found.as_str(),
                required.as_str(),
                found.as_str()
            ),
            Refusal::Uncaptured { reason } => write!(
                f,
                "the native blob is uncaptured ({reason}); a native claim without its bytes is \
                 an unproved edge, not a proof"
            ),
            Refusal::Identity { recorded, got } => write!(
                f,
                "the blob digest is {got}, the record names {recorded}: these are not the same \
                 bytes"
            ),
            Refusal::Unidentifiable { reason } => {
                write!(f, "the bytes are not an identifiable native container: {reason}")
            }
            Refusal::BlobDrift { field, recorded, got } => write!(
                f,
                "the blob's {field} is {got}, the record identified {recorded}"
            ),
            Refusal::NoSubject => write!(
                f,
                "a native record names no PTX digest: the blob would float free of the virtual \
                 artifact the chain vouches for"
            ),
            Refusal::UnknownTier { text } => write!(
                f,
                "unknown tier {text:?} (expected wgsl, spirv, ptx or native)"
            ),
        }
    }
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// Identify the native container: ELF64 little-endian, `EM_CUDA`, a section
/// table that names at least one `.text.*` section with bytes in it.
///
/// This is the whole native claim `scripts/gpu/boundary` can make without a
/// decoder: the container is real, it is a CUDA executable for the chip the
/// driver reported, and its SASS digest is recorded. The plan's
/// "do not use disassembly text alone as a semantics proof" is why this stops
/// at the container — see [`LIMITS`].
pub fn identify(file: &str, bytes: &[u8]) -> Result<Blob, Refusal> {
    let bad = |reason: String| Refusal::Unidentifiable { reason };
    if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" {
        return Err(bad("no ELF magic".to_string()));
    }
    if bytes[4] != 2 {
        return Err(bad(format!("ELF class {} is not 64-bit", bytes[4])));
    }
    if bytes[5] != 1 {
        return Err(bad(format!("ELF data encoding {} is not little-endian", bytes[5])));
    }
    let machine = read_u16(bytes, 18).ok_or_else(|| bad("truncated ELF header".to_string()))?;
    if machine != EM_CUDA {
        return Err(bad(format!("e_machine is {machine}, not {EM_CUDA} (CUDA)")));
    }
    let shoff = read_u64(bytes, 40).ok_or_else(|| bad("truncated ELF header".to_string()))? as usize;
    let shentsize = read_u16(bytes, 58).ok_or_else(|| bad("truncated ELF header".to_string()))? as usize;
    let shnum = read_u16(bytes, 60).ok_or_else(|| bad("truncated ELF header".to_string()))? as usize;
    let shstrndx = read_u16(bytes, 62).ok_or_else(|| bad("truncated ELF header".to_string()))? as usize;
    if shentsize < 64 || shnum == 0 {
        return Err(bad("no section table".to_string()));
    }
    let section = |index: usize| -> Option<(u32, u32, usize, usize)> {
        let base = shoff.checked_add(index.checked_mul(shentsize)?)?;
        let name = read_u32(bytes, base)?;
        let kind = read_u32(bytes, base + 4)?;
        let offset = read_u64(bytes, base + 24)? as usize;
        let size = read_u64(bytes, base + 32)? as usize;
        Some((name, kind, offset, size))
    };
    let names = section(shstrndx).ok_or_else(|| bad("no section-name table".to_string()))?;
    let names = bytes
        .get(names.2..names.2.checked_add(names.3).ok_or_else(|| bad("truncated names".to_string()))?)
        .ok_or_else(|| bad("truncated section-name table".to_string()))?;
    let name_at = |offset: u32| -> String {
        let start = offset as usize;
        let rest = match names.get(start..) {
            Some(r) => r,
            None => return String::new(),
        };
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        String::from_utf8_lossy(&rest[..end]).into_owned()
    };

    let mut sections = Vec::new();
    let mut sass_bytes = 0usize;
    for index in 0..shnum {
        let (name, kind, offset, size) = match section(index) {
            Some(s) => s,
            None => break,
        };
        let name = name_at(name);
        // Only PROGBITS carries bytes; a section table entry for NOBITS (.bss-
        // style) must not be counted as executable text.
        if kind != 1 || size == 0 {
            continue;
        }
        let body = match bytes.get(offset..offset.saturating_add(size)) {
            Some(b) => b,
            None => return Err(bad(format!("section {name:?} runs past the file"))),
        };
        if name.starts_with(".text") {
            sass_bytes += size;
        }
        sections.push(Section {
            name,
            size,
            sha256: sha256_hex(body),
        });
    }
    if sass_bytes == 0 {
        return Err(bad("the container holds no `.text.*` bytes to execute".to_string()));
    }
    Ok(Blob {
        file: file.to_string(),
        size: bytes.len(),
        format: "elf64-little".to_string(),
        machine: "cuda".to_string(),
        sass_bytes,
        sections,
    })
}

impl BoundaryRecord {
    /// The record for a virtual artifact: the tier says what the bytes are,
    /// and `native_gap` says why no native artifact exists yet.
    pub fn virtual_artifact(
        entry: &str,
        tier: Tier,
        artifact_sha256: String,
        abi: Abi,
        native_gap: String,
    ) -> BoundaryRecord {
        BoundaryRecord {
            schema: SCHEMA.to_string(),
            entry: entry.to_string(),
            tier,
            artifact_sha256,
            ptx_sha256: None,
            native_gap: Some(native_gap),
            abi,
            device: None,
            blob: None,
            limits: LIMITS.to_string(),
        }
    }

    /// Check the record against a policy tier. `bytes` is the blob content for
    /// a native record (the caller reads the file); a virtual record needs no
    /// bytes, because its claim is that nothing native exists.
    pub fn verify(&self, required: Tier, bytes: Option<&[u8]>) -> Result<(), Refusal> {
        if self.tier < required {
            return Err(Refusal::BelowPolicy {
                required,
                found: self.tier,
            });
        }
        if self.tier != Tier::Native {
            return Ok(());
        }
        let subject = self.ptx_sha256.as_deref().ok_or(Refusal::NoSubject)?;
        if subject.len() != 64 {
            return Err(Refusal::NoSubject);
        }
        let bytes = bytes.ok_or_else(|| Refusal::Uncaptured {
            reason: "no bytes were supplied for the recorded blob".to_string(),
        })?;
        let got = sha256_hex(bytes);
        if got != self.artifact_sha256 {
            return Err(Refusal::Identity {
                recorded: self.artifact_sha256.clone(),
                got,
            });
        }
        let recorded = self.blob.as_ref().ok_or_else(|| Refusal::Uncaptured {
            reason: "the record names no blob facts".to_string(),
        })?;
        let identified = identify(&recorded.file, bytes)?;
        let compare = |field: &str, a: String, b: String| -> Result<(), Refusal> {
            if a == b {
                Ok(())
            } else {
                Err(Refusal::BlobDrift {
                    field: field.to_string(),
                    recorded: a,
                    got: b,
                })
            }
        };
        compare("size", recorded.size.to_string(), identified.size.to_string())?;
        compare("machine", recorded.machine.clone(), identified.machine)?;
        compare("sass_bytes", recorded.sass_bytes.to_string(), identified.sass_bytes.to_string())?;
        if recorded.sections.len() != identified.sections.len() {
            return Err(Refusal::BlobDrift {
                field: "section count".to_string(),
                recorded: recorded.sections.len().to_string(),
                got: identified.sections.len().to_string(),
            });
        }
        for (a, b) in recorded.sections.iter().zip(&identified.sections) {
            compare("section name", a.name.clone(), b.name.clone())?;
            compare("section size", a.size.to_string(), b.size.to_string())?;
            compare(&format!("section {} digest", a.name), a.sha256.clone(), b.sha256.clone())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abi() -> Abi {
        Abi {
            entry: "main".to_string(),
            params: 15,
            extent: 65,
            workgroup: 64,
            block: 64,
            grid: 2,
        }
    }

    fn ptx_record() -> BoundaryRecord {
        BoundaryRecord::virtual_artifact(
            "add",
            Tier::Ptx,
            "a".repeat(64),
            abi(),
            "the lowering stops at PTX text".to_string(),
        )
    }

    /// A minimal ELF64/EM_CUDA container: one `.text` section and a name table.
    /// Shape only — these bytes are not instructions and are never executed.
    fn container(text: &[u8]) -> Vec<u8> {
        let names: &[u8] = b"\0.text\0.shstrtab\0";
        let shentsize = 64usize;
        let shnum = 3usize;
        let shoff = 64usize;
        let names_off = shoff + shentsize * shnum;
        let text_off = names_off + names.len();
        // The name table and the text go at the offsets the section table
        // records; nothing is written twice.
        let mut out = vec![0u8; names_off];
        out[..4].copy_from_slice(b"\x7fELF");
        out[4] = 2;
        out[5] = 1;
        out[16..18].copy_from_slice(&1u16.to_le_bytes()); // ET_REL
        out[18..20].copy_from_slice(&EM_CUDA.to_le_bytes());
        out[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
        out[58..60].copy_from_slice(&(shentsize as u16).to_le_bytes());
        out[60..62].copy_from_slice(&(shnum as u16).to_le_bytes());
        out[62..64].copy_from_slice(&2u16.to_le_bytes()); // shstrndx
        let mut put = |index: usize, name: u32, kind: u32, off: usize, size: usize| {
            let base = shoff + index * shentsize;
            out[base..base + 4].copy_from_slice(&name.to_le_bytes());
            out[base + 4..base + 8].copy_from_slice(&kind.to_le_bytes());
            out[base + 24..base + 32].copy_from_slice(&(off as u64).to_le_bytes());
            out[base + 32..base + 40].copy_from_slice(&(size as u64).to_le_bytes());
        };
        put(0, 0, 0, 0, 0);
        put(1, 1, 1, text_off, text.len());
        put(2, 7, 3, names_off, names.len());
        out.extend_from_slice(names);
        assert_eq!(out.len(), text_off);
        out.extend_from_slice(text);
        out
    }

    fn native_record(bytes: &[u8]) -> BoundaryRecord {
        let blob = identify("native.cubin", bytes).expect("identify");
        BoundaryRecord {
            schema: SCHEMA.to_string(),
            entry: "add".to_string(),
            tier: Tier::Native,
            artifact_sha256: sha256_hex(bytes),
            ptx_sha256: Some("b".repeat(64)),
            native_gap: None,
            abi: abi(),
            device: Some(DeviceFacts {
                name: "test".to_string(),
                capability: "6.1".to_string(),
                chip: "sm_61".to_string(),
                driver_version: 13000,
            }),
            blob: Some(blob),
            limits: LIMITS.to_string(),
        }
    }

    #[test]
    fn tiers_are_ordered_and_parse_both_ways() {
        assert!(Tier::Wgsl < Tier::Spirv);
        assert!(Tier::Spirv < Tier::Ptx);
        assert!(Tier::Ptx < Tier::Native);
        for tier in [Tier::Wgsl, Tier::Spirv, Tier::Ptx, Tier::Native] {
            assert_eq!(Tier::parse(tier.as_str()), Some(tier));
            assert!(tier.is_virtual() == (tier != Tier::Native));
        }
        assert_eq!(Tier::parse("sass"), None);
    }

    #[test]
    fn a_ptx_record_cannot_satisfy_a_native_policy() {
        let record = ptx_record();
        assert!(record.verify(Tier::Ptx, None).is_ok());
        let err = record.verify(Tier::Native, None).unwrap_err();
        assert!(matches!(
            err,
            Refusal::BelowPolicy { required: Tier::Native, found: Tier::Ptx }
        ));
        // The message names the consequence, not just the numbers.
        let text = err.to_string();
        assert!(text.contains("cannot satisfy a native policy"), "{text}");
        assert!(text.contains("virtual ISA"), "{text}");
    }

    #[test]
    fn a_wgsl_record_cannot_satisfy_a_ptx_policy_either() {
        let record = BoundaryRecord::virtual_artifact(
            "add",
            Tier::Wgsl,
            "c".repeat(64),
            abi(),
            "WGSL is the source the emitter publishes first".to_string(),
        );
        assert!(matches!(
            record.verify(Tier::Ptx, None),
            Err(Refusal::BelowPolicy { required: Tier::Ptx, found: Tier::Wgsl })
        ));
        assert!(record.verify(Tier::Wgsl, None).is_ok());
    }

    #[test]
    fn identifying_a_container_records_the_sass_and_digests() {
        let bytes = container(b"\x01\x02\x03\x04\x05\x06\x07\x08");
        let blob = identify("native.cubin", &bytes).expect("identify");
        assert_eq!(blob.format, "elf64-little");
        assert_eq!(blob.machine, "cuda");
        assert_eq!(blob.sass_bytes, 8);
        assert_eq!(blob.size, bytes.len());
        let text = blob.sections.iter().find(|s| s.name == ".text").unwrap();
        assert_eq!(text.size, 8);
        assert_eq!(text.sha256, sha256_hex(b"\x01\x02\x03\x04\x05\x06\x07\x08"));
    }

    #[test]
    fn non_cuda_and_non_elf_bytes_are_refused() {
        let mut bytes = container(b"1234");
        bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
        assert!(matches!(
            identify("native.cubin", &bytes),
            Err(Refusal::Unidentifiable { .. })
        ));
        assert!(matches!(
            identify("native.cubin", b"not an elf at all, but long enough to pass the length check"),
            Err(Refusal::Unidentifiable { .. })
        ));
    }

    #[test]
    fn a_container_without_text_is_not_a_native_executable() {
        let bytes = container(b"");
        let err = identify("native.cubin", &bytes).unwrap_err();
        assert!(err.to_string().contains("no `.text.*` bytes"), "{err}");
    }

    #[test]
    fn a_native_record_needs_bytes_and_satisfies_native() {
        let bytes = container(b"\xde\xad\xbe\xef");
        let record = native_record(&bytes);
        assert!(record.verify(Tier::Native, Some(&bytes)).is_ok());
        assert!(matches!(
            record.verify(Tier::Native, None),
            Err(Refusal::Uncaptured { .. })
        ));
    }

    #[test]
    fn a_changed_blob_is_not_the_recorded_one() {
        let bytes = container(b"\xde\xad\xbe\xef");
        let record = native_record(&bytes);
        let mut other = bytes.clone();
        let last = other.len() - 1;
        other[last] ^= 0x01;
        assert!(matches!(
            record.verify(Tier::Native, Some(&other)),
            Err(Refusal::Identity { .. })
        ));
    }

    /// Same size and digest fields but a different `.text` digest cannot be
    /// passed off as the identified blob: the record is compared section by
    /// section, not by its top-level digest alone.
    #[test]
    fn a_record_with_a_different_text_digest_is_refused() {
        let bytes = container(b"\xde\xad\xbe\xef");
        let mut record = native_record(&bytes);
        record.artifact_sha256 = sha256_hex(&bytes);
        if let Some(blob) = record.blob.as_mut() {
            for s in &mut blob.sections {
                if s.name == ".text" {
                    s.sha256 = "0".repeat(64);
                }
            }
        }
        assert!(matches!(
            record.verify(Tier::Native, Some(&bytes)),
            Err(Refusal::BlobDrift { .. })
        ));
    }

    #[test]
    fn a_native_record_without_its_virtual_subject_is_refused() {
        let bytes = container(b"\x01");
        let mut record = native_record(&bytes);
        record.ptx_sha256 = None;
        assert!(matches!(
            record.verify(Tier::Native, Some(&bytes)),
            Err(Refusal::NoSubject)
        ));
    }

    #[test]
    fn the_record_survives_json_round_trips() {
        let bytes = container(b"\x01\x02");
        let record = native_record(&bytes);
        let text = serde_json::to_string(&record).unwrap();
        let back: BoundaryRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(record, back);
        assert!(back.verify(Tier::Native, Some(&bytes)).is_ok());
    }
}
