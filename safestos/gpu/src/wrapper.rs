//! Fixed-pattern audit of the generated memory wrapper in parsed Naga IR.
//!
//! This is deliberately narrow. It checks the exact v1 wrapper shape:
//! one compute entry at workgroup size 64, storage bindings 0..arity-1 read-only
//! and binding arity read_write, an identity invocation index sourced from
//! `global_invocation_id.x`, one dominating bounds guard against the first
//! input's length, and exactly one output store at that same index. Scalar
//! helpers may not touch storage globals. Loops, switches, barriers, atomics
//! and image stores are refused.
//!
//! It is not a general shader verifier and does not prove that the scalar
//! computation implements KernelIR.

use naga::{
    AddressSpace, BinaryOperator, Block, BuiltIn, Expression, Function, GlobalVariable, Handle,
    Module, ShaderStage, Statement, StorageAccess,
};

fn as_global(f: &Function, h: Handle<Expression>) -> Option<Handle<GlobalVariable>> {
    match f.expressions[h] {
        Expression::GlobalVariable(g) => Some(g),
        _ => None,
    }
}

fn block_contains_return(block: &Block) -> bool {
    block.span_iter().any(|(s, _)| match s {
        Statement::Return { .. } => true,
        Statement::Block(b) => block_contains_return(b),
        _ => false,
    })
}

fn is_comparison(op: BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Less
            | BinaryOperator::LessEqual
            | BinaryOperator::Greater
            | BinaryOperator::GreaterEqual
    )
}

/// Is this expression exactly `global_invocation_id.x`? A `var` holding a
/// derived index is a `LocalVariable` and must not pass: provenance is the
/// expression itself, not a name that happens to be bound.
fn is_invocation_x(f: &Function, h: Handle<Expression>) -> bool {
    match f.expressions[h] {
        Expression::Access { base, index } => match f.expressions[base] {
            Expression::FunctionArgument(0) => matches!(f.expressions[index], Expression::Literal(_)),
            _ => false,
        },
        Expression::AccessIndex { base, index } => {
            matches!(f.expressions[base], Expression::FunctionArgument(0)) && index == 0
        }
        _ => false,
    }
}

/// Recognise `index <op> arrayLength(&input)` (either operand order).
fn guard_of(
    f: &Function,
    inputs: &[Handle<GlobalVariable>],
    condition: Handle<Expression>,
) -> Option<(Handle<Expression>, Handle<GlobalVariable>)> {
    let e = &f.expressions[condition];
    let (op, left, right) = match e {
        Expression::Binary { op, left, right } => (*op, *left, *right),
        _ => return None,
    };
    if !is_comparison(op) {
        return None;
    }
    for (a, b) in [(left, right), (right, left)] {
        let g = match f.expressions[a] {
            Expression::ArrayLength(base) => match f.expressions[base] {
                Expression::GlobalVariable(g) => g,
                _ => continue,
            },
            _ => continue,
        };
        if !inputs.contains(&g) {
            continue;
        }
        if is_invocation_x(f, b) {
            return Some((b, g));
        }
    }
    None
}

struct Audit {
    output: Handle<GlobalVariable>,
    inputs: Vec<Handle<GlobalVariable>>,
    guard: Option<(Handle<Expression>, Handle<GlobalVariable>)>,
    stores: Vec<(Handle<GlobalVariable>, Handle<Expression>)>,
    reads: Vec<(Handle<GlobalVariable>, Handle<Expression>)>,
}

impl Audit {
    fn record_reads(&mut self, f: &Function) {
        for (_h, expr) in f.expressions.iter() {
            if let Expression::Access { base, index } = expr {
                if let Some(g) = as_global(f, *base) {
                    if g != self.output && self.inputs.contains(&g) {
                        self.reads.push((g, *index));
                    }
                }
            }
        }
    }

    fn walk_block(&mut self, f: &Function, block: &Block) -> Result<(), String> {
        for (stmt, _) in block.span_iter() {
            match stmt {
                Statement::Emit(_) => {}
                Statement::Block(b) => self.walk_block(f, b)?,
                Statement::If {
                    condition,
                    accept,
                    reject,
                } => {
                    if let Some(candidate) = guard_of(f, &self.inputs, *condition) {
                        if block_contains_return(accept) {
                            self.guard.get_or_insert(candidate);
                        }
                    }
                    self.walk_block(f, accept)?;
                    self.walk_block(f, reject)?;
                }
                Statement::Loop { .. } => {
                    return Err("loop statements are not part of the v1 wrapper".to_string())
                }
                Statement::Switch { .. } => {
                    return Err("switch statements are not part of the v1 wrapper".to_string())
                }
                Statement::Break | Statement::Continue => {
                    return Err("loop control is not part of the v1 wrapper".to_string())
                }
                Statement::Kill => {
                    return Err("kill is not part of the v1 wrapper".to_string())
                }
                Statement::ControlBarrier(_) | Statement::MemoryBarrier(_) => {
                    return Err("barriers are not part of the v1 wrapper".to_string())
                }
                Statement::Atomic { .. } => {
                    return Err("atomics are not part of the v1 wrapper".to_string())
                }
                Statement::ImageStore { .. } => {
                    return Err("image stores are not part of the v1 wrapper".to_string())
                }
                Statement::Store { pointer, .. } => {
                    let (base, index) = match f.expressions[*pointer] {
                        Expression::Access { base, index } => (base, index),
                        _ => {
                            return Err(
                                "output store is not an indexed storage access".to_string()
                            )
                        }
                    };
                    let g = as_global(f, base)
                        .ok_or_else(|| "store target is not a storage global".to_string())?;
                    if !is_invocation_x(f, index) {
                        return Err(format!(
                            "output store index is not global_invocation_id.x: {:?}",
                            f.expressions[index]
                        ));
                    }
                    self.stores.push((g, index));
                }
                Statement::Call { .. } => {
                    // The scalar entry call. Called functions are separately
                    // checked: none may reference storage globals.
                }
                Statement::Return { .. } => {
                    // The guard's early return is part of the wrapper. Entry
                    // points return no value, so this cannot fabricate one.
                }
                Statement::WorkGroupUniformLoad { .. } => {
                    return Err("workgroup loads are not part of the v1 wrapper".to_string())
                }
                other => {
                    return Err(format!(
                        "statement {other:?} is not part of the v1 wrapper"
                    ))
                }
            }
        }
        Ok(())
    }
}

pub fn audit(module: &Module, input_arity: u32) -> Result<(), String> {
    if module.entry_points.len() != 1 {
        return Err(format!(
            "v1 requires exactly one entry point, found {}",
            module.entry_points.len()
        ));
    }
    let ep = &module.entry_points[0];
    if ep.name != "main" {
        return Err(format!("entry point is {:?}, expected \"main\"", ep.name));
    }
    if ep.stage != ShaderStage::Compute {
        return Err("entry point is not a compute shader".to_string());
    }
    if ep.workgroup_size != [64, 1, 1] {
        return Err(format!(
            "workgroup size is {:?}, expected [64,1,1]",
            ep.workgroup_size
        ));
    }
    if ep.function.arguments.len() != 1 {
        return Err("wrapper entry must take exactly the invocation id".to_string());
    }
    match &ep.function.arguments[0].binding {
        Some(naga::Binding::BuiltIn(BuiltIn::GlobalInvocationId)) => {}
        _ => return Err("wrapper entry argument is not global_invocation_id".to_string()),
    }

    let mut inputs = Vec::new();
    let mut output = None;
    let mut globals = 0;
    for (h, var) in module.global_variables.iter() {
        if var.space == AddressSpace::Function || var.space == AddressSpace::Private {
            continue;
        }
        globals += 1;
        let access = match var.space {
            AddressSpace::Storage { access } => access,
            _ => return Err("wrapper may only declare storage globals".to_string()),
        };
        let binding = var
            .binding
            .ok_or_else(|| "storage global has no binding".to_string())?;
        if binding.group != 0 {
            return Err("storage globals must be in bind group 0".to_string());
        }
        if binding.binding < input_arity {
            if !access.contains(StorageAccess::LOAD) || access.contains(StorageAccess::STORE) {
                return Err(format!("input binding {} is not read-only", binding.binding));
            }
            inputs.push(h);
        } else if binding.binding == input_arity {
            if !access.contains(StorageAccess::STORE) {
                return Err("output binding is not writable".to_string());
            }
            if output.is_some() {
                return Err("more than one output binding".to_string());
            }
            output = Some(h);
        } else {
            return Err(format!(
                "unexpected storage binding {} for arity {input_arity}",
                binding.binding
            ));
        }
    }
    if globals != input_arity as usize + 1 {
        return Err(format!(
            "expected {} storage globals, found {globals}",
            input_arity + 1
        ));
    }
    let output = output.ok_or_else(|| "no output storage binding".to_string())?;

    for (_h, f) in module.functions.iter() {
        for (_e, expr) in f.expressions.iter() {
            match expr {
                Expression::GlobalVariable(_) | Expression::ArrayLength(_) => {
                    return Err("a scalar helper accesses a storage global".to_string())
                }
                _ => {}
            }
        }
    }

    let mut a = Audit {
        output,
        inputs: inputs.clone(),
        guard: None,
        stores: Vec::new(),
        reads: Vec::new(),
    };
    a.record_reads(&ep.function);
    a.walk_block(&ep.function, &ep.function.body)?;

    let (_guard_local, guard_input) = a
        .guard
        .ok_or_else(|| "no dominating bounds guard over the first input".to_string())?;
    if let Some(expected) = inputs.first() {
        if guard_input != *expected {
            return Err("bounds guard does not compare the first input's length".to_string());
        }
    }
    if a.stores.len() != 1 {
        return Err(format!(
            "expected exactly one output store, found {}",
            a.stores.len()
        ));
    }
    let (store_target, store_index) = a.stores[0];
    if store_target != output {
        return Err("the only store does not target the output binding".to_string());
    }
    if !is_invocation_x(&ep.function, store_index) {
        return Err("output index is not the invocation index used by the guard".to_string());
    }
    for (g, l) in &a.reads {
        if !a.inputs.contains(g) {
            return Err("a read targets a non-input storage binding".to_string());
        }
        if !is_invocation_x(&ep.function, *l) {
            return Err("input read index differs from the invocation index".to_string());
        }
    }
    for (i, g) in inputs.iter().enumerate() {
        if !a.reads.iter().any(|(rg, _)| rg == g) {
            return Err(format!("input {i} is never read by the wrapper"));
        }
    }
    Ok(())
}
