# Cranelift Backend Bridge

**Overview**: Thread-safe JIT compilation bridge between Austral OCaml compiler and Cranelift IR.

**Status**: ✅ Working demo, 🔄 Tail calls need implementation

## Quick Test (Verify It Works)

```bash
cd safestos
make cranelift-test

# Output should be:
# Cranelift version: 0x83000
# Ready before init: 0
# Ready after init: 1
# Compiled function at: 0x...
# Result: 42 (expected 42)
# SUCCESS: Cranelift bridge works!
```

## Files

### `src/lib.rs` - Main Bridge
- Thread-local `JITModule` via `RefCell`
- Exports C FFI: `compile_to_function()`, `init()`, `ready()`, `shutdown()`
- Compiles simple demo function returning 42
- **Next**: Accept CPS IR via parameters

### `src/cps.rs` - CPS → CLIF Transpiler
- Binary IR reader (magic: 0x43505331 "CPS1")
- Type mapping: TypeTag → Cranelift types
- Instruction emission: IntLit, Var, Let, If, BinOp
- **Gap**: App (function calls) needs tail_call support
- **Endgame**: Accept IR, emit CLIF, return compiled function

### `src/tailcall.rs` - Tail Call Demo (Experimental)
- Shows how to build a recursive function with `call` instruction
- Demonstrates tail call pattern in CLIF
- **Goal**: Modify `emit_instructions` to use this pattern

### `test_bridge.c` - Working Integration Test
- Loads .so, calls `compile_to_function()`
- Demonstrates live JIT compilation and execution
- Self-contained with scheduler_dispatch stub

### `test_cps.c` - CPS IR Generator
- Builds binary IR blobs ("CPS1" format)
- Sends to Rust for compilation
- **Fix Required**: Update to match new CPS module API

### `test_cps` (compiled)
- **Bug**: Segfaults - probably data alignment or IR format mismatch
- Check: Encoded bytes vs. what cp.rs expects

### `Cargo.toml`
- Dependencies: cranelift 0.131 (latest)
- Builds `libaustral_cranelift_bridge.so`

## Architecture

### Compile Pipeline
```
OCaml AST → CPS IR (binary) 
            ↓
Rust: CpsReader → [emit_instructions]
            ↓
Cranelift: FunctionBuilder → CLIF → JIT
            ↓
Native Function
```

### Thread Safety Pattern
```rust
thread_local! {
    static JIT: RefCell<Option<JITModule>> = RefCell::new(None);
}
// Each thread gets its own JIT - no Send/Sync needed
```

### IR Format (from OCaml)
```
Header:  [magic: u32 = 0x43505331]
         [func_count: u32]
Per func: [name_len, name_bytes]
          [param_count]
          [param_types...]
          [return_type]
          [body_len]
          [instructions...]
```

## ⚠️ Known Issues

1. **test_cps segfault**  
   - Likely causes: Binary format mismatch, pointer alignment
   - Debug: Run GDB, check first instruction crash
   - Fix: Verify IR builder in test writes exact format `src/cps.rs` expects

2. **scheduler_dispatch linkage**  
   - Rust .so needs this symbol at load or call time
   - Workaround: Link with `-rdynamic`, provide stub in C driver
   - Fix: Make it optional or lazy lookup

3. **Tail calls**  
   - Lines 247-258 in src/cps.rs: `App` instruction returns placeholder
   - Needs: Detect tail position, use native `tail_call`
   - Evidence: Cranelift 0.131 docs say it's supported in builder API

## 🔧 How to Fix Components

### Fix test_cps Segfault
```bash
# Step 1: Print IR bytes from C
hexdump -C test_ir.bin

# Step 2: Read IR bytes in Rust test
cargo test -- --nocapture 2>&1

# Step 3: Compare format
# Check: magic, func_count, name_len, name, etc.
```

### Implement Tail Calls
```rust
// In src/cps.rs, emit_instructions, case 0x04 (App):
let is_tail = match reader.peek_u8() {
    Some(0x07) => { /* Next is Return wrapper */ true }
    _ => false
};

let call = if is_tail {
    // Cranelift has `create_tail_call()` on some instructions
    // Use `call` and mark as tail for backend optimization
    builder.ins().call(func_ref, &args) // Future: set tail flag
} else {
    builder.ins().call(func_ref, &args)
};
```

### Connect OCaml
```ocaml
(* lib/cps_gen.ml *)
let generate_cps_bytes module_ast =
  let ir = transform_to_cps module_ast in
  BinaryWriter.write ir  (* Writes our exact format *)

(* compiler.ml *)
let compile_for_jit code =
  let cps = generate_cps_bytes (parse code) in
  call_rust_bridge(cps)
```

## 📈 Test Coverage

| Component | Test | Status |
|-----------|------|--------|
| JIT init | `cranelift_init()` | ✅ 100% |
| JIT compile | `compile_to_function()` | ✅ Returns 42 |
| Function call | `test_bridge` | ✅ Works |
| CPS parse | N/A | 📋 Exists, needs test |
| Tail call | Recursive demo | 📋 Pending |
| Full pipeline | OCaml → CLIF → load | 📋 Design |

## 🚀 Next Commands To Run

After making code changes:

1. Rebuild: `cd cranelift && cargo build --release`
2. Test bridge: `./test_bridge` (or `make cranelift-test`)
3. Test CPS: `./test_cps` (debug segfault)
4. Verify: Make it return 52 (or other value)

This gets you from "demo compiles to 42" to "demo compiles tail-recursive functions" to "demo handles real CPS IR".

The skeleton is running. Wire up the rest!