//! Typed mirror of `kernel.json` (`austral.gpu.kernel.v1`) and KernelIR.
//!
//! Field order and names here define the canonical IR bytes: this crate must
//! reproduce the OCaml writer's digest, so the structs are serialized with
//! `serde_json` in declaration order and every value is explicitly typed.
//! Unknown fields are rejected (`deny_unknown_fields`).

use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "austral.gpu.kernel.v1";
pub const BACKEND: &str = "wgsl";
/// The default integer profile.
pub const NUMERIC_PROFILE: &str = "u32-modular-v1";
/// The explicit Float32 profile (`--gpu-numeric-profile=wgsl-f32-v1`).
pub const NUMERIC_PROFILE_F32: &str = "wgsl-f32-v1";
pub const MAX_ELEMENTS: u32 = 1_048_576;
pub const WORKGROUP_SIZE: u32 = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: String,
    /// Which GPU source admission produced the kernel (`v1` or
    /// `bounded-fold-v1`). Absent in bundles written before G17b, so older
    /// artifacts still load; the default is the strict profile.
    #[serde(default)]
    pub source_profile: Option<String>,
    pub shader_validation: String,
    pub device_kind: String,
    pub execution: String,
    pub proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Expr {
    #[serde(rename = "param")]
    Param { index: u32 },
    #[serde(rename = "local")]
    Local { index: u32 },
    #[serde(rename = "u32")]
    U32 { value: String },
    /// A binary32 literal, stored as the shortest decimal that round-trips to
    /// the exact f32 value (validated on load).
    #[serde(rename = "f32")]
    F32 { value: String },
    #[serde(rename = "bool")]
    Bool { value: bool },
    #[serde(rename = "intrinsic")]
    Intrinsic { op: String, args: Vec<Expr> },
    #[serde(rename = "compare")]
    Compare { op: String, left: Box<Expr>, right: Box<Expr> },
    #[serde(rename = "boolean")]
    Boolean { op: String, left: Box<Expr>, right: Box<Expr> },
    #[serde(rename = "not")]
    Not { value: Box<Expr> },
    #[serde(rename = "conditional")]
    Conditional { condition: Box<Expr>, then: Box<Expr>, r#else: Box<Expr> },
    #[serde(rename = "call")]
    Call { helper: u32, args: Vec<Expr> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Stmt {
    #[serde(rename = "let")]
    Let { local: u32, value: Expr },
    #[serde(rename = "seq")]
    Seq { items: Vec<Stmt> },
    #[serde(rename = "if")]
    If { condition: Expr, then: Box<Stmt>, r#else: Box<Stmt> },
    #[serde(rename = "return")]
    Return { value: Expr },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValueType {
    #[serde(rename = "u32")]
    U32,
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "f32")]
    F32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Helper {
    pub id: u32,
    pub params: Vec<ValueType>,
    pub ret: ValueType,
    pub body: Stmt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MapKind {
    #[serde(rename = "map1")]
    Map1,
    #[serde(rename = "map2")]
    Map2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MemoryPlan {
    pub map: MapKind,
    pub output_index: String,
    pub guard_before_load: bool,
    pub workgroup_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KernelIr {
    pub entry: String,
    pub input_arity: u32,
    pub helpers: Vec<Helper>,
    pub body: Stmt,
    pub memory: MemoryPlan,
    /// Part of the semantic IR (and its digest): the same expression is a
    /// different instruction under a different profile.
    pub numeric_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KernelBundle {
    pub schema: String,
    pub backend: String,
    pub numeric_profile: String,
    pub entry: String,
    pub input_arity: u32,
    pub compiler: String,
    pub sources: Vec<Source>,
    pub kernel_ir: KernelIr,
    pub ir_sha256: String,
    pub wgsl_sha256: String,
    pub requirements: serde_json::Value,
    pub evidence: Evidence,
    pub proof_manifest: Option<serde_json::Value>,
}

/// The storage element a numeric profile maps over. Both profiles use 4-byte
/// words; the difference is interpretation, so host buffers stay `u32` words
/// carrying the element's bit pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    U32,
    F32,
}

impl Element {
    pub fn of_profile(profile: &str) -> Result<Element, String> {
        match profile {
            NUMERIC_PROFILE => Ok(Element::U32),
            NUMERIC_PROFILE_F32 => Ok(Element::F32),
            other => Err(format!("unknown numeric profile {other:?}")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Element::U32 => "u32",
            Element::F32 => "f32",
        }
    }
}

impl KernelIr {
    pub fn element(&self) -> Result<Element, String> {
        Element::of_profile(&self.numeric_profile)
    }

    /// The canonical semantic IR bytes. Source locations and paths are not part
    /// of this value; the digest is over exactly these bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| format!("cannot serialize KernelIR: {e}"))
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
