/-
`scripts/lean-module-decls.lean` — print the declarations OWNED by a module.

Why this exists: `lean4export Mod` exports `Mod` **and every transitive
import**. In a Mathlib-based project that is ~50 000 declarations and a ~300 MB
NDJSON file, and `scripts/lean2mm0-stmts` refuses it (MAX_DECLS = 4000) rather
than emitting a degenerate statement file. lean4export takes an explicit
declaration list after `--`, so the fix is to ask Lean which declarations the
module itself introduced — `Environment.getModuleIdxFor?` answers exactly that,
without any name-convention guessing (a module's decls need not share its
namespace prefix).

Usage (from the Lean project root, so `lake env` supplies LEAN_PATH):

    lake env lean --run /path/to/lean-module-decls.lean Layout
    # Groovy: one fully qualified declaration name per line, stdout

The caller then runs:

    lake env lean4export Layout -- $(lake env lean --run … Layout)
-/
import Lean

open Lean

def main (args : List String) : IO Unit := do
  let mods := args.filter (!·.startsWith "-")
  if mods.isEmpty then
    IO.eprintln "usage: lean-module-decls.lean <Module> [<Module> …]"
    IO.Process.exit 2
  -- `toName` splits on '.', so dotted module names (`Book.AlignedDeepLearning`)
  -- work as well as top-level ones (`Layout`).
  let imports : Array Import :=
    (mods.map (fun m => { module := m.toName : Import })).toArray
  let env ← importModules imports {}
  -- Name of the module whose declarations we want → its index in this env.
  let wanted : List ModuleIdx := mods.filterMap fun m =>
    env.header.moduleNames.findIdx? (· == m.toName)
  if wanted.isEmpty then
    IO.eprintln s!"none of {mods} are loaded in this environment"
    IO.Process.exit 3
  let mut names : Array Name := #[]
  for (n, _) in env.constants.toList do
    if let some idx := env.getModuleIdxFor? n then
      if wanted.contains idx then names := names.push n
  -- Sorted so two runs over the same module produce byte-identical arguments
  -- (and therefore a byte-identical export that the drift gate can hash).
  for n in names.qsort (·.toString < ·.toString) do
    IO.println n.toString
