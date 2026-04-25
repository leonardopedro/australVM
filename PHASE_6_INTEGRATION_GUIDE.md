# Phase 6 Integration Guide: Step-by-Step

**Summary**: This guide shows exactly how to integrate Compiler_cps into the Austral compiler pipeline.

---

## 📋 Prerequisites

**Files You Should Have** (from Phase 5):
```
/media/leo/.../lib/CpsGen.ml                   ✅ 7,375 bytes
/media/leo/.../lib/CamlCompiler_rust_bridge.ml ✅ 3,207 bytes  
/media/leo/.../lib/rust_bridge.c               ✅ 989 bytes
/media/leo/.../safestos/lib/libaustral_cranelift_bridge.so ✅ 4.3 MB
```

**New Files** (from Phase 6):
```
/media/leo/.../lib/Compiler_cps.ml             (just created)
```

---

## 🔧 STEP 1: Fix the Dune Build System

### Problem
The existing dune has module conflicts. We need explicit lists.

### Solution A: Automated Module List Generator

Create `generate_dune.ml` in `/media/leo/.../lib/`:

```ocaml
#!/usr/bin/env ocaml
(* Save as generate_dune.ml, run with ocaml generate_dune.ml *)

(* Get all .ml files in lib/ *)
let files = Sys.readdir "." |> Array.to_list |> List.filter (fun f -> Filename.check_suffix f ".ml") in

(* Core compiler files - everything EXCEPT phase 5/6 modules *)
let core_files = List.filter (fun f ->
  not (List.mem f ["CpsGen.ml"; "Compiler_cps.ml"; "CamCompiler.ml"; 
                   "CamCompiler_rust_bridge.ml"; "RustBridge.ml"; "CpsGen_test.ml"])
) files in

(* Generate dune *)
let () = 
  Printf.printf "(ocamllex Lexer)\n\n";
  
  (* Core library *)
  Printf.printf "; Core Austral compiler\n(library\n  (name austral_core)\n  (libraries unix str sexplib zarith yojson)\n  (preprocess (pps ppx_deriving.eq ppx_deriving.show ppx_sexp_conv))\n  (flags :standard -w -39)\n  (modules";
  List.iter (fun f -> 
    let name = String.capitalize_ascii (Filename.chop_suffix f ".ml") in
    Printf.printf " %s" name
  ) core_files;
  Printf.printf "))\n\n";
  
  (* CPS Gen library *)
  Printf.printf "; CPS IR Generator\n(library\n  (name austral_cps_gen)\n  (libraries austral_core)\n  (modules CpsGen Compiler_cps))\n\n";
  
  (* Rust bridge library *)
  Printf.printf "; Rust Bridge\n(library\n  (name austral_rust_bridge)\n  (libraries austral_core austral_cps_gen)\n  (modules CamlCompiler_rust_bridge)\n  (foreign_stubs\n    (language c)\n    (names rust_bridge)\n    (flags :standard -fPIC -ldl -L../cranelift/target/release -laustral_cranelift_bridge)))\n"
```

**Run it**:
```bash
cd /media/leo/.../lib
mv dune dune.backup
ocaml generate_dune.ml > dune
```

### Solution B: Manual Fix (If Automated Fails)

**Backup**:
```bash
cp /media/leo/.../lib/dune /media/leo/.../lib/dune.backup
```

**Edit `/media/leo/.../lib/dune` to look like this**:

```dune
(ocamllex Lexer)

; Core Austral compiler - EXPLICIT modules list
(library
  (name austral_core)
  (public_name austral.austral_core)
  (synopsis "The bootstrapping compiler for Austral.")
  (libraries unix str sexplib zarith yojson)
  (preprocess (pps ppx_deriving.eq ppx_deriving.show ppx_sexp_conv))
  (flags :standard -w -39)
  (modules_without_implementation BuiltInModules)
  (modules
    ; All core files EXCEPT phase 5/6 modules:
    AbstractionPass BodyExtractionPass BuiltIn CRenderer CRepr Cst CstUtil
    Cli CliEngine CliParser CliUtil CodeGen CombiningPass Common Compiler
    DeclIdSet DesugarBorrows DesugarPaths DesugaringPass Entrypoint Error
    Escape ExportInstantiation ExtractionPass HtmlError Id Identifier
    Linked LinearityCheck LocalBindings MemoryModule Monomorphize Mtast
    Mtlc Names PervasiveModule Pointer PolyType Reporter ReturnCheck
    Scoring Scope Span Stages Tast Temp Token TopologicalOrder TrackedCell
    TypeClasses TypingPass Universes Variables Version Visibility
    ; NOTE: CpsGen, Compiler_cps, CamlCompiler_rust_bridge NOT listed here
    ; They are in separate libraries
  ))

; OCaml library with C FFI
(library
  (name austral_caml)
  (public_name austral.austral_caml)
  (libraries austral_core caml)
  (foreign_stubs
    (language c)
    (names CamlCompiler_stubs)
    (flags :standard -fPIC))
  (modules CamlCompiler))

; CPS IR Generator
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

; Test
(executable
  (name test_cps)
  (libraries austral_core austral_caml austral_cps_gen)
  (modules CpsGen_test))
```

**Verify** the edit worked:
```bash
dune build lib/Compiler_cps.cmx
```

---

## 🔍 STEP 2: Test the Compiler_cps Module

**Create** `/media/leo/.../lib/test_compiler_cps.ml`:

```ocaml
(* Test file for Compiler_cps *)
open Stages.Mtast
open Compiler_cps
open Identifier
open Id

let () =
  Printf.printf "=== Phase 6: Compiler_cps Test ===\n";
  
  (* Build a simple MAST constant: let x = 42; return x *)
  let x_id = make_identifier "x" in
  let const_expr = MIntConstant "42" in
  let let_stmt = MLet (x_id, MonoInteger, MReturn (MLocalVar (x_id, MonoInteger))) in
  
  (* Build function definition *)
  let func_decl = MFunction (
    DeclId.really_make 1,
    make_identifier "test_func",
    [],
    MonoInteger,
    let_stmt
  ) in
  
  (* Create mono_module *)
  let mono_module = MonoModule (
    make_mod_name "Test",
    [func_decl]
  ) in
  
  Printf.printf "Input: MAST module with one function\n";
  
  (* Convert to CPS *)
  let cps_funcs = compile_module_cps mono_module in
  Printf.printf "Output: %d CPS functions\n" (List.length cps_funcs);
  
  match cps_funcs with
  | [func] ->
      Printf.printf "Function: %s\n" func.name;
      Printf.printf "Params: %s\n" (String.concat "," func.params);
      Printf.printf "Return type: %s\n" (match func.return_type with I64 -> "I64" | _ -> "other");
      Printf.printf "Body: %s\n" (string_of_stmt func.body)
  | _ -> Printf.printf "ERROR: Expected 1 function\n"
```

**Run it**:
```bash
cd /media/leo/.../lib
dune exec ./test_compiler_cps.exe
```

**Expected output**:
```
=== Phase 6: Compiler_cps Test ===
Input: MAST module with one function
Output: 1 CPS functions
Function: test_func
Params: 
Return type: I64
Body: Let(x, IntLit 42L, Return(Var x))
```

---

## 🔗 STEP 3: Integrate into Compiler.ml

**Backup** Compiler.ml:
```bash
cp /media/leo/.../lib/Compiler.ml /media/leo/.../lib/Compiler.ml.backup
```

**Edit** `/media/leo/.../lib/Compiler.ml`:

**Add import** (near line 33):
```ocaml
open Compiler_cps
```

**Add flag** (after line 34):
```ocaml
(* Phase 6: Use CPS JIT instead of C codegen *)
let use_cps_jit : bool ref = ref false
```

**Modify** the `compile_mod` function (around line 169):
```ocaml
let (env, mono): (env * mono_module) = monomorphize env typed in

(* PHASE 6: CPS Integration Point *)
if !use_cps_jit then begin
  Printf.printf "Using CPS JIT path for module %s\n" (mod_name_string name);
  
  (* Generate CPS binary *)
  let cps_path = "/tmp/cps_output_" ^ (mod_name_string name) ^ ".bin" in
  let funcs = Compiler_cps.compile_module_cps mono in
  Compiler_cps.write_cps_binary funcs cps_path;
  
  (* Read binary and compile via Rust bridge *)
  let ic = open_in_bin cps_path in
  let binary = really_input_string ic (in_channel_length ic) in
  close_in ic;
  
  (* Call Rust bridge *)
  let fn_ptr = CamlCompiler_rust_bridge.compile_mast binary in
  
  (* TODO: Store fn_ptr in env for runtime lookup *)
  Printf.printf "CPS JIT compiled, fn_ptr = %Ld\n" fn_ptr;
  
  (* Return empty CUnit (JIT handles execution) *)
  (env, CUnit ("cps_" ^ mod_name_string name, []))
end
else begin
  (* Original C codegen path *)
  let unit: c_unit = gen_module env mono in
  let unit_code: string = render_unit unit in
  let code: string = (compiler_code c) ^ "\n" ^ unit_code in
  Compiler (env, code)
end
```

---

## 🧪 STEP 4: End-to-End Test

**Create test file** `/media/leo/.../test_cps_integration.aun`:

```austral
module test_cps;

function main(): Int64 is
  let x: Int64 = 42;
  return x;
end;
```

**Compile with CPS flag**:
```bash
# Set the flag
export USE_CPS_JIT=1

# Compile using Austral with CPS enabled
cd /media/leo/.../lib
dune exec -- ./AustralCompiler.exe --use-cps test_cps_integration.aun

# Should generate:
# - Binary CPS file
# - Rust bridge compilation
# - JIT function pointer
# - Runtime execution
```

---

## 📊 STEP 5: Comparison Testing

**Compile same file twice**:

```bash
# Without CPS (default)
cd /media/leo/.../lib
dune exec -- ./AustralCompiler.exe test.aun
# Generates: test.c, compiled with gcc

# With CPS
USE_CPS_JIT=1 dune exec -- ./AustralCompiler.exe test.aun
# Generates: test.cps.bin, compiled via Rust JIT
```

**Compare**:
- Output correctness
- Compilation time
- Run time performance

---

## ⚠️ TROUBLESHOOTING

### Issue: "Module Version is used in several stanzas"
**Fix**: Add explicit `(modules ...)` to EVERY library stanza

### Issue: "Unbound module Stages" in Compiler_cps
**Fix**: Ensure Compiler_cps is in `austral_cps_gen` which has `austral_core` as dependency

### Issue: "CamlCompiler_rust_bridge not found"
**Fix**: Check that:
1. `rust_bridge.c` exists in `lib/`
2. `lib/libaustral_cranelift_bridge.so` exists in `safestos/lib/`
3. Foreign stubs path is correct

---

## ✅ VERIFICATION CHECKLIST

- [ ] `dune build lib/Compiler_cps.cmx` succeeds
- [ ] `dune exec ./test_compiler_cps.exe` shows correct conversion
- [ ] `dune build lib/austral_cps_gen.cma` compiles
- [ ] Compiler.ml modification saves without errors
- [ ] Test Austral program compiles with CPS flag
- [ ] Binary IR matches format spec
- [ ] Rust bridge compiles the binary
- [ ] JIT execution produces correct output

---

## 🎯 SUCCESS!

Once verified, you have:
1. **Working Compiler_cps** converting MAST → CPS IR
2. **Integration** into main compiler pipeline  
3. **Hybrid backend** (C codegen + CPS JIT)
4. **Verified end-to-end** Austral → IR → JIT → Native

**Document what worked** in `PHASE_6_FINAL_REPORT.md`!
