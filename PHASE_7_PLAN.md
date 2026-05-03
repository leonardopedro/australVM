# Phase 7: Full MAST & Runtime Integration

## 🎯 Objective
Transition the stabilized JIT pipeline from a standalone prototype to a fully integrated backend for the Austral compiler, supporting all remaining control flow structures and runtime primitives.

## 📋 Task List

### 1. Loop Support (While/For)
- **Opcode**: Add `While` (0x09) to binary IR. ✅
- **Generator**: Update `CpsGen.ml` to emit loop headers and bodies. ✅
- **Backend**: Implement loop-back edges in Rust `BlockManager`. ✅
- **Status**: [x] Completed

### 2. Pattern Matching (Match)
- **Opcode**: Add `Match` (0x0A) with branch table. ✅
- **Generator**: Implement multi-branch serialization for `Match`. ✅
- **Backend**: Use Cranelift's conditional branches for native branching. ✅
- **Status**: [x] Completed

### 3. Runtime Builtins
- **Mechanism**: Implement a symbol lookup table in the Rust bridge. ✅
- **Builtins**: Map `ExitSuccess`, `PrintInt`, `Alloc`, `Free` to native symbols. ✅
- **Status**: [x] Completed

### 4. Compiler Integration
- **CLI Flag**: Add `--jit` to `bin/austral.ml`. ✅
- **Hybrid Path**: If `--jit` is set, use `Compiler_cps` instead of `CodeGen`. ✅
- **Status**: [x] Completed

## 🧪 Verification Plan
1. [x] **Test Loop**: Verified with summation test. ✅
2. [x] **Test Match**: Verified with dispatcher test. ✅
3. [x] **Test StdLib**: Verified with `au_print_int` integration. ✅

---
**Status**: COMPLETED ✅
**Next Phase**: Phase 8: Data Layout & Records
