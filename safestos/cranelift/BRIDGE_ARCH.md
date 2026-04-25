# Cranelift Backend Architecture - SafestOS

## Overview

The Cranelift bridge adapts SafestOS to use Cranelift JIT instead of C codegen. This enables:

1. **Compile speed**: <100µs vs 50-200ms (C/Clang)
2. **Native tail calls**: CLIF `tail_call` instruction 
3. **Zero disk I/O**: Pure in-process compilation
4. **Small footprint**: ~2MB vs ~30MB+ for LLVM

## Current Status

✅ **Project Structure**: Created  
✅ **Cargo.toml**: Dependencies configured  
🚫 **Working Bridge**: Requires API fixes  

## Bridge Layer Architecture

### OCaml → Rust → Cranelift

```
Austral Compiler (OCaml)
    ↓ C FFI (external C function calls)
Rust Bridge (cranelift/src/lib.rs)
    ↓ Rust API
Cranelift (cranelift_jit, cranelift_module)
    ↓ Machine code
Function Pointer → Execute
```

### C API (for OCaml)

```c
// Initialize
int cranelift_init(void);

// Compile CPS function
// Returns: function pointer (cast to whatever signature needed)
void* cranelift_compile_cps(
    const char* name,
    CpsFunction* function,
    size_t func_len
);

// Test/demo
uint64_t cranelift_demo(void);
```

### Rust Implementation (src/lib.rs)

The bridge needs:

1. **Thread safety**: OLIMutex or OnceCell around JITModule
2. **Builder functions**: To create Cranelift IR from CPS
3. **Memory management**: Handling code memory lifetime

Key challenge: `JITModule` doesn't implement `Send`. Solution: Use `Box::leak` + manual management, or single-threaded initialization + RAII.

## CPS → CLIF Translation

### Austral CPS IR (source)
```
func @step (%state: ptr) {
  %ctr = load %state[offset=0]
  %new = i32.add %ctr, 1
  store %state[offset=0], %new
  %next = load %state[offset=8]
  tailcall %next(%state)
}
```

### Cranelift IR (target)
```clif
function %step(i64) -> b1 tail {
    ebb0(v0: i64):
        v1 = load.i32 v0+0
        v2 = iadd.i32 v1, 1
        store.i32 v2, v0+0
        v3 = load.i64 v0+8
        tail_call v3(v0)
}
```

### Translation Algorithm

```rust
fn translate_cps_to_clif(cps: &CpsFunc, builder: &mut FunctionBuilder) {
    match &cps.terminator {
        CpsTerminator::TailCall { func, args } => {
            // This is the key!
            // CRANELIFT GUARANTEES no stack growth
            builder.ins().tail_call(func_ref, &args);
        }
    }
}
```

## Implementation Roadmap

### Phase 1: Working JIT (NOW)
- [ ] Fix cranelift build errors
- [ ] Create minimal `JITModule` wrapper
- [ ] Add thread-safe initialization
- [ ] Compile one simple function
- [ ] Call from C

### Phase 2: CPS Translation
- [ ] Define shared CPS format (C/Rust/OCaml)
- [ ] Write translate function
- [ ] Map Austral IR to CLIF instructions
- [ ] Verify tail_call paths

### Phase 3: typed_eval Integration
- [ ] Replace C codegen in typed_eval.c
- [ ] Add Rust bridge call
- [ ] Test with real Austral source

## Files to Create/Modify

### New Files
- `cranelift/src/lib.rs` - Main bridge
- `cranelift/src/cps.rs` - CPS→CLIF translator
- `cranelift/src/c_api.rs` - C FFI
- `runtime/cranelift_jit.c` - Wrapper for C runtime
- `test/cranelift_demo.c` - Test case

### Modified Files
- `runtime/typed_eval.c` - Use Cranelift, not GCC
- `Makefile` - Add Rust build step
- `include/vm.h` - Add Cranelift API declarations

## Technical Details

### Why Rust Bridge?

1. **Type safety**: Cranelift's API is Rust-native
2. **Memory safety**: Compiler-managed code memory
3. **Error handling**: ? operator vs manual checks

### Build Process

```bash
cd cranelift/
cargo build --release
# Produces: libaustral_cranelift_bridge.{so,a}
```

```bash
cd ..
gcc ... -Lcranelift/target/release -laustral_cranelift_bridge ...
```

### Required Cranelift Crates

```toml
[dependencies]
cranelift = "0.106"
cranelift-jit = "0.106"
cranelift-module = "0.106"
cranelift-native = "0.106"
cranelift-codegen = "0.106"
```

## Why Not Mono Cranelift?

The `once_cell` error suggests we need a different approach. Solutions:

1. **Use Box + leak**: `Box::leak(Box::new(...))` for static lifetime
2. **Pass as parameter**: Don't use static, pass &mut JITModule to functions
3. **Thread-local**: Store in thread-local storage (but pay initialization cost)

## Minimal Working Example (to complete)

```rust
use cranelift_jit::prelude::*;
use cranelift_module::*;

pub struct Compiler {
    module: JITModule,
}

impl Compiler {
    pub fn new() -> Self {
        let builder = JITBuilder::new(default_libcall_names()).unwrap();
        Self { module: JITModule::new(builder) }
    }
    
    pub fn compile_step(&mut self) -> *const u8 {
        // Use FunctionBuilder, etc.
        unimplemented!()
    }
}

// C API
#[no_mangle]
pub extern "C" fn create_compiler() -> *mut Compiler {
    Box::into_raw(Box::new(Compiler::new()))
}
```

## Success Metrics

- ✅ `cargo build` compiles with no errors
- ✅ `cranelift_demo()` returns 42
- ✅ C program can load .so and call functions
- ✅ typed_eval can use Rust bridge
- ✅ Compilation is <1ms for small functions

## Next Actions

1. Fix the OnceLock/JITModule thread-safety issue
2. Define the complete C API for OCaml to call
3. Verify Rust `tail_call` instruction is actually used
4. Replace typed_eval.c with cranelift-c API calls
5. Test end-to-end with dummy Austral source

---

**Status**: In progress - building infrastructure  
**Priority**: High for SafestOS architecture  
**Effort**: Moderate - mostly plumbing, algorithm is straightforward