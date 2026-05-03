# Phase 8: Data Layout & Records

## 🎯 Objective
Enable support for complex data structures and pointer-based memory management in the JIT pipeline. This will allow the JIT to handle Austral records, unions, and heap-allocated objects.

## 📋 Task List

### 1. Pointer & Memory Ops
- **Load/Store**: Implement `Deref` (opcode 0x0C) and `Assign` to pointer destinations in `cps.rs`.
- **Arithmetic**: Support `__ptr_slot_get` via `GEP`-like offset calculations.
- **Status**: [ ] Pending

### 2. Structural Builtins (Record/Union)
- **__record_new**: Map to a native runtime function that allocates memory and stores fields.
- **__union_new**: Support tagged union allocation and initialization.
- **__slot_get**: Native field access via offsets.
- **Status**: [ ] Pending

### 3. Pointer Types
- **Mapping**: Ensure `MonoPointer` and `MonoAddress` are correctly mapped to `I64` (native pointers).
- **Status**: [ ] Pending

### 4. Runtime Support
- **au_alloc / au_free**: Verify alignment and size calculations for native allocations.
- **Status**: [ ] Pending

## 🧪 Verification Plan
1. [ ] **Test Record**: 
   ```austral
   record Point is x: Int64; y: Int64; end;
   function get_x(p: Point): Int64 is return p.x; end;
   ```
2. [ ] **Test Union**:
   ```austral
   union MaybeInt is Just: Int64; Nothing; end;
   ```
3. [ ] **Test Heap**: 
   Allocate a record on the heap and access its fields.

---
**Status**: IN PLANNING  
**Current Dependency**: Phase 7 Integration (Verified ✅)
