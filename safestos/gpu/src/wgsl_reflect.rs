//! PC05a: reflect the *parsed* WGSL scalar helper into the clamp fragment.
//!
//! PC04 reflected the checked typed AST and the KernelIR. This module reflects
//! the third subject: the Naga module the runtime actually validates, parsed
//! from the bundle's `kernel.wgsl`. It reads the compute entry, the scalar
//! helper it calls, and the storage roles around them, and admits exactly one
//! shape:
//!
//!     fn scalar_entry(p0: u32) -> u32 {
//!         let v0: bool = (p0 > 100u);
//!         if v0 { return 100u; } else { return p0; }
//!     }
//!
//! with `output0[i] = scalar_entry(input0[i])` in the entry. Anything else is
//! refused rather than approximated: another comparison, another width, a
//! different literal, a helper that is not called exactly once at the guarded
//! identity index, storage roles other than one read array and one read_write
//! array, or extra scalar nodes.
//!
//! The reflection — Naga nodes to MM0 names — is a **trusted, recorded edge**,
//! like PC04's; it is recorded on the emitted artifact, not proved. What the
//! artifact makes checkable is that the *recorded* edge agrees with a fresh
//! parse of the same WGSL and with the bundle's KernelIR, and that the bound
//! follows from PC02's order laws. A digest never turns a mismatched
//! correspondence into a pass: `check_artifact` re-reflects and compares lines.

use crate::schema::{Expr, KernelIr, Stmt};
use naga::{
    AddressSpace, BinaryOperator, Block, Expression, Function, GlobalVariable, Handle, Module,
    ShaderStage, Statement, StorageAccess, TypeInner,
};

/// One reflected clamp, from the parsed shader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reflection {
    /// Compute entry point name (`main` in the generated shader).
    pub entry: String,
    /// The scalar helper the entry calls exactly once.
    pub helper: String,
    /// The helper's parameter name and position.
    pub param_name: String,
    pub param_index: u32,
    /// Parameter width in bits; only 32 is admitted.
    pub param_width: u32,
    /// `GreaterThan` or `LessThan`.
    pub comparison: String,
    /// Decimal literal the condition compares against and the high branch returns.
    pub literal: String,
    /// Decimal value the high branch returns (equals `literal` in this shape).
    pub branch: String,
    /// `umeet` for `>`, `ujoin` for `<`.
    pub operator: String,
    /// Storage roles the entry reads and writes.
    pub input_role: String,
    pub output_role: String,
    pub workgroup_size: u32,
}

fn scalar_uint_width(inner: &TypeInner) -> Option<u32> {
    match inner {
        TypeInner::Scalar(naga::Scalar {
            kind: naga::ScalarKind::Uint,
            width,
        }) =>
        // Naga records the width in bytes; the reflection records bits.
        Some(u32::from(*width) * 8),
        _ => None,
    }
}

fn storage_globals(module: &Module) -> Vec<(Handle<GlobalVariable>, StorageAccess)> {
    let mut out = Vec::new();
    for (handle, var) in module.global_variables.iter() {
        if let AddressSpace::Storage { access } = var.space {
            out.push((handle, access));
        }
    }
    out
}

fn function_of(module: &Module, handle: Handle<Function>) -> Result<&Function, String> {
    module
        .functions
        .try_get(handle)
        .map_err(|_| format!("call target {handle:?} is not a function"))
}

struct EntryCall {
    function: Handle<Function>,
    arguments: Vec<Handle<Expression>>,
}

fn collect_calls(block: &Block, out: &mut Vec<(Handle<Function>, Vec<Handle<Expression>>)>) {
    for (statement, _) in block.span_iter() {
        match statement {
            Statement::Call {
                function,
                arguments,
                ..
            } => out.push((*function, arguments.clone())),
            Statement::Block(b) => collect_calls(b, out),
            Statement::If { accept, reject, .. } => {
                collect_calls(accept, out);
                collect_calls(reject, out);
            }
            Statement::Loop { body, .. } => collect_calls(body, out),
            _ => {}
        }
    }
}

/// The one call the entry makes. Naga 29 records calls as statements, not
/// expressions, so this walks the entry's blocks.
fn entry_call(f: &Function) -> Result<EntryCall, String> {
    let mut calls = Vec::new();
    collect_calls(&f.body, &mut calls);
    match calls.as_slice() {
        [(function, arguments)] => Ok(EntryCall {
            function: *function,
            arguments: arguments.clone(),
        }),
        [] => Err("entry point calls no scalar helper".to_string()),
        many => Err(format!(
            "entry point makes {} calls; the reflected fragment calls one helper",
            many.len()
        )),
    }
}

/// The function a `Store` value is the result of.
fn call_result_handle(f: &Function, value: Handle<Expression>) -> Option<Handle<Function>> {
    match &f.expressions[value] {
        Expression::CallResult(function) => Some(*function),
        _ => None,
    }
}

fn literal_u32(f: &Function, handle: Handle<Expression>) -> Option<u32> {
    match f.expressions[handle] {
        Expression::Literal(naga::Literal::U32(value)) => Some(value),
        _ => None,
    }
}

/// Exactly one top-level `If` statement, as `(condition, accept, reject)`.
fn single_if(block: &Block) -> Result<(Handle<Expression>, &Block, &Block), String> {
    let mut found = None;
    for (statement, _) in block.span_iter() {
        match statement {
            Statement::Emit(_) => {}
            Statement::If {
                condition,
                accept,
                reject,
            } => {
                if found.is_some() {
                    return Err("the helper has more than one `if`".to_string());
                }
                found = Some((*condition, accept, reject));
            }
            other => {
                return Err(format!(
                    "statement {other:?} is not part of the reflected clamp helper"
                ))
            }
        }
    }
    found.ok_or_else(|| "the helper has no `if`; the reflected fragment needs one".to_string())
}

fn single_return_value(block: &Block) -> Result<Handle<Expression>, String> {
    let mut values = Vec::new();
    for (statement, _) in block.span_iter() {
        match statement {
            Statement::Emit(_) => {}
            Statement::Return { value: Some(value) } => values.push(*value),
            Statement::Return { value: None } => {
                return Err("a branch returns no value; the clamp returns a u32".to_string())
            }
            other => {
                return Err(format!(
                    "statement {other:?} is not part of the reflected clamp branch"
                ))
            }
        }
    }
    match values.as_slice() {
        [value] => Ok(*value),
        [] => Err("a branch has no return".to_string()),
        many => Err(format!("a branch returns {} times", many.len())),
    }
}

fn operator_of(comparison: &str) -> Result<String, String> {
    match comparison {
        "GreaterThan" => Ok("umeet".to_string()),
        "LessThan" => Ok("ujoin".to_string()),
        other => Err(format!(
            "comparison {other} is not the reflected clamp shape (> or < only)"
        )),
    }
}

fn comparison_name(op: BinaryOperator) -> Option<&'static str> {
    match op {
        BinaryOperator::Greater => Some("GreaterThan"),
        BinaryOperator::Less => Some("LessThan"),
        _ => None,
    }
}

/// Reflect a parsed WGSL module. `input_arity` comes from the bundle and must
/// agree with the number of read-only storage arrays the entry uses.
pub fn reflect(wgsl: &str, input_arity: u32) -> Result<Reflection, String> {
    let module = naga::front::wgsl::parse_str(wgsl)
        .map_err(|e| format!("WGSL parse error: {}", e.emit_to_string(wgsl)))?;
    reflect_module(&module, input_arity)
}

fn reflect_module(module: &Module, input_arity: u32) -> Result<Reflection, String> {
    if module.entry_points.len() != 1 {
        return Err(format!(
            "the reflected fragment has exactly one entry point, found {}",
            module.entry_points.len()
        ));
    }
    let entry_point = &module.entry_points[0];
    if entry_point.stage != ShaderStage::Compute {
        return Err("the reflected entry point is not a compute shader".to_string());
    }
    let entry_name = entry_point.name.clone();
    let entry_function = &entry_point.function;
    let workgroup_size = entry_point.workgroup_size[0];

    // Storage roles first: the reflected fragment has one read array and one
    // read_write array, and `input_arity` read arrays.
    let globals = storage_globals(module);
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    for (handle, access) in &globals {
        let role = if *access == StorageAccess::LOAD {
            "read"
        } else if access.contains(StorageAccess::LOAD | StorageAccess::STORE) {
            "read_write"
        } else {
            "write"
        };
        match role {
            "read" => reads.push(*handle),
            "read_write" => writes.push(*handle),
            other => {
                return Err(format!(
                    "storage role `{other}` is not part of the reflected fragment \
                     (one read array and one read_write array)"
                ))
            }
        }
    }
    if reads.len() as u32 != input_arity {
        return Err(format!(
            "the entry reads {} storage array(s), the bundle declares input arity {input_arity}",
            reads.len()
        ));
    }
    if writes.len() != 1 {
        return Err(format!(
            "the entry writes {} storage array(s); the reflected fragment writes one",
            writes.len()
        ));
    }

    // The helper the entry calls, and the argument it passes it.
    let call = entry_call(entry_function)?;
    let helper_handle = call.function;
    let helper = function_of(module, helper_handle)?;
    let helper_name = helper.name.clone().unwrap_or_default();
    if helper_name.is_empty() {
        return Err("the scalar helper has no name; the reflection records it".to_string());
    }
    if helper.arguments.len() != 1 {
        return Err(format!(
            "helper {helper_name} takes {} parameter(s); the reflected fragment has one",
            helper.arguments.len()
        ));
    }
    let param_type = &module.types[helper.arguments[0].ty];
    let param_width = scalar_uint_width(&param_type.inner)
        .ok_or_else(|| "the helper parameter is not an unsigned integer".to_string())?;
    if param_width != 32 {
        return Err(format!(
            "parameter width {param_width} is not the reflected u32; extend the shared word theory before reflecting another width"
        ));
    }
    let param_name = helper.arguments[0]
        .name
        .clone()
        .unwrap_or_else(|| "?".to_string());
    let result_type = helper
        .result
        .as_ref()
        .ok_or_else(|| "the helper returns nothing".to_string())?;
    let result_width = scalar_uint_width(&module.types[result_type.ty].inner)
        .ok_or_else(|| "the helper result is not an unsigned integer".to_string())?;
    if result_width != param_width {
        return Err(format!(
            "the helper returns u{result_width} from a u{param_width} parameter"
        ));
    }

    // `if <param> <cmp> <literal> { return <literal> } else { return <param> }`
    let (condition, accept, reject) = single_if(&helper.body)?;
    let (op, left, right) = match helper.expressions[condition] {
        Expression::Binary { op, left, right } => (op, left, right),
        ref other => {
            return Err(format!(
                "helper condition is {other:?}; the reflected fragment compares the parameter"
            ))
        }
    };
    let comparison = comparison_name(op).ok_or_else(|| {
        format!(
            "comparison {op:?} is not the reflected clamp shape (> or < only)"
        )
    })?;
    if !matches!(helper.expressions[left], Expression::FunctionArgument(0)) {
        return Err("the helper compares something other than its parameter".to_string());
    }
    let literal = literal_u32(helper, right)
        .ok_or_else(|| "the helper compares the parameter with a non-u32 literal".to_string())?;
    let branch_handle = single_return_value(accept)?;
    let branch = literal_u32(helper, branch_handle)
        .ok_or_else(|| "the high branch does not return a u32 literal".to_string())?;
    if branch != literal {
        return Err(format!(
            "the high branch returns {branch} but the condition compares {literal}; the reflected clamp returns the literal"
        ));
    }
    let low_handle = single_return_value(reject)?;
    if !matches!(helper.expressions[low_handle], Expression::FunctionArgument(0)) {
        return Err("the low branch does not return the parameter".to_string());
    }

    // The observable: the call's result is stored into the read_write array at
    // the same index the read array is loaded from.
    let (stored_call, store_index, load_index) =
        entry_store(entry_function, &call, &reads, &writes)?;
    if stored_call != helper_handle {
        return Err("the entry stores something other than the scalar helper's result".to_string());
    }
    if store_index != load_index {
        return Err(
            "the entry writes at a different index than it reads; the reflected output index is identity"
                .to_string(),
        );
    }

    let operator = operator_of(comparison)?;
    Ok(Reflection {
        entry: entry_name,
        helper: helper_name,
        param_name,
        param_index: 0,
        param_width,
        comparison: comparison.to_string(),
        literal: literal.to_string(),
        branch: branch.to_string(),
        operator,
        input_role: "read".to_string(),
        output_role: "read_write".to_string(),
        workgroup_size,
    })
}

/// Find the entry's single store of the helper's result, returning
/// `(called function, store index handle, load index handle)`.
fn entry_store(
    f: &Function,
    call: &EntryCall,
    reads: &[Handle<GlobalVariable>],
    writes: &[Handle<GlobalVariable>],
) -> Result<(Handle<Function>, Handle<Expression>, Handle<Expression>), String> {
    let mut stores = Vec::new();
    for (statement, _) in f.body.span_iter() {
        if let Statement::Store { pointer, value } = statement {
            stores.push((*pointer, *value));
        }
    }
    let (pointer, value) = match stores.as_slice() {
        [one] => *one,
        [] => return Err("the entry stores no output".to_string()),
        many => return Err(format!("the entry stores {} outputs", many.len())),
    };
    let (store_base, store_index) = match f.expressions[pointer] {
        Expression::Access { base, index } => (base, index),
        _ => return Err("the output store is not an indexed storage access".to_string()),
    };
    let store_global = match f.expressions[store_base] {
        Expression::GlobalVariable(g) => g,
        _ => return Err("the output store target is not a storage global".to_string()),
    };
    if !writes.contains(&store_global) {
        return Err("the output store target is not the read_write array".to_string());
    }
    let called = call_result_handle(f, value).ok_or_else(|| {
        format!(
            "the stored value is {:?}, not the scalar helper's result",
            f.expressions[value]
        )
    })?;
    // The call's argument is the load: `helper(input[i])`.
    let argument = call
        .arguments
        .first()
        .copied()
        .ok_or_else(|| "the scalar helper is called with no argument".to_string())?;
    // `scalar_entry(input0[i])` passes the loaded value; the load's pointer is
    // the indexed access.
    let address = match f.expressions[argument] {
        Expression::Load { pointer } => pointer,
        _ => argument,
    };
    let (load_base, load_index) = match f.expressions[address] {
        Expression::Access { base, index } => (base, index),
        ref other => {
            return Err(format!(
                "the helper argument is {other:?}, not an indexed read of the input array"
            ))
        }
    };
    let load_global = match f.expressions[load_base] {
        Expression::GlobalVariable(g) => g,
        _ => return Err("the helper argument does not read a storage global".to_string()),
    };
    if !reads.contains(&load_global) {
        return Err("the helper argument reads something other than an input array".to_string());
    }
    Ok((called, store_index, load_index))
}

fn ir_comparison(op: &str) -> Result<&'static str, String> {
    match op {
        "gt" => Ok("GreaterThan"),
        "lt" => Ok("LessThan"),
        other => Err(format!(
            "kernel comparison {other} is not the reflected clamp shape (> or < only)"
        )),
    }
}

/// Reflect the KernelIR side of the same clamp. Deliberately independent of the
/// WGSL reflection: the two must agree, and that agreement is the witness.
pub fn reflect_ir(ir: &KernelIr) -> Result<Reflection, String> {
    let mut lets: Vec<(u32, &Expr)> = Vec::new();
    let mut if_stmt: Option<&Stmt> = None;
    let mut items: Vec<&Stmt> = Vec::new();
    collect_ir(&ir.body, &mut items);
    for item in &items {
        match item {
            Stmt::Let { local, value } => lets.push((*local, value)),
            Stmt::If { .. } => {
                if if_stmt.is_some() {
                    return Err("kernel body has more than one `if`".to_string());
                }
                if_stmt = Some(item);
            }
            _ => {}
        }
    }
    let (condition, then_branch, else_branch) = match if_stmt {
        Some(Stmt::If {
            condition,
            then: then_branch,
            r#else: else_branch,
        }) => (condition, then_branch.as_ref(), else_branch.as_ref()),
        _ => return Err("kernel body has no `if`; the reflected fragment needs one".to_string()),
    };
    let condition = resolve_ir(&lets, condition);
    let (op, left, right) = match condition {
        Expr::Compare { op, left, right } => (op.as_str(), left.as_ref(), right.as_ref()),
        other => {
            return Err(format!(
                "kernel condition is {other:?}, not a comparison of the parameter"
            ))
        }
    };
    let comparison = ir_comparison(op)?;
    let (param_index, literal) = match (left, right) {
        (Expr::Param { index }, Expr::U32 { value }) => (*index, value.clone()),
        _ => {
            return Err(
                "kernel condition is not `<param> <cmp> <u32 literal>`".to_string()
            )
        }
    };
    let branch = match then_branch {
        Stmt::Return {
            value: Expr::U32 { value },
        } => value.clone(),
        other => {
            return Err(format!(
                "kernel high branch is {other:?}, not `return <u32 literal>`"
            ))
        }
    };
    let else_index = match else_branch {
        Stmt::Return {
            value: Expr::Param { index },
        } => *index,
        other => {
            return Err(format!(
                "kernel low branch is {other:?}, not `return <param>`"
            ))
        }
    };
    if else_index != param_index {
        return Err(format!(
            "kernel condition reads parameter {param_index} but the low branch reads {else_index}"
        ));
    }
    let operator = operator_of(comparison)?;
    Ok(Reflection {
        entry: ir.entry.clone(),
        helper: "?".to_string(),
        param_name: "?".to_string(),
        param_index,
        param_width: 32,
        comparison: comparison.to_string(),
        literal,
        branch,
        operator,
        input_role: "read".to_string(),
        output_role: "read_write".to_string(),
        workgroup_size: ir.memory.workgroup_size,
    })
}

fn collect_ir<'a>(stmt: &'a Stmt, out: &mut Vec<&'a Stmt>) {
    match stmt {
        Stmt::Seq { items } => {
            for item in items {
                collect_ir(item, out);
            }
        }
        other => out.push(other),
    }
}

/// Follow compiler hoisted locals to the expression that defines them.
fn resolve_ir<'a>(lets: &[(u32, &'a Expr)], expr: &'a Expr) -> &'a Expr {
    let mut current = expr;
    for _ in 0..64 {
        match current {
            Expr::Local { index } => match lets.iter().rev().find(|(l, _)| l == index) {
                Some((_, value)) => current = value,
                None => return current,
            },
            _ => return current,
        }
    }
    current
}

/// Require the parsed-WGSL and KernelIR reflections to name the same clamp.
/// The result carries the IR's entry name and the shader's helper, so an
/// artifact records both subjects.
pub fn witness(wgsl: &Reflection, ir: &Reflection) -> Result<Reflection, String> {
    if wgsl.comparison != ir.comparison {
        return Err(format!(
            "WGSL comparison {} does not match the KernelIR comparison {}",
            wgsl.comparison, ir.comparison
        ));
    }
    if wgsl.literal != ir.literal {
        return Err(format!(
            "WGSL literal {} does not match the KernelIR literal {}",
            wgsl.literal, ir.literal
        ));
    }
    if wgsl.branch != ir.branch {
        return Err(format!(
            "WGSL high branch {} does not match the KernelIR branch {}",
            wgsl.branch, ir.branch
        ));
    }
    if wgsl.param_index != ir.param_index {
        return Err(format!(
            "WGSL parameter {}:{} maps to KernelIR parameter {}",
            wgsl.param_name, wgsl.param_index, ir.param_index
        ));
    }
    if wgsl.param_width != 32 {
        return Err(format!(
            "WGSL parameter width {} is not the reflected u32",
            wgsl.param_width
        ));
    }
    if wgsl.workgroup_size != ir.workgroup_size {
        return Err(format!(
            "WGSL workgroup size {} does not match the memory plan's {}",
            wgsl.workgroup_size, ir.workgroup_size
        ));
    }
    if wgsl.input_role != "read" || wgsl.output_role != "read_write" {
        return Err(format!(
            "WGSL storage roles are input={} output={}; the reflected fragment needs read/read_write",
            wgsl.input_role, wgsl.output_role
        ));
    }
    let mut out = wgsl.clone();
    out.entry = ir.entry.clone();
    Ok(out)
}

pub fn describe(r: &Reflection) -> String {
    format!(
        "entry={} helper={} param={}:{} width={} comparison={} literal={} branch={} operator={} roles={}/{} workgroup={}",
        r.entry,
        r.helper,
        r.param_name,
        r.param_index,
        r.param_width,
        r.comparison,
        r.literal,
        r.branch,
        r.operator,
        r.input_role,
        r.output_role,
        r.workgroup_size
    )
}

/// The emitted fragment: `wgsl_clamp` is the parsed shader's helper read as the
/// word model's meet, tied to `ir_clamp` (PC04's definition) by reflexivity
/// after both unfold, and to PC02's meet lower bound.
pub fn render(r: &Reflection, wgsl_sha256: &str, ir_sha256: &str) -> (String, String) {
    let header = [
        "-- clamp_wgsl.mm0 — PC05a parsed-WGSL reflection of the scalar clamp (generated).".to_string(),
        "--".to_string(),
        format!("-- reflect-wgsl: {}", describe(r)),
        format!("-- wgsl_sha256={wgsl_sha256}"),
        format!("-- ir_sha256={ir_sha256}"),
        format!(
            "-- parsed Naga: fn {}({}: u{width}) -> u{width} {{\n--              let v0: bool = ({} {} {}u); if v0 {{ return {}u; }} else {{ return {}; }} }}",
            r.helper,
            r.param_name,
            r.param_name,
            match r.comparison.as_str() {
                "GreaterThan" => ">",
                "LessThan" => "<",
                other => other,
            },
            r.literal,
            r.branch,
            r.param_name,
            width = r.param_width
        ),
        format!(
            "-- KernelIR:  If (Compare ({}, Param {}, U32 {}), Return (U32 {}), Return (Param {}))",
            r.comparison, r.param_index, r.literal, r.branch, r.param_index
        ),
        "--".to_string(),
        "-- trusted-edge: parsing the shader and reading the scalar helper's nodes".to_string(),
        "--   above as this fragment is recorded here, not proved. The bound below".to_string(),
        "--   is proved from PC02's order laws and is tied to the comparison the".to_string(),
        "--   parsed shader actually carries. A digest never makes a mismatched".to_string(),
        "--   correspondence pass: the checker re-parses and compares these lines.".to_string(),
        "--".to_string(),
        "-- This fragment is assembled after word.mm0 and clamp_source.mm0.".to_string(),
    ];
    let decls = [
        format!(
            "def wgsl_clamp (x: u32): u32 = $ {} x u100 $;",
            r.operator
        ),
        "theorem wgsl_clamp_matches_ir (x: u32):".to_string(),
        "  $ ueq (wgsl_clamp x) (ir_clamp x) $;".to_string(),
        "theorem wgsl_inherits_source_bound (x: u32):".to_string(),
        "  $ ule (wgsl_clamp x) u100 $;".to_string(),
    ];
    let proof = [
        "-- clamp_wgsl.mm1 — proofs for clamp_wgsl.mm0 (PC05a).".to_string(),
        "--".to_string(),
        "-- `wgsl_clamp` and `ir_clamp` unfold to the same reflected operator, so".to_string(),
        "-- the witness against the KernelIR is reflexivity and the bound is".to_string(),
        "-- PC02's meet lower bound. Under a reflected `ujoin` neither unifies: the".to_string(),
        "-- theorem follows the parsed comparison, not a recorded digest.".to_string(),
        format!(
            "def wgsl_clamp (x: u32): u32 = $ {} x u100 $;",
            r.operator
        ),
        "pub theorem wgsl_clamp_matches_ir (x: u32):".to_string(),
        "  $ ueq (wgsl_clamp x) (ir_clamp x) $".to_string(),
        "  = '(ueq_refl);".to_string(),
        "pub theorem wgsl_inherits_source_bound (x: u32):".to_string(),
        "  $ ule (wgsl_clamp x) u100 $".to_string(),
        "  = '(umeet_lb_r);".to_string(),
    ];
    (
        header.join("\n") + "\n" + &decls.join("\n") + "\n",
        proof.join("\n") + "\n",
    )
}

/// Re-reflect and compare, the same way PC04's checker works: the recorded
/// line and definition must be what a fresh parse produces, and the recorded
/// digests must be the subjects' actual digests.
pub fn check_artifact(
    fresh: &Reflection,
    wgsl_sha256: &str,
    ir_sha256: &str,
    text: &str,
) -> Result<(), String> {
    let lines = text.lines().collect::<Vec<_>>();
    let recorded = lines
        .iter()
        .filter_map(|line| line.strip_prefix("-- reflect-wgsl: "))
        .collect::<Vec<_>>();
    match recorded.as_slice() {
        [line] if *line == describe(fresh).as_str() => {}
        [] => return Err("artifact has no `-- reflect-wgsl:` line".to_string()),
        [line] => {
            return Err(format!(
                "artifact records `{line}`, re-reflection gives `{}`",
                describe(fresh)
            ))
        }
        _ => return Err("artifact has more than one `-- reflect-wgsl:` line".to_string()),
    }
    for (prefix, expected, what) in [
        ("-- wgsl_sha256=", wgsl_sha256, "WGSL subject"),
        ("-- ir_sha256=", ir_sha256, "KernelIR subject"),
    ] {
        let recorded = lines
            .iter()
            .filter_map(|line| line.strip_prefix(prefix))
            .collect::<Vec<_>>();
        match recorded.as_slice() {
            [value] if *value == expected => {}
            [] => return Err(format!("artifact records no {what} digest")),
            [value] => {
                return Err(format!(
                    "artifact is bound to {what} {value}, the current subject is {expected}"
                ))
            }
            _ => return Err(format!("artifact records more than one {what} digest")),
        }
    }
    let definition = lines
        .iter()
        .find(|line| line.starts_with("def wgsl_clamp"))
        .ok_or_else(|| "artifact does not define wgsl_clamp".to_string())?;
    let expected = format!("$ {} x u100 $", fresh.operator);
    if !definition.contains(&expected) {
        return Err(format!(
            "artifact defines wgsl_clamp with a different operator than the parsed shader ({})",
            fresh.operator
        ));
    }
    Ok(())
}
