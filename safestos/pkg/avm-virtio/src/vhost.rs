//! The vhost-user message codec, implemented from the wire layouts that
//! SpectrumOS's four local patches add to rust-vmm `vhost` — read firsthand
//! from `../spectrum/pkgs/cloud-hypervisor/vhost/`:
//!
//! * `0001-vhost_user-add-get_size-to-MsgHeader.patch` — the reply header's
//!   `size` is the reply payload length (framing assumption below).
//! * `0003-vhost_user-add-shared-memory-region-support.patch` — adds
//!   `GET_SHARED_MEMORY_REGIONS = 1004` (frontend), `SHMEM_MAP = 1000` /
//!   `SHMEM_UNMAP = 1001` (backend, non-standard), the packed structs
//!   `VhostSharedMemoryRegion`, `VhostUserShmemMapMsg`,
//!   `VhostUserShmemUnmapMsg`, and the reply framing
//!   (`VhostUserU64` count followed by the region records).
//! * `0004-vhost_user-add-protocol-flag-for-shmem.patch` — the non-standard
//!   protocol feature bit `SHARED_MEMORY_REGIONS = 0x8000_0000`, so a
//!   frontend only sends `1004` to backends that advertise it.
//!
//! `0002-vhost-fix-receiving-reply-payloads.patch` is a receive-loop fix in
//! the socket transport (nothing to model on the wire); it is why this
//! codec's reply decoder always reads exactly `header.size` bytes.

/// Negotiated protocol version in every `MsgHeader` (vhost-user spec).
pub const VERSION: u32 = 0x1;
/// `MsgHeader.flags` bit 2: this message is a reply (vhost-user spec §5.1).
pub const FLAG_REPLY: u32 = 0x4;

pub const GET_PROTOCOL_FEATURES: u32 = 15;
pub const SET_PROTOCOL_FEATURES: u32 = 16;
/// Non-standard (SpectrumOS patch 0003): query the device's shared memory
/// regions.
pub const GET_SHARED_MEMORY_REGIONS: u32 = 1004;
/// Non-standard (SpectrumOS patch 0003): backend asks the frontend to map a
/// file into a shared memory region.
pub const SHMEM_MAP: u32 = 1000;
/// Non-standard (SpectrumOS patch 0003): backend asks to unmap part of one.
pub const SHMEM_UNMAP: u32 = 1001;

/// Patch 0004: `VhostUserProtocolFeatures::SHARED_MEMORY_REGIONS`.
pub const PROTOCOL_F_SHARED_MEMORY_REGIONS: u64 = 0x8000_0000;

#[derive(Debug, PartialEq, Eq)]
pub enum CodecError {
    /// Frame shorter than the fixed header / body it claims to be.
    Truncated(&'static str),
    /// Header version != 1.
    BadVersion(u32),
    /// Message id this codec does not model.
    UnknownRequest(u32),
    /// Shmem map flags outside `MAP_R | MAP_W`.
    BadMapFlags(u8),
    /// `offset + len` overflows u64 (mirrors the patch's `is_valid`).
    Overflow(&'static str),
    /// Reply payload does not match `8 + 16 × count`.
    BadRegionReply,
}

/// vhost-user frame header: `request, version, flags, size`, little-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgHeader {
    pub request: u32,
    pub version: u32,
    pub flags: u32,
    pub size: u32,
}

impl MsgHeader {
    pub fn request(request: u32) -> Self {
        MsgHeader {
            request,
            version: VERSION,
            flags: 0,
            size: 0,
        }
    }

    pub fn reply(request: u32, size: u32) -> Self {
        MsgHeader {
            request,
            version: VERSION,
            flags: FLAG_REPLY,
            size,
        }
    }

    pub fn encode(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.request.to_le_bytes());
        out[4..8].copy_from_slice(&self.version.to_le_bytes());
        out[8..12].copy_from_slice(&self.flags.to_le_bytes());
        out[12..16].copy_from_slice(&self.size.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() < 16 {
            return Err(CodecError::Truncated("header"));
        }
        let word =
            |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        let h = MsgHeader {
            request: word(0),
            version: word(4),
            flags: word(8),
            size: word(12),
        };
        if h.version != VERSION {
            return Err(CodecError::BadVersion(h.version));
        }
        Ok(h)
    }
}

/// `VhostSharedMemoryRegion { id: u8, padding: [u8; 7], length: u64 }`
/// (patch 0003), 16 bytes packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedMemoryRegion {
    pub id: u8,
    pub length: u64,
}

impl SharedMemoryRegion {
    pub const ENCODED_LEN: usize = 16;

    pub fn encode(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0] = self.id;
        out[8..16].copy_from_slice(&self.length.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() < Self::ENCODED_LEN {
            return Err(CodecError::Truncated("region"));
        }
        Ok(SharedMemoryRegion {
            id: bytes[0],
            length: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        })
    }
}

/// Reply payload for `GET_SHARED_MEMORY_REGIONS`: `count: u64` followed by
/// `count` region records (patch 0003's handler builds exactly this).
pub fn encode_regions_reply(regions: &[SharedMemoryRegion]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + regions.len() * SharedMemoryRegion::ENCODED_LEN);
    out.extend_from_slice(&(regions.len() as u64).to_le_bytes());
    for r in regions {
        out.extend_from_slice(&r.encode());
    }
    out
}

pub fn decode_regions_reply(bytes: &[u8]) -> Result<Vec<SharedMemoryRegion>, CodecError> {
    if bytes.len() < 8 {
        return Err(CodecError::Truncated("region count"));
    }
    let count = u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize;
    if bytes.len() != 8 + count * SharedMemoryRegion::ENCODED_LEN {
        return Err(CodecError::BadRegionReply);
    }
    (0..count)
        .map(|i| SharedMemoryRegion::decode(&bytes[8 + i * SharedMemoryRegion::ENCODED_LEN..]))
        .collect()
}

/// `MAP_R` / `MAP_W` bits of `VhostUserShmemMapMsgFlags` (patch 0003).
pub const MAP_R: u8 = 0x1;
pub const MAP_W: u8 = 0x2;

/// `VhostUserShmemMapMsg` (patch 0003), 32 bytes packed:
/// `flags: u8, shmid: u8, padding: [u8; 6], shm_offset: u64, fd_offset: u64,
/// len: u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShmemMapMsg {
    pub flags: u8,
    pub shmid: u8,
    pub shm_offset: u64,
    pub fd_offset: u64,
    pub len: u64,
}

impl ShmemMapMsg {
    pub const ENCODED_LEN: usize = 32;

    pub fn new(shmid: u8, shm_offset: u64, fd_offset: u64, len: u64, flags: u8) -> Self {
        ShmemMapMsg {
            flags,
            shmid,
            shm_offset,
            fd_offset,
            len,
        }
    }

    /// Wire validity, mirroring the patch's `VhostUserMsgValidator`:
    /// flags ⊆ {MAP_R, MAP_W} and both `offset + len` sums must not
    /// overflow.
    pub fn is_valid(&self) -> bool {
        self.flags & !(MAP_R | MAP_W) == 0
            && self.fd_offset.checked_add(self.len).is_some()
            && self.shm_offset.checked_add(self.len).is_some()
    }

    pub fn encode(&self) -> Result<[u8; 32], CodecError> {
        if !self.is_valid() {
            return Err(if self.flags & !(MAP_R | MAP_W) != 0 {
                CodecError::BadMapFlags(self.flags)
            } else {
                CodecError::Overflow("shmem map")
            });
        }
        let mut out = [0u8; 32];
        out[0] = self.flags;
        out[1] = self.shmid;
        out[8..16].copy_from_slice(&self.shm_offset.to_le_bytes());
        out[16..24].copy_from_slice(&self.fd_offset.to_le_bytes());
        out[24..32].copy_from_slice(&self.len.to_le_bytes());
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() < Self::ENCODED_LEN {
            return Err(CodecError::Truncated("shmem map"));
        }
        let msg = ShmemMapMsg {
            flags: bytes[0],
            shmid: bytes[1],
            shm_offset: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            fd_offset: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            len: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        };
        if !msg.is_valid() {
            return Err(if msg.flags & !(MAP_R | MAP_W) != 0 {
                CodecError::BadMapFlags(msg.flags)
            } else {
                CodecError::Overflow("shmem map")
            });
        }
        Ok(msg)
    }
}

/// `VhostUserShmemUnmapMsg` (patch 0003), 24 bytes packed:
/// `shmid: u8, padding: [u8; 7], shm_offset: u64, len: u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShmemUnmapMsg {
    pub shmid: u8,
    pub shm_offset: u64,
    pub len: u64,
}

impl ShmemUnmapMsg {
    pub const ENCODED_LEN: usize = 24;

    pub fn new(shmid: u8, shm_offset: u64, len: u64) -> Self {
        ShmemUnmapMsg {
            shmid,
            shm_offset,
            len,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.shm_offset.checked_add(self.len).is_some()
    }

    pub fn encode(&self) -> Result<[u8; 24], CodecError> {
        if !self.is_valid() {
            return Err(CodecError::Overflow("shmem unmap"));
        }
        let mut out = [0u8; 24];
        out[0] = self.shmid;
        out[8..16].copy_from_slice(&self.shm_offset.to_le_bytes());
        out[16..24].copy_from_slice(&self.len.to_le_bytes());
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() < Self::ENCODED_LEN {
            return Err(CodecError::Truncated("shmem unmap"));
        }
        let msg = ShmemUnmapMsg {
            shmid: bytes[0],
            shm_offset: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            len: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
        };
        if !msg.is_valid() {
            return Err(CodecError::Overflow("shmem unmap"));
        }
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_and_byte_layout() {
        let h = MsgHeader::request(GET_SHARED_MEMORY_REGIONS);
        let bytes = h.encode();
        // Patch parity: the query id is exactly 1004.
        assert_eq!(&bytes[0..4], &1004u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &0u32.to_le_bytes());
        assert_eq!(MsgHeader::decode(&bytes).unwrap(), h);

        let r = MsgHeader::reply(SHMEM_MAP, 32);
        let bytes = r.encode();
        assert_eq!(&bytes[8..12], &FLAG_REPLY.to_le_bytes());
        assert_eq!(MsgHeader::decode(&bytes).unwrap().size, 32);
    }

    #[test]
    fn short_or_wrong_version_headers_rejected() {
        assert_eq!(
            MsgHeader::decode(&[0u8; 8]),
            Err(CodecError::Truncated("header"))
        );
        let mut bytes = MsgHeader::request(1).encode();
        bytes[4] = 9; // version
        assert_eq!(MsgHeader::decode(&bytes), Err(CodecError::BadVersion(9)));
    }

    #[test]
    fn protocol_flag_matches_patch_0004() {
        // Non-standard bit, exactly as the patch defines it.
        assert_eq!(PROTOCOL_F_SHARED_MEMORY_REGIONS, 0x8000_0000);
        // It must not collide with the spec-standard bits below it.
        assert_eq!(PROTOCOL_F_SHARED_MEMORY_REGIONS & 0x0004_0000, 0); // SHARED_OBJECT
        assert_eq!(PROTOCOL_F_SHARED_MEMORY_REGIONS & 0x0008_0000, 0); // DEVICE_STATE
    }

    #[test]
    fn region_record_layout_pinned() {
        let r = SharedMemoryRegion {
            id: 3,
            length: 0x1_0000,
        };
        let bytes = r.encode();
        assert_eq!(bytes[0], 3);
        assert_eq!(&bytes[1..8], &[0u8; 7], "padding must be zero");
        assert_eq!(&bytes[8..16], &0x1_0000u64.to_le_bytes());
        assert_eq!(SharedMemoryRegion::decode(&bytes).unwrap(), r);
    }

    #[test]
    fn region_reply_framing_matches_patch_handler() {
        let regions = vec![
            SharedMemoryRegion {
                id: 0,
                length: 4096,
            },
            SharedMemoryRegion {
                id: 1,
                length: 1 << 20,
            },
        ];
        let payload = encode_regions_reply(&regions);
        assert_eq!(
            &payload[0..8],
            &2u64.to_le_bytes(),
            "count first (VhostUserU64)"
        );
        assert_eq!(payload.len(), 8 + 2 * 16);
        assert_eq!(decode_regions_reply(&payload).unwrap(), regions);
        // A truncated or padded reply fails loudly.
        assert_eq!(
            decode_regions_reply(&payload[..payload.len() - 1]),
            Err(CodecError::BadRegionReply)
        );
        assert_eq!(
            decode_regions_reply(&[0u8; 4]),
            Err(CodecError::Truncated("region count"))
        );
    }

    #[test]
    fn shmem_map_layout_and_validity() {
        let m = ShmemMapMsg::new(2, 0x2000, 0x1_0000, 0x4000, MAP_R | MAP_W);
        let bytes = m.encode().unwrap();
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes[0], MAP_R | MAP_W);
        assert_eq!(bytes[1], 2);
        assert_eq!(&bytes[2..8], &[0u8; 6], "padding must be zero");
        assert_eq!(&bytes[8..16], &0x2000u64.to_le_bytes());
        assert_eq!(&bytes[16..24], &0x1_0000u64.to_le_bytes());
        assert_eq!(&bytes[24..32], &0x4000u64.to_le_bytes());
        assert_eq!(ShmemMapMsg::decode(&bytes).unwrap(), m);

        // Invalid flags (patch validator rejects unknown bits).
        let bad = ShmemMapMsg::new(0, 0, 0, 8, 0x4);
        assert!(!bad.is_valid());
        assert_eq!(bad.encode(), Err(CodecError::BadMapFlags(0x4)));
        // Overflowing offset+len (patch validator's checked_add).
        let over = ShmemMapMsg::new(0, u64::MAX, 0, 16, MAP_R);
        assert!(!over.is_valid());
        assert!(matches!(over.encode(), Err(CodecError::Overflow(_))));
    }

    #[test]
    fn shmem_unmap_layout_and_validity() {
        let m = ShmemUnmapMsg::new(1, 0x2000, 0x1000);
        let bytes = m.encode().unwrap();
        assert_eq!(bytes.len(), 24);
        assert_eq!(bytes[0], 1);
        assert_eq!(&bytes[1..8], &[0u8; 7]);
        assert_eq!(ShmemUnmapMsg::decode(&bytes).unwrap(), m);
        assert!(matches!(
            ShmemUnmapMsg::new(0, u64::MAX, 8).encode(),
            Err(CodecError::Overflow(_))
        ));
        assert_eq!(
            ShmemUnmapMsg::decode(&[0u8; 8]),
            Err(CodecError::Truncated("shmem unmap"))
        );
    }
}
