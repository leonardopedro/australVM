ALL IN THIS REPOSITORY IS WORK IN PROGRESS, MOSTLY AI GENERATED, PROBABLY NOTHING WORKS

# Austral Policy-Driven VM (SafestOS Extension)

A high-performance, secure runtime for Austral (extended with Tail Call Optimization) based on **Cranelift JIT** and **AWS Cedar**, inspired by the **Theseus OS** architecture.

## 🚀 Current Status: Phase 11 (Policy-Driven OS VM)
The project has successfully integrated a multi-tier security model combining compile-time linear type checks with JIT-time Cedar policy enforcement.

### Key Accomplishments (Phase 11)
- [x] **AWS Cedar Integration**: JIT-time static analysis blocks unauthorized calls.
- [x] **Multi-Tier Capabilities**: Linear tokens for Network, Memory, and Hot-Swapping.
- [x] **SafestOS Runtime Linkage**: Linked C-based scheduler and cell loader into the JIT bridge.
- [x] **Hot-Swappable Cells**: Metadata generation for `CellDescriptor` is operational.

### 🏛 Architecture
1. **Frontend (OCaml)**: Compiles Austral to monomorphized CPS IR.
2. **Bridge (C/Rust)**: 
    *   **Cedar Engine (Rust)**: Manages `PolicySet` and `Entities`.
    *   **Cranelift JIT (Rust)**: Translates CPS to machine code, performing Cedar checks on every `App` (Application) node.
3. **Runtime (C)**: Provides the lock-free scheduler, cell loader, and memory management (derived from SafestOS).

## 🛠 Features
- **Static Policy Enforcement**: Cedar queries during JIT compilation provide zero-runtime overhead security.
- **Linear Type Safety**: Capabilities are unforgeable tokens that must be consumed to perform privileged operations.
- **Hot-Swapping**: Structural type-safety checks allow replacing modules without restarting the VM.
- **Cranelift Optimized**: Native machine code generation for x86_64.

## 📂 Directory Structure
- `lib/`: OCaml compiler frontend and CPS generator.
- `safestos/cranelift/`: Rust bridge containing Cedar and Cranelift logic.
- `safestos/runtime/`: C runtime providing the VM execution environment.
- `test_programs/`: Austral examples for capabilities and hot-swapping.
