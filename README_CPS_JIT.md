# CPS JIT Integration - Current State

## Status: Pipeline Working, Awaiting OCaml Recompile

### What Works
1. **OCaml CPS Generator** (`lib/CpsGen.ml`)
   - MAST → CPS binary conversion for all node types
   - `MIfExpression` now emits `0x08` opcode (cond, then, else)
   - `MIf` and `MWhile` statements emit `0x08`
   - Binary format v2 with parameter names

2. **Rust CPS → Cranelift Compiler** (`cranelift/src/cps.rs`, 644 lines)
   - Three-pass: parse headers → declare functions → define bodies
   - All opcodes: 0x01-0x08, 0x10, 0x13-0x19
   - 0x08 implemented as `select(cond_bool, then, else)` via Cranelift
   - Tail call: `return_call` when 0x04 followed by 0x07
   - Automatic import scanning and stub generation
   - Known stubs: trappingAdd, trappingSubtract, trappingMultiply, ExitSuccess

3. **FFI Bridge** (`cranelift/src/lib.rs`)
   - `compile_to_function_named(ir, ir_len, name, name_len) → *const c_void`
   - Thread-local JITModule with lazy init
   - `cranelift_init()`, `cranelift_shutdown()`, `cranelift_is_ready()`

### Test Results

```
fib(0) = 0  ✅
fib(1) = 1  ✅
fib(2) = 2  ❌ (should be 1)
fib(3) = 3  ❌ (should be 2)
fib(10) = 10 ❌ (should be 55)
```

### Root Cause of Wrong Results

The `cps_Fib_only.bin` test binary was generated with the **old** `CpsGen.ml` that:
- Ignored `MIfExpression`'s else-branch (only emitted `cond; then`)
- This made `fib(n)` always return `n` (the then-branch of `if n < 2 then n`)

After recompiling OCaml and regenerating the binary, the new format will be:
```
cond(n < 2) → 0x08 → then(n) → else(fib(n-1)+fib(n-2))
```
And Rust's `select` will choose the correct branch.

### Steps to Complete

```bash
# 1. Recompile OCaml
cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/australVM
dune build

# 2. Regenerate CPS binary
cd examples/fib && make clean && make

# 3. Rebuild Rust (if needed)
cd safestos/cranelift && cargo build --release

# 4. Run test
cd safestos && ./test_fib_math
# Expected: fib(10) = 55
```

### Known Issues

1. **Debug output**: `cps.rs` has verbose `eprintln!`/`println!` statements. Remove after verification.
2. **Comparison opcode mapping**: Verify OCaml `compile_binop` opcode order matches Rust `emit_expr`.
3. **Missing opcodes**: MSizeOf, MSlotAccessor, etc. fall through to `IntLit(0)` stub in CpsGen.ml.
4. **MWhile 0x08**: While loop emits `0x08` (select) which is incorrect for looping — needs proper block/jump implementation.
