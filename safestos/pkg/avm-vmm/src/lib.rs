//! `avm-vmm` — cloud-hypervisor orchestration for AustralVM driver domains
//! (docs/spectrum_plan.md Sprint 6).
//!
//! * [`config`] — the `vm.create` payload, field-for-field the documented
//!   shape of `../cloud-hypervisor-build/cloud-hypervisor/docs/api.md`
//!   (SpectrumOS's accepted VMM, decision record
//!   `../spectrum/Documentation/doc/decisions/005-virtual-machine-monitor.adoc`).
//! * [`launcher`] — REST-over-Unix-socket client for the documented API
//!   (`PUT /api/v1/vm.create`, `PUT /api/v1/vm.boot`, `GET /api/v1/vm.info`,
//!   `PUT /api/v1/vm.shutdown`), plus `find_vmm()` and an honest refusal
//!   when no VMM binary exists — the PLAN_avm_pkg §4.7 CLI-honesty rule
//!   applied to VM launches.
//! * [`lifecycle`] — CAS-backed driver-domain planning: resolve the kernel
//!   cell manifest, hydrate it to a bootable path, assemble the config,
//!   probe `/dev/kvm` (probe, never assume — same discipline as
//!   `avm-exec/src/runner.rs` probes `unshare`).

pub mod config;
pub mod launcher;
pub mod lifecycle;
