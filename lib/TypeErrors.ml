open Error
open ErrorText
open Identifier
open Type

let arithmetic_incompatible_types ~lhs ~rhs =
  austral_raise TypeError [
    Text "Both operands to an arithmetic expression must be compatible types. The LHS has type ";
    Type lhs;
    Text " but the RHS has type ";
    Type rhs;
    Text "."
  ]

let arithmetic_not_numeric ty =
  austral_raise TypeError [
    Text "Both operands of an arithmetic operator must be numeric types, but ";
    Type ty;
    Text " is not."
  ]

let array_indexing_disallowed ty =
  austral_raise TypeError [
    Text "The array indexing operator is not allowed for the type ";
    Type ty
  ]

let borrow_constant () =
  austral_raise TypeError [
    Text "Constants cannot be borrowed."
  ]

let reborrow_constant () =
  austral_raise TypeError [
    Text "Constants cannot be borrowed."
  ]

let reborrow_wrong_type (ty: ty) =
  austral_raise TypeError [
      Text "Reborrow expressions take a variable of mutable reference type, but the expression has type ";
      Type ty
  ]

let borrow_non_linear ty =
  austral_raise TypeError [
      Text "Cannot borrow ";
      Type ty;
      Text " because it is not a linear type."
    ]

let cannot_borrow_immutable_var_mutably (name: identifier) =
  austral_raise TypeError [
      Text "Cannot borrow the variable ";
      Code (ident_string name);
      Text " mutably because the variable is immutable."
    ]

let cannot_borrow_param_mutably (name: identifier) (ty: ty) =
  austral_raise TypeError [
      Text "Cannot borrow the variable ";
      Code (ident_string name);
      Text " mutably because it is a function parameter, and therefore immutable.";
      Text "\nYou can move it to a mutable local variable: ";
      Code ("var mut_" ^ (ident_string name) ^ ": " ^ (type_string ty) ^ " := " ^ (ident_string name) ^ ";");
    ]

let call_non_callable ty =
  austral_raise TypeError [
    Text "The type ";
    Type ty;
    Text " is neither a function, method or function pointer, and cannot be called."
  ]

let call_wrong_arity ~name ~expected ~actual =
  austral_raise TypeError [
    Text "The callable ";
    Code (ident_string name);
    Text " expects ";
    Text (string_of_int expected);
    Text " argument";
    Text (if expected = 1 then " " else "s ");
    Text "but ";
    Text (string_of_int actual);
    Text (if actual = 1 then " was " else " were ");
    Text "given."
  ]

let case_non_exhaustive () =
  austral_raise TypeError [
    Text "Non-exhaustive case statement."
  ]

let case_non_public () =
  austral_raise TypeError [
    Text "Union must be public or from the same module to be used in a case statement."
  ]

let case_non_union ty =
  austral_raise TypeError [
    Text "The case expression has type ";
    Type ty;
    Text " which is not a union type."
  ]

let case_wrong_slots (binding_names, slot_names) =
  austral_raise TypeError [
    Text "The set of slots (";
    Code (String.concat ", " binding_names);
    Text ") in the case statement doesn't match the set of slots in the union definition (";
    Code (String.concat ", " slot_names);
    Text ")."
  ]

let cast_different_references ~different_types ~different_regions =
  let text = match different_types, different_regions with
  | true, false -> [
      Text "Cannot cast a reference to one with a different underlying type."
    ]
  | false, true -> [
      Text "Cannot cast a reference to one with a different underlying region."
    ]
  | _ -> [
      Text "Cannot cast to a different region"
    ]
  in
  austral_raise TypeError text

let cast_invalid ~target ~source =
  austral_raise TypeError [
    Text "Cannot cast to ";
    Type target;
    Text " from ";
    Type source
  ]

let cast_write_ref_to_non_ref () =
  austral_raise TypeError [
    Text "Write references can only be casted to a read reference."
  ]

let condition_not_boolean ~kind ~form ~ty =
  austral_raise TypeError [
    Text "The condition in ";
    Code kind;
    Text "-";
    Text form;
    Text "s should be a boolean type, but is ";
    Type ty
  ]

let constructor_not_named declaration =
  austral_raise TypeError [
    Text "Arguments to ";
    Text declaration;
    Text " constructor must be named."
  ]

let constructor_wrong_args declaration =
  austral_raise TypeError [
    Text "Arguments to ";
    Text declaration;
    Text " constructor have the wrong names."
  ]

let dereference_non_reference ty =
  austral_raise TypeError [
    Text "Only references can be dereferenced, but ";
    Type ty;
    Text " is not."
  ]

let dereference_non_free ty =
  austral_raise LinearityError [
    Text "Cannot dereference non-free type ";
    Type ty
  ]

let destructure_non_record ty =
  austral_raise TypeError [
    Text "The type ";
    Type ty;
    Text " cannot be destructured because it is not a record."
  ]

let destructure_not_public ty =
  austral_raise TypeError [
    Text "The type ";
    Type ty;
    Text " is not a public record and so cannot be destructured."
  ]

let destructure_wrong_slots ty =
  austral_raise TypeError [
    Text "The set of slots mentioned differs from the set of slots in ";
    Type ty;
  ]

let discard_linear universe =
  austral_raise TypeError [
    Text "Cannot discard a value in the ";
    Code (universe_string universe);
    Text " universe."
  ]

let expression_not_constant () =
  austral_raise TypeError [
    Text "The expression is not a valid constant expression."
  ]

let for_bounds_non_integral ~bound ~ty =
  austral_raise TypeError [
    Text "The type of the ";
    Text bound;
    Text " value in the loop range is ";
    Type ty;
    Text " which is not ";
    Code "Index";
    Text "."
  ]

let foreign_in_safe_module () =
  austral_raise DeclarationError [
    Text "Can't declare a foreign function in a safe module."
  ]

let foreign_type_parameters () =
  austral_raise DeclarationError [
    Text "Foreign functions can't have type parameters."
  ]

let rust_ownership_required ~name =
  austral_raise DeclarationError [
    Text "The foreign import ";
    Code name;
    Text " uses the extern rust namespace (external names beginning ";
    Code "rs_";
    Text "), which requires a ";
    Code "Rust_Ownership";
    Text " pragma declaring each parameter's and the return value's ownership: ";
    Code "borrowed";
    Text " (the callee must not retain the value) or ";
    Code "owned";
    Text " (the value is consumed by the call)."
  ]

let rust_ownership_not_tier1 ~name =
  austral_raise DeclarationError [
    Text "The ";
    Code "Rust_Ownership";
    Text " pragma applies only to extern rust imports, i.e. foreign imports whose external name begins with ";
    Code "rs_";
    Text ". The external name ";
    Code name;
    Text " belongs to a different namespace, whose ownership conventions are fixed by that ABI."
  ]

let rust_ownership_not_foreign () =
  austral_raise DeclarationError [
    Text "The ";
    Code "Rust_Ownership";
    Text " pragma can only annotate a foreign import declared with ";
    Code "Foreign_Import";
    Text "."
  ]

let rust_ownership_arity ~expected ~got =
  austral_raise DeclarationError [
    Text "The ";
    Code "Rust_Ownership";
    Text " pragma's Params list must have one ownership annotation per parameter: the function has ";
    Text (string_of_int expected);
    Text (if expected = 1 then " parameter but " else " parameters but ");
    Text (string_of_int got);
    Text (if got = 1 then " annotation." else " annotations.")
  ]

let rust_ownership_annotation ~annotation =
  austral_raise DeclarationError [
    Text "Invalid ownership annotation ";
    Code annotation;
    Text ". Ownership annotations must be ";
    Code "borrowed";
    Text " or ";
    Code "owned";
    Text "."
  ]

let handle_ownership_not_foreign () =
  austral_raise DeclarationError [
    Text "The ";
    Code "Handle_Ownership";
    Text " pragma can only annotate a foreign import declared with ";
    Code "Foreign_Import";
    Text ": it describes the provider symbols the generated handle wrappers call, and a function with a body is not an import."
  ]

let handle_ownership_bad_target ~name =
  austral_raise DeclarationError [
    Text "Invalid handle target ";
    Code name;
    Text ". The external name of a ";
    Code "Handle_Ownership";
    Text " import is the handle's target: a plain identifier from which the wrappers derive ";
    Code "<target>__new";
    Text " and ";
    Code "<target>__dispose";
    Text ", so it must be non-empty, use only letters, digits and underscores, and not already name one of those wrappers."
  ]

let handle_ownership_bad_type ~type_name =
  austral_raise DeclarationError [
    Text "Invalid handle type name ";
    Code type_name;
    Text ". It is rendered into the handle registry's type tag, so it must be a plain identifier (letters, digits and underscores)."
  ]

let handle_ownership_symbols ~constructor ~destructor =
  austral_raise DeclarationError [
    Text "Invalid provider symbols in a ";
    Code "Handle_Ownership";
    Text " pragma: ";
    Code constructor;
    Text " and ";
    Code destructor;
    Text " must both be non-empty and different, because one symbol cannot both allocate and free the value."
  ]

let handle_ownership_accessor ~use_symbol =
  austral_raise DeclarationError [
    Text "Invalid ";
    Code "Use";
    Text " symbol ";
    Code use_symbol;
    Text " in a ";
    Code "Handle_Ownership";
    Text " pragma. It names the provider's reader, which the generated ";
    Code "__use";
    Text " wrapper calls as ";
    Code "unsafe extern \"C\" fn(*mut c_void) -> i64";
    Text ", so it must be a non-empty plain identifier and must not be the constructor or the destructor: reading a value and creating or freeing it cannot share one provider symbol."
  ]

let handle_ownership_shape () =
  austral_raise DeclarationError [
    Text "A ";
    Code "Handle_Ownership";
    Text " import must have the dispose wrapper's signature: two ";
    Code "Int64";
    Text " words in (the handle's two words) and one ";
    Code "Int64";
    Text " status word out (0 done, non-zero refused). The wrappers are generated with exactly that ABI, so any other signature could not name a wrapper that exists."
  ]

let rust_ownership_shape ~ty =
  austral_raise DeclarationError [
    Text "The type ";
    Type ty;
    Text " is not part of the buffer protocol that may cross an extern rust boundary. Parameters and return values of `rs_*` imports must be scalar values (integers, floats, booleans) or addresses/pointers, because those are the shapes the Rust side's ABI speaks."
  ]

let rust_adapter_unsupported ~ty =
  austral_raise DeclarationError [
    Text "The type ";
    Type ty;
    Text " has no encoding at the extern rust boundary. An `rs_*` parameter or result is carried as a 64-bit word by a generated adapter (`<name>__adapter`), and that adapter ABI defines encodings for Boolean, Int32, Nat32, Int64, Nat64, Float32 and Float64 (plus addresses/pointers) only. An eight- or sixteen-bit integer, Index or ByteSize would arrive truncated, so it is refused here rather than as a call to an adapter that cannot exist."
  ]

let rust_adapter_buffer_unsupported ~subject ~ty =
  austral_raise DeclarationError [
    Text subject;
    Text " has type ";
    Type ty;
    Text ", which cannot cross the extern rust boundary in this signature: buffer transport versions 1 and 2 carry a Nat8 byte buffer as an ";
    Code "Address[Nat8]";
    Text " or ";
    Code "Pointer[Nat8]";
    Text " parameter with its length as the next parameter, version 2 also carries 64-bit integer words, and the result is a scalar. A buffer result, a span, or a pointer to anything but Nat8 bytes is a different protocol, which needs its own version and its own tests. Keep the result word-native instead (Int64/Nat64 — those calls name the declared symbol and build no adapter)."
  ]

let rust_adapter_buffer_needs_length ~name ~ty =
  austral_raise DeclarationError [
    Text "The parameter ";
    Code name;
    Text " has type ";
    Type ty;
    Text " and is a byte buffer, but the parameter after it is not its length. A buffer cannot cross the extern rust boundary without one: the length is the extent the wrapper (";
    Code "<name>__adapter";
    Text ") uses to build the slice, and it must be a ";
    Code "Nat64";
    Text " or ";
    Code "Int64";
    Text " parameter directly after the buffer."
  ]

let rust_adapter_buffer_scalar_beside ~name ~ty =
  austral_raise DeclarationError [
    Text "The parameter ";
    Code name;
    Text " has type ";
    Type ty;
    Text " in a signature that carries a byte buffer. Buffer transport version 2 describes a signature as Nat8 byte buffers, the 64-bit length after each one, 64-bit integer words, and a scalar result. A narrower integer, Bool, float or non-Nat8 pointer beside a buffer would need marshalling the descriptor has no per-parameter type for, so its wrapper cannot be generated."
  ]

let rust_adapter_buffer_length_types ~first ~second =
  austral_raise DeclarationError [
    Text "The buffer lengths in this signature have different types (";
    Type first;
    Text " and ";
    Type second;
    Text "). One descriptor states one ";
    Code "lengths";
    Text " type, so every length parameter must be the same 64-bit integer type."
  ]

let rust_interop_transport ~ty =
  austral_raise DeclarationError [
    Text "The type ";
    Type ty;
    Text " cannot cross the extern rust boundary under the baseline transport: the JIT passes host-native 64-bit words in and reads one Int64 out, so an ";
    Code "rs_";
    Text " parameter must be ";
    Code "Int64";
    Text " or a byte-buffer address/pointer, and the result must be ";
    Code "Int64";
    Text ". Floats travel in vector registers, and narrower integers would arrive as truncated words, so both need a typed adapter with tested marshal and range checking rather than a guessed word."
  ]

let fun_pointer_named_args () =
  austral_raise TypeError [
    Text "You can't call a function pointer with a named argument list, because function pointers don't preserve parameter names, so we wouldn't know how to assign arguments to parameters."
  ]

let fun_invalid_pragmas () =
  austral_raise DeclarationError [
    Text "Only import, export, Liquid contract, and Rust_Ownership pragmas are allowed inside functions."
  ]

let if_inequal ~lhs ~rhs =
  austral_raise TypeError [
    Text "The branches of the ";
    Code "if";
    Text "-expression have different types: ";
    Type lhs;
    Text " and ";
    Type rhs
  ]

let logical_operands_not_boolean ~operator ~types = match types with
| [ty] ->
    austral_raise TypeError [
      Text "The operand of a ";
      Text operator;
      Text " operator must be a boolean expression, but it has the type ";
      Type ty
    ]
| [first; second] ->
    austral_raise TypeError [
      Text "Both operands of a ";
      Text operator;
      Text " operator must be boolean expressions, but they have types ";
      Type first;
      Text " and ";
      Type second
    ]
  | _ ->
    austral_raise TypeError [
      Text "All operands of a logical operator must be boolean expressions."
    ]

let lvalue_index () =
  austral_raise TypeError [
    Text "The array index operator doesn't work in lvalues."
  ]

let no_such_slot ~type_name ~slot_name =
  austral_raise TypeError [
    Text "The type ";
    Code (ident_string type_name);
    Text " has no slot with name ";
    Code (ident_string slot_name)
  ]

let no_suitable_instance ~typeclass ~ty =
  austral_raise TypeError [
    Text "The typeclass ";
    Code (ident_string typeclass);
    Text " has no instance which matches for ";
    Type ty
  ]

let path_not_free ty =
  austral_raise TypeError [
    Text "The path ends with the type ";
    Type ty;
    Text " which is not in the ";
    Code "Free";
    Text " universe."
  ]

let path_not_public ~type_name ~slot_name =
  austral_raise TypeError [
    Text "The slot ";
    Code (ident_string slot_name);
    Text " is not publically visible in the type ";
    Code (ident_string type_name);
    Text " and so cannot be read."
  ]

let path_not_record ty =
  austral_raise TypeError [
    Text "Cannot take a path of the type ";
    Type ty;
    Text " because it is not a record."
  ]

let slot_wrong_type ~name ~expected ~actual =
  austral_raise TypeError [
    Text "The slot ";
    Code (ident_string name);
    Text " should have type ";
    Type expected;
    Text " but is declared to have type ";
    Type actual
  ]

let unconstrained_generic_function name =
  austral_raise TypeError [
    Code (ident_string name);
    Text " is generic but unconstrained, so its type is ambiguous.";
    Break;
    Text "Consider clarifying its type using a cast."
  ]

let unconstrained_type_parameter ~typeclass ~typaram =
  austral_raise TypeError [
    Text "The type parameter ";
    Code (ident_string typaram);
    Text " does not implement the type class ";
    Code (ident_string typeclass);
    Break;
    Text "Consider adding a constraint to the type parameter."
  ]

let unknown_name ~kind ~name =
  austral_raise GenericError [
    Text "I can't find a ";
    Text kind;
    Text " named ";
    Code (ident_string name);
    Text "."
  ]

let cannot_assign_to_constant _ =
  austral_raise GenericError [
      Text "Cannot assign a value to a constant."
    ]

let cannot_assign_to_parameter _ =
  austral_raise GenericError [
      Text "Cannot assign a value to a function parameter."
    ]

let cant_assign_to_immutable_var (name: identifier) =
  austral_raise GenericError [
      Text "Cannot assign a value to ";
      Code (ident_string name);
      Text " because it is immutable."
    ]

let ref_transform_needs_ref ty =
  austral_raise GenericError [
      Text "The head of a reference transform expression must be of a reference type, but I got ";
      Type ty
    ]

let ref_transform_not_record ty =
  austral_raise GenericError [
      Text "Tried to transform a reference to a record into a reference into a slot, but the type ";
      Type ty;
      Text " is not a record"
    ]

let var_collides_with_decl (name: identifier) =
  austral_raise GenericError [
      Text "The variable ";
      Code (ident_string name);
      Text " collides with the name of a declaration."
    ]

let cant_access_slot_not_record (slot_name: identifier) (ty: ty) =
  austral_raise TypeError [
      Text "Can't access the slot ";
      Code (ident_string slot_name);
      Text " because the type ";
      Type ty;
      Text " is not a record."
    ]

let cant_index_not_span (ty: ty) =
  austral_raise TypeError [
      Text "I can't use the array indexing operator here, because the type ";
      Type ty;
      Text " is not a span."
    ]
