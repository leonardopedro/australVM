//! The shared-memory-region registry: file-backed regions registered for a
//! vhost-user backend, and the wire records a `GET_SHARED_MEMORY_REGIONS`
//! reply / `SHMEM_MAP` request need.
//!
//! Deliberately **no mmap and no `unsafe`**: the region's *mapping* is the
//! VMM backend's job (it receives the file and offsets); this side only
//! owns the backing file and the bookkeeping. That preserves PLAN_avm_pkg
//! §6.3's invariant (the only `unsafe`/`mmap` in `pkg/` is avm-cas's hot
//! store) while still making every wire byte testable here.

use crate::vhost::{CodecError, SharedMemoryRegion, ShmemMapMsg, MAP_R, MAP_W};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

#[derive(Debug, PartialEq, Eq)]
pub enum ShmemError {
    /// More regions than the `u8 shmid` wire field can name.
    OutOfIds,
    /// No such region id.
    UnknownRegion(u8),
    /// Requested range leaves the region.
    RangeOutOfBounds {
        shmid: u8,
        shm_offset: u64,
        len: u64,
        region_len: u64,
    },
    /// Backing file could not be sized.
    Io(String),
    /// A wire message this registry produced failed its own codec check
    /// (should be unreachable; surfaced rather than swallowed).
    Codec(CodecError),
}

impl From<std::io::Error> for ShmemError {
    fn from(e: std::io::Error) -> Self {
        // Owned message: io::Error is not Eq, and every ShmemError variant
        // must compare in tests.
        ShmemError::Io(e.to_string())
    }
}

struct Region {
    id: u8,
    length: u64,
    file: NamedTempFile,
}

/// Registered shared-memory regions, ids assigned in registration order.
#[derive(Default)]
pub struct ShmemRegistry {
    regions: Vec<Region>,
}

impl ShmemRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a fresh backing file of `length` bytes and register it.
    pub fn add_region(&mut self, length: u64) -> Result<u8, ShmemError> {
        let file = NamedTempFile::new()?;
        file.as_file().set_len(length)?;
        self.add_backing(length, file)
    }

    /// Register an existing file (e.g. a hydrated CAS cell) as a region.
    pub fn add_region_file(&mut self, path: &Path) -> Result<u8, ShmemError> {
        let len = std::fs::metadata(path)?.len();
        let file = NamedTempFile::new()?;
        std::fs::copy(path, file.path())?;
        self.add_backing(len, file)
    }

    fn add_backing(&mut self, length: u64, file: NamedTempFile) -> Result<u8, ShmemError> {
        let id = self.regions.len();
        if id > u8::MAX as usize {
            return Err(ShmemError::OutOfIds);
        }
        let id = id as u8;
        self.regions.push(Region { id, length, file });
        Ok(id)
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    pub fn region_path(&self, id: u8) -> Result<PathBuf, ShmemError> {
        self.find(id).map(|r| r.file.path().to_path_buf())
    }

    fn find(&self, id: u8) -> Result<&Region, ShmemError> {
        self.regions
            .iter()
            .find(|r| r.id == id)
            .ok_or(ShmemError::UnknownRegion(id))
    }

    fn check_range(&self, shmid: u8, shm_offset: u64, len: u64) -> Result<(), ShmemError> {
        let region = self.find(shmid)?;
        match shm_offset.checked_add(len) {
            Some(end) if end <= region.length => Ok(()),
            _ => Err(ShmemError::RangeOutOfBounds {
                shmid,
                shm_offset,
                len,
                region_len: region.length,
            }),
        }
    }

    /// The `GET_SHARED_MEMORY_REGIONS` reply payload contents.
    pub fn records(&self) -> Vec<SharedMemoryRegion> {
        self.regions
            .iter()
            .map(|r| SharedMemoryRegion {
                id: r.id,
                length: r.length,
            })
            .collect()
    }

    /// Build a validated `SHMEM_MAP` message for `shmid` — the range must
    /// exist in the registered region before it can go on the wire.
    pub fn map_msg(
        &self,
        shmid: u8,
        shm_offset: u64,
        fd_offset: u64,
        len: u64,
        writable: bool,
    ) -> Result<ShmemMapMsg, ShmemError> {
        self.check_range(shmid, shm_offset, len)?;
        let flags = if writable { MAP_R | MAP_W } else { MAP_R };
        let msg = ShmemMapMsg::new(shmid, shm_offset, fd_offset, len, flags);
        msg.encode().map_err(ShmemError::Codec)?;
        Ok(msg)
    }

    /// Build a validated `SHMEM_UNMAP` message for `shmid`.
    pub fn unmap_msg(&self, shmid: u8, shm_offset: u64, len: u64) -> Result<[u8; 24], ShmemError> {
        self.check_range(shmid, shm_offset, len)?;
        crate::vhost::ShmemUnmapMsg::new(shmid, shm_offset, len)
            .encode()
            .map_err(ShmemError::Codec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vhost::{decode_regions_reply, encode_regions_reply};

    #[test]
    fn records_roundtrip_through_the_reply_codec() {
        let mut reg = ShmemRegistry::new();
        let a = reg.add_region(4096).unwrap();
        let b = reg.add_region(1 << 20).unwrap();
        assert_eq!((a, b), (0, 1));
        assert_eq!(reg.len(), 2);

        let records = reg.records();
        assert_eq!(
            records[0],
            SharedMemoryRegion {
                id: 0,
                length: 4096
            }
        );
        assert_eq!(
            records[1],
            SharedMemoryRegion {
                id: 1,
                length: 1 << 20
            }
        );
        // The exact bytes a frontend decodes from a `1004` reply.
        let payload = encode_regions_reply(&records);
        assert_eq!(decode_regions_reply(&payload).unwrap(), records);
    }

    #[test]
    fn map_msg_range_is_checked_against_the_region() {
        let mut reg = ShmemRegistry::new();
        let id = reg.add_region(0x4000).unwrap();

        let msg = reg.map_msg(id, 0x1000, 0x2000, 0x3000, true).unwrap();
        assert_eq!(msg.len, 0x3000);
        assert_eq!(msg.flags, MAP_R | MAP_W);

        // Exactly to the end is fine; one byte past is not.
        assert!(reg.map_msg(id, 0x1000, 0, 0x3000, false).is_ok());
        assert!(matches!(
            reg.map_msg(id, 0x1000, 0, 0x3001, false),
            Err(ShmemError::RangeOutOfBounds { .. })
        ));
        assert!(matches!(
            reg.map_msg(id, u64::MAX, 0, 1, false),
            Err(ShmemError::RangeOutOfBounds { .. })
        ));
        assert!(matches!(
            reg.map_msg(9, 0, 0, 1, false),
            Err(ShmemError::UnknownRegion(9))
        ));
    }

    #[test]
    fn unmap_message_validated_the_same_way() {
        let mut reg = ShmemRegistry::new();
        let id = reg.add_region(8192).unwrap();
        let bytes = reg.unmap_msg(id, 0, 4096).unwrap();
        let msg = crate::vhost::ShmemUnmapMsg::decode(&bytes).unwrap();
        assert_eq!(msg.shmid, id);
        assert!(matches!(
            reg.unmap_msg(id, 6000, 4096),
            Err(ShmemError::RangeOutOfBounds { .. })
        ));
    }

    #[test]
    fn region_files_exist_with_the_declared_length() {
        let mut reg = ShmemRegistry::new();
        let id = reg.add_region(65536).unwrap();
        let path = reg.region_path(id).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 65536);
    }

    #[test]
    fn existing_file_registration_copies_contents() {
        let mut src = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut src, b"kernel-bytes").unwrap();
        let mut reg = ShmemRegistry::new();
        let id = reg.add_region_file(src.path()).unwrap();
        let back = std::fs::read(reg.region_path(id).unwrap()).unwrap();
        assert_eq!(back, b"kernel-bytes");
        assert_eq!(reg.records()[0].length, 12);
    }
}
