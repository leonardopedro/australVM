# CPS JIT Implementation Order

## Working Files

### 1. CpsGen.ml (lib/CpsGen.ml)
- **Status**: COMMITED ea2a391e
- Takes MAST → Binary IR
- Uses opcode-first format (correct!)

### 2. Rust Bridge (safestos/cranelift/src/cps.rs)
- **Current bug**: Name sanitization not applied everywhere
- **Inputs**: Binary from CpsGen
- **Operations**:
  a. Lists functions
  b. Declares them (must sanitize 'Example.Fibonacci::Fibonacci' → valid names)
  c. Compiles bodies
  d. Definitions must resolve

### 3. lib.rs (matching HEAD format)
- **Status**: WORKING with commit 56774252
- Exposes compile_to_function_named with correct FFI

## The Runtime Error

**Error**: `can't resolve symbol Example.Fibonacci::Fibonacci`
**Happens**: During JIT finalize
**Root cause**: \(occurs in 3 places\)  → must pick all three

1. In Pass 1 (collect headers) - names read from binary
2. In Pass 2 (declare in JIT) - clean names before inserting
3. In Pass 3 (emit_expr) - clean before lookup

## Fix Checklist

```
[ ] Declared functions with __ separators
[ ] name_map uses clean names
[ ] lookup in emit_expr uses clean names  
[ ] __union_new and other externals also stubbed or configured
[ ] return_call only for chosen inside recursive function
[ ] test_fib works with Fibonacci only
```

## Current Best File: /tmp/head_cps.rs

The file is the HEAD version. The specific problems:
1. Pass 2 insert line 
2. emit_expr 0x04 lookup
3. Return-call toggle

Final state: WAS compiled with 3 warnings, OK.

Next: Apply these guarantees carefully in Rust.
