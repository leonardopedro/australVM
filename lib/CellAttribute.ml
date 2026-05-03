(*
    Cell Attribute Module
    Part of SafestOS extension
    
    Handles @cell attribute and generates CellDescriptor
*)

open Identifier
open Id
open Stages.Mtast
open CRepr

(* Cell module information *)
type cell_info = {
    module_name: identifier;
    state_type: identifier option;  (* Optional state record *)
    alloc_func: decl_id option;
    step_func: decl_id option;
    save_func: decl_id option;
    restore_func: decl_id option;
    type_hash: string;
    required_caps: string list;
  }

(* Check if module should have cell descriptor generated *)
let is_cell_module (decls: mdecl list): bool =
  (* Check for presence of cell functions or @cell attribute *)
  List.exists (function
    | MFunction (_, name, _, _, _) ->
        let n = ident_string name in
        List.mem n ["cell_alloc"; "cell_step"; "cell_save"; "cell_restore"]
    | _ -> false
  ) decls

(* Extract cell information from declarations *)
let extract_cell_info (module_name: module_name) (decls: mdecl list): cell_info option =
  if not (is_cell_module decls) then
    (* Check if there's explicit attribute in parse tree *)
    (* For now, just check function presence *)
    None
  else
    let alloc_id = List.find_map (function
      | MFunction (id, name, _, _, _) when ident_string name = "cell_alloc" -> Some id
      | _ -> None
    ) decls in
    
    let step_id = List.find_map (function
      | MFunction (id, name, _, _, _) when ident_string name = "cell_step" -> Some id
      | _ -> None
    ) decls in
    
    let save_id = List.find_map (function
      | MFunction (id, name, _, _, _) when ident_string name = "cell_save" -> Some id
      | _ -> None
    ) decls in
    
    let restore_id = List.find_map (function
      | MFunction (id, name, _, _, _) when ident_string name = "cell_restore" -> Some id
      | _ -> None
    ) decls in
    
    (* Find state record *)
    let state_type = List.find_map (function
      | MRecord (_, name, _) when ident_string name = "State" -> Some name
      | _ -> None
    ) decls in
    
    match (alloc_id, step_id, save_id, restore_id) with
    | (Some alloc, Some step, Some save, Some restore) ->
        Some {
          module_name = make_ident (mod_name_string module_name);
          state_type;
          alloc_func = Some alloc;
          step_func = Some step;
          save_func = Some save;
          restore_func = Some restore;
          type_hash = "hash_" ^ (mod_name_string module_name);
          required_caps = ["CAP_ENV"];
        }
    | _ -> None

(* Generate C code for cell descriptor *)
let generate_cell_descriptor (info: cell_info): c_decl list =
  let module_name_str = ident_string info.module_name in
  let desc_name = "cell_desc_" ^ module_name_str in
  
  (* Initialize descriptor to match vm.h layout *)
  let desc_val = CStruct (Some "CellDescriptor", [
    CSlot ("type_hash", CString ("0x" ^ info.type_hash));
    CSlot ("required_caps", CInt 1L); (* CAP_ENV by default *)
    CSlot ("alloc", CNull);
    CSlot ("drop", CNull);
    CSlot ("step", CNull);
    CSlot ("save", CNull);
    CSlot ("restore", CNull);
    CSlot ("migrate", CNull);
    CSlot ("_jit_fn_ptr", CNull);
  ]) in

  let desc_decl = CVarDefinition (
    Desc ("Static cell descriptor"),
    desc_name,
    CNamedType "CellDescriptor",
    desc_val
  ) in

  let get_desc_func = CFunctionDefinition (
    Desc ("Get cell descriptor for " ^ module_name_str),
    "get_cell_descriptor_" ^ module_name_str,
    [],
    CPointer (CNamedType "CellDescriptor"),
    CBlock [
      CReturn (CAddressOf (CVar desc_name))
    ]
  ) in
  
  [desc_decl; get_desc_func]

(* Generate wrapper functions for cell protocol *)
let generate_cell_wrappers (info: cell_info): c_decl list =
  (* These wrappers glue Austral functions to C CellDescriptor protocol if needed *)
  (* For JIT, we often resolve these dynamically, but static wrappers are useful too *)
  let _ = info in
  []
