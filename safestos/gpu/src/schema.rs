//! Typed mirror of `kernel.json` (`austral.gpu.kernel.v1`) and KernelIR.
//!
//! Field order and names here define the canonical IR bytes: this crate must
//! reproduce the OCaml writer's digest, so the structs are serialized with
//! `serde_json` in declaration order and every value is explicitly typed.
//! Unknown fields are rejected (`deny_unknown_fields`).

use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "austral.gpu.kernel.v1";
pub const BACKEND: &str = "wgsl";
pub const NUMERIC_PROFILE: &str = "u32-modular-v1";
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

impl KernelIr {
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
