# Phase 9: Error Handling & Diagnostics

## 🎯 Objective
Replace panic-on-error behavior in the JIT pipeline with structured, recoverable errors
that flow back cleanly to the OCaml compiler and produce Austral-style diagnostics.

## ✅ Phase 8 Summary (Completed)
- Records: allocation via `au_alloc`, field init via `store`, field access via `__slot_get` ✅
- Unions: tagged allocation, discriminant dispatch in `MCase` ✅  
- Pointers: `Deref` (opcode 0x20), `Store` (opcode 0x30) ✅
- `execute_function_2` added for two-arg JIT calls ✅
- All 9 tests pass ✅

## 📋 Phase 9 Task List

### 1. Structured Error Type (Rust) ✅
- Define `JitError { kind: ErrorKind, message: String, location: Option<String> }` enum
- Replace `Result<_, String>` with `Result<_, JitError>` in `cps.rs`
- Serialize error to C-compatible string buffer via FFI

### 2. Error Buffer API (C Bridge) ✅
- Add `cranelift_last_error() -> *const c_char` to expose the last error string to OCaml
- Store errors in a thread-local `LAST_ERROR: RefCell<Option<CString>>`

### 3. OCaml Error Propagation ✅
- Update `CamlCompiler_rust_bridge.ml` to call `cranelift_last_error` when compilation fails
- Convert error string into a proper `Austral_error` using the `CliError` constructor

### 4. Graceful Fallback Strategy ✅
- When JIT compilation fails: log structured diagnostic, fall back to C backend
- Remove silent `Printf.printf "CPS JIT compilation failed"` calls in `Compiler.ml`
- Emit a proper compiler warning with `Reporter`

### 5. Verifier Feedback ✅
- Capture Cranelift verifier errors and surface field/offset context
- Improve error messages for "Undefined variable" and "Unknown opcode" to include source location hints

## 🧪 Verification Plan
1. [x] Attempt to compile an intentionally broken CPS binary → clean error message
2. [x] Undefined variable → message names the variable
3. [x] End-to-end: `austral --use-cps-jit` on unsupported construct → graceful fallback + warning

---
**Status**: COMPLETE
**Dependency**: Phase 8 (Completed ✅)
