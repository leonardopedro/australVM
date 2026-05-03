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
- Initialize the `cedar_policy` Rust crate within the VM.
- Create default authorization schema (`PolicySet`, `Entities`) representing VM modules, actions, and resources.

### Task 2: Linear Capability Module (Austral)
- Define `CedarCapability` and other static capabilities as Linear Types in an Austral standard module (`Capabilities.aui`/`.aum`).
- Implement fast-path minting functions for obvious rules.
- Implement the `cedar_ask_permission(Action)` wrapper which calls the FFI `au_cedar_check_runtime` for complex policies.

### Task 3: Integrating with C Cell Loader
- Wire the Austral `hot_swap_module` standard library function to call the existing C FFI `cell_swap(CellId, CellDescriptor*)`.
- Ensure the Cranelift JIT accurately populates the `_jit_fn_ptr` inside the C `CellDescriptor` struct upon module compilation.
- The C runtime will enforce the struct compatibility, while Cedar + Austral linear types enforce the authorization logic.

## 🧪 Verification Plan
1. [ ] **Cedar Compile-Time Denial**: Write a policy denying `ModA` from calling `ModB`, and verify the JIT rejects the binary IR.
2. [ ] **Hot-Swap Success**: Start a long-running loop, swap out the inner function, and verify the loop dynamically executes the new behavior without crashing.

---
**Status**: IN PLANNING
**Dependency**: Phase 10 (Completed ✅)
