# Phase 6: Complete Integration Walkthrough

**Current State**:
- ✅ All Phase 5 components working and tested
- ✅ Compiler_cps.ml created and ready
- ✅ Documentation complete
- ⚠️ Dune system needs fixed (MODULE CONFLICT)

**The Fix**: Add explicit module lists to dune

## Step-by-Step Integration

### 1. FIX THE DUNE FILE

**The Problem**: Dune 3.20.2 autodiscovers all .ml files in lib/ and puts them in every library → conflicts'

**The Solution**: Explicit `modules` field for each library

**Current dune** (from git):
```dune
(library
  (name austral_core)
  ... NO modules field ...  ← autodiscovers everything
)

(library  
  (name austral_caml)
  ... NO modules field ...  ← autodiscovers everything
)
```

**Required dune**:
```dune
(library
  (name austral_core)
  (libraries unix str sexplib zarith yojson)
  (preprocess (pps ppx_deriving.eq ppx_deriving.show ppx_sexp_conv))
  (flags :standard -w -39)
  (modules AbstractionPass BodyExtractionPass ... (around 70 modules))
)

(library
  (name austral_caml)
  (libraries austral_core caml)
  (foreign_stubs (language c) (names CamlCompiler_stubs) (flags :standard -fPIC))
  (modules CamlCompiler)
)

(library
  (name austral_cps_gen)
  (libraries austral_core austral_caml)
  (modules CpsGen Compiler_cps)  ← Both new files!
)

(library
  (name austral_rust_bridge)
  (libraries austral_core austral_caml austral_cps_gen)
  (modules CamlCompiler_rust_bridge)
  (foreign_stubs (language c) (names rust_bridge) ...)
)
```

**Command to fix it**:
```bash
cd /media/leo/.../lib
# Use the generator script or manually edit
# See PHASE_6_INTEGRATION_GUIDE.md Step 1 for complete list
```

### 2. TEST COMPILATION

After fixing dune:
```bash
cd /media/leo/.../lib
dune build lib/austral_cps_gen.cma
```

Expected: **SUCCESS** - no errors

### 3. VERIFY Compiler_cps MODULE

Create test file:
```bash
cd /media/leo/.../lib
cat > test_integration.ml << 'TESTEOF'
open Compiler_cps
open Stages.Mtast
open Identifier
open Id

let () =
  (* Simple MAST: let x = 42; return x *)
  let x = make_identifier "x" in
  let stmt = MLet (x, MonoInteger, MReturn (MLocalVar (x, MonoInteger))) in
  let func = MFunction (DeclId.really_make 1, make_identifier "f", [], MonoInteger, stmt) in
  let mm = MonoModule (make_mod_name "Test", [func]) in
  let cps = compile_module_cps mm in
  Printf.printf "Converted to %d functions\n" (List.length cps)
TESTEOF

dune exec ./test_integration.exe
```

Expected: **SUCCESS** - prints "Converted to 1 functions"

### 4. ADD TO Compiler.ml

See: `PHASE_6_INTEGRATION_GUIDE.md` Step 3 for exact edits

### 5. END-TO-END TEST

```bash
echo "function main(): Int64 is return 42; end;" > test.aun
dune exec -- ./AustralCompiler.exe --cps test.aun
./test                      # Should output 42
```

---

## THE ACTUAL DUNE EDIT (Copy/Paste Ready)

**File**: `/media/leo/.../lib/dune`

**Change**: Replace entire file with:

```dune
(ocamllex Lexer)

; Core Austral compiler - EXPLICIT modules
(library
  (name austral_core)
  (public_name austral.austral_core)
  (synopsis "The bootstrapping compiler for Austral.")
  (libraries unix str sexplib zarith yojson)
  (preprocess (pps ppx_deriving.eq ppx_deriving.show ppx_sexp_conv))
  (flags :standard -w -39)
  (modules
    AbstractionPass BodyExtractionPass BuiltIn CRenderer CRepr Cst CstUtil
    Cli CliEngine CliParser CliUtil CodeGen CombiningPass Common Compiler
    DeclIdSet DesugarBorrows DesugarPaths DesugaringPass Entrypoint Env
    EnvExtras EnvTypes EnvUtils Error ErrorText Escape ExportInstantiation
    ExtractionPass HtmlError Id Identifier IdentifierMap IdentifierSet
    ImportResolution Imports LexEnv LiftControlPass LinearityCheck
    ModIdSet ModuleNameSet MonoType MonoTypeBindings Monomorphize
    MtastUtil Names ParserInterface Qualifier Region RegionMap Reporter
    ReturnCheck SourceContext Span Stages StringSet TailCallAnalysis
    TailCallUtil TastUtil Type TypeBindings TypeCheckExpr TypeClasses
    TypeErrors TypeMatch TypeParameter TypeParameters TypeParser
    TypeReplace TypeSignature TypeStripping TypeSystem TypeVarSet
    TypingPass Util Version
  ))

; OCaml library with C FFI for SafestOS
(library
  (name austral_caml)
  (public_name austral.austral_caml)
  (synopsis "OCaml compiler with C FFI for SafestOS")
  (libraries austral_core caml)
  (foreign_stubs
    (language c)
    (names CamlCompiler_stubs)
    (flags :standard -fPIC))
  (modules CamlCompiler))

; CPS IR Generator - NEW
(library
  (name austral_cps_gen)
  (libraries austral_core austral_caml)
  (modules CpsGen Compiler_cps))

; Rust Cranelift Bridge
(library
  (name austral_rust_bridge)
  (libraries austral_core austral_caml austral_cps_gen)
  (modules CamlCompiler_rust_bridge)
  (foreign_stubs
    (language c)
    (names rust_bridge)
    (flags :standard -fPIC -ldl -L../cranelift/target/release -laustral_cranelift_bridge)))

; CPS Test (optional)
(executable
  (name test_cps)
  (libraries austral_core austral_caml austral_cps_gen))

(documentation)
```

**Note**: Removed `CamlCompiler_stubs` from modules (it's a C file, not OCaml).

---

## SEE ALSO

- `PHASE_6_INTEGRATION_GUIDE.md` - Full step-by-step
- `PHASE_6_PLAN.md` - Architecture and timeline  
- `PHASE_6_STATUS.md` - Current blockers

## Quick Fix Command

```bash
cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/australVM/lib
# The file dune should now be replaced with the explicit version above
# Then:
dune build lib/austral_cps_gen.cma
```

This should work. The key is the **explicit modules list**.
