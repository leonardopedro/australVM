# Phase 11: Policy-Driven OS VM (Cedar & Theseus Architecture)

## 🎯 Objective
Upgrade the Austral JIT VM from a standard fast execution environment into a **Safe, Live-Updatable, Policy-Driven OS Runtime** modeled after Theseus OS. The VM will support fine-grained module hot-swapping and enforce strict access control boundaries statically and dynamically using the AWS **Cedar** policy language.

## 🏛 Architecture

### 1. The Multi-Tier Linear Capability Model
The bedrock of the VM's security is **Austral's Linear Type System**. Capabilities are represented as unforgeable Linear tokens, ensuring zero-overhead access control at the invocation site.
- **Static Capabilities**: Minted at compile-time or system boot (e.g., `MemoryPage`, `ProcessContext`). They require no runtime checks and never interact with Cedar.
- **Dynamic Fast-Path Capabilities**: Simple runtime rules evaluated directly by native code for performance (e.g., checking if a pointer is within bounds before returning a `ReadCapability`).
- **Cedar Policy-Driven Capabilities**: For complex governance (e.g., cross-module hot-swapping or sensitive IO). Only these capabilities invoke the AWS Cedar engine via `Cedar.authorize(action)`.

### 2. Utilizing the SafestOS Cell Architecture
The existing `SafestOS` C runtime natively implements the Theseus-like "Cell" architecture via `vm.h` and `cell_loader.c`. 
- **Cell Swap Mechanism**: The `cell_swap` routine already handles state pausing, structural subtyping hashes, and capability migration.
- **Integration**: Instead of building a new dispatch table in Rust, the Cranelift compiler will populate the existing `CellDescriptor::_jit_fn_ptr` and delegate scheduling back to the lock-free C `scheduler_dispatch()` loop.

## 📋 Task List

### Task 1: Integrate Cedar into the Rust Bridge
- [x] Initialize the `cedar_policy` Rust crate within the VM. ✅
- [x] Create default authorization schema (`PolicySet`, `Entities`) representing VM modules, actions, and resources. ✅
- [x] JIT-time static check integrated into `cps.rs`. ✅

### Task 2: Linear Capability Module (Austral)
- [x] Define `CedarRuntimeCapability` and static capabilities in `capabilities.aui`. ✅
- [x] Implement the `cedar_authorize` wrapper which calls the Rust FFI. ✅
- [x] Implement tiered access control patterns in `capabilities.aum`. ✅

### Task 3: Integrating with C Cell Loader
- [x] Wire `hot_swap_module` to call `__au_swap_module` FFI. ✅
- [x] Update `CellAttribute.ml` to generate valid `CellDescriptor` C structs. ✅
- [x] Link C runtime (`cell_loader.c`) into Rust bridge via `build.rs`. ✅
- [x] Populate `_jit_fn_ptr` in the JIT compilation path. ✅

## 🧪 Verification Plan
1. [x] **Cedar Compile-Time Denial**: SUCCESS. Verified that `ForbiddenFunc` calls are blocked at JIT-time with `test_jit.exe`. ✅
2. [x] **Hot-Swap Success**: SUCCESS. Verified live logic migration from `ManagedCell` to `AdvancedCell` with `test_hotswap.exe`. ✅

---
**Status**: COMPLETED ✅
**Dependency**: Phase 10 (Completed ✅)
