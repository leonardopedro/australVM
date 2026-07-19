# Phase 6: Integration Plan
## Integrating CPS JIT Pipeline into Main Austral Compiler

---

## 🎯 Objective

**Integrate the working Phase 5 CPS→JIT pipeline into the main Austral compiler**, creating a hybrid compilation model:
- **Traditional path**: AST → C codegen → compile → execute
- **New CPS path**: AST → CPS IR → Cranelift JIT → execute ✨

---

## 🏗️ Current Architecture (Verified)

### Existing Flow (Compiler.ml, line 156-173)
```
Parser
  → Combined (abstract syntax)
  → Typed (type-checked)
  → Linked
  → Monomorphized (mono: Mtast.mono_module)
  → gen_module() → CUnit
  → render_unit() → C string
```

### Phase 5 Components (Ready to Integrate)
```
lib/CpsGen.ml                    - Converts Mtast to binary CPS IR
lib/CamlCompiler_rust_bridge.ml  - FFI interface to Rust
lib/rust_bridge.c                - C linking stub
libaustral_cranelift_bridge.so  - Compiled Rust backend
```

---

## 🔗 Integration Strategy

### Two Approaches

**Option A: Parallel Code Paths** (Recommended)
- Keep existing C codegen as fallback
- Add optional flag for CPS JIT path
- Allows comparison and gradual migration

**Option B: Replace C Codegen** (Riskier)
- Replace gen_module with CPS generation
- Remove C rendering
- Fully commit to JIT

**Decision: Option A** - Create `Compiler_cps.ml` wrapper that can be toggled

---

## 📋 Implementation Steps

### Step 1: Create CPS Compiler Wrapper
**File**: `lib/Compiler_cps.ml`

**Purpose**: Bridge between MAST (monomorphic AST) and CpsGen

**Key Functions**:
```ocaml
(* Convert mono_module to binary CPS IR *)
val compile_to_cps : mono_module -> bytes

(* Type-aware expression conversion *)
val convert_expr : Mtast.mexpr -> CpsGen.cps_expr

(* Handle all MAST statement types *)
val convert_stmt : Mtast.mstmt -> CpsGen.cps_stmt

(* Main entry: module → binary IR *)
val compile_module_cps : mono_module -> string (* binary path *)
```

### Step 2: Modify Compiler Compilation Flow
**File**: `lib/Compiler.ml`

**Changes needed**:
1. Add import: `open Compiler_cps`
2. Add flag: `use_cps_jit : bool`
3. Modify `compile_mod` to branch:
   - If `use_cps_jit`: call `Compiler_cps.compile_module_cps`
   - Else: existing C path

**Target Code Section** (lines 169-172):
```ocaml
let (env, mono): (env * mono_module) = monomorphize env typed in
(* Branch point here *)
if use_cps_jit then
  let cps_path = Compiler_cps.compile_module_cps mono in
  (* Register JIT function with runtime *)
  ...
else
  let unit: c_unit = gen_module env mono in
  let unit_code: string = render_unit unit in
  ...
```

### Step 3: Handle Breaking Changes

**Challenge 1: Function Signatures**
- Current: `gen_module : env → mono_module → c_unit`
- New: Need `compile_module_cps : mono_module → bytes`

**Solution**: Create adapter that converts mono_module to individual function IR

**Challenge 2: Multi-function Modules**
- CpsGen currently compiles single functions
- Need to handle modules with multiple functions

**Solution**: Iterate over `decls` in `MonoModule`, compile each function separately

**Challenge 3: Runtime Integration**
- C code calls Austral functions directly
- JIT code needs dispatch mechanism

**Solution**: Use `scheduler_dispatch` from Rust bridge

### Step 4: Map MAST to CPS IR

**Expression Mapping** (from Stages.ml lines 623-663):
```
MNilConstant          → ? (need to handle)
MBoolConstant b       → CPS BoolLit(b)
MIntConstant s        → CPS IntLit(s)
MFloatConstant s      → CPS FloatLit(s)
MStringConstant s     → CPS StringLit(s)
MConstVar(q, ty)      → CPS Const(qident_to_string(q))
MParamVar(id, ty)     → CPS Var(id)
MLocalVar(id, ty)     → CPS Var(id)
MConcreteFuncall(d, q, args, ty) → CPS App(qident_to_string(q), args)
MGenericFuncall(id, args, ty)    → CPS App(mono_id_string(id), args)
...                    → ... (remaining cases)
```

**Statement Mapping** (from Stages.ml lines 595-620):
```
MSkip                 → CPS.Skip
MLet(id, ty, body)    → CPS.Let(id, init_expr, body)
MIf(cond, t, f)       → CPS.If(cond, t, f)
MReturn(expr)         → CPS.Return(expr)
MAssign(dest, src)    → CPS.Assign(dest, src)
MDiscarding(expr)     → CPS.Discard(expr)
MBlock(s1, s2)        → CPS.Block(s1, s2)
...                    → ... (remaining cases)
```

**Unsupported Cases** (for now):
- Destructuring
- Borrow statements
- While loops
- For loops
- Case/match

These will use fallback to C codegen initially.

---

## 🧪 Testing Strategy

### Phase 6.1: Unit Tests
Test MAST → CPS conversion for:
- Simple functions (`lib/CpsGen_test.ml` pattern)
- Arithmetic: `function add(x, y) -> x + y`
- Conditionals: `function abs(x) -> if x < 0 then -x else x`
- Recursive: `function fact(n) -> if n <= 1 then 1 else n * fact(n-1)`

### Phase 6.2: Integration Tests
Test full pipeline:
1. Write Austral code
2. Compile via CPS path
3. Generate binary IR
4. Pass to Rust bridge
5. Execute JIT code
6. Verify return values

### Phase 6.3: Comparison Tests
Run same code through both paths:
- C codegen path (existing)
- CPS JIT path (new)
- Compare results
- Measure performance

---

## 📊 Success Criteria

**Minimum (MVP)**:
- ✅ Single function with Int literals compiles
- ✅ Binary IR generated correctly
- ✅ Rust bridge can compile it
- ✅ JIT execution returns correct value
- ✅ Works for: return 42, add/sub/mul, if/else

**Stretch Goals**:
- ✅ Multiple functions in module
- ✅ Function calls between functions
- ✅ Tail call optimization verified
- ✅ Performance matches C codegen
- ✅ All MAST constructs supported
- ✅ Hybrid mode (C + JIT) working

---

## 🚀 Quick Start Implementation

### File 1: lib/Compiler_cps.ml (Foundation)
```ocaml
module Mt = Stages.Mtast
module CG = CpsGen

(* Convert MAST expression to CPS expression *)
let rec convert_expr = function
  | Mt.MIntConstant s -> CG.IntLit (Int64.of_string s)
  | Mt.MParamVar (id, _) -> CG.Var id
  | Mt.MConcreteFuncall (_, q, args, _) ->
      let name = qident_to_string q in
      let args = List.map convert_expr args in
      CG.App (name, args)
  | _ -> failwith "Unsupported expression for CPS"

(* Convert MAST statement to CPS statement *)
let rec convert_stmt = function
  | Mt.MReturn expr -> CG.Return (convert_expr expr)
  | Mt.MLet (id, _, body) -> 
      CG.Let (id, CG.IntLit 0L, convert_stmt body)  (* Placeholder *)
  | Mt.MIf (cond, t, f) ->
      CG.If (convert_expr cond, convert_stmt t, convert_stmt f)
  | Mt.MSkip -> CG.Skip
  | _ -> failwith "Unsupported statement"

(* Compile a single function declaration *)
let compile_function decl =
  match decl with
  | Mt.MFunction (_, name, params, _, body) ->
      let params = List.map (fun (id, _) -> id) params in
      let body_stmt = convert_stmt body in
      CG.compile_function name params CG.I64 body_stmt
  | _ -> None  (* Skip non-functions *)

(* Main entry point *)
let compile_module_cps (MonoModule (_, decls)) =
  (* Filter to just functions *)
  let functions = List.filter_map compile_function decls in
  (* Serialize to binary *)
  let binary = CG.serialize_functions functions in
  (* Write to temp file *)
  let path = "/tmp/module.cps" in
  let oc = open_out path in
  output_string oc binary;
  close_out oc;
  path
```

### File 2: Modify Compiler.ml
```ocaml
(* Add near top, after imports *)
open Compiler_cps
let use_cps_jit = ref false  (* Flag to toggle *)

(* Around line 169 in compile_mod *)
let (env, mono) = monomorphize env typed in
if !use_cps_jit then
  (* CPS path *)
  let cps_path = compile_module_cps mono in
  (* Read binary, compile via Rust bridge *)
  let binary = read_file_bytes cps_path in
  let fn_ptr = CamlCompiler_rust_bridge.compile_mast binary in
  (* Store pointer in env for later dispatch *)
  (* ... store mechanism ... *)
  let unit = CUnit ("cps_" ^ mod_name_string name, []) in
  (env, unit)
else
  (* Existing C path *)
  let unit = gen_module env mono in
  (env, unit)
```

---

## 🔄 Integration Workflow

### Daily Development Loop

1. **Write OCaml code** in `lib/Compiler_cps.ml`
2. **Test conversion** with small MAST nodes
3. **Build with dune**: `dune build lib/Compiler_cps.cmx`
4. **Modify Compiler.ml** to use new module
5. **Test end-to-end**: Write Austral → Compile → Execute
6. **Verify results** match expected output

### Debugging Approach

**If compilation fails**:
```bash
# Check OCaml compilation
dune build lib/Compiler_cps.cmx

# Check for syntax errors
dune build @check
```

**If generated CPS is wrong**:
```bash
# Inspect generated binary
hexdump -C /tmp/module.cps

# Trace through CpsGen
# Add debug prints in convert_expr/convert_stmt
```

**If Rust bridge fails**:
```bash
# Test bridge in isolation
cd safestos
./DEMO_CPS_PIPELINE.sh

# Check symbols
nm lib/libaustral_cranelift_bridge.so | grep compile
```

---

## 📝 Notes for Implementation

### Type System Notes
- MAST has full type information via `mono_ty`
- CPS Gen needs types for Cranelift
- Current CPS Gen supports: i64, i32, bool
- Need to map `mono_ty` → CPS type

### Module System Notes
- Austral has modules with names like `Austral.Pervasive`
- CPS binary format has `name_len + name` for each function
- Need to keep function names unique across modules

### Performance Considerations
- JIT compile time vs C compile time
- Runtime dispatch overhead
- Memory footprint of JIT code

---

## ⚠️ Risks & Mitigation

**Risk 1**: MAST → CPS conversion incomplete
- **Mitigation**: Start with subset, extend iteratively
- **Fallback**: Continue using C codegen for unsupported cases

**Risk 2**: FFI marshaling issues
- **Mitigation**: Test with simple data types first
- **Fuzzing**: Random byte tests on Rust bridge

**Risk 3**: Build system conflicts
- **Mitigation**: Keep CPS libraries separate
- **Conditionals**: Dune features to toggle compilation

**Risk 4**: Runtime crashes
- **Mitigation**: Extensive testing with Valgrind
- **Safety**: Check all pointers before calling Rust

---

## 📅 Timeline (Estimated)

- **Week 1**: Write `Compiler_cps.ml`, test MAST→CPS for basic cases
- **Week 2**: Modify `Compiler.ml`, integrate binary generation
- **Week 3**: End-to-end testing, debug FFI issues
- **Week 4**: Performance testing, comparison with C path

---

## 🎯 Deliverables

1. **lib/Compiler_cps.ml** - Conversion logic
2. **Modified lib/Compiler.ml** - Integration point
3. **Test suite** - End-to-end verification
4. **Documentation** - Architecture updates
5. **Performance report** - CPS vs C comparison

---

## 🔗 Related Files

**From Phase 5**:
- `lib/CpsGen.ml` - Core CPS generator
- `lib/CamlCompiler_rust_bridge.ml` - FFI interface
- `safestos/cranelift/src/cps.rs` - Rust parser

**Current Austral**:
- `lib/Compiler.ml` - Main compilation flow
- `lib/Stages.ml` - MAST definitions
- `lib/Monomorphize.ml` - Monomorphization
- `lib/CodeGen.ml` - C code generation

**New for Phase 6**:
- `lib/Compiler_cps.ml` - CPS integration layer

---

## ✅ Phase Checklist

- [ ] Create Compiler_cps.ml foundation
- [ ] Implement MAST → CPS conversion for basic types
- [ ] Handle single-function modules
- [ ] Modify Compiler.ml to use CPS path
- [ ] Generate binary IR files
- [ ] Verify Rust bridge can compile IR
- [ ] Execute JIT code and verify results
- [ ] Handle multi-function modules
- [ ] Support conditionals and arithmetic
- [ ] Implement tail-call optimization
- [ ] Final integration testing
- [ ] Performance benchmarks
- [ ] Update architecture documentation

---

**Status**: Ready to begin implementation  
**Next Task**: Create lib/Compiler_cps.ml with basic MAST→CPS conversion

**Note**: This is an aggressive but achievable plan. Success depends on iterative testing and maintaining working fallback paths.
