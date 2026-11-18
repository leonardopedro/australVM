//! U10 (bounded start) — the versioned adapter contract.
//!
//! An adapter is described before it is generated: which native symbol is
//! called, with which scalar classes, and which scalar it returns. The
//! descriptor is the thing both sides agree on — it carries the schema number
//! *and* the scalar-encoding version, so a wrapper generated against one
//! encoding cannot be mistaken for another. Its text form is canonical
//! (fixed field order, one field per line) so regenerating it is deterministic.
//!
//! This slice deliberately stops at scalars: aggregate layouts, lifetimes and
//! variadics are refused with an actionable error rather than guessed, and the
//! generated Rust is only what a scalars-only descriptor can express.

use std::fmt::Write as _;

/// Schema of the descriptor text produced here.
pub const DESCRIPTOR_SCHEMA: u32 = 1;

/// Must equal `austral_cranelift_bridge::adapters::SCALAR_ABI_VERSION`; the
/// bridge test pins that equality so a rule change cannot leave descriptors
/// claiming the old encoding.
pub const ADAPTER_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    Unit,
    Bool,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl Scalar {
    pub fn name(self) -> &'static str {
        match self {
            Scalar::Unit => "Unit",
            Scalar::Bool => "Bool",
            Scalar::I32 => "Int32",
            Scalar::U32 => "Nat32",
            Scalar::I64 => "Int64",
            Scalar::U64 => "Nat64",
            Scalar::F32 => "Float32",
            Scalar::F64 => "Float64",
        }
    }

    pub fn parse(name: &str) -> Result<Scalar, String> {
        Ok(match name {
            "Unit" => Scalar::Unit,
            "Bool" => Scalar::Bool,
            "Int32" => Scalar::I32,
            "Nat32" => Scalar::U32,
            "Int64" => Scalar::I64,
            "Nat64" => Scalar::U64,
            "Float32" => Scalar::F32,
            "Float64" => Scalar::F64,
            other => {
                return Err(format!(
                    "unsupported adapter type `{other}`: only scalars cross a descriptor \
                     (Unit, Bool, Int32, Nat32, Int64, Nat64, Float32, Float64); \
                     records, spans, references and lifetimes need a buffer or a \
                     versioned layout that does not exist yet"
                ))
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub schema: u32,
    /// Version of the scalar word encoding the generated wrapper uses.
    pub abi_version: u32,
    /// The symbol the wrapper exports (what the CPS caller imports).
    pub symbol: String,
    /// The native function the wrapper calls.
    pub target: String,
    pub params: Vec<Scalar>,
    pub returns: Scalar,
}

impl Adapter {
    pub fn new(
        symbol: impl Into<String>,
        target: impl Into<String>,
        params: Vec<Scalar>,
        returns: Scalar,
    ) -> Adapter {
        Adapter {
            schema: DESCRIPTOR_SCHEMA,
            abi_version: ADAPTER_ABI_VERSION,
            symbol: symbol.into(),
            target: target.into(),
            params,
            returns,
        }
    }

    /// Refuse what this schema cannot express, before anything is generated.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != DESCRIPTOR_SCHEMA {
            return Err(format!(
                "descriptor schema {} is not {DESCRIPTOR_SCHEMA}; regenerate it",
                self.schema
            ));
        }
        if self.abi_version != ADAPTER_ABI_VERSION {
            return Err(format!(
                "descriptor claims scalar encoding version {}, this build knows \
                 {ADAPTER_ABI_VERSION}; a wrapper generated for another encoding must \
                 not be used",
                self.abi_version
            ));
        }
        if self.symbol.is_empty() || self.target.is_empty() {
            return Err("descriptor needs a non-empty symbol and target".to_string());
        }
        for (index, param) in self.params.iter().enumerate() {
            if *param == Scalar::Unit {
                return Err(format!(
                    "parameter {index} is Unit, which occupies no transport word; \
                     drop the parameter instead"
                ));
            }
        }
        if self.returns == Scalar::Unit {
            return Err(
                "a Unit return has no transport word; return a status scalar \
                        (for example Int64 zero) and record the unit in the descriptor's \
                        business schema"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Canonical text form. Field order is fixed and each line is one fact, so
    /// two descriptors compare equal exactly when their meanings do.
    pub fn descriptor_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "schema {}", self.schema);
        let _ = writeln!(out, "abi scalar {}", self.abi_version);
        let _ = writeln!(out, "symbol {}", self.symbol);
        let _ = writeln!(out, "target {}", self.target);
        let params: Vec<&str> = self.params.iter().map(|p| p.name()).collect();
        let _ = writeln!(out, "params {}", params.join(","));
        let _ = writeln!(out, "returns {}", self.returns.name());
        out
    }

    pub fn parse_descriptor(text: &str) -> Result<Adapter, String> {
        let mut schema = None;
        let mut abi_version = None;
        let mut symbol = None;
        let mut target = None;
        let mut params: Option<Vec<Scalar>> = None;
        let mut returns = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once(' ')
                .ok_or_else(|| format!("descriptor line `{line}` has no value"))?;
            match key {
                "schema" => {
                    schema = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| format!("schema `{value}` is not a number"))?,
                    )
                }
                "abi" => {
                    let rest = value
                        .strip_prefix("scalar ")
                        .ok_or_else(|| format!("unknown abi line `{line}`"))?;
                    abi_version = Some(
                        rest.parse::<u32>()
                            .map_err(|_| format!("abi version `{rest}` is not a number"))?,
                    );
                }
                "symbol" => symbol = Some(value.to_string()),
                "target" => target = Some(value.to_string()),
                "params" => {
                    let mut parsed = Vec::new();
                    for name in value.split(',').filter(|s| !s.is_empty()) {
                        parsed.push(Scalar::parse(name)?);
                    }
                    params = Some(parsed);
                }
                "returns" => returns = Some(Scalar::parse(value)?),
                other => return Err(format!("unknown descriptor field `{other}`")),
            }
        }
        let adapter = Adapter {
            schema: schema.ok_or("descriptor has no `schema` line")?,
            abi_version: abi_version.ok_or("descriptor has no `abi scalar` line")?,
            symbol: symbol.ok_or("descriptor has no `symbol` line")?,
            target: target.ok_or("descriptor has no `target` line")?,
            params: params.ok_or("descriptor has no `params` line")?,
            returns: returns.ok_or("descriptor has no `returns` line")?,
        };
        adapter.validate()?;
        Ok(adapter)
    }
}

fn scalar_type_path(scalar: Scalar) -> &'static str {
    match scalar {
        Scalar::Unit => "ScalarType::Unit",
        Scalar::Bool => "ScalarType::Bool",
        Scalar::I32 => "ScalarType::I32",
        Scalar::U32 => "ScalarType::U32",
        Scalar::I64 => "ScalarType::I64",
        Scalar::U64 => "ScalarType::U64",
        Scalar::F32 => "ScalarType::F32",
        Scalar::F64 => "ScalarType::F64",
    }
}

fn scalar_value_ctor(scalar: Scalar) -> &'static str {
    match scalar {
        Scalar::Unit => "ScalarValue::Unit",
        Scalar::Bool => "ScalarValue::Bool",
        Scalar::I32 => "ScalarValue::I32",
        Scalar::U32 => "ScalarValue::U32",
        Scalar::I64 => "ScalarValue::I64",
        Scalar::U64 => "ScalarValue::U64",
        Scalar::F32 => "ScalarValue::F32",
        Scalar::F64 => "ScalarValue::F64",
    }
}

fn rust_scalar(scalar: Scalar) -> &'static str {
    match scalar {
        Scalar::Unit => "()",
        Scalar::Bool => "bool",
        Scalar::I32 => "i32",
        Scalar::U32 => "u32",
        Scalar::I64 => "i64",
        Scalar::U64 => "u64",
        Scalar::F32 => "f32",
        Scalar::F64 => "f64",
    }
}

/// Render the Rust wrapper a descriptor calls for.
///
/// The wrapper takes one 64-bit word per parameter and returns one, decodes
/// each argument with the bridge's scalar adapter, calls the native function
/// with its real typed signature, and encodes the result the same way. A
/// refused word panics with the adapter's message: the transport has no
/// per-call error word, so an invalid word is a contract failure, not a value.
pub fn render_wrapper(adapter: &Adapter) -> Result<String, String> {
    adapter.validate()?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by umbrella from `{}` — do not edit.",
        adapter.symbol
    );
    let _ = writeln!(
        out,
        "// scalar encoding version {} (descriptor schema {})",
        adapter.abi_version, adapter.schema
    );
    let _ = writeln!(
        out,
        "use austral_cranelift_bridge::adapters::{{decode, encode, ScalarType, ScalarValue}};"
    );
    let _ = writeln!(out);
    let native_params: Vec<String> = adapter
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| format!("a{i}: {}", rust_scalar(*p)))
        .collect();
    if adapter.params.is_empty() {
        let _ = writeln!(
            out,
            "extern \"C\" {{\n    fn {}( ) -> {};\n}}",
            adapter.target,
            rust_scalar(adapter.returns)
        );
    } else {
        let _ = writeln!(
            out,
            "extern \"C\" {{\n    fn {}({}) -> {};\n}}",
            adapter.target,
            native_params.join(", "),
            rust_scalar(adapter.returns)
        );
    }
    let _ = writeln!(out);
    let words: Vec<String> = (0..adapter.params.len())
        .map(|i| format!("word{i}: u64"))
        .collect();
    let _ = writeln!(
        out,
        "#[no_mangle]\npub extern \"C\" fn {}({}) -> u64 {{",
        adapter.symbol,
        words.join(", ")
    );
    let mut arg_names = Vec::new();
    for (i, param) in adapter.params.iter().enumerate() {
        let _ = writeln!(
            out,
            "    let a{i} = match decode({}, word{i}) {{\n        Ok({}(v)) => v,\n        Ok(other) => panic!(\"{}: parameter {i} decoded as {{other:?}}\"),\n        Err(e) => panic!(\"{}: parameter {i}: {{e}}\"),\n    }};",
            scalar_type_path(*param),
            scalar_value_ctor(*param),
            adapter.symbol,
            adapter.symbol
        );
        arg_names.push(format!("a{i}"));
    }
    let call = format!("{}({})", adapter.target, arg_names.join(", "));
    let _ = writeln!(out, "    let result = unsafe {{ {call} }};");
    let _ = writeln!(
        out,
        "    encode({}, {}(result)).expect(\"{}: result is not its declared type\")",
        scalar_type_path(adapter.returns),
        scalar_value_ctor(adapter.returns),
        adapter.symbol
    );
    let _ = writeln!(out, "}}");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Adapter {
        Adapter::new(
            "rs_scale__adapter",
            "rs_scale",
            vec![Scalar::I32, Scalar::F64],
            Scalar::F64,
        )
    }

    #[test]
    fn the_descriptor_text_is_canonical() {
        assert_eq!(
            sample().descriptor_text(),
            "schema 1\nabi scalar 1\nsymbol rs_scale__adapter\ntarget rs_scale\n\
             params Int32,Float64\nreturns Float64\n"
        );
        let text = sample().descriptor_text();
        let parsed = Adapter::parse_descriptor(&text).expect("roundtrip");
        assert_eq!(parsed, sample());
        assert_eq!(
            parsed.descriptor_text(),
            text,
            "roundtrip must be byte-identical"
        );
    }

    #[test]
    fn an_encoding_version_mismatch_is_refused_by_name() {
        let mut adapter = sample();
        adapter.abi_version = 2;
        let err = adapter.validate().unwrap_err();
        assert!(err.contains("scalar encoding version 2"), "{err}");
        assert!(render_wrapper(&adapter).is_err());
    }

    #[test]
    fn aggregate_and_layout_types_are_refused_with_guidance() {
        let err = Scalar::parse("Span").unwrap_err();
        assert!(err.contains("unsupported adapter type `Span`"), "{err}");
        assert!(err.contains("buffer"), "{err}");
    }

    #[test]
    fn unit_in_either_position_is_refused() {
        let mut adapter = Adapter::new("s", "t", vec![Scalar::Unit], Scalar::I64);
        assert!(adapter
            .validate()
            .unwrap_err()
            .contains("parameter 0 is Unit"));
        adapter.params = vec![];
        adapter.returns = Scalar::Unit;
        assert!(adapter.validate().unwrap_err().contains("Unit return"));
    }

    #[test]
    fn a_wrapper_decodes_arguments_in_declared_order() {
        let text = render_wrapper(&sample()).expect("wrapper");
        assert!(
            text.contains("fn rs_scale(a0: i32, a1: f64) -> f64;"),
            "{text}"
        );
        assert!(
            text.contains("pub extern \"C\" fn rs_scale__adapter(word0: u64, word1: u64) -> u64"),
            "{text}"
        );
        let first = text
            .find("let a0 = match decode(ScalarType::I32, word0)")
            .expect("a0");
        let second = text
            .find("let a1 = match decode(ScalarType::F64, word1)")
            .expect("a1");
        assert!(first < second, "parameters must decode in order");
        assert!(text.contains("unsafe { rs_scale(a0, a1) }"), "{text}");
        assert!(
            text.contains("encode(ScalarType::F64, ScalarValue::F64(result))"),
            "{text}"
        );
    }

    fn golden(name: &str, actual: &str) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/adapters")
            .join(name);
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("fixtures dir");
            std::fs::write(&path, actual).expect("write golden");
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert_eq!(actual, expected, "{} is stale", path.display());
    }

    #[test]
    fn the_golden_descriptor_and_wrapper_are_byte_identical() {
        let adapter = sample();
        golden("rs_scale.descriptor", &adapter.descriptor_text());
        golden(
            "rs_scale.wrapper.rs",
            &render_wrapper(&adapter).expect("wrapper"),
        );
    }

    #[test]
    fn an_empty_parameter_list_renders_without_arguments() {
        let adapter = Adapter::new("rs_now__adapter", "rs_now", vec![], Scalar::I64);
        let text = render_wrapper(&adapter).expect("wrapper");
        assert!(text.contains("fn rs_now( ) -> i64;"), "{text}");
        assert!(text.contains("fn rs_now__adapter() -> u64"), "{text}");
        assert!(text.contains("unsafe { rs_now() }"), "{text}");
    }
}
