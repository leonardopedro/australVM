#!/bin/bash
# Generate correct dune with explicit module allocation

cd /media/leo/e7ed9d6f-5f0a-4e19-a74e-83424bc154ba/australVM/lib

# Lists of files for each library
CORE_FILES=(
  "AbstractionPass" "BodyExtractionPass" "BuiltIn" "BuiltInModules" "CRenderer" "CRepr" "Cst" "CstUtil" 
  "Cli" "CliEngine" "CliParser" "CliUtil" "CodeGen" "CombiningPass" "Common" "Compiler"
  "DeclIdSet" "DesugarBorrows" "DesugarPaths" "DesugaringPass" "Entrypoint" "Env" "EnvExtras" "EnvTypes"
  "EnvUtils" "Error" "ErrorText" "Escape" "ExportInstantiation" "ExtractionPass" "HtmlError" "Id"
  "Identifier" "IdentifierMap" "IdentifierSet" "ImportResolution" "Imports" "LexEnv" "LiftControlPass"
  "LinearityCheck" "ModIdSet" "ModuleNameSet" "MonoType" "MonoTypeBindings" "Monomorphize" "MtastUtil"
  "Names" "ParserInterface" "Qualifier" "Region" "RegionMap" "Reporter" "ReturnCheck" "SourceContext"
  "Span" "Stages" "StringSet" "TailCallAnalysis" "TailCallUtil" "TastUtil" "Type" "TypeBindings"
  "TypeCheckExpr" "TypeClasses" "TypeErrors" "TypeMatch" "TypeParameter" "TypeParameters" "TypeParser"
  "TypeReplace" "TypeSignature" "TypeStripping" "TypeSystem" "TypeVarSet" "TypingPass" "Util" "Version"
)

CAML_FILES=("CamlCompiler" "CamlCompiler_stubs")

CPS_FILES=("CpsGen" "Compiler_cps")

BRIDGE_FILES=("CamlCompiler_rust_bridge")

# Write dune file
cat > dune << 'EOF'
(ocamllex Lexer)

; Core Austral compiler
(library
  (name austral_core)
  (public_name austral.austral_core)
  (synopsis "The bootstrapping compiler for Austral.")
  (libraries unix str sexplib zarith yojson)
  (preprocess (pps ppx_deriving.eq ppx_deriving.show ppx_sexp_conv))
  (flags :standard -w -39)
  (modules_without_implementation BuiltInModules)
  (modules
EOF

# Add core modules
first=true
for mod in "${CORE_FILES[@]}"; do
  if $first; then
    printf "    %s" "$mod" >> dune
    first=false
  else
    printf " %s" "$mod" >> dune
  fi
done
echo "))" >> dune

# OCaml C FFI library
cat >> dune << 'EOF'

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

EOF

# CPS Gen library
cat >> dune << 'EOF'

; CPS IR Generator
(library
  (name austral_cps_gen)
  (libraries austral_core austral_caml)
  (modules CpsGen Compiler_cps))

EOF

# Rust Bridge library
cat >> dune << 'EOF'

; Rust Cranelift Bridge
(library
  (name austral_rust_bridge)
  (libraries austral_core austral_caml austral_cps_gen)
  (modules CamlCompiler_rust_bridge)
  (foreign_stubs
    (language c)
    (names rust_bridge)
    (flags :standard -fPIC -ldl -L../cranelift/target/release -laustral_cranelift_bridge)))

(documentation)
EOF

echo "Dune file generated successfully"
