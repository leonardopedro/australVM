//! `avm-virtio` — the VirtIO front-end substrate for AustralVM driver
//! domains (docs/spectrum_plan.md Sprint 6).
//!
//! Three layers, each offline-testable without any VM or device:
//!
//! * [`ring`] — the split VirtQueue (virtio spec §2.7.7) over an explicit
//!   guest-memory byte image: descriptor chains, available/used rings,
//!   wrap-around arithmetic. Ownership handoff (submit → device → complete)
//!   is proven by a simulated-device round-trip test.
//! * [`vhost`] — the vhost-user message codec: `MsgHeader` framing, and the
//!   shared-memory-region extension SpectrumOS carries as four local patches
//!   to rust-vmm `vhost` (`../spectrum/pkgs/cloud-hypervisor/vhost/`):
//!   `GET_SHARED_MEMORY_REGIONS = 1004`, `SHMEM_MAP = 1000`,
//!   `SHMEM_UNMAP = 1001`, and the non-standard protocol feature bit
//!   `SHARED_MEMORY_REGIONS = 0x8000_0000`. Byte layouts are pinned by
//!   tests against the `#[repr(C, packed)]` structs the patches define.
//! * [`shmem`] — a registry of file-backed shared-memory regions, producing
//!   the wire records for `GET_SHARED_MEMORY_REGIONS` replies and
//!   `SHMEM_MAP`/`SHMEM_UNMAP` backend requests. The registry never maps
//!   anything itself: the mmap belongs to the VMM/backend process, which
//!   keeps PLAN_avm_pkg §6.3's "exactly one `unsafe` (the CAS hot store)"
//!   invariant intact.

pub mod ring;
pub mod shmem;
pub mod vhost;
