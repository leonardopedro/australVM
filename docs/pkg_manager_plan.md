# `avm-pkg`: Implementation Plan & Engineering Specification
**A Total, Capability-Secure, Content-Addressed Package Engine for AustralVM**

---

## 1. System Architecture & Core Invariants

`avm-pkg` replaces Nix’s dynamically typed, lazy, non-hermetic shell model with a **statically typed, total, capability-checked computation graph** targeting AustralVM. 

Build recipes are pure Austral functions evaluated into an immutable Directed Acyclic Graph (DAG) of derivations. Storage is split into a **persistent, deduplicated cold store** (derived from the Vykar backup engine) and a **transient, demand-paged hot store** acting as a read-only executable page cache.

```
                         [ Austral Package Definition (.au) ]
                                          │
                  (Total Functional Evaluation: Folds / Bounded TCO)
                                          ▼
                               [ Derivation Merkle DAG ]
                                          │
                           (BLAKE3 Content Addressing)
                                          ▼
                         [ Typst-Style Memoization Engine ]
                                   ╱             ╲
                         [ Cache Hit ]         [ Cache Miss ]
                                │                     │
                (Demand Hydration / mmap)     (Capability-Gated Sandbox)
                                │                     │
                                │              [ Cranelift JIT ]
                                │                     │
                                │                     ▼
                                │             [ .avm Cell Output ]
                                │                     │
                                │            (FastCDC v3 Chunking)
                                │                     │
                                ▼                     ▼
     ┌────────────────────────────────────┐       ┌───────────────────────────────┐
     │      EPHEMERAL HOT STORE           │       │       PERSISTENT COLD CAS     │
     │  - Analogous to read-only swap     │◄──────┤  - Vykar-derived packfiles    │
     │  - Bounded RAM / tmpfs / memfd     │Hydrate│  - BLAKE3 chunk DAG           │
     │  - Page-aligned for zero-copy mmap │       │  - LZ4 / Zstandard compressed │
     │  - Zero writeback on eviction      │       │  - S3 / REST remote sync      │
     └─────────────────┬──────────────────┘       └───────────────────────────────┘
                       │
             (Direct Pointer Link)
                       ▼
          [ AustralVM / Theseus Runtime ]
```

### Core Invariants

1. **Totality:** The package DSL is restricted to a strictly total subset of Austral (no unbounded `while` loops; only structural catamorphisms/folds and provably bounded tail recursion). Every evaluation is guaranteed to terminate.
2. **Type-Level Hermeticism:** Network access, filesystem mutations, and compiler executions require unforgeable linear capability tokens (`&NetworkCap`, `&FsCap`, `&ExecCap`) validated by AWS Cedar policies. A pure derivation is physically unable to access host state.
3. **Sub-File Deduplication (FastCDC + BLAKE3):** Artifacts are not coarse tarballs/NARs. They are split into variable-sized chunks (16 KiB – 256 KiB) using Content-Defined Chunking, achieving fine-grained deduplication across minor package revisions.
4. **Disposable Hot Store (The "Read-Only Swap" Model):** The local uncompressed store contains **only currently active cells loaded into memory**. Because every cell is reproducibly re-constructible from the cold chunk store, eviction carries zero dirty-writeback penalty.
5. **Native Cell Execution:** Artifacts are relocatable `.avm` machine code cells, designed for direct memory-mapped linking into AustralVM and single-address-space microkernels like Theseus OS.

---

## 2. Repository Layout & Topology

The implementation extends the existing `australVM` repository:

```
australVM/
├── lib/                             # Austral OCaml Compiler
│   └── Pkg/                         # [NEW] Totality checker & package derivation AST
│       ├── Totality.ml              # Enforces bounded recursion & no unbounded loops
│       └── DerivAst.ml              # Intermediate AST for package derivations
├── safestos/
│   ├── cranelift/                   # Existing Rust-to-Cranelift JIT bridge
│   ├── cedar/                       # Existing Cedar capability policy engine
│   └── pkg/                         # [NEW] Rust Core Package Subsystem Workspace
│       ├── Cargo.toml               # Workspace configuration
│       ├── avm-cas/                 # Storage engine: Vykar CAS + Ephemeral Hot Store
│       │   ├── Cargo.toml
│       │   └── src/
│       │       ├── lib.rs
│       │       ├── chunker.rs       # FastCDC chunker & boundary finder
│       │       ├── engine.rs        # Cold store: packfiles, indexes, manifests
│       │       ├── hot_store.rs     # Ephemeral Hot Store (page-cache / memfd)
│       │       ├── compression.rs   # LZ4 (hot streaming) & Zstd (cold archival)
│       │       ├── crypto.rs        # Ed25519 signatures (replacing Vykar's AES)
│       │       └── remote.rs        # S3 / HTTP REST binary cache client
│       ├── avm-eval/                # Pure memoization and DAG evaluation engine
│       │   ├── Cargo.toml
│       │   └── src/
│       │       ├── lib.rs
│       │       ├── graph.rs         # Derivation DAG structures & dependency orders
│       │       ├── memo.rs          # Typst/comemo-style in-memory evaluation cache
│       │       └── hash.rs          # BLAKE3 derivation input-hash calculus
│       ├── avm-exec/                # Hermetic executor and cell builder
│       │   ├── Cargo.toml
│       │   └── src/
│       │       ├── lib.rs
│       │       ├── runner.rs        # Capability-sandboxed execution harness
│       │       └── cell_builder.rs  # Cranelift .avm linker & header serializer
│       └── avm-cli/                 # Command-line interface (`avm-pkg`)
│           ├── Cargo.toml
│           └── src/main.rs
└── stdlib/
    └── pkg/                         # Austral Standard Library Package Modules
        ├── Derivation.au            # Derivation records and capability tokens
        └── Stdenv.au                # Base toolchain constructors and fold primitives
```

---

## 3. Subsystem Specifications

### 3.1 Austral Package DSL & Totality Enforcement

Package declarations use pure Austral records. The OCaml frontend pass (`lib/Pkg/Totality.ml`) verifies structural termination prior to IR lowering.

#### `stdlib/pkg/Derivation.au`
```austral
module AustralPK.Derivation is
    import SafestOS.Capabilities (NetworkCap, FsCap, ExecCap);

    record StoreHash is
        digest: FixedArray[Nat8, 32];
    end;

    record StorePath is
        hash: StoreHash;
        name: String;
    end;

    record Derivation is
        name: String;
        version: String;
        sources: List[StorePath];
        dependencies: List[StorePath];
        env_vars: List[Pair[String, String]];
        builder_symbol: String;
    end;

    linear record BuildSandbox is
        fs_cap: FsCap;
        exec_cap: ExecCap;
    end;

    function make_derivation(
        name: String,
        version: String,
        sources: List[StorePath],
        deps: List[StorePath],
        builder: String
    ): Derivation;
end module.
```

#### Totality Rules (`lib/Pkg/Totality.ml`)
The totality checker rejects an Austral package definition if:
1. It contains a `while` loop whose termination condition is not an induction variable with a statically provable upper bound.
2. It contains recursive function definitions not recognized as structurally descending or annotated for tail-call optimization.
3. It performs unmediated side effects (all I/O must take an explicit capability reference).

**Scheduling (release contract).** The checker is *heuristic*: rules 1–2
syntactically reject anything they cannot prove bounded, and may therefore
reject programs that happen to terminate. So the requirement is scoped to a
single opt-in gate — `--pkg-totality=true` on `austral compile`
(`Compiler.pkg_totality`, default `false`, checked in `lib/Compiler.ml`
immediately after linearity and before derivation export). Nothing else in
the pipeline runs it: ordinary typechecking is untouched, and the separate
liquid totality gate (`LiquidCheck`) only ever examines functions carrying a
`Liquid_*` pragma.

**Plan record.** Running with `--pkg-totality=true` *selects* the
compilations whose declarations are vetted non-recursive; everything not
selected must still be visible, so the exporter stamps each derivation in
plan.json with `"totality": "proven"` (flag on, pass accepted) or
`"totality": "unproven"` (flag off — unknown, never assumed). The status is
a pure function of invocation + module, merges deterministically (last
deployment of a name wins), and a plan file predating the field decodes as
`unproven`. Evaluators treat anything but an explicit `proven` as unproven.
Persisting per-`.aum` caches is explicitly out of scope — plan.json is the
only carrier.

---

### 3.2 Dual-Tier Storage Architecture (`avm-cas`)

Storage is divided into two decoupled layers:

#### A. The Cold CAS (Vykar Adaptation)
Adapted from Vykar's block engine, this provides append-only, deduplicated, content-addressed storage.

* **Chunking:** `FastCDC v3` with target parameters: `min = 16 KiB`, `avg = 64 KiB`, `max = 256 KiB`.
* **Addressing:** Unkeyed `BLAKE3` hash of the plaintext chunk data.
* **Compression:** 
  * In-flight/Local packs: `LZ4_flex` for speed.
  * Cold/Remote sync: `zstd` level 3 (or level 19 for archival).
* **Signatures:** Every pack and root manifest contains an `Ed25519` signature from the builder, replacing Vykar's symmetric encryption passwords.

#### B. The Ephemeral Hot Store (The Swapfile Analogue)
The Hot Store does **not** persist packages long term. It is an in-memory or `tmpfs`/`memfd`-backed read-only page cache for active `.avm` cells.

```rust
// safestos/pkg/avm-cas/src/hot_store.rs

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use memmap2::{Mmap, MmapOptions};
use dashmap::DashMap;
use blake3::Hash;
use crate::engine::ChunkStore;

pub struct HotCell {
    pub hash: Hash,
    pub size: usize,
    pub mmap_handle: Arc<Mmap>,
}

pub struct EphemeralHotStore {
    cold_store: Arc<ChunkStore>,
    active_cells: DashMap<Hash, HotCell>,
    max_capacity_bytes: usize,
    current_usage_bytes: AtomicUsize,
}

impl EphemeralHotStore {
    pub fn new(cold_store: Arc<ChunkStore>, max_capacity_bytes: usize) -> Self {
        Self {
            cold_store,
            active_cells: DashMap::new(),
            max_capacity_bytes,
            current_usage_bytes: AtomicUsize::new(0),
        }
    }

    /// Demand-hydrate a cell from the cold store into executable hot memory.
    pub fn acquire_cell(&self, cell_hash: &Hash) -> Result<Arc<Mmap>, Box<dyn std::error::Error>> {
        // Fast path: already paged in
        if let Some(cell) = self.active_cells.get(cell_hash) {
            return Ok(cell.mmap_handle.clone());
        }

        // Slow path: fault-in from Vykar Cold Store
        self.hydrate_from_cold(cell_hash)
    }

    fn hydrate_from_cold(&self, cell_hash: &Hash) -> Result<Arc<Mmap>, Box<dyn std::error::Error>> {
        let manifest = self.cold_store.get_manifest(cell_hash)?;
        self.evict_if_needed(manifest.total_size);

        // Allocate page-aligned executable memory buffer
        let mut buffer = Vec::with_capacity(manifest.total_size);
        for chunk_hash in &manifest.chunks {
            let chunk_data = self.cold_store.read_chunk_decompressed(chunk_hash)?;
            buffer.extend_from_slice(&chunk_data);
        }

        // On Linux: use memfd_create; on bare-metal/Theseus: direct page allocation
        let mmap = unsafe {
            MmapOptions::new().map_copy_read_only(&buffer)?
        };
        let mmap_arc = Arc::new(mmap);

        self.active_cells.insert(*cell_hash, HotCell {
            hash: *cell_hash,
            size: manifest.total_size,
            mmap_handle: mmap_arc.clone(),
        });

        self.current_usage_bytes.fetch_add(manifest.total_size, Ordering::Relaxed);
        Ok(mmap_arc)
    }

    /// Eviction is zero-cost: clean pages are dropped without writeback.
    pub fn evict_cell(&self, cell_hash: &Hash) {
        if let Some((_, cell)) = self.active_cells.remove(cell_hash) {
            self.current_usage_bytes.fetch_sub(cell.size, Ordering::Relaxed);
        }
    }

    fn evict_if_needed(&self, incoming_size: usize) {
        while self.current_usage_bytes.load(Ordering::Relaxed) + incoming_size > self.max_capacity_bytes {
            // Evict an arbitrary unreferenced entry (LRU key tracking omitted for brevity)
            if let Some(entry) = self.active_cells.iter().next() {
                let hash = *entry.key();
                drop(entry);
                self.evict_cell(&hash);
            } else {
                break;
            }
        }
    }
}
```

---

### 3.3 The Typst-Style Pure Memoization Evaluator (`avm-eval`)

The evaluation engine represents package recipes as pure mathematical functions. It calculates derivation IDs using deterministic input hashes and caches graph fragments.

#### Derivation Hash Calculus (`avm-eval/src/hash.rs`)
$$\text{DerivationID} = \text{BLAKE3}\Big( \text{Magic} \parallel \text{Name} \parallel \text{Version} \parallel \text{BuilderSymbol} \parallel \sum_{i} \text{DepID}_i \parallel \sum_{j} \text{Env}_j \Big)$$

```rust
// safestos/pkg/avm-eval/src/hash.rs

pub fn calculate_derivation_hash(drv: &Derivation) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"AVM_DRV_V1\0");
    hasher.update(drv.name.as_bytes());
    hasher.update(b"\0");
    hasher.update(drv.version.as_bytes());
    hasher.update(b"\0");
    hasher.update(drv.builder_symbol.as_bytes());
    hasher.update(b"\0");

    let mut sorted_deps = drv.dependencies.clone();
    sorted_deps.sort_by(|a, b| a.digest.cmp(&b.digest));
    for dep in sorted_deps {
        hasher.update(&dep.digest);
    }

    let mut sorted_env = drv.env_vars.clone();
    sorted_env.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in sorted_env {
        hasher.update(k.as_bytes());
        hasher.update(b"=");
        hasher.update(v.as_bytes());
        hasher.update(b"\0");
    }

    hasher.finalize()
}
```

#### Evaluation Memoizer (`avm-eval/src/memo.rs`)
* **L1 Cache (In-Memory DashMap):** Remembers AST node evaluation results during the current session (resolves within nanoseconds).
* **L2 Cache (Local Vykar Manifest Index):** Avoids re-evaluating known `.au` subgraphs if input hashes are already present in the local database.
* **L3 Cache (Remote S3 / REST Binary Cache):** Queries remote endpoints for pre-built chunk manifests.

---

### 3.4 Hermetic Executor & `.avm` Cell Format (`avm-exec`)

#### `.avm` Binary Layout
Unlike Linux FHS packages that install trees of files (`bin/`, `lib/`, `share/`), AustralVM natively compiles derivations into relocatable `.avm` cells:

```
Offset   Size       Field
─────────────────────────────────────────────────────────────
0x00     4 bytes    Magic: 0x41564D43 ('AVMC')
0x04     2 bytes    Version: 0x0001
0x06     2 bytes    Target Architecture (e.g., x86_64, aarch64)
0x08     32 bytes   Derivation Input Hash (BLAKE3)
0x28     4 bytes    Length of Capability Table (N bytes)
0x2C     N bytes    Cedar Capability Table (JSON / Bincode)
0x2C+N   4 bytes    Length of Text Section (M bytes)
...      M bytes    Cranelift Machine Code (Page-Aligned)
...      K bytes    Constant Pool & Relocation Table
...      64 bytes   Ed25519 Cryptographic Signature
```

#### Sandboxed Execution Engine (`avm-exec/src/runner.rs`)
1. **Network Phase (Fetchers):** Only derivations holding an active `NetworkCap` can initiate outbound sockets. Downloader derivations run in an isolated process that emits raw bytes to `avm-cas` and exits.
2. **Build Phase (Pure Builders):** Builders receive no network capabilities. On Linux, this is enforced via `unshare(CLONE_NEWNET | CLONE_NEWNS)`. In Theseus OS / SafestOS, capability tokens are validated at function calls by the Cedar policy engine.

---

## 4. Phase-by-Phase Implementation Roadmap

```
SPRINT 1                  SPRINT 2                  SPRINT 3                  SPRINT 4                  SPRINT 5
Storage Subsystem         Evaluator & Totality      Executor & Linker         CLI & Package Set         Integration & Hardening
(avm-cas)                 (avm-eval + OCaml)        (avm-exec + Cranelift)    (avm-cli + stdlib)        (Theseus / Performance)
├─ FastCDC chunker        ├─ Totality AST pass      ├─ Linux namespace runner ├─ CLI tool               ├─ Zero-copy loader
├─ Vykar packfile index   ├─ BLAKE3 DAG hash        ├─ Cranelift .avm emitter ├─ Bootstrap toolchain    ├─ Remote S3 sync
├─ Ephemeral Hot Store    ├─ Memoization engine     ├─ Cedar capability check ├─ Musl / TCC / Hello     ├─ Memory limit stress
└─ Ed25519 signer         └─ Benchmarking vs Nix    └─ Hot store integration  └─ Automated test suite   └─ Formal acceptance
```

---

### Sprint 1: The Storage Subsystem (`avm-cas`)

- [ ] **Task 1.1: FastCDC Chunker (`avm-cas/src/chunker.rs`)**
  - Integrate `fastcdc = "3.1"` and `blake3 = "1.5"`.
  - Implement `chunk_payload(data: &[u8], cfg: &ChunkerConfig) -> Vec<Chunk>`.
  - **Acceptance Test:** Test against a 64 MiB random buffer with single-byte mutations; assert that $> 95\%$ of chunks match between original and mutated versions.

- [ ] **Task 1.2: Packfile Engine & Manifests (`avm-cas/src/engine.rs`)**
  - Extract and adapt Vykar’s packfile index format for append-only storage.
  - Implement manifest serialization containing chunk lists and total uncompressed size.
  - Implement `LZ4_flex` compression for packfile entries.

- [ ] **Task 1.3: Cryptographic Signatures (`avm-cas/src/crypto.rs`)**
  - Replace Vykar’s Argon2id/AES symmetric cipher with `ed25519-dalek`.
  - Implement `sign_manifest(manifest: &mut Manifest, key: &SigningKey)` and `verify_manifest(manifest: &Manifest, key: &VerifyingKey) -> bool`.

- [ ] **Task 1.4: Ephemeral Hot Store (`avm-cas/src/hot_store.rs`)**
  - Implement `EphemeralHotStore` managing an LRU cache of `memmap2::Mmap` handles.
  - Ensure memory mappings use page alignment (`4096` bytes).
  - Implement capacity checking with zero-writeback eviction.
  - **Acceptance Test:** Populate the hot store with 2 GB of cells against a 1 GB limit; assert that least recently used cells are dropped without modifying cold packfiles.

---

### Sprint 2: The Pure Evaluator & Totality Checker (`avm-eval`)

- [ ] **Task 2.1: Austral Totality Pass (`lib/Pkg/Totality.ml`)**
  - Implement AST validation in the Austral OCaml compiler.
  - Reject non-structurally recursive loops and unannotated recursion in package definition modules.
  - Expose JSON derivation export for consumption by the Rust toolchain.

- [ ] **Task 2.2: Merkle DAG Engine (`avm-eval/src/graph.rs`)**
  - Implement `Derivation` struct, topological sorting, and dependency cycle detection.
  - Implement `calculate_derivation_hash` using BLAKE3.
  - **Acceptance Test:** Assert that swapping the order of declaration of independent dependencies yields an identical derivation hash.

- [ ] **Task 2.3: Typst-Style Memoization Engine (`avm-eval/src/memo.rs`)**
  - Implement `MemoTable` backed by `dashmap::DashMap`.
  - Implement short-circuit graph traversal: if a derivation hash exists in `avm-cas`, skip traversing its sub-dependencies during warm evaluation.

---

### Sprint 3: The Hermetic Executor (`avm-exec`) & Cranelift Bridge

- [ ] **Task 3.1: Capability-Checked Sandbox (`avm-exec/src/runner.rs`)**
  - Implement the Linux namespace isolation runner: isolate mount, PID, IPC, and network namespaces.
  - Integrate Cedar capability queries before spawning builder processes.
  - Forbid `/bin`, `/usr`, `/tmp` access; mount only declared inputs from `EphemeralHotStore`.

- [ ] **Task 3.2: Cranelift `.avm` Cell Builder (`avm-exec/src/cell_builder.rs`)**
  - Connect to AustralVM’s Cranelift bridge (`safestos/cranelift`).
  - Serialize compiled machine code into `.avm` binary cell layout with Cedar metadata and BLAKE3 signatures.
  - Pipe output directly into `avm-cas` chunking.

- [ ] **Task 3.3: Hot Store Dynamic Linking Integration**
  - Implement symbol resolution that requests missing dependencies from `EphemeralHotStore::acquire_cell()`.
  - Demonstrate a cell resolving an imported function directly from another mapped cell's address space.

---

### Sprint 4: CLI & Minimal Bootstrap Package Set

- [ ] **Task 4.1: The `avm-pkg` CLI Interface (`avm-cli/src/main.rs`)**
  - Implement subcommands:
    * `avm-pkg build <file.au>`: Parse, check totality, evaluate DAG, hydrate or compile.
    * `avm-pkg push --remote <s3-url>`: Upload missing Vykar packfile chunks to remote store.
    * `avm-pkg pull <hash>`: Fetch and reconstruct manifests from remote store.
    * `avm-pkg gc --keep-roots`: Prune unreferenced chunks in cold storage.
    * `avm-pkg hot-stats`: Inspect memory usage and active mappings of the Ephemeral Hot Store.

- [ ] **Task 4.2: Standard Library Packages (`stdlib/pkg/`)**
  - Write `Stdenv.au`: Standard builder derivation taking compiler flags and source tarballs.
  - Port seed bootstrap recipes:
    1. `bootstrap_seed`: Pre-compiled minimal TinyCC (`tcc`) and static Musl.
    2. `musl`: Complete C library compiled via `tcc`.
    3. `busybox`: Core POSIX utility cell (`sh`, `sed`, `tar`, `awk`).
    4. `hello`: GNU Hello validation package.

---

### Sprint 5: Hardening, Remote Sync & Performance Benchmarking

- [ ] **Task 5.1: Remote S3 / REST Cache Sync (`avm-cas/src/remote.rs`)**
  - Implement asynchronous chunk upload and download using `reqwest` and `tokio`.
  - Provide content-addressed chunk streaming: only missing BLAKE3 chunk hashes are requested over HTTP.

- [ ] **Task 5.2: Memory Pressure & Fault-In Stress Testing**
  - Run continuous loop executing 100 distinct cells under a restricted Hot Store memory limit (`--hot-limit=256M`).
  - Validate that page-fault hydration and eviction maintain stability without memory leaks.

- [ ] **Task 5.3: Benchmarks vs. Nix**
  - Measure warm evaluation speed of a 1,000-node dependency graph.
  - Measure disk space consumption after 10 minor package rebuilds.

---

## 5. Verification Gates & Acceptance Criteria

Every milestone must satisfy strict quantitative and security criteria:

| Subsystem | Metric | Acceptance Test Target |
| :--- | :--- | :--- |
| **Language Totality** | Termination Guarantee | $100\%$ of cyclic or unbounded AST constructs rejected at compile-time by `Totality.ml`. |
| **Evaluation Speed** | Typst-Style Memoization | Warm evaluation of a 5,000-node derivation DAG must resolve in **$< 20\text{ ms}$** (vs. seconds in Nix). |
| **Storage Efficiency** | FastCDC Deduplication | Rebuilding a package with a minor source patch must achieve **$\ge 85\%$ chunk deduplication** in `avm-cas`. |
| **Hot Store Latency** | Direct `mmap` Hydration | Acquiring an active hot cell must complete in **$< 1\ \mu\text{s}$**; faulting-in an evicted 50 MB cell must complete in **$< 15\text{ ms}$**. |
| **Hot Store Eviction** | Zero Dirty Writeback | Evicting 10 GB of hot memory cells must generate **0 bytes of disk writes**. |
| **Security / Sandbox** | Capability Gating | A builder without `NetworkCap` attempting to bind or connect a socket is blocked by Cedar at the type level and terminated with `EPERM` at the OS level. |

---

## 6. Agent Handover Directives

When executing this plan:

1. **Start with Sprint 1, Task 1.1.** Do not jump ahead to the Austral frontend until the low-level `avm-cas` chunker and packfile engine pass all unit tests.
2. **Strict Workspace Separation:** Do not modify Austral's core linear type checker (`lib/Linear/`). Implement package totality as an independent pass in `lib/Pkg/Totality.ml`.
3. **Rust Code Quality:** All Rust crates must compile with `#![deny(unsafe_op_in_unsafe_fn)]`. Unsafe code is restricted exclusively to the `mmap` calls within `avm-cas/src/hot_store.rs`.
4. **Deterministic Hashes:** Never serialize hash structures using non-deterministic formats (e.g., standard hash map iterations). All inputs must be topologically or lexicographically sorted before computing BLAKE3 digests.
