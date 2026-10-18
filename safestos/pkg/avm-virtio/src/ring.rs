//! The split VirtQueue (virtio specification §2.7.7, "Split Virtqueue
//! Rings") — the ring format the SpectrumOS-style driver domains negotiate
//! over virtio-pci (`VIRTIO=y`, `VIRTIO_PCI=y` in `../spectrum/vm/sys/net`).
//!
//! The queue is an explicit byte image of guest memory, so every offset is
//! pinned by a test rather than by `#[repr(C)]` layout:

//! ```text
//!  descriptor table : size × { addr: u64, len: u32, flags: u16, next: u16 }
//!  available ring  : flags: u16, idx: u16, ring: size × u16, used_event: u16
//!  used ring       : flags: u16, idx: u16, ring: size × { id: u32, len: u32 },
//!                    avail_event: u16
//! ```

/// Descriptor flag: another descriptor of this chain follows at `next`.
pub const VRING_DESC_F_NEXT: u16 = 0x1;
/// Descriptor flag: the device writes this buffer (device-readable becomes
/// device-writable from the driver's point of view).
pub const VRING_DESC_F_WRITE: u16 = 0x2;

const DESC_SIZE: usize = 16;
const USED_ELEM_SIZE: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum QueueError {
    /// Queue size must be a power of two, per spec §2.7.1.
    NotPowerOfTwo(u16),
    /// Index outside the ring.
    OutOfRange(&'static str),
    /// A chain walked more descriptors than the ring holds: the available
    /// ring is corrupt (or the device is reading freed descriptors).
    CorruptChain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

/// One split queue over a guest-memory image, able to play both roles:
/// driver (push available, reclaim used) and device (pop chains, push used).
#[derive(Debug, PartialEq, Eq)]
pub struct SplitQueue {
    size: u16,
    mem: Vec<u8>,
    avail_ring: usize,
    used_ring: usize,
    /// Device-side cursor: next available chain to consume.
    next_avail: u16,
    /// Frontend cursor: next used element to reclaim.
    last_used: u16,
}

impl SplitQueue {
    pub fn new(size: u16) -> Result<Self, QueueError> {
        if size == 0 || !size.is_power_of_two() {
            return Err(QueueError::NotPowerOfTwo(size));
        }
        let table = DESC_SIZE * size as usize;
        let avail_ring = table;
        let used_ring = (avail_ring + 6 + 2 * size as usize + 3) & !3; // 4-aligned
        let total = used_ring + 6 + USED_ELEM_SIZE * size as usize;
        Ok(SplitQueue {
            size,
            mem: vec![0u8; total],
            avail_ring,
            used_ring,
            next_avail: 0,
            last_used: 0,
        })
    }

    pub fn size(&self) -> u16 {
        self.size
    }

    pub fn memory(&self) -> &[u8] {
        &self.mem
    }

    fn desc_offset(&self, i: u16) -> Result<usize, QueueError> {
        if i >= self.size {
            return Err(QueueError::OutOfRange("descriptor"));
        }
        Ok(DESC_SIZE * i as usize)
    }

    /// Guest-side: install descriptor `i` (used when building chains).
    pub fn set_desc(
        &mut self,
        i: u16,
        addr: u64,
        len: u32,
        flags: u16,
        next: u16,
    ) -> Result<(), QueueError> {
        let off = self.desc_offset(i)?;
        self.mem[off..off + 8].copy_from_slice(&addr.to_le_bytes());
        self.mem[off + 8..off + 12].copy_from_slice(&len.to_le_bytes());
        self.mem[off + 12..off + 14].copy_from_slice(&flags.to_le_bytes());
        self.mem[off + 14..off + 16].copy_from_slice(&next.to_le_bytes());
        Ok(())
    }

    /// Driver side: publish descriptor-chain head `head` and return the new
    /// available-ring index (monotone, wrapping at 2^16).
    pub fn push_avail(&mut self, head: u16) -> Result<u16, QueueError> {
        if head >= self.size {
            return Err(QueueError::OutOfRange("head"));
        }
        let idx =
            u16::from_le_bytes([self.mem[self.avail_ring + 2], self.mem[self.avail_ring + 3]]);
        let slot = self.avail_ring + 4 + 2 * (idx % self.size) as usize;
        self.mem[slot..slot + 2].copy_from_slice(&head.to_le_bytes());
        let idx = idx.wrapping_add(1);
        self.mem[self.avail_ring + 2..self.avail_ring + 4].copy_from_slice(&idx.to_le_bytes());
        Ok(idx)
    }

    fn avail_count(&self) -> u16 {
        let idx =
            u16::from_le_bytes([self.mem[self.avail_ring + 2], self.mem[self.avail_ring + 3]]);
        idx.wrapping_sub(self.next_avail)
    }

    /// Device side: consume the next available chain, following `NEXT`
    /// links. Returns `Ok(None)` when the ring is empty; `Err` on a chain
    /// longer than the ring (corrupt/cyclic).
    pub fn pop_chain(&mut self) -> Result<Option<Vec<Descriptor>>, QueueError> {
        let count = self.avail_count();
        if count == 0 {
            return Ok(None);
        }
        if count > self.size {
            return Err(QueueError::CorruptChain);
        }
        let slot = self.avail_ring + 4 + 2 * (self.next_avail % self.size) as usize;
        let head = u16::from_le_bytes([self.mem[slot], self.mem[slot + 1]]);
        let mut cur = head;
        let mut chain: Vec<Descriptor> = Vec::new();
        loop {
            let off = self.desc_offset(cur)?;
            let mut addr = [0u8; 8];
            addr.copy_from_slice(&self.mem[off..off + 8]);
            let mut len = [0u8; 4];
            len.copy_from_slice(&self.mem[off + 8..off + 12]);
            let mut flags = [0u8; 2];
            flags.copy_from_slice(&self.mem[off + 12..off + 14]);
            let mut next = [0u8; 2];
            next.copy_from_slice(&self.mem[off + 14..off + 16]);
            let d = Descriptor {
                addr: u64::from_le_bytes(addr),
                len: u32::from_le_bytes(len),
                flags: u16::from_le_bytes(flags),
                next: u16::from_le_bytes(next),
            };
            let follows = d.flags & VRING_DESC_F_NEXT != 0;
            chain.push(d);
            if chain.len() > self.size as usize {
                return Err(QueueError::CorruptChain);
            }
            if follows {
                cur = d.next;
            } else {
                self.next_avail = self.next_avail.wrapping_add(1);
                break;
            }
        }
        Ok(Some(chain))
    }

    /// Device side: hand descriptor-chain head `head` back with `written`
    /// bytes written by the device.
    pub fn push_used(&mut self, head: u16, written: u32) -> Result<(), QueueError> {
        if head >= self.size {
            return Err(QueueError::OutOfRange("head"));
        }
        let idx = u16::from_le_bytes([self.mem[self.used_ring + 2], self.mem[self.used_ring + 3]]);
        let slot = self.used_ring + 4 + USED_ELEM_SIZE * (idx % self.size) as usize;
        self.mem[slot..slot + 4].copy_from_slice(&(head as u32).to_le_bytes());
        self.mem[slot + 4..slot + 8].copy_from_slice(&written.to_le_bytes());
        let idx = idx.wrapping_add(1);
        self.mem[self.used_ring + 2..self.used_ring + 4].copy_from_slice(&idx.to_le_bytes());
        Ok(())
    }

    /// Driver side: reclaim the next completed chain `(head, written)`.
    pub fn pop_used(&mut self) -> Option<(u16, u32)> {
        let idx = u16::from_le_bytes([self.mem[self.used_ring + 2], self.mem[self.used_ring + 3]]);
        if idx == self.last_used {
            return None;
        }
        let slot = self.used_ring + 4 + USED_ELEM_SIZE * (self.last_used % self.size) as usize;
        let mut id = [0u8; 4];
        id.copy_from_slice(&self.mem[slot..slot + 4]);
        let mut len = [0u8; 4];
        len.copy_from_slice(&self.mem[slot + 4..slot + 8]);
        self.last_used = self.last_used.wrapping_add(1);
        Some((u32::from_le_bytes(id) as u16, u32::from_le_bytes(len)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_offsets_pinned() {
        let q = SplitQueue::new(8).unwrap();
        // 8 descriptors = 128 bytes; avail at 128; used 4-aligned after
        // avail (flags+idx+8×u16 ring+used_event = 24 bytes -> 152).
        assert_eq!(q.avail_ring, 128);
        assert_eq!(q.used_ring, 152);
        assert_eq!(q.memory().len(), 152 + 6 + 8 * 8);
    }

    #[test]
    fn non_power_of_two_rejected() {
        assert_eq!(SplitQueue::new(6), Err(QueueError::NotPowerOfTwo(6)));
        assert_eq!(SplitQueue::new(0), Err(QueueError::NotPowerOfTwo(0)));
    }

    #[test]
    fn submit_device_complete_reclaim_roundtrip() {
        let mut q = SplitQueue::new(4).unwrap();
        // A two-descriptor chain: desc0 --NEXT--> desc1 (WRITE: device fills).
        q.set_desc(0, 0x1_0000, 64, VRING_DESC_F_NEXT, 1).unwrap();
        q.set_desc(1, 0x2_0000, 128, VRING_DESC_F_WRITE, 0).unwrap();

        let idx = q.push_avail(0).unwrap();
        assert_eq!(idx, 1);
        assert!(q.pop_used().is_none(), "nothing completed yet");

        // Device: read the chain, then complete it with 96 bytes written.
        let chain = q.pop_chain().unwrap().expect("chain available");
        assert_eq!(
            chain,
            vec![
                Descriptor {
                    addr: 0x1_0000,
                    len: 64,
                    flags: VRING_DESC_F_NEXT,
                    next: 1
                },
                Descriptor {
                    addr: 0x2_0000,
                    len: 128,
                    flags: VRING_DESC_F_WRITE,
                    next: 0
                },
            ]
        );
        assert!(q.pop_chain().unwrap().is_none(), "ring empty again");
        q.push_used(0, 96).unwrap();

        // Frontend: the ownership token resolves back to (head, written).
        assert_eq!(q.pop_used(), Some((0, 96)));
        assert_eq!(q.pop_used(), None, "exactly one completion");
    }

    #[test]
    fn wraparound_preserves_pairing() {
        let mut q = SplitQueue::new(2).unwrap();
        q.set_desc(0, 1, 8, 0, 0).unwrap();
        q.set_desc(1, 2, 8, 0, 0).unwrap();
        // More submissions than the ring holds, forcing idx wrap (u16 and
        // modulo-size), each completed before the next submit would overrun.
        for i in 0u32..10 {
            let head = (i % 2) as u16;
            q.push_avail(head).unwrap();
            let chain = q.pop_chain().unwrap().unwrap();
            assert_eq!(chain[0].addr, if head == 0 { 1 } else { 2 });
            q.push_used(head, i).unwrap();
            assert_eq!(q.pop_used(), Some((head, i)));
        }
    }

    #[test]
    fn six_five_five_three_six_index_wrap() {
        // Push/pop across the u16 index wrap itself: 65536 + 3 iterations
        // keep pairing with size 4 (4096 divides 65536).
        let mut q = SplitQueue::new(4).unwrap();
        for i in 0u32..(65536 + 3) {
            let head = (i % 4) as u16;
            q.set_desc(head, i as u64, 1, 0, 0).unwrap();
            q.push_avail(head).unwrap();
            let chain = q.pop_chain().unwrap().unwrap();
            assert_eq!(chain[0].addr, i as u64, "desc survived idx wrap");
            q.push_used(head, 1).unwrap();
            assert_eq!(q.pop_used(), Some((head, 1)));
        }
    }

    #[test]
    fn corrupt_cyclic_chain_is_an_error_not_a_hang() {
        let mut q = SplitQueue::new(4).unwrap();
        // desc0 --NEXT--> desc0: a cycle the walk must refuse.
        q.set_desc(0, 1, 8, VRING_DESC_F_NEXT, 0).unwrap();
        q.push_avail(0).unwrap();
        assert_eq!(q.pop_chain(), Err(QueueError::CorruptChain));
    }

    #[test]
    fn out_of_range_indices_rejected() {
        let mut q = SplitQueue::new(2).unwrap();
        assert_eq!(
            q.set_desc(2, 0, 0, 0, 0),
            Err(QueueError::OutOfRange("descriptor"))
        );
        assert_eq!(q.push_avail(2), Err(QueueError::OutOfRange("head")));
        assert_eq!(q.push_used(2, 0), Err(QueueError::OutOfRange("head")));
    }
}
