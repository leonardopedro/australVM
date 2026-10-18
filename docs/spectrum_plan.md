# Driver Domains via Cloud-Hypervisor & SpectrumOS — grounded implementation plan (Sprints 6 & 7)

**Status (2026-10-18): Sprints 6 (tasks 6.1–6.3) and 7 (7.1–7.2) shipped
and measured; Sprint 8 — a cloud profile using Hermit unikernel driver
domains (`../hermit-rs/`, C stacks via `../hermit-c/`) — tasks 8.1–8.3
executed and measured the same day (8.4 host-gated) — two new crates with
33 tests, the `stdlib/sys/Virtio` linear
binding with a typecheck gate and a pinned use-after-submit compile failure,
the driver-kernel derivation recipe hash-stable, and firmware dedup at
98.6 % against the ≥ 80 % target. Task 7.3 (the live *run*) is host-gated:
real VMM launch, boot latency, and fault quarantine ran on this host
2026-10-18 (§5). Every claim below traces to a file that exists in
`../spectrum/`, `../cloud-hypervisor-build/`, `cloud_hypervisor_vm/`, or
`safestos/`; measured numbers and reproduction commands live in §5.

**Follow-up pass, same day (2026-10-18, later):** the three rows the first
pass recorded as *not covered* for want of instrumentation or hardware
were attacked head-on. (1) **Uhyve now boots Hermit guests on this
host** — the `BackendError(Error(22))` was root-caused (newer KVM
validates `KVM_SET_SREGS` EFER bits against the *current* vCPU CPUID;
uhyve 0.10.0 writes long mode before installing CPUID) and fixed in a
reviewable compat patch (`docs/uhyve-0.10.0-kvm-compat.patch`), with two
further guest-visible gaps patched (COM1 emulation, MMIO scan
tolerance); spawn→`Hello, world!` measured **245.7–262.6 ms** (full
image) and **160.3–173.1 ms** (compile-time-pruned image + PIT), §5.
The < 50 ms target is still **not met**, now with the remainder
quantified (hermit's own timer calibration, incl. a verified 30 ms APIC
wait in `../kernel`). (2) The **VirtIO-ready row is now measured with
in-guest instrumentation** — `boot_once` captures the guest's timestamped
console (cloud-hypervisor `serial` File sink) and reads the first
virtio/`1af4` PCI-enumeration line: **284.999–292.423 ms guest-clock**,
3 runs (§5); the < 50 ms target is not met by a full Linux guest and the
number is real, not a placeholder. (3) **Task 8.6's seam swap shipped**:
the bridge's static import registry + C-ABI registrar (3 integration
tests) is the dlopen-free resolution path for Hermit-side images; the
in-guest boot of the Austral+Rust domain itself remains open, but the
runner gap that blocked it is closed. The ≥ 92 % of 10 GbE row stays
*not covered* — this machine's inventory (Wi-Fi + a 1 GbE USB NIC, no
10 GbE NIC and no VF to pass through) makes it physically unmeasurable
here; recorded with the inventory in §6. Gates after the pass: `cargo
fmt --all --check` clean (both workspaces), `cargo test --workspace`
108 passed / 0 failed / 5 ignored, bridge suite green incl. the 3 new
seam tests, `dune build` + `dune runtest` OK, runner suite **311 PASS /
0 fail**, `check-links` 233 documents OK.

This plan is the grounded rewrite of the original aspirational extension
(received 2026-10-17). What changed and why, section by section:

* **§1 convergence** — kept; patch filenames corrected to the files that
  actually exist in `../spectrum/pkgs/cloud-hypervisor/`, and the VMM choice
  cited to the accepted SpectrumOS decision record.
* **§2 topology** — the original proposed new top-level `safestos/vmm/` and
  `safestos/virtio/` cargo workspaces. **Deviation (recorded, §4.6):** both
  land as member crates of the existing standalone `safestos/pkg/` workspace
  instead, preserving PLAN_avm_pkg §4.6's offline-cargo discipline (no new
  external crates; the `vhost` crate is *not* vendored — the protocol codec
  is implemented in-tree from the wire layouts, exactly like the in-house
  FastCDC precedented chunker).
* **§3 specs** — the VirtIO Austral module and the driver-VM derivation now
  use this compiler's real module system (flat module names per `stdlib/`
  convention; the parser *does* accept dotted names, but every existing
  stdlib module — `Derivation`, `Stdenv` — is flat, and `import` lists are
  per-module), real `Derivation` field types (`Span[Nat8, Static]` text, not
  a nonexistent `String`/`StorePath` builtin), and the real vhost-user wire
  layouts from the SpectrumOS patches.
* **§4 roadmap** — tasks re-scoped to what executes in this workspace
  (offline cargo, `/dev/kvm` present but no VMM binary, no network, no
  10 GbE NIC); each task states what is proven by a gate here versus what a
  capable host must still run.
* **§5/§6** — acceptance table gained a measured/status column and a
  reproduce-locally block; hardware rows say *not covered*, never "pass".
* **§8 cloud profile** — added 2026-10-18 after studying the `../hermit-rs/`
  unikernel checkout (firsthand: `README.md`, `hermit/`, `hermit-abi/`,
  `k8s/httpd.yml`, CI, licences). §1–§7 stay the bare-metal profile (Linux
driver domains, because only Linux carries VFIO + vendor firmware); §8
  adds a second profile for cloud deployments where devices are
  virtio-only and a whole Linux guest is pure overhead — a Hermit
  unikernel domain (one image, no userspace) launched by the same
  `avm-vmm` path. Tasks 8.1–8.3 and 8.5 were executed and measured the
  same day (§5); 8.4 stays host-gated. Task 8.6 records how the
  umbrella compiler (PLAN_valen §5 P3) would put Austral logic — linear
  types and the manifest gate included — on Hermit guests (planned).

---

## 1. Architectural convergence

AustralVM acts as the capability-secure host OS. Messy, vendor-specific
hardware interaction (PCIe, Wi-Fi, GPU, firmware blobs) is delegated to
disposable Linux micro-VMs ("driver domains"), communicating with the host
over **vhost-user + shared-memory VirtIO channels**.

This is exactly SpectrumOS's proven shape, not a guess:

* **VMM = cloud-hypervisor**, per SpectrumOS's accepted decision record
  `../spectrum/Documentation/doc/decisions/005-virtual-machine-monitor.adoc`,
  which names cloud-hypervisor as Spectrum's primary VMM. (Those decision
  docs are GFDL-1.3/CC-BY-SA-4.0, not MIT/Apache — the verbatim excerpt is
  therefore kept in the GPL project at
  `../emthin/docs/spectrum-decision-evidence.md`, and this plan
  paraphrases. See the workspace `../ATTRIBUTION.md` licence-routing
  policy.)
* **Patched vhost-user**: `../spectrum/pkgs/cloud-hypervisor/default.nix`
  builds upstream cloud-hypervisor plus rust-vmm `vhost` (rev
  `vhost-user-backend-v0.20.0`) carrying four patches whose names this plan
  now quotes exactly:
  * `0001-vhost_user-add-get_size-to-MsgHeader.patch`
  * `0002-vhost-fix-receiving-reply-payloads.patch`
  * `0003-vhost_user-add-shared-memory-region-support.patch`
  * `0004-vhost_user-add-protocol-flag-for-shmem.patch`
  plus the two cloud-hypervisor patches `0001-build-use-local-vhost.patch`
  and `0002-virtio-devices-add-a-GPU-device.patch`.
* **A driver VM already exists as reference**: `../spectrum/vm/sys/net/` —
  a musl Linux with `VIRTIO`, `VIRTIO_BLK`, `VIRTIO_CONSOLE`, `VIRTIO_PCI`,
  `EROFS_FS` forced on and console/framebuffer baggage forced off, packaged
  by `../spectrum/vm-lib/make-vm.nix` (erofs image + `providers/` dirs for
  the vhost-user backends the host starts — see the host's
  `vhost-user-fs`/`vhost-user-gpu` services under
  `../spectrum/host/rootfs/image/etc/s6-linux-init/run-image/service/vm-services/`).
* **The launch tooling already exists in this repo family**:
  `cloud_hypervisor_vm/` (this repo) pins and regenerates the patched tree
  (`setup.sh`: cloud-hypervisor `9a24680a…`, crosvm `e36cf699…`, vhost
  `eae4f737…`, SpectrumOS patch tarball sha256-pinned), and
  `../cloud-hypervisor-build/` is that regenerated tree; `../unfer/unfer_nixvm/`
  composes it into NixOS VMs. Licences for all of it are already rowed in the
  workspace-root `../ATTRIBUTION.md`.

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│                                  AustralVM Host OS                                  │
│  ┌────────────────────────┐  ┌────────────────────────┐  ┌──────────────────────┐   │
│  │   Austral User Cells    │  │   avm-pkg Package CAS   │  │ Cedar/manifest auth  │   │
│  │  (linear-type modules)  │  │ (kernel, rootfs, fw)    │  │ (auth.rs + avm-exec) │   │
│  └────────────┬───────────┘  └────────────┬───────────┘  └──────────┬───────────┘   │
│               │                           │                         │               │
│               ▼                           ▼                         │               │
│  ┌──────────────────────────────────────────────────┐               │               │
│  │  Linear VirtIO frontends — stdlib/sys/Virtio.aui  │               │               │
│  │   LinearDmaBuffer (linear record), BufferInFlight │               │               │
│  │   submit/complete ownership transfer, avm-virtio  │               │               │
│  └────────────────────────┬─────────────────────────┘               │               │
│                           │                                          │               │
│  ┌────────────────────────┴─────────────────────────┐               │               │
│  │ avm-vmm: vm.config JSON ↔ cloud-hypervisor REST   │◄──────────────┘ (lifecycle)   │
│  │ (docs/api.md), UDS API socket, KVM probe          │                               │
│  └────────────────────────┬─────────────────────────┘                               │
└───────────────────────────┼─────────────────────────────────────────────────────────┘
                            │ vhost-user: MsgHeader framing, GET_SHARED_MEMORY_REGIONS (1004),
                            │ SHMEM_MAP (1000) / SHMEM_UNMAP (1001) — SpectrumOS vhost patches
                            ▼
┌─────────────────────────────────────────────────────────────────────────────────────┐
│      Cloud-Hypervisor VMM (patched build per cloud_hypervisor_vm/setup.sh pins)      │
│        └── Linux Driver Domain (musl, VIRTIO*=y, erofs — model: spectrum/vm/sys/net) │
│              └── vendor firmware cells hydrated from avm-cas hot store               │
└─────────────────────────────────────────────────┬───────────────────────────────────┘
                                                  │ (physical I/O via IOMMU/VFIO)
                                                  ▼
                                       [ Physical Hardware ]
```

---

## 2. Grounded workspace topology

```
australVM/
├── safestos/pkg/                    # the standalone offline cargo workspace (PLAN_avm_pkg §4.6)
│   ├── avm-cas/                     # existing: CAS, hot store, chunker
│   ├── avm-eval/                    # existing: derivation DAG evaluation
│   ├── avm-exec/                    # existing: Cedar sandbox + unshare runner
│   ├── avm-cli/                     # existing: avm-pkg CLI
│   ├── avm-virtio/                  # [NEW] split VirtQueue + vhost-user codec + shmem registry
│   │   └── src/{lib,ring,vhost,shmem}.rs
│   └── avm-vmm/                     # [NEW] cloud-hypervisor topology config + REST launcher + lifecycle
│       └── src/{lib,config,launcher,lifecycle}.rs
└── stdlib/sys/                      # [NEW] Austral driver-domain interfaces (typecheck-gated)
    └── Virtio.aui / Virtio.aum      # linear DMA buffers; module `Virtio` (flat name, repo convention)
```

Two further Sprint-7 declarations live beside the existing package DSL, so
they share one plan file: `stdlib/pkg/LinuxDriverKernel.aui` (module
`LinuxDriverKernel`).

---

## 3. Subsystem technical specifications

### 3.1 VirtIO rings with Austral linear types (`stdlib/sys/Virtio.aui`)

Ownership of DMA buffers moves between the Austral side (frontend) and the
driver domain (backend) exactly like `LinearDmaBuffer`/`BufferInFlight` in
the original draft, grounded in what this compiler actually has:

* `linear record` is real (suite `007-borrowing`, `019-extern-rust/007`);
* there is **no** `MemoryCap` import and no `Address` without a type
  argument — buffers are `Address[Nat8]` plus an `Int64`/`Nat64` length,
  matching the existing extern-Rust boundary shape
  (`examples/kernel/UnferKernel.aui` convention: byte buffers are
  `Address[Nat8]` + `Int64` length);
* the safety property does **not** need Cedar to be provable at compile
  time: `submit_buffer` consumes the linear record, so touching the buffer
  after submit is a *type error*, pinned by an expected-compile-failure
  suite case (runner.py `TestFailure`).

### 3.2 vhost-user wire layer (`avm-virtio/src/vhost.rs`)

Implemented from the exact layouts the SpectrumOS patches add to
`vhost/src/vhost_user/message.rs` (read firsthand from
`../spectrum/pkgs/cloud-hypervisor/vhost/0003-…` and `0004-…`):

* `MsgHeader { request: u32, version: u32, flags: u32, size: u32 }`
  (little-endian), with the `0001-get_size` semantics: reply `size` is the
  reply payload length;
* request ids used here: `GET_SHARED_MEMORY_REGIONS = 1004` (frontend),
  `SHMEM_MAP = 1000`, `SHMEM_UNMAP = 1001` (backend, non-standard);
* `VhostSharedMemoryRegion { id: u8, padding: [u8;7], length: u64 }`;
* `VhostUserShmemMapMsg { flags: u8, shmid: u8, padding: [u8;6],
  shm_offset: u64, fd_offset: u64, len: u64 }` with `MAP_R = 0x1`,
  `MAP_W = 0x2`;
* `VhostUserShmemUnmapMsg { shmid: u8, padding: [u8;7], shm_offset: u64,
  len: u64 }`;
* protocol feature bit `SHARED_MEMORY_REGIONS = 0x8000_0000` (non-standard,
  patch 0004) — the flag that makes a frontend skip `1004` against
  backends that lack it.

### 3.3 VM topology config ↔ cloud-hypervisor REST (`avm-vmm`)

`config.rs` serializes a `VmConfig` subset to exactly the documented
`vm.create` payload of `../cloud-hypervisor-build/cloud-hypervisor/docs/api.md`:

```json
{"cpus":{"boot_vcpus":4,"max_vcpus":4},
 "payload":{"kernel":"…/vmlinux","cmdline":"console=ttyS0 …"},
 "disks":[{"path":"…"}], "rng":{"src":"//dev/urandom"},
 "net":[{"ip":"…","mask":"…","mac":"…"}]}
```

`launcher.rs` speaks the documented REST verbs over the API socket
(`PUT /api/v1/vm.create`, `PUT /api/v1/vm.boot`, `GET /api/v1/vm.info`,
`PUT /api/v1/vm.shutdown`) — HTTP-over-Unix-socket, hand-rolled on
`std::os::unix::net::UnixStream` (no new crates; offline discipline).
`lifecycle.rs` wires the CAS in through the assembled read path —
`ChunkStore::read_assembled` hydrates the kernel cell to a boot file,
because cloud-hypervisor takes a *file path* for the payload. Bridging the
hot store's zero-copy `acquire(manifest_hash) -> Arc<Mmap>` mapping
straight into a VMM fd is the recorded follow-up (§6 boot-latency row);
this is the honest current shape, not a pretended zero-copy. KVM/VMM
availability is **probed, never assumed** — `/dev/kvm` is really opened
and the same discipline `avm-exec/src/runner.rs` already uses for `unshare`
flags applies.

### 3.4 Packaging the driver domain with `avm-pkg`

The Linux driver kernel and vendor firmware are ordinary `Derivation`
constants (`stdlib/pkg/LinuxDriverKernel.aui`), exported by the same
`--pkg-totality=true --emit-derivations=plan.json` path as the seed package
set (PLAN_avm_pkg §4.8) and hashed by `avm-pkg drv-hash`. Firmware
ingestion is the one Sprint-7 pipeline fully executable here: synthetic
but structurally `.mbn`-shaped bundles (header + segment table + payloads,
three minor revisions) go through the real `avm-cas::put_buffer` and
FastCDC chunk reuse is measured (acceptance ≥ 80 % — measured at 98.6 %,
§5).

---

## 4. Roadmap (grounded re-scoping of Sprints 6 & 7)

### Sprint 6 — VirtIO + VMM substrate (all offline-testable)

* [x] **Task 6.1 `avm-virtio` ring + protocol + shmem registry.** Split
  VirtQueue (16-byte descriptors, avail/used rings, descriptor chains) with
  a simulated device proving ownership handoff; vhost-user codec per §3.2
  with byte-level layout tests pinned against the patch-defined structs;
  shmem registry mapping file-backed regions to `VhostSharedMemoryRegion`
  records — **no `mmap`, no `unsafe`** (the region file's mapping is the VMM
  backend's job; keeps PLAN_avm_pkg §6.3's single-mmap invariant).
* [x] **Task 6.2 `avm-vmm` config + launcher + lifecycle.** §3.3 config
  serialization pinned to the documented payload; UDS REST client tested
  against an in-crate mock server speaking the documented endpoints; launch
  **refuses honestly** (precise reason + pointer to
  `cloud_hypervisor_vm/setup.sh`) when no VMM binary is present; lifecycle
  hydrates a CAS manifest through the hot store and records the real KVM
  probe result.
* [x] **Task 6.3 `stdlib/sys/Virtio.aui` linear binding.** Compile-time
  buffer ownership transfer; expected-compile-failure suite case for
  use-after-submit. (Cedar `NetworkCap`-per-queue: the *type* half ships
  here as the linear token; the *policy* half belongs to the JIT manifest
  layer and is recorded, not faked.)

### Sprint 7 — driver-domain packaging

* [x] **Task 7.1 `stdlib/pkg/LinuxDriverKernel.aui`.** Data-only derivation
  whose kconfig intent mirrors `../spectrum/vm/sys/net/default.nix`
  (`VIRTIO=y`, `VIRTIO_BLK=y`, `VIRTIO_CONSOLE=y`, `VIRTIO_PCI=y`,
  `EROFS_FS=y`, console/VT baggage off). Exports into plan.json and
  `drv-hash`s; the kernel-compiling *builder* cannot run in this workspace
  (no kernel sources, offline) and says so rather than pretending.
* [x] **Task 7.2 firmware ingestion + dedup measurement.** Synthetic but
  structurally realistic `.mbn` bundles (3 minor revisions, shared content)
  through the real CAS; chunk-reuse percentage measured against the ≥ 80 %
  target.
* [x] **Task 7.3 end-to-end orchestration — executed 2026-10-18.** The
  pinned `../cloud-hypervisor-build` tree builds (cloud-hypervisor
  `v50.0-2-g2ffa2c05a`), and `avm-vmm/examples/boot_once` runs the whole
  path for real: CAS `put_buffer` of Linux 6.12.112 `bzImage` (12.8 MB) →
  hydrate → `vm.create` → `vm.boot` → `state=Running` in 1 poll
  (create→Running **1.1–1.2 ms**, repeatable) → `vm.shutdown` OK; the
  headless guest panics internally and the host survives and relaunches.
  Running it against a real VMM exposed and fixed three real bugs (HTTP
  keep-alive hang, `run()` never spawning the VMM, `wait_ready` retrying
  an answered 500) — §5. The pass-through-NIC line-rate row remains in
  §6.

### Sprint 8 — cloud profile: Hermit unikernel driver domains (planned 2026-10-18, not executed)

Grounded spec and fit assessment in §8. Tasks re-scoped to this workspace
the same way Sprint 6/7 were:

* [x] **Task 8.1 `avm-vmm` deployment profiles.**
  `DriverDomainProfile::{LinuxMicroVm, HermitUnikernel}` in `config.rs`
  (vCPU/memory policy: 2/unmodelled vs 1/64 MiB, both pinned by tests);
  the Hermit domain descriptor pinned to the shape `../hermit-rs/k8s/httpd.yml`
  actually deploys (1 vCPU, 64 MiB, no disks, virtio-only net via
  `with_net`) — a separate `HermitConfig` record, because the Uhyve/QEMU
  runners take CLI resources, not a `vm.create` body (recorded deviation
  from the earlier wording of this task, §8.6); runner probe order Uhyve →
  QEMU + Hermit loader (both named in `hermit/README.md`; QEMU counts only
  with the loader present), with the same refuse-honestly semantics as
  Task 6.2. **Measured:** 24 `avm-vmm` tests pass (§5).
* [x] **Task 8.2 `stdlib/pkg/HermitDomain.aui` data-only derivation.**
  Mirrors `LinuxDriverKernel.aui` (§3.4): image, cargo-feature kernel
  config list, 64 MiB budget, pinned toolchain `nightly-2026-08-01`;
  exports via `--emit-derivations=plan.json` and byte-stable `drv-hash`.
  **Measured:** 6 derivations export `proven`; `hermit-domain` drv-hash
  `fd014a84…bbb41` byte-stable (§5).
* [x] **Task 8.3 boot-latency harness.** `vm.info`-polling latency log in
  `lifecycle.rs` ([`measure_boot_latency`]), tested against the in-crate
  mock server (offline). **Measured:** mock guest Created→Running in 2
  polls over ≥ 1 poll interval; a never-running guest reports
  `running: false`, API errors propagate — never a fake success. The real
  < 50 ms number stays host-gated (§6).
* [x] **Task 8.4 boot a Hermit guest — executed 2026-10-18.**
  `hello_world` (app + kernel, one 8.9 KB ELF) built with the pinned
  `nightly-2026-08-01` and the `../kernel` clone, booted under QEMU 10.2.4
  + hermit-loader v0.5.7 exactly as CI does; spawn→`Hello, world!`
  **200.8–209.0 ms (TCG) / 257.7–289.9 ms (KVM)**, clean guest exit.
  **Same day, follow-up:** Uhyve v0.10.0's VM-setup failure was
  root-caused and fixed (`docs/uhyve-0.10.0-kvm-compat.patch`) — uhyve
  now boots the same guest here: **245.7–262.6 ms**, and
  **160.3–173.1 ms** for the compile-time-pruned image with `--pit`
  (§5). The < 50 ms target is still not met; the remainder is hermit's
  own timer calibration (verified in `../kernel`'s `calibrate_timer`).
* [x] **Task 8.5 `stdlib/pkg/HermitCDomain.aui` data-only derivation.**
  The C-flavour sibling of Task 8.2 pinning `../hermit-c/`'s build facts:
  `x86_64-hermit-gcc` + `libhermit.a`, the `meson.build` kernel feature
  list (`acpi,dhcpv4,dns,mman,newlib,smp,tcp,udp,virtio-net,loader`),
  same 64 MiB/1-vCPU shape. **Measured:** 7 derivations export `proven`;
  `hermit-c-domain` drv-hash `9fc68b02…c228b` byte-stable (§5).
* [ ] **Task 8.6 umbrella → Hermit target (seam swap executed 2026-10-18).**
  §8.4: cross-build `umbrella/modules/rscrate` for `x86_64-unknown-hermit`,
  swap the `dlopen` registration for static linkage behind the bridge's
  import seam, and boot the Austral+Rust domain as a §8.2 unikernel guest.
  **Done:** the cross-build (`librscrate.a`, staticlib added beside cdylib)
  and the CPS bake (2 functions / 297 B emitted; the JIT's
  `can't resolve symbol rs_demo_checksum` is the expected ungated
  refusal) — §5; **and the seam swap itself** — the bridge grew a static
  import registry (`register_static_symbol` + the C-ABI
  `austral_register_static_symbol` a Hermit-side image calls in place of
  dlopen), every JIT builder consults it at init, and 3 integration
  tests pin the contract: a symbol from *neither* the CPS binary *nor*
  the baked `rs_*` table resolves and executes; registration does **not**
  bypass the manifest gate (UK-4001 under DenyAll); late registration is
  refused, not silently ineffective (`safestos/cranelift/tests/static_seam.rs`).
  **Outstanding:** the in-guest boot itself (bake the CPS into a Hermit
  image beside the statically registered `librscrate.a`; W^X in-guest
  the open question). The runner gap is closed: uhyve boots Hermit
  images on this host after `docs/uhyve-0.10.0-kvm-compat.patch` (§5).

## 5. Measured results & reproduce block

Measured 2026-10-18 on this workspace (offline; `/dev/kvm` present, no VMM
binary, no pass-through NIC). See `PLAN_avm_pkg.md` §3 for the underlying
CAS/eval numbers this pipeline reuses.

| item | measured | evidence |
|---|---|---|
| `avm-virtio` | **19 tests**: split-ring submit→device→complete roundtrip incl. 65 539 iterations across the u16 index wrap, cyclic-chain refusal (error, not hang), vhost-user byte layouts pinned to the four SpectrumOS patch structs, shmem map/unmap range validation — **no `mmap`, no `unsafe`** | `cargo test -p avm-virtio` |
| `avm-vmm` | **14 tests**: `vm.create` payload keys pinned to `docs/api.md`'s exact example (cpus/payload/disks/rng/net, nothing invented); mock UDS server records the four documented verbs `PUT vm.create` → `PUT vm.boot` → `GET vm.info` → `PUT vm.shutdown`; non-2xx → `Http` error; dead socket → `Io` error (**never a fake success**); no VMM binary → refusal naming `setup.sh`; `/dev/kvm` actually opened or the probe result quoted | `cargo test -p avm-vmm` |
| Rust workspace gates | `cargo fmt --all --check` **clean**; `cargo test --workspace` → **106 passed, 0 failed, 5 ignored** (the 5 are PLAN_avm_pkg's heavy acceptances) | reproduce block below |
| VirtIO typecheck + ownership gate | `stdlib/sys/Virtio.{aui,aum}` compiles through the full pipeline via `021-virtio/001-ownership-roundtrip` (prints `64`: allocate → submit → complete → read → free-exactly-once) and standalone with `--target-type=tc` (new CI step *typecheck stdlib/sys (Virtio)*) | `python test-programs/runner.py` |
| Buffer invariance (use-after-submit) | expected compile failure pinned to the exact JSON linearity diagnostic *"The variable `buffer` has already been consumed."* on the use-after-submit line | `test-programs/suites/021-virtio/002-use-after-submit/austral-stderr.txt` |
| Driver-kernel derivation | all **5** derivations export with `"totality": "proven"` (4 seed + linux-driver-kernel); `avm-pkg drv-hash … linux-driver-kernel` = `2a71d4ceada7da7f19b46863bcc023a8f438d56013fc8e7a98502d64e33afeee`, **byte-stable across reruns** | reproduce block below |
| Firmware ingestion (≥ 80 % acceptance) | 3 revisions of a synthetic 4 MiB `.mbn`-shaped bundle through the real CAS: rev2 **98.6 %** (71/72) and rev3 **98.6 %** (71/72) chunks already in store ⇒ follow-on reuse **142/144 = 98.6 %** — at/above the ≥ 80 % target; storage saving 74 distinct/216 total = **65.7 %** (the ~⅔ floor three near-identical revisions imply — printed because it is the different, bytes-not-stored question) | `cargo test -p avm-cas firmware_bundle_dedup -- --nocapture` |
| Full e2e + docs gates | runner suite **311 PASS / 0 fail** (incl. both 021-virtio cases), `dune build bin/ lib/ test/` + `dune runtest` **OK**, `check-links`: **233 documents OK** | commands below |
| **Sprint 8, offline (cloud profile)** | `avm-vmm` grew to **24 tests** (was 14): Hermit domain shape pinned to `../hermit-rs/k8s/httpd.yml` (1 vCPU, 64 MiB, no disks, virtio net via `with_net`), profile policy pinned (2/unmodelled vs 1/64 MiB), runner probe Uhyve → QEMU+loader with both-or-neither and an honest refusal naming `hermit-os/uhyve` + `hermit-os/loader`, boot-latency harness mock-verified (Created→Running in 2 polls over ≥ 1 poll interval; a never-running guest reports `running: false`; API errors propagate) | `cargo test -p avm-vmm` |
| Hermit domain derivations (Sprint 8) | **7** derivations export `"totality": "proven"` (4 seed + linux-driver-kernel + hermit-domain + hermit-c-domain); `avm-pkg drv-hash … hermit-domain` = `fd014a84d68c8e281fa534ea9b0c8730c77d429525dcfd1e381fef2bf91bbb41`, `… hermit-c-domain` = `9fc68b02d419c5116514754e81f0e34686e1da7ab0c7513969016d5ad76c228b`, both byte-stable across reruns | reproduce block, §5 step 2 |
| Hermit guest boot (Task 8.4) | **measured**: `hello_world` unikernel (app+kernel, one 8.9 KB ELF) boots under QEMU 10.2.4 + hermit-loader v0.5.7, 64 MiB guest, clean exit via `isa-debug-exit` (code 3 = exit(0)). spawn→`Hello, world!`: **200.8–209.0 ms (TCG)**, **257.7–289.9 ms (KVM, `-cpu host`)**. Uhyve v0.10.0 as published fails its VM setup here — `BackendError(Error(22))` at `uhyve-0.10.0/src/vm.rs:505` — see the next row for the root cause and the fix | reproduce block, §5 step 5 |
| Uhyve on a new-KVM host (follow-up, Task 8.4) | **measured**: root cause isolated with an env-gated cumulative bisection (patch block `UHYVE_SREGS_DIAG`): KVM on Linux 7.2.9 validates EFER bits written via `KVM_SET_SREGS` against the **current vCPU CPUID** — any nonzero EFER before `KVM_SET_CPUID2` is EINVAL; uhyve 0.10.0's `init()` runs `setup_long_mode` (the EFER write) before `setup_cpuid`. The fix is a reorder plus two guest-visible gaps fixed (COM1 UART emulation for hermit's polled serial console; MMIO scan tolerance for device-less virtio windows) — `docs/uhyve-0.10.0-kvm-compat.patch`, applied to a crates.io source copy. After the patch the **same loader image** boots: full-feature image spawn→`Hello, world!` **245.7–262.6 ms** (5 runs); compile-time-pruned image (`--no-default-features`, the §8.2 "prune at compile time" story) + uhyve `--pit`: **160.3–173.1 ms** (7 runs, median 160.9 ms), clean guest exit (status 0). The < 50 ms target is **still not met**; the remainder is the guest itself — hermit's own timer calibration, including a verified **30 ms** APIC-timer wait (`../kernel/src/arch/x86_64/kernel/apic.rs` `calibrate_timer`: "Wait 30ms for accuracy") — plus ~74 ms to system-time init | reproduce block, §5 step 5b |
| Task 7.3 real orchestration run | **measured**: cloud-hypervisor `v50.0-2-g2ffa2c05a` (built from the pinned `../cloud-hypervisor-build` tree); `avm-vmm/examples/boot_once`: CAS `put_buffer` of Linux 6.12.112 `bzImage` (12.8 MB) → hydrate → `vm.create` (the documented payload is accepted as-is; CH defaults memory to 512 MiB) → `vm.boot` → `state=Running` in **1 poll** (create→Running **1.1–1.2 ms**, repeatable) → `vm.shutdown` OK. The headless guest panics internally (no init) and the host survives and relaunches — the 100 ms restart budget is met VMM-side. Still **not covered**: guest-VirtIO-ready latency (< 50 ms) and ≥ 92 % of 10 GbE (no pass-through NIC) — both rows advanced by the follow-up rows below | reproduce block, §5 step 5 |
| Real-run repairs (found by the Task 7.3 run) | three real bugs exposed by the first real VMM binary, each fixed with a regression test: `lifecycle::run` mapped the *binary path* into the API socket and never spawned the VMM; the HTTP client's `read_to_string` hung on cloud-hypervisor's keep-alive connections (now Content-Length-framed reads + 10 s backstop, pinned by `keep_alive_server_is_read_by_content_length_not_read_to_end`); `wait_ready` retried an answered 500 "VM is not created" forever (a non-2xx answer now counts as API-up, pinned by `wait_ready_counts_a_non_2xx_answer_as_api_up`). `avm-vmm` = **28 tests** after the follow-up pass (below) | `cargo test -p avm-vmm` |
| In-guest VirtIO-ready (follow-up, Task 8.3/7.3) | **measured with in-guest instrumentation**: `boot_once` now spawns the VMM with its console captured (`spawn_vmm_logged`) and sends the guest `ttyS0` to a file via cloud-hypervisor's `serial` device (mode `File` — its default is `Null`, output dropped; opt-in through the documented `ConsoleConfig` shape, pinned by `serial_sink_is_opt_in_and_matches_the_console_config_shape`). The guest kernel's own `printk.time` stamps come back over the captured serial: the first virtio/`1af4` PCI-enumeration line lands at **guest-clock +0.284999 / +0.286876 / +0.292423 s** (3 runs; create→Running stays **0.2–0.9 ms** VMM-side). The < 50 ms VirtIO-ready target is **not met** by a full Linux guest (the number is now real and in-guest, not "not covered"); the Hermit profile's pruned 160 ms boot (row above) is the credible path to it | reproduce block, §5 step 5 |
| Static import seam (follow-up, Task 8.6) | the bridge's dlopen-free resolution path ships: `register_static_symbol` + C-ABI `austral_register_static_symbol`, consulted by every JIT builder at init. `safestos/cranelift/tests/static_seam.rs` (**3 tests**): a symbol present in neither the CPS binary nor the baked `rs_*` table resolves through the registry and executes over a live buffer; registration does not bypass the manifest gate (UK-4001 under DenyAll, granted-by-name passes); registration after the thread's JIT init is refused, not silently ineffective. Bridge suite green after the change | `cargo test --test static_seam` in `safestos/cranelift` |
| Umbrella → Hermit (Task 8.6) | `umbrella/modules/rscrate` cross-builds for `x86_64-unknown-hermit` → `librscrate.a` (staticlib crate-type added beside cdylib); the CPS bake emits **2 functions / 297 B** (`--emit-cps`); the ungated JIT refuses exactly as designed (`can't resolve symbol rs_demo_checksum` without the dlopen'd crate). The dlopen→static seam swap is now built (row above); the in-guest boot remains (§8.4) | reproduce block, §5 step 5 |

Reproduce every number:

```sh
# 1. Rust workspace: fmt + fast suite (5 heavy acceptances ignored by default)
cd safestos/pkg
CARGO_NET_OFFLINE=true cargo fmt --all --check
CARGO_NET_OFFLINE=true cargo test --workspace
cargo test -p avm-virtio                           # 19 Sprint-6 ring/codec tests
cargo test -p avm-vmm                              # 14 config/launcher/lifecycle tests
cargo test -p avm-cas firmware_bundle_dedup -- --nocapture   # ≥ 80 % row

# 2. Driver-kernel + cloud-profile derivation export + stable drv-hashes
cd ../..                                           # australVM/
export LD_LIBRARY_PATH=$PWD/safestos/cranelift/target/release
./austral compile stdlib/pkg/Derivation.aui,stdlib/pkg/Derivation.aum \
    stdlib/pkg/Stdenv.aui,stdlib/pkg/Stdenv.aum stdlib/pkg/Bootstrap.aum \
    stdlib/pkg/LinuxDriverKernel.aui,stdlib/pkg/LinuxDriverKernel.aum \
    stdlib/pkg/HermitDomain.aui,stdlib/pkg/HermitDomain.aum \
    stdlib/pkg/HermitCDomain.aui,stdlib/pkg/HermitCDomain.aum \
    --target-type=tc --pkg-totality=true --emit-derivations=plan.json
cd safestos/pkg && cargo run -p avm-cli -- drv-hash ../../plan.json linux-driver-kernel && \
    cargo run -p avm-cli -- drv-hash ../../plan.json hermit-domain && \
    cargo run -p avm-cli -- drv-hash ../../plan.json hermit-c-domain

# 3. VirtIO standalone typecheck gate + the e2e suite (python from the flake)
cd ../..
nix develop -c bash -c 'export LD_LIBRARY_PATH=$PWD/safestos/cranelift/target/release; \
    ./austral compile stdlib/sys/Virtio.aui,stdlib/sys/Virtio.aum --target-type=tc && \
    python test-programs/runner.py'

# 4. OCaml + docs gates
nix develop -c bash -c 'export LD_LIBRARY_PATH=$PWD/safestos/cranelift/target/release; \
    dune build bin/ lib/ test/ && dune runtest && python3 scripts/check-links'
```

# 5. Cloud-profile runs (2026-10-18; network + KVM host — the §6 rows)
#    Hermit guest: build with the pinned nightly + the ../kernel clone,
#    boot under QEMU + hermit-loader v0.5.7 (hermit-os/loader release),
#    and time spawn -> "Hello, world!" (see §5's boot row).
#    (HERMIT_MANIFEST_DIR must be absolute: hermit's build.rs resolves a
#    relative value against the hermit crate dir, where the kernel
#    submodule is not checked out in this workspace.)
cd ../hermit-rs
HERMIT_MANIFEST_DIR="$(realpath ../kernel)" cargo +nightly-2026-08-01 build \
    -Zbuild-std=std,panic_abort --target=x86_64-unknown-hermit --release \
    --package hello_world --features hermit/loader
nix shell nixpkgs#qemu -c qemu-system-x86_64 -display none -smp 1 -m 64M \
    -serial stdio -kernel /path/to/hermit-loader-x86_64-multiboot \
    -initrd target/x86_64-unknown-hermit/release/hello_world \
    -cpu qemu64,apic,fsgsbase,rdtscp,xsave,xsaveopt,fxsr \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04
#    Task 7.3's real run (VMM from the pinned tree, real guest kernel):
cd ../australVM/safestos/pkg
cargo run --release --example boot_once -- /path/to/linux/bzImage /tmp/avm-boot
#    ...with the in-guest VirtIO-ready read-out: the third argument is
#    appended to the guest cmdline; boot_once's serial sink + settle
#    poll print `[virtio] first in-guest virtio log line at guest-clock
#    +T s` (§5's in-guest row). The kernel image can be hydrated back
#    out of the CAS the first run built:
cargo run -p avm-cli -- --store /tmp/avm-boot/store get <manifest-hex> \
    -o /tmp/linux-6.12-bzImage
# 5b. Uhyve on a new-KVM host (§5's uhyve row): apply the compat patch
#     to a crates.io source copy and build it offline.
cp -r ~/.cargo/registry/src/index.crates.io-*/uhyve-0.10.0 ~/uhyve-patched
chmod -R u+w ~/uhyve-patched && cd ~/uhyve-patched
patch -p0 < /path/to/australVM/docs/uhyve-0.10.0-kvm-compat.patch
cargo build --release --offline
#     full image (the same loader build as 5):
./target/release/uhyve /path/to/hermit-rs/target/x86_64-unknown-hermit/release/hello_world
#     pruned image + PIT (the 160 ms row):
HERMIT_MANIFEST_DIR="$(realpath ../kernel)" cargo +nightly-2026-08-01 build \
    -Zbuild-std=std,panic_abort --target=x86_64-unknown-hermit --release \
    --package hello_world --features hermit/loader --no-default-features
./target/release/uhyve --pit target/x86_64-unknown-hermit/release/hello_world
#     then time spawn -> "Hello, world!" across runs (§5's numbers).
#     UHYVE_SREGS_DIAG=1 runs the env-gated EFER/CPUID bisection instead
#     of booting — the evidence for the root cause in §5.

`PLAN_avm_pkg.md` §3 remains authoritative for the CAS/eval numbers the
Sprint-7 pipeline reuses (chunker, hot store, warm evaluation).

## 6. Not covered — hardware- and network-gated rows

| criterion | target | status in this workspace | first action on a capable host |
|---|---|---|---|
| VM boot latency | CAS-hydrated driver VM to VirtIO-ready < 50 ms | **measured in-guest** (follow-up, §5): the full path runs — `vm.create`→`vm.boot`→`Running` = **0.2–1.5 ms** VMM-ack — and the guest-side number is now real: the timestamped console puts the first virtio/`1af4` PCI-enumeration at **284.999–292.423 ms guest-clock** (full Linux guest). The < 50 ms target is **not met** by a full Linux guest; the Hermit profile boots to user code in **160.3–173.1 ms** (pruned image, patched uhyve + PIT) with the remainder quantified as hermit's own timer calibration (30 ms APIC wait verified in `../kernel`) — a pruned Hermit driver domain is the credible < 50 ms path, still to be measured *to VirtIO-ready* rather than to user code | §5 steps 5/5b (`boot_once` in-guest run; patched uhyve) |
| Fault quarantine | host survives driver-VM panic; restart < 100 ms | **measured** (§5): the headless guest panics internally; host + VMM survive, relaunch create→Running **1.2 ms** (restart budget met VMM-side). Deliberate `sysrq` panic induction still open | §5 step 5 (`boot_once` re-run) |
| VirtIO line rate | ≥ 92 % of 10 GbE | **not covered — and now inventoried**: this machine's only network hardware is a MediaTek MT7922 Wi-Fi 6E adapter and a USB Realtek NIC negotiated at **1000 Mb/s**; there is no 10 GbE NIC and no SR-IOV VF to bind via VFIO, so the row is physically unmeasurable here (never "passed"). First action on capable hardware unchanged | VFIO bind + `iperf3` through the driver domain |
| Firmware dedup | ≥ 80 % chunk reuse, 3 minor revisions | **measured** (§5) | re-run `cargo test -p avm-cas firmware_bundle_dedup -- --nocapture` |
| Buffer invariance | use-after-submit fails at compile time | **measured** — `021-virtio/002-use-after-submit` pins the exact *"has already been consumed"* JSON diagnostic | `nix develop -c python test-programs/runner.py` |
| Launch honesty (no VMM binary) | refusal names `setup.sh`, never a fake success | **measured** (`avm-vmm` unit tests: dead socket → `Io`, missing binary → refusal) | `cargo test -p avm-vmm` |
| Driver-kernel recipe | export + byte-stable `drv-hash` in this workspace | **measured** (5 derivations `proven`; hash `2a71d4ce…ee`, §5) | reproduce block, §5 |
| vhost-user shmem codec | byte-layout parity with SpectrumOS patches | **measured** (unit tests pinned to patch structs) | `cargo test -p avm-virtio` |

## 7. Provenance

Design-level adaptations, no vendored code: SpectrumOS vhost-user shmem
patches (`../spectrum/pkgs/cloud-hypervisor/vhost/`, Apache-2.0 +
LicenseRef-BSD-3-Clause-Google per the firsthand-licence row in the
workspace-root ledger), rust-vmm `vhost` message framing (Apache-2.0 +
BSD-3-Clause), and the cloud-hypervisor REST surface (docs, Apache-2.0 +
BSD-3-Clause; docs CC-BY-4.0) — rowed in `../ATTRIBUTION.md` in this repo's
ledger.

Hermit (`../hermit-rs/`, **MIT OR Apache-2.0** — `LICENSE-MIT` and
`LICENSE-APACHE` read firsthand 2026-10-18) is credited for the Sprint-8
cloud-profile shape only: unikernel-as-VM deployment (the KubeVirt
`VirtualMachineInstanceReplicaSet` in `k8s/httpd.yml`: 1 core, 64 MiB,
containerdisk), the runner set (Uhyve or QEMU + the Hermit loader, per
`hermit/README.md` and CI), and the cargo-feature kernel-configuration
model (`hermit/virtio-net`, `hermit/dhcpv4`, `hermit/fs`, …). Design-level
adaptation, no code copied. The `hermit-kernel` source is the sibling
clone `../kernel` — licence **MIT OR Apache-2.0** and the kernel-internal
facts of §8.2 read firsthand there 2026-10-18. Rowed in
`../ATTRIBUTION.md` and this repo's ledger.

---

## 8. Cloud profile (planned, not executed) — Hermit unikernel driver domains

### 8.1 Why a second profile

§1's shape is the **bare-metal profile**: AustralVM owns real PCIe/Wi-Fi/
GPU hardware and delegates the messy vendor stacks to disposable Linux
driver domains, because only Linux carries VFIO pass-through and vendor
firmware. That choice is hardware-driven, not taste.

In a **cloud** deployment there is no such hardware to pass through — the
devices are virtio-only (net/block/console). There, a full Linux guest is
overhead (init, userspace, distro patching) and attack surface for zero
vendor-driver benefit. `../hermit-rs/` is the counter-proposal: a Rust
unikernel where the application is statically linked with the kernel
library into a single image (`hermit/README.md`: "This crate builds and
links against the Hermit kernel … to create a Hermit unikernel image").

### 8.2 What `../hermit-rs/` actually is (firsthand, 2026-10-18)

* **Shape:** app + kernel library = one ELF; `hermit-abi` is the `no_std`
  syscall surface between them. No userspace, no init, no shell.
* **Targets:** `x86_64-unknown-hermit`, `aarch64-unknown-hermit`,
  `riscv64gc-unknown-hermit` (CI runs all three), nightly
  `-Zbuild-std=std,panic_abort` (toolchain pinned `nightly-2026-08-01`) or
  `rust-std-hermit` on stable.
* **Runners:** the specialized Uhyve unikernel hypervisor, or QEMU via the
  Hermit loader (`hermit/README.md` "Running"); CI boots `rusty_demo`,
  `httpd` (virtio-net-pci + DHCP), and `hermit-wasm` (virtio-fs-pci) under
  QEMU.
* **Kernel configuration is a cargo-feature list** — `hermit/virtio-net`,
  `hermit/dhcpv4`, `hermit/udp`, `hermit/fs`, smoltcp TCP/IP, log levels
  (`hermit/Cargo.toml`) — i.e. the guest is pruned at compile time, not
  configured down at boot. This maps directly onto §3.4's data-only
  derivation idea: the recipe records the feature list.
* **Dev mode:** the `common-os` feature runs the same binary on a common
  OS, so profile logic can be developed without a hypervisor.
* **Cloud deployment shape (the decisive evidence):** `k8s/httpd.yml` is a
  KubeVirt `VirtualMachineInstanceReplicaSet` — 2 replicas, 1 core, **64
  MiB** memory, `containerdisk`, bridged `rtl8139`. Unikernel-as-VM on
  Kubernetes is the intended cloud story.
* **Licence:** MIT OR Apache-2.0 (both licence files + `hermit/Cargo.toml`).
* **Kernel source (verified 2026-10-18):** the `hermit-kernel` checkout
  is the sibling clone `../kernel` (the repos' `kernel/` submodules point
  at `hermit-os/kernel`). Firsthand from it: licence **MIT OR
  Apache-2.0** (`LICENSE-MIT`, `LICENSE-APACHE`, `Cargo.toml`); feature
  set real — `mman` (`src/syscalls/mman.rs`: `sys_mmap` +
  `MemoryProtection`), `newlib`, `loader`, `acpi`, `pci`, `smp`,
  smoltcp `tcp`/`udp`/`dhcpv4`/`dns`, `virtio` +
  `virtio-net`/`console`/`fs`/`rng`/`vsock` (`src/drivers/virtio/`);
  Uhyve hypercall support in-tree (`src/uhyve.rs`); a
  `measure_startup_time` test app ships in `tests/`.

### 8.3 C driver stacks — `../hermit-c/`

The bare-metal driver domains exist to wrap messy **vendor C** stacks. The
cloud profile would be a poor substitute if only Rust apps could run in a
Hermit domain — and `../hermit-c/` (read firsthand 2026-10-18) closes
exactly that gap: it builds C applications against the Hermit kernel.

* **Toolchain:** `x86_64-hermit-gcc` (the cross compiler shipped as the
  `hermit-os/hermit-gcc` Docker image), linking against `libhermit.a`
  built from the kernel repo via its `xtask`; Meson `cross/` files cover
  x86_64, aarch64, and riscv64.
* **Kernel feature set for C workloads** (pinned in `meson.build`):
  `acpi,dhcpv4,dns,mman,newlib,smp,tcp,udp,virtio-net,loader` (+ `pci`
  except riscv64) — note `newlib`: a libc on the unikernel, sockets and
  pthreads included (`src/`: `http_server*.c`, `pthreads.c`, `fs.c`,
  `getaddrinfo.c`, …).
* **Same runners, same shape:** its README boots with QEMU + the Hermit
  loader (`-kernel hermit-loader-x86_64-multiboot -initrd <app>`) or
  Uhyve — the §8.2 runner set unchanged, so a C domain launches through
  the same `avm-vmm` path and measures on the same §8.6 harness.
* **Fit:** a vendor C daemon (storage target, protocol offload, a
  virtio-backend shim) becomes a cloud-profile driver domain at the same
  64 MiB/1-vCPU budget; the safety story shifts from "Rust kernel" to
  "single-purpose, feature-pruned, no userspace, capability host in front".
* **Licence:** MIT OR Apache-2.0 (read firsthand). The shared kernel
  source is `../kernel` (§8.2) — its `newlib` feature is exactly what
  C workloads need, now verified in the kernel's own `Cargo.toml`.

### 8.4 Austral programs on Hermit — the umbrella compiler (`umbrella/`)

Firsthand (2026-10-18): `umbrella/` (PLAN_valen §5 P3) is the
australVM-as-a-service host that runs the extern-Rust pipeline end to end
without one umbrella-specific line inside australVM — cargo builds an
arbitrary crate (`modules/rscrate`) to a cdylib,
`dlopen(RTLD_NOW|RTLD_GLOBAL)` makes its `rs_*` symbols visible to the
Cranelift bridge's `dlsym(RTLD_DEFAULT)` fallback, the stock `austral`
binary serializes the Austral module to CPS (`--use-cps-jit --allow-all
--target-type=tc --emit-cps`), and the bridge compiles and runs it
in-process with the `module.toml` grant gate live (no grant ⇒ UK-4001,
exit 1 before any foreign code runs).

**Why it matters for the cloud profile:** the umbrella is the piece that
would let a driver domain's *logic* be Austral — linear types and the
manifest gate included — on top of Hermit, instead of a hand-written C or
Rust daemon:

* the Rust side already compiles for Hermit (`x86_64-unknown-hermit`,
  `rust-std-hermit` or `-Zbuild-std`, §8.2) — that is what the umbrella's
  cargo step produces, for another target triple;
* the Austral side is target-independent CPS; what changes is *where the
  CPS is compiled and executed* — in-guest instead of in the host.

**Two recorded integration options (Task 8.6; option 1's resolver is now
built, 2026-10-18 follow-up):**

1. **Static image (the first step):** cross-build `modules/rscrate` for
   `*-unknown-hermit`, link it into the image beside a Hermit build of the
   umbrella host, and replace the `dlopen` step with a static symbol
   resolver — `dlsym(RTLD_DEFAULT)` is a Linux-ism and does not exist on
   Hermit; the bridge's import seam is the only place needing a Hermit
   variant. CPS is serialized at build time and either baked into the
   image (AOT through the existing bridge compile step) or compiled
   in-guest.
2. **In-guest JIT:** the Cranelift bridge JITs at runtime inside the
   unikernel. Hermit's `mman` kernel feature provides `sys_mmap` with
   `MemoryProtection` flags (verified in `../kernel/src/syscalls/mman.rs`),
   so the page permissions exist; W^X/JIT viability in-guest is the one
   remaining question for this option.

**Status after the 2026-10-18 follow-up pass:** the cross-build exists
(`librscrate.a` beside the cdylib), and the seam swap from option 1 is
*implemented in the bridge itself* — `register_static_symbol` + the
C-ABI `austral_register_static_symbol` a Hermit-side image calls in
place of dlopen, consulted by every JIT builder at init, with the
manifest gate provably unchanged (registration is resolution, not
permission — pinned by `safestos/cranelift/tests/static_seam.rs`, §5).
What remains is the in-guest boot: bake the CPS into a Hermit image
beside the statically registered archive and run it — the runner gap is
closed (patched uhyve boots Hermit images on this host, §5), leaving
only the image composition and the W^X question of option 2. The gate
moving *inside* the domain is unchanged: a strict improvement, since
the capability gate then travels with the guest.

### 8.5 Fit — what the cloud profile replaces, and what it does not

| | bare-metal profile (§1–§7) | cloud profile (this section) |
|---|---|---|
| driver guest | Linux (musl netvm shape, `../spectrum/vm/sys/net`) | Hermit unikernel image |
| devices | VFIO/PCIe pass-through + virtio | virtio only (net/blk/console) |
| vendor firmware | Linux-only problem | not applicable — no vendor hardware |
| memory class | hundreds of MiB | 64 MiB (the `k8s/httpd.yml` shape) |
| boot latency target | < 50 ms, **not covered** (§6) | < 50 ms credible (one image, no userspace) — still **not covered until measured** |
| safety story | capability host + disposable guest | plus: Rust kernel, no shell/init, feature-pruned attack surface |

**What it does not replace:** bare-metal driver domains stay Linux. Hermit
cannot host VFIO vendor firmware, and this plan does not pretend otherwise.

### 8.6 Integration shape

* `avm-vmm` grows a `DriverDomainProfile` (Task 8.1); both profiles share
  the existing `vm.create`/REST path (§3.3) and the CAS hydration path
  (§3.3) — the profile only changes the payload (no disks, 64 MiB, 1 vCPU,
  `payload.kernel` = unikernel image) and the runner probe (Uhyve → QEMU +
  loader, never assumed; refusal names the missing piece exactly as
  Task 6.2 does).
* The host side of the vhost-user/shmem channel (§3.2) is unchanged. In the
  Hermit profile the guest side is virtio within the guest (the QEMU/
  KubeVirt shape CI proves); a hermit-side backend speaking §3.2's
  patch-defined shmem wire is a recorded follow-up that requires the
  `kernel` submodule to be checked out first — do not assume wire parity.
* `stdlib/pkg/HermitDomain.aui` (Task 8.2) is the sibling of
  `LinuxDriverKernel.aui`: a data-only derivation pinning image, feature
  list, memory budget, and toolchain, exported and `drv-hash`ed exactly as
  §3.4 does.

### 8.7 Honest caveats

* The `hermit-kernel` source is the sibling clone `../kernel`: licence
  (MIT OR Apache-2.0) and the §8.2/§8.3 kernel-internal facts are verified
  firsthand. The §3.2 shmem-wire parity question stays a recorded
  follow-up — the kernel's virtio is a device *frontend*, not a vhost-user
  backend.
* The nightly pin (`nightly-2026-08-01`) is part of reproducibility; the
  derivation records it.
* Tasks 8.1–8.5 are measured (§5) — 8.4's Hermit guests really boot
  (201–290 ms to user code under QEMU+loader; **245.7–262.6 ms under
  the now-patched uhyve, 160.3–173.1 ms pruned+PIT**) and task 7.3's
  real run works (create→Running 0.2–1.5 ms). The VirtIO-ready number
  is now measured in-guest (**284.999–292.423 ms** for the full Linux
  guest; §5) — still above the < 50 ms target, with the remainder
  attributed to guest-side timer calibration rather than unknown.
  Line rate (≥ 92 % of 10 GbE) stays *not covered*: this machine has no
  10 GbE NIC and no VF (inventory in §6). Task 8.6's in-guest boot is
  the one open item — its runner gap is closed and its seam is built.
