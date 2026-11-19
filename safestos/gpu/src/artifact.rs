//! Bounded bundle loading, strict schema parsing and digest reproduction.

use crate::schema::{KernelBundle, MAX_ELEMENTS, SCHEMA, WORKGROUP_SIZE};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

pub const MAX_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_INPUT_JSON_BYTES: u64 = 32 * 1024 * 1024;

pub struct LoadedBundle {
    pub dir: PathBuf,
    pub bundle: KernelBundle,
    pub wgsl: String,
}

/// A JSON value that rejects duplicate object keys. `serde_json::Value`
/// silently keeps the last duplicate, so the strict pass runs first and the
/// typed parse happens only on a duplicate-free document.
enum Strict {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Strict>),
    Object(Vec<(String, Strict)>),
}

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict::Null)
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<Strict, E> {
                Ok(Strict::Bool(b))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<Strict, E> {
                Ok(Strict::Number(n.into()))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<Strict, E> {
                Ok(Strict::Number(n.into()))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<Strict, E> {
                serde_json::Number::from_f64(n)
                    .map(Strict::Number)
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Strict, E> {
                Ok(Strict::String(s.to_string()))
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<Strict, E> {
                Ok(Strict::String(s))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
                let mut items = Vec::new();
                while let Some(v) = seq.next_element()? {
                    items.push(v);
                }
                Ok(Strict::Array(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
                let mut seen = HashSet::new();
                let mut entries = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, Strict>()? {
                    if !seen.insert(k.clone()) {
                        return Err(de::Error::custom(format!("duplicate JSON key {k:?}")));
                    }
                    entries.push((k, v));
                }
                Ok(Strict::Object(entries))
            }
        }
        deserializer.deserialize_any(V)
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if metadata.len() > limit {
        return Err(format!(
            "{} is {} bytes, over the {}-byte limit",
            path.display(),
            metadata.len(),
            limit
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} grew past the byte limit while reading", path.display()));
    }
    Ok(bytes)
}

pub fn strict_json_check(bytes: &[u8]) -> Result<(), String> {
    strict_check(bytes)
}

fn strict_check(bytes: &[u8]) -> Result<(), String> {
    serde_json::from_slice::<Strict>(bytes)
        .map(|_| ())
        .map_err(|e| format!("strict JSON parse failed: {e}"))
}

pub fn load_bundle(dir: &Path) -> Result<LoadedBundle, String> {
    if !dir.is_dir() {
        return Err(format!("{} is not a bundle directory", dir.display()));
    }
    let json_bytes = read_bounded(&dir.join("kernel.json"), MAX_ARTIFACT_BYTES)?;
    let wgsl_bytes = read_bounded(&dir.join("kernel.wgsl"), MAX_ARTIFACT_BYTES)?;
    strict_check(&json_bytes)?;
    let bundle: KernelBundle = serde_json::from_slice(&json_bytes)
        .map_err(|e| format!("kernel.json does not match {SCHEMA}: {e}"))?;
    let wgsl = String::from_utf8(wgsl_bytes)
        .map_err(|_| "kernel.wgsl is not valid UTF-8".to_string())?;
    if bundle.schema != SCHEMA {
        return Err(format!("schema is {:?}, expected {SCHEMA}", bundle.schema));
    }
    if bundle.backend != "wgsl" {
        return Err(format!("backend is {:?}, expected \"wgsl\"", bundle.backend));
    }
    if bundle.numeric_profile != bundle.kernel_ir.numeric_profile {
        return Err(format!(
            "numeric_profile is {:?} but kernel_ir.numeric_profile is {:?}",
            bundle.numeric_profile, bundle.kernel_ir.numeric_profile
        ));
    }
    // Refuses an unknown profile name before anything interprets the element.
    crate::schema::Element::of_profile(&bundle.numeric_profile)?;
    if bundle.kernel_ir.entry != bundle.entry {
        return Err("entry disagrees with kernel_ir.entry".to_string());
    }
    if bundle.kernel_ir.input_arity != bundle.input_arity
        || bundle.input_arity == 0
        || bundle.input_arity > 2
    {
        return Err("input_arity must be 1 or 2 and agree with the IR".to_string());
    }
    if bundle.kernel_ir.memory.workgroup_size != WORKGROUP_SIZE {
        return Err(format!(
            "workgroup_size is {}, v1 requires {WORKGROUP_SIZE}",
            bundle.kernel_ir.memory.workgroup_size
        ));
    }
    if bundle.kernel_ir.memory.output_index != "identity"
        || !bundle.kernel_ir.memory.guard_before_load
    {
        return Err("v1 requires the identity output index and pre-load guard".to_string());
    }
    if bundle.sources.is_empty() {
        return Err("sources must record the admitted source closure".to_string());
    }
    for source in &bundle.sources {
        if source.sha256.len() != 64
            || !source.sha256.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(format!("source {} has a malformed SHA-256", source.path));
        }
    }
    // Recompute the semantic IR digest from the parsed value.
    let canonical = bundle.kernel_ir.canonical_bytes()?;
    let ir_digest = crate::schema::sha256_hex(&canonical);
    if ir_digest != bundle.ir_sha256 {
        return Err(format!(
            "ir_sha256 does not reproduce (staged {}, recomputed {})",
            bundle.ir_sha256, ir_digest
        ));
    }
    let wgsl_digest = crate::schema::sha256_hex(wgsl.as_bytes());
    if wgsl_digest != bundle.wgsl_sha256 {
        return Err(format!(
            "wgsl_sha256 does not reproduce (staged {}, recomputed {})",
            bundle.wgsl_sha256, wgsl_digest
        ));
    }
    check_ir(&bundle.kernel_ir)?;
    Ok(LoadedBundle {
        dir: dir.to_path_buf(),
        bundle,
        wgsl,
    })
}

/// Structural validation independent of WGSL: literal bounds, references and
/// the release element limit.
fn check_ir(ir: &crate::schema::KernelIr) -> Result<(), String> {
    fn u32_literal(s: &str) -> Result<u32, String> {
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("u32 literal {s:?} is not a decimal integer"));
        }
        let v: u64 = s
            .parse()
            .map_err(|_| format!("u32 literal {s:?} is not a decimal integer"))?;
        if v > u32::MAX as u64 {
            return Err(format!("u32 literal {s:?} is out of range"));
        }
        Ok(v as u32)
    }

    /// The literal must parse to a finite binary32. `parse::<f32>` in Rust
    /// rounds a decimal to the nearest f32, the same contract the OCaml writer
    /// and Naga use.
    fn f32_literal(s: &str) -> Result<f32, String> {
        match s.parse::<f32>() {
            Ok(v) if v.is_finite() && (v == 0.0 || v.abs() >= f32::MIN_POSITIVE) => {
                Ok(v)
            }
            Ok(v) if v.is_finite() => {
                Err(format!("f32 literal {s:?} is subnormal, outside the admitted domain"))
            }
            Ok(_) => Err(format!("f32 literal {s:?} is not finite")),
            Err(_) => Err(format!("f32 literal {s:?} is not a decimal float")),
        }
    }

    let element = ir.element()?;

    fn check_expr(
        arity: u32,
        locals: &[u32],
        helpers: &[(u32, usize)],
        element: crate::schema::Element,
        e: &crate::schema::Expr,
    ) -> Result<(), String> {
        use crate::schema::Expr as E;
        match e {
            E::Param { index } => {
                if *index >= arity {
                    return Err(format!("param index {index} outside arity {arity}"));
                }
                Ok(())
            }
            E::Local { index } => {
                if !locals.contains(index) {
                    return Err(format!("reference to undefined local {index}"));
                }
                Ok(())
            }
            E::U32 { value } => {
                if element != crate::schema::Element::U32 {
                    return Err("a u32 literal is not part of the bundle's numeric profile".to_string());
                }
                u32_literal(value).map(|_| ())
            }
            E::F32 { value } => {
                if element != crate::schema::Element::F32 {
                    return Err("an f32 literal is not part of the bundle's numeric profile".to_string());
                }
                f32_literal(value).map(|_| ())
            }
            E::Bool { .. } => Ok(()),
            E::Intrinsic { op, args } => {
                let float_op = matches!(op.as_str(), "f32Add" | "f32Subtract" | "f32Multiply");
                if float_op && element != crate::schema::Element::F32 {
                    return Err(format!("intrinsic {op} requires the {}", crate::schema::NUMERIC_PROFILE_F32));
                }
                if !float_op && element != crate::schema::Element::U32 {
                    return Err(format!("integer intrinsic {op} is not part of the bundle's numeric profile"));
                }
                let expected = match op.as_str() {
                    "add" | "subtract" | "multiply" | "bitwiseAnd" | "bitwiseOr"
                    | "bitwiseXor" | "f32Add" | "f32Subtract" | "f32Multiply" => 2,
                    "bitwiseNot" => 1,
                    other => return Err(format!("unknown intrinsic {other:?}")),
                };
                if args.len() != expected {
                    return Err(format!(
                        "intrinsic {op} takes {expected} argument(s), got {}",
                        args.len()
                    ));
                }
                for a in args {
                    check_expr(arity, locals, helpers, element, a)?;
                }
                Ok(())
            }
            E::Compare { op, left, right } => {
                if element != crate::schema::Element::U32 {
                    return Err("comparisons are not part of the bundle's numeric profile".to_string());
                }
                if !matches!(op.as_str(), "eq" | "ne" | "lt" | "le" | "gt" | "ge") {
                    return Err(format!("unknown comparison {op:?}"));
                }
                check_expr(arity, locals, helpers, element, left)?;
                check_expr(arity, locals, helpers, element, right)
            }
            E::Boolean { op, left, right } => {
                if !matches!(op.as_str(), "and" | "or") {
                    return Err(format!("unknown boolean operation {op:?}"));
                }
                check_expr(arity, locals, helpers, element, left)?;
                check_expr(arity, locals, helpers, element, right)
            }
            E::Not { value } => check_expr(arity, locals, helpers, element, value),
            E::Conditional { condition, then, r#else } => {
                check_expr(arity, locals, helpers, element, condition)?;
                check_expr(arity, locals, helpers, element, then)?;
                check_expr(arity, locals, helpers, element, r#else)
            }
            E::Call { helper, args } => {
                let expected = helpers
                    .iter()
                    .find(|(id, _)| id == helper)
                    .map(|(_, n)| *n)
                    .ok_or_else(|| format!("call to undefined helper {helper}"))?;
                if args.len() != expected {
                    return Err(format!(
                        "helper {helper} takes {expected} argument(s), got {}",
                        args.len()
                    ));
                }
                for a in args {
                    check_expr(arity, locals, helpers, element, a)?;
                }
                Ok(())
            }
        }
    }

    fn check_stmt(
        arity: u32,
        locals: &mut Vec<u32>,
        helpers: &[(u32, usize)],
        element: crate::schema::Element,
        s: &crate::schema::Stmt,
    ) -> Result<(), String> {
        use crate::schema::Stmt as S;
        match s {
            S::Let { local, value } => {
                check_expr(arity, locals, helpers, element, value)?;
                if locals.contains(local) {
                    return Err(format!("local {local} is bound twice"));
                }
                locals.push(*local);
                Ok(())
            }
            S::Seq { items } => {
                for item in items {
                    check_stmt(arity, locals, helpers, element, item)?;
                }
                Ok(())
            }
            S::If { condition, then, r#else } => {
                check_expr(arity, locals, helpers, element, condition)?;
                let mut t = locals.clone();
                check_stmt(arity, &mut t, helpers, element, then)?;
                let mut f = locals.clone();
                check_stmt(arity, &mut f, helpers, element, r#else)
            }
            S::Return { value } => check_expr(arity, locals, helpers, element, value),
        }
    }

    if ir.helpers.len() > 128 {
        return Err("more than 128 helpers".to_string());
    }
    let helpers: Vec<(u32, usize)> = ir
        .helpers
        .iter()
        .map(|h| (h.id, h.params.len()))
        .collect();
    for (i, (id, _)) in helpers.iter().enumerate() {
        if helpers[..i].iter().any(|(other, _)| other == id) {
            return Err(format!("duplicate helper id {id}"));
        }
    }
    for helper in &ir.helpers {
        let mut locals = Vec::new();
        check_stmt(helper.params.len() as u32, &mut locals, &helpers, element, &helper.body)?;
    }
    let mut locals = Vec::new();
    check_stmt(ir.input_arity, &mut locals, &helpers, element, &ir.body)?;
    if ir.memory.map == crate::schema::MapKind::Map1 && ir.input_arity != 1
        || ir.memory.map == crate::schema::MapKind::Map2 && ir.input_arity != 2
    {
        return Err("memory map kind disagrees with input arity".to_string());
    }
    if MAX_ELEMENTS == 0 {
        return Err("unreachable".to_string());
    }
    Ok(())
}

pub fn check_requirements(bundle: &KernelBundle) -> Result<(), String> {
    let reqs = &bundle.requirements;
    let bindings = reqs
        .get("bindings")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "requirements.bindings is missing".to_string())?;
    if bindings.len() != bundle.input_arity as usize + 1 {
        return Err(format!(
            "requirements declares {} binding(s), expected {}",
            bindings.len(),
            bundle.input_arity + 1
        ));
    }
    for (i, binding) in bindings.iter().enumerate() {
        let group = binding.get("group").and_then(|v| v.as_u64());
        let index = binding.get("binding").and_then(|v| v.as_u64());
        if group != Some(0) || index != Some(i as u64) {
            return Err(format!("binding {i} is not group 0 binding {i}"));
        }
        let expected_role = if i < bundle.input_arity as usize { "input" } else { "output" };
        if binding.get("role").and_then(|v| v.as_str()) != Some(expected_role) {
            return Err(format!("binding {i} role is not {expected_role}"));
        }
        let element = bundle.kernel_ir.element()?;
        if binding.get("element").and_then(|v| v.as_str()) != Some(element.name()) {
            return Err(format!(
                "binding {i} element is not {}",
                element.name()
            ));
        }
    }
    let numeric = reqs
        .get("numeric")
        .ok_or_else(|| "requirements.numeric is missing".to_string())?;
    if numeric.get("profile").and_then(|v| v.as_str())
        != Some(bundle.numeric_profile.as_str())
    {
        return Err("requirements.numeric.profile disagrees with the bundle".to_string());
    }
    let element = bundle.kernel_ir.element()?;
    if numeric.get("element").and_then(|v| v.as_str()) != Some(element.name()) {
        return Err("requirements.numeric.element disagrees with the profile".to_string());
    }
    let expected_contraction = match element {
        crate::schema::Element::U32 => "not-applicable",
        // A driver may fuse multiply+add; the profile documents that instead
        // of forbidding something the backend cannot guarantee.
        crate::schema::Element::F32 => "permitted",
    };
    if numeric.get("contraction").and_then(|v| v.as_str()) != Some(expected_contraction) {
        return Err(format!(
            "requirements.numeric.contraction must be {expected_contraction} for {}",
            element.name()
        ));
    }
    let expected_tolerance = match element {
        crate::schema::Element::U32 => 0,
        crate::schema::Element::F32 => 1,
    };
    if numeric
        .get("tolerance_ulp_per_operation")
        .and_then(|v| v.as_u64())
        != Some(expected_tolerance)
    {
        return Err(format!(
            "requirements.numeric.tolerance_ulp_per_operation must be {expected_tolerance} for {}",
            element.name()
        ));
    }
    let ops = numeric
        .get("operations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "requirements.numeric.operations is missing".to_string())?;
    if ops.is_empty() || ops.iter().any(|o| o.as_str().is_none()) {
        return Err("requirements.numeric.operations must name the admitted operations".to_string());
    }
    Ok(())
}
