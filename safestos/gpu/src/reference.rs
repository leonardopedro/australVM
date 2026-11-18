//! Bounded reference evaluation of validated KernelIR.
//!
//! This is the independent CPU oracle: it executes exactly the typed IR with
//! Rust `wrapping_*` u32 operations, so it does not share the OCaml or WGSL
//! evaluation paths. Instruction/call budgets bound runaway artifacts.

use crate::schema::{Expr, Helper, KernelIr, Stmt, ValueType};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    U32(u32),
    Bool(bool),
}

impl Value {
    fn as_u32(self) -> Result<u32, String> {
        match self {
            Value::U32(v) => Ok(v),
            Value::Bool(_) => Err("Boolean used where u32 was required".to_string()),
        }
    }
    fn as_bool(self) -> Result<bool, String> {
        match self {
            Value::Bool(v) => Ok(v),
            Value::U32(_) => Err("u32 used where Boolean was required".to_string()),
        }
    }
}

pub struct Limits {
    pub max_elements: usize,
    pub max_steps: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_elements: crate::schema::MAX_ELEMENTS as usize,
            max_steps: 200_000_000,
        }
    }
}

pub struct Evaluator<'a> {
    ir: &'a KernelIr,
    helpers: HashMap<u32, &'a Helper>,
    steps: u64,
    limits: Limits,
}

impl<'a> Evaluator<'a> {
    pub fn new(ir: &'a KernelIr, limits: Limits) -> Result<Self, String> {
        let mut helpers = HashMap::new();
        for h in &ir.helpers {
            if helpers.insert(h.id, h).is_some() {
                return Err(format!("duplicate helper id {}", h.id));
            }
        }
        Ok(Evaluator {
            ir,
            helpers,
            steps: 0,
            limits,
        })
    }

    fn tick(&mut self) -> Result<(), String> {
        self.steps += 1;
        if self.steps > self.limits.max_steps {
            return Err(format!(
                "reference evaluation exceeded the {} step budget",
                self.limits.max_steps
            ));
        }
        Ok(())
    }

    fn literal(s: &str) -> Result<u32, String> {
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("u32 literal {s:?} is not a decimal integer"));
        }
        s.parse::<u32>()
            .map_err(|_| format!("u32 literal {s:?} is out of range"))
    }

    pub fn eval_expr(
        &mut self,
        e: &Expr,
        params: &[Value],
        locals: &HashMap<u32, Value>,
        depth: u32,
    ) -> Result<Value, String> {
        self.tick()?;
        if depth > 128 {
            return Err("helper call depth exceeded 128".to_string());
        }
        Ok(match e {
            Expr::Param { index } => *params
                .get(*index as usize)
                .ok_or_else(|| format!("param {index} is not bound"))?,
            Expr::Local { index } => *locals
                .get(index)
                .ok_or_else(|| format!("local {index} is not bound"))?,
            Expr::U32 { value } => Value::U32(Self::literal(value)?),
            Expr::Bool { value } => Value::Bool(*value),
            Expr::Intrinsic { op, args } => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval_expr(a, params, locals, depth)?.as_u32()?);
                }
                let v = match (op.as_str(), vals.as_slice()) {
                    ("add", [a, b]) => a.wrapping_add(*b),
                    ("subtract", [a, b]) => a.wrapping_sub(*b),
                    ("multiply", [a, b]) => a.wrapping_mul(*b),
                    ("bitwiseAnd", [a, b]) => a & b,
                    ("bitwiseOr", [a, b]) => a | b,
                    ("bitwiseXor", [a, b]) => a ^ b,
                    ("bitwiseNot", [a]) => !a,
                    (other, _) => return Err(format!("unknown intrinsic {other:?}")),
                };
                Value::U32(v)
            }
            Expr::Compare { op, left, right } => {
                let a = self.eval_expr(left, params, locals, depth)?.as_u32()?;
                let b = self.eval_expr(right, params, locals, depth)?.as_u32()?;
                let v = match op.as_str() {
                    "eq" => a == b,
                    "ne" => a != b,
                    "lt" => a < b,
                    "le" => a <= b,
                    "gt" => a > b,
                    "ge" => a >= b,
                    other => return Err(format!("unknown comparison {other:?}")),
                };
                Value::Bool(v)
            }
            Expr::Boolean { op, left, right } => {
                let a = self.eval_expr(left, params, locals, depth)?.as_bool()?;
                let b = self.eval_expr(right, params, locals, depth)?.as_bool()?;
                let v = match op.as_str() {
                    "and" => a && b,
                    "or" => a || b,
                    other => return Err(format!("unknown boolean operation {other:?}")),
                };
                Value::Bool(v)
            }
            Expr::Not { value } => {
                Value::Bool(!self.eval_expr(value, params, locals, depth)?.as_bool()?)
            }
            Expr::Conditional { condition, then, r#else } => {
                let c = self
                    .eval_expr(condition, params, locals, depth)?
                    .as_bool()?;
                if c {
                    self.eval_expr(then, params, locals, depth)?
                } else {
                    self.eval_expr(r#else, params, locals, depth)?
                }
            }
            Expr::Call { helper, args } => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval_expr(a, params, locals, depth)?);
                }
                let h = *self
                    .helpers
                    .get(helper)
                    .ok_or_else(|| format!("call to undefined helper {helper}"))?;
                if vals.len() != h.params.len() {
                    return Err(format!(
                        "helper {helper} expected {} argument(s), got {}",
                        h.params.len(),
                        vals.len()
                    ));
                }
                for (v, ty) in vals.iter().zip(h.params.iter()) {
                    match (v, ty) {
                        (Value::U32(_), ValueType::U32) | (Value::Bool(_), ValueType::Bool) => {}
                        _ => {
                            return Err(format!(
                                "helper {helper} argument has the wrong type"
                            ))
                        }
                    }
                }
                self.exec_stmt(&h.body, &vals, depth + 1)?
                    .ok_or_else(|| format!("helper {helper} did not return"))?
            }
        })
    }

    pub fn exec_stmt(
        &mut self,
        s: &Stmt,
        params: &[Value],
        depth: u32,
    ) -> Result<Option<Value>, String> {
        let mut locals = HashMap::new();
        self.exec_stmt_in(s, params, &mut locals, depth)
    }

    fn exec_stmt_in(
        &mut self,
        s: &Stmt,
        params: &[Value],
        locals: &mut HashMap<u32, Value>,
        depth: u32,
    ) -> Result<Option<Value>, String> {
        self.tick()?;
        if depth > 128 {
            return Err("helper call depth exceeded 128".to_string());
        }
        match s {
            Stmt::Let { local, value } => {
                let v = self.eval_expr(value, params, locals, depth)?;
                if locals.insert(*local, v).is_some() {
                    return Err(format!("local {local} is assigned twice"));
                }
                Ok(None)
            }
            Stmt::Seq { items } => {
                for item in items {
                    if let Some(v) = self.exec_stmt_in(item, params, locals, depth)? {
                        return Ok(Some(v));
                    }
                }
                Ok(None)
            }
            Stmt::If { condition, then, r#else } => {
                let c = self
                    .eval_expr(condition, params, locals, depth)?
                    .as_bool()?;
                let mut branch = locals.clone();
                if c {
                    self.exec_stmt_in(then, params, &mut branch, depth)
                } else {
                    self.exec_stmt_in(r#else, params, &mut branch, depth)
                }
            }
            Stmt::Return { value } => Ok(Some(self.eval_expr(value, params, locals, depth)?)),
        }
    }

    pub fn run(&mut self, inputs: &[Vec<u32>]) -> Result<Vec<u32>, String> {
        let arity = self.ir.input_arity as usize;
        if inputs.len() != arity {
            return Err(format!(
                "expected {arity} input array(s), got {}",
                inputs.len()
            ));
        }
        let n = inputs.first().map(|v| v.len()).unwrap_or(0);
        if n > self.limits.max_elements {
            return Err(format!(
                "N={n} exceeds the {} element limit",
                self.limits.max_elements
            ));
        }
        for (i, input) in inputs.iter().enumerate() {
            if input.len() != n {
                return Err(format!(
                    "input {i} has length {}, expected {n}",
                    input.len()
                ));
            }
        }
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let params: Vec<Value> = inputs.iter().map(|v| Value::U32(v[i])).collect();
            let v = self
                .exec_stmt(&self.ir.body, &params, 0)?
                .ok_or_else(|| format!("kernel body returned no value at index {i}"))?;
            out.push(v.as_u32()?);
        }
        Ok(out)
    }
}
