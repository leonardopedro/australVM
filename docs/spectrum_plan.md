# Implementation Plan Extension: Driver Domains via Cloud-Hypervisor & SpectrumOS Integration

---

## 1. Architectural Convergence

This extension integrates **Path A (Driver Domains)** directly into the `avm-pkg` and `australVM` implementation plan. 

By leveraging the patched **Cloud-Hypervisor** architecture from **SpectrumOS** and the orchestration work in **`leonardopedro/unfer`**, AustralVM acts as the capability-secure host OS. It delegates messy, vendor-specific hardware interactions (PCIe, Wi-Fi, GPU, firmware blobs) to lightweight, disposable Linux micro-VMs ("Driver Domains"), communicating via **`vhost-user` and shared-memory VirtIO channels**.

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                   AustralVM Host OS                                    │
│   ┌───────────────────────────┐    ┌───────────────────────────┐    ┌───────────────┐  │
│   │   Austral User Cells      │    │    avm-pkg Package CAS    │    │  Cedar Policy │  │
│   │ (Linear Type Applications)│    │(Firmware/Kernel Artifacts)│    │    Engine     │  │
│   └─────────────┬─────────────┘    └─────────────┬─────────────┘    └───────┬───────┘  │
│                 │                                │                          │          │
│                 ▼                                ▼                          │          │
│   ┌────────────────────────────────────────────────────────────┐            │          │
│   │             Linear VirtIO Frontends (Austral)              │            │          │
│   │   - LinearDmaBuffer / VirtQueue descriptors                │            │          │
│   │   - Zero-copy borrowed memory slices                       │            │          │
│   └─────────────────────────────┬──────────────────────────────┘            │          │
└─────────────────────────────────┼───────────────────────────────────────────┼──────────┘
                                  │ (Shared Memory / vhost-user)              │
                                  ▼                                           │ (Lifecycle)
┌─────────────────────────────────────────────────────────────────────────┐   │
│           Cloud-Hypervisor VMM (Rust / unfer / SpectrumOS Patches)       │◄──┘
│   - crosvm-compatible shmem mapping (`0003-vhost_user-shmem.patch`)     │
│   - Direct kernel boot via Ephemeral Hot Store (zero-copy GPA mapping)  │
│   - VFIO PCIe device passthrough                                        │
└─────────────────────────────────┬───────────────────────────────────────┘
                                  │
                                  ▼
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                              Isolated Linux Driver Domain                              │
│   ┌──────────────────────────────────────┐  ┌──────────────────────────────────────┐   │
│   │     Vendor Firmware (via avm-cas)    │  │        Linux Host-Side Drivers       │   │
│   │  (e.g., Snapdragon X Elite .mbn)    │  │     (ath12k, remoteproc, DRM/KMS)    │   │
│   └──────────────────────────────────────┘  └──────────────────────────────────────┘   │
└─────────────────────────────────┬──────────────────────────────────────────────────────┘
                                  │ (Physical I/O via IOMMU)
                                  ▼
                         [ Physical Hardware ]
```

---

## 2. Updated Workspace Topology

Extend `safestos/` and `stdlib/` to incorporate hypervisor orchestration and driver domain packaging:

```
australVM/
├── safestos/
│   ├── pkg/                         # (From Phase 1-5: avm-cas, avm-eval, avm-exec)
│   ├── vmm/                         # [NEW] Cloud-Hypervisor orchestration & unfer bridge
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── launcher.rs          # Cloud-Hypervisor process & API socket controller
│   │       ├── config.rs            # VM topology (CPUs, RAM, VFIO devices, shmem)
│   │       └── lifecycle.rs         # Cedar-gated VM spin-up/teardown
│   └── virtio/                      # [NEW] Rust backend for vhost-user & VirtIO queues
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── ring.rs              # Split & packed VirtQueue implementations
│           ├── net.rs               # vhost-user-net client
│           ├── block.rs             # vhost-user-block client
│           └── shmem.rs             # SpectrumOS-compatible crosvm shmem mapper
└── stdlib/
    └── sys/                         # [NEW] Austral OS Driver Interfaces
        ├── Virtio.au                # Linear buffers & ring interfaces
        ├── Net.au                   # Capability-checked network device wrapper
        └── Storage.au               # Block device abstractions
```

---

## 3. Subsystem Technical Specifications

### 3.1 VirtIO Ring Buffers with Austral Linear Types (`stdlib/sys/Virtio.au`)

VirtIO queues pass ownership of DMA memory buffers back and forth between host and guest. Austral's linear type system guarantees compile-time freedom from use-after-free and double-submit bugs:

```austral
module AustralOS.Sys.Virtio is
    import SafestOS.Capabilities (MemoryCap);

    -- An unforgeable, linear chunk of DMA memory
    linear record LinearDmaBuffer is
        physical_addr: Nat64;
        virtual_ptr: Address;
        length: Nat32;
        mem_cap: MemoryCap;
    end;

    -- The submission token proving a buffer was loaned to the VirtIO queue
    linear record BufferInFlight is
        descriptor_idx: Nat16;
        buffer_ref: Address;
    end;

    -- Loan buffer to the Driver VM (consumes the linear buffer)
    function submit_buffer(
        queue_handle: &VirtQueue,
        buf: LinearDmaBuffer
    ): BufferInFlight;

    -- Reclaim buffer after Driver VM signals completion interrupt
    function complete_buffer(
        queue_handle: &VirtQueue,
        token: BufferInFlight
    ): LinearDmaBuffer;
end module.
```

---

### 3.2 Packaging Driver VMs & Firmware with `avm-pkg`

Instead of relying on an opaque host filesystem, the Linux Driver VM (kernel, minimal initramfs, and vendor firmware) is declared as a **pure derivation** in Austral:

```austral
module AustralPK.Pkgs.DriverVMs is
    import AustralPK.Derivation (Derivation, StorePath, make_derivation);

    -- Compiles a stripped micro-kernel containing only PCI, VirtIO, and target vendor drivers
    function make_linux_driver_kernel(
        kconfig: StorePath,
        sources: StorePath
    ): Derivation;

    -- Ingests and packages vendor firmware (e.g., Lenovo Yoga Slim 7 / Snapdragon X Elite)
    function package_qcom_firmware(raw_firmware_src: StorePath): Derivation is
        return make_derivation(
            name => "snapdragon-x-elite-firmware",
            version => "yoga-slim7x-v1",
            sources => singleton(raw_firmware_src),
            deps => nil(),
            builder => "extract_and_hash_firmware_mbn"
        );
    end;
end module.
```

---

### 3.3 Zero-Copy VM Boot via the Ephemeral Hot Store

Traditional hypervisors read kernel images from disk into RAM. Cloud-Hypervisor can boot a guest kernel directly using memory-mapped descriptors:

1. **CAS Extraction:** The Linux driver kernel (`vmlinux`) and rootfs chunks exist in `avm-cas` cold packfiles.
2. **Ephemeral Hot Store Hydration:** When a Driver VM is requested, `avm-cas/src/hot_store.rs` hydrates the kernel image into an aligned `memfd`.
3. **Zero-Copy GPA Mapping:** Cloud-Hypervisor uses the SpectrumOS shared memory compatibility layer (`0003-vhost_user-add-crosvm-shmem-map-compat.patch`) to map the `memfd` directly into the guest’s physical address space (GPA).
4. **Boot Latency:** The Driver VM boots to an active VirtIO state in **$< 35\text{ ms}$**.

---

## 4. Extended Implementation Roadmap (Sprints 6 & 7)

```
SPRINT 1-5           ► SPRINT 6                       ► SPRINT 7
Base avm-pkg Engine    Hypervisor & VirtIO Subsystem    Driver VM & Firmware Pipeline
(CAS, Eval, Exec)      ├─ Cloud-Hypervisor bridge       ├─ Minimal Linux kernel derivation
                       ├─ SpectrumOS shmem integration  ├─ Firmware ingestion pipeline
                       ├─ Linear VirtIO queues          ├─ Net/Block Driver VM launch
                       └─ Cedar VM lifecycle policies   └─ End-to-end bare-metal demo
```

---

### Sprint 6: Cloud-Hypervisor & VirtIO Subsystem (`safestos/vmm` & `safestos/virtio`)

- [ ] **Task 6.1: Cloud-Hypervisor Controller (`safestos/vmm/src/launcher.rs`)**
  - Vendor the SpectrumOS-patched `cloud-hypervisor` crate (with crosvm shmem compatibility patches).
  - Implement a programmatic Rust controller communicating over Cloud-Hypervisor's UNIX domain API socket (`/run/avm/vmm-<id>.sock`).
  - Configure VFIO device passthrough for physical network and storage controllers.

- [ ] **Task 6.2: SpectrumOS Shared Memory Bridge (`safestos/virtio/src/shmem.rs`)**
  - Implement shared-memory ring allocation matching crosvm/SpectrumOS semantics.
  - Implement `vhost-user-net` and `vhost-user-block` frontend handlers.

- [ ] **Task 6.3: Austral Linear VirtIO Binding (`stdlib/sys/Virtio.au`)**
  - Write Austral bindings exposing linear buffer submit/complete cycles.
  - Integrate Cedar policy checks: ensure a cell calling `submit_buffer` on a network queue possesses the corresponding `NetworkCap`.

---

### Sprint 7: Driver VM Packaging & Firmware Pipeline

- [ ] **Task 7.1: Minimal Linux Kernel Derivation (`stdlib/pkg/LinuxDriverKernel.au`)**
  - Define an Austral derivation that compiles an upstream Linux LTS kernel with `allnoconfig` + minimal driver set (`CONFIG_VIRTIO`, `CONFIG_VIRTIO_RING`, `CONFIG_VFIO`, `CONFIG_NET_VENDOR_QUALCOMM`, `CONFIG_ATH12K`).
  - The resulting output must be a single, compressed `vmlinux` cell stored in `avm-cas`.

- [ ] **Task 7.2: Firmware Extraction & Ingestion Pipeline**
  - Implement a recipe in `avm-pkg` that extracts firmware binaries (e.g., Qualcomm `.mbn` blobs from vendor disks or ISOs), hashes each blob with BLAKE3, and generates a content-addressed manifest.
  - Verify that firmware blobs deduplicate efficiently via `avm-cas` FastCDC chunking.

- [ ] **Task 7.3: End-to-End Driver Domain Orchestration Test**
  - Boot AustralVM on hardware or QEMU with a passed-through PCI network card.
  - AustralVM queries `avm-cas`, hydrates the Linux Driver VM into the Ephemeral Hot Store, and launches Cloud-Hypervisor.
  - The Linux Driver VM initializes the physical NIC, loads firmware, and exposes `vhost-user-net`.
  - An Austral user application cell transmits raw Ethernet frames using linear `LinearDmaBuffer` types, receiving line-rate connectivity.

---

## 5. Extended Acceptance Criteria

| Subsystem | Metric | Acceptance Test Target |
| :--- | :--- | :--- |
| **VM Boot Latency** | Direct Memory-Mapped Boot | Driver VM must boot from `EphemeralHotStore` to active VirtIO ready state in **$< 50\text{ ms}$**. |
| **Memory Isolation** | Fault Quarantine | Inducing a simulated kernel panic inside the Linux Driver VM leaves the AustralVM host OS running and able to cleanly restart the VM within **$100\text{ ms}$**. |
| **VirtIO Line Rate** | Network Throughput | Austral linear VirtIO network stack must achieve **$\ge 92\%$ of line rate** on a 10 GbE interface. |
| **Linear Memory Safety** | Buffer Invariance | Attempting to access a `LinearDmaBuffer` after calling `submit_buffer` must fail to compile at the Austral language level. |
| **Firmware Deduplication** | CAS Efficiency | Storing three minor revisions of Qualcomm DSP firmware bundles must achieve **$\ge 80\%$ chunk deduplication** in `avm-cas`. |
