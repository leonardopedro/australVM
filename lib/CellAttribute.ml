(*
    Cell Attribute Module
    Part of SafestOS extension
    
    Handles @cell attribute and generates CellDescriptor
*)

open Identifier
open Type
open MonoType
open Stages.Mtast
open CRepr
open Util

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
      | MRecord (id, name, _, _, _) when ident_string name = "State" -> Some name
      | _ -> None
    ) decls in
    
    match (alloc_id, step_id, save_id, restore_id) with
    | (Some alloc, Some step, Some save, Some restore) ->
        Some {
          module_name = module_name_to_string module_name;
          state_type;
          alloc_func = Some alloc;
          step_func = Some step;
          save_func = Some save;
          restore_func = Some restore;
          type_hash = "hash_" ^ (module_name_to_string module_name);
          required_caps = ["CAP_ENV"];
        }
    | _ -> None

(* Generate C code for cell descriptor *)
let generate_cell_descriptor (info: cell_info): c_decl list =
  (* Generate the get_cell_descriptor function that returns the descriptor *)
  let desc_name = "cell_descriptor_" ^ info.module_name in
  let type_info_name = "typeinfo_" ^ info.module_name in
  
  (* Generate type info *)
  let type_info_struct = CStructDefinition (
    Desc ("Type info for " ^ info.module_name),
    CStruct (Some "TypeInfo", [
      CSlot ("name", CPointer (CNamedType "char"));
      CSlot ("required_caps", CNamedType "uint64_t");
    ])
  ) in
  
  (* Generate descriptor pointer *)
  (*
  let alloc_func = match info.alloc_func with
    | Some id -> CVar (gen_decl_id id)
    | None -> CNull
  in
  *) (* Simplified for now *)
  
  (* We'll generate a simplified descriptor *)
  let desc_func = CFunctionDefinition (
    Desc ("Get cell descriptor for " ^ info.module_name),
    "get_cell_descriptor",
    [],
    CPointer (CNamedType "CellDescriptor"),
    CBlock []
  ) in
  
  [type_info_struct; desc_func]

(* Generate wrapper functions for cell protocol *)
let generate_cell_wrappers (info: cell_info): c_decl list =
  (* These wrappers glue Austral functions to C CellDescriptor protocol *)
  []
