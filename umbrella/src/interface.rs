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

    /// Parse a whole `--emit-adapters` document: one descriptor per blank-line
    /// separated block, `#` comment lines allowed anywhere (the compiler's
    /// header is three of them). An empty or comment-only document is no
    /// adapters, which is a real state — a module whose calls are all
    /// word-native needs no wrapper. A repeated symbol is refused: two blocks
    /// for one call site would let the last one silently win.
    pub fn parse_descriptors(text: &str) -> Result<Vec<Adapter>, String> {
        fn finish(block: &str, adapters: &mut Vec<Adapter>) -> Result<(), String> {
            let has_fact = block.lines().any(|line| {
                let line = line.trim();
                !line.is_empty() && !line.starts_with('#')
            });
            if !has_fact || is_buffer_block(block) || is_handle_block(block) {
                return Ok(());
            }
            let adapter = Adapter::parse_descriptor(block)?;
            if adapters.iter().any(|a| a.symbol == adapter.symbol) {
                return Err(format!(
                    "duplicate descriptor for `{}`; one call site gets one block",
                    adapter.symbol
                ));
            }
            adapter.validate()?;
            adapters.push(adapter);
            Ok(())
        }

        let mut adapters = Vec::new();
        let mut block = String::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                finish(&block, &mut adapters)?;
                block.clear();
            } else {
                block.push_str(line);
                block.push('\n');
            }
        }
        finish(&block, &mut adapters)?;
        Ok(adapters)
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

// ── U12 (bounded start): the buffer descriptor ──────────────────────────────
//
// A byte buffer is not one word, so it cannot be a `Scalar`. Its descriptor
// fixes the conventions a generated wrapper must obey *before* anything is
// generated, and refuses every other reading:
//
//   * a buffer parameter is an address; the length is a **separate 64-bit
//     parameter** named by index (`lengths Nat64` / `lengths Int64`), so no
//     length is ever inferred from the pointer or the allocation;
//   * the empty convention is `null-len0`: length 0 means empty, and only then
//     may the pointer be null; length > 0 requires a non-null pointer that
//     covers at least that many bytes. A non-null pointer alone says nothing;
//   * each buffer names its direction (`read`, `write`, `readwrite`), because
//     a borrowed input may not be written and a caller-owned output must not
//     be read before it is filled;
//   * capacity is the length: a write buffer's length is what the caller
//     granted, never what the callee would like to have.
//
// The emitter does not produce `abi buffer` blocks yet — the CPS lowering
// refuses a buffer in an adapter-mediated signature (`rust_adapter_buffer_unsupported`),
// because this layout has no word encoding. Fixing the reader first is what
// makes the layout a contract instead of a guess: whatever the emitter writes
// later is already checked here.

/// ABI version of the buffer transport described below. Separate from the
/// scalar one: a buffer wrapper and a scalar wrapper are not interchangeable.
pub const BUFFER_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferMode {
    /// The callee may read the caller's bytes.
    Read,
    /// The callee fills caller-owned bytes; reading them first is undefined.
    Write,
    /// The callee reads and writes the same bytes.
    ReadWrite,
}

impl BufferMode {
    pub fn name(self) -> &'static str {
        match self {
            BufferMode::Read => "read",
            BufferMode::Write => "write",
            BufferMode::ReadWrite => "readwrite",
        }
    }

    fn parse(name: &str) -> Result<BufferMode, String> {
        Ok(match name {
            "read" => BufferMode::Read,
            "write" => BufferMode::Write,
            "readwrite" => BufferMode::ReadWrite,
            other => {
                return Err(format!(
                    "unknown buffer mode `{other}`: every buffer parameter is `read`, \
                     `write` or `readwrite`; a buffer whose direction is not stated has \
                     no aliasing rule"
                ))
            }
        })
    }
}

/// The empty-buffer convention. One is defined; a second needs its own
/// version and its own tests, not a silently accepted synonym.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyConvention {
    /// Length 0 is the empty buffer, and only then may the pointer be null.
    /// Length > 0 requires a non-null pointer covering at least `length`
    /// bytes; a non-null pointer alone is never sufficient.
    NullLen0,
}

impl EmptyConvention {
    pub fn name(self) -> &'static str {
        match self {
            EmptyConvention::NullLen0 => "null-len0",
        }
    }

    fn parse(name: &str) -> Result<EmptyConvention, String> {
        Ok(match name {
            "null-len0" => EmptyConvention::NullLen0,
            other => {
                return Err(format!(
                    "unknown empty-buffer convention `{other}`: the only defined one is \
                     `null-len0` (length 0 means empty and only then a null pointer is \
                     allowed); a different convention needs its own ABI version"
                ))
            }
        })
    }
}

/// One buffer parameter: which parameter holds the address, which holds its
/// length, and which way the bytes cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferParam {
    pub mode: BufferMode,
    pub buffer: usize,
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferAdapter {
    pub schema: u32,
    pub abi_version: u32,
    pub symbol: String,
    pub target: String,
    /// How many parameters the wrapper takes in total (addresses, lengths and
    /// scalars together); buffer entries index into this list.
    pub params: usize,
    /// The type of every length parameter. 32-bit lengths are refused: a
    /// buffer larger than 4 GiB must not wrap, and the transport word is 64-bit
    /// anyway.
    pub lengths: Scalar,
    pub buffers: Vec<BufferParam>,
    pub empty: EmptyConvention,
    pub returns: Scalar,
}

impl BufferAdapter {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != DESCRIPTOR_SCHEMA {
            return Err(format!(
                "descriptor schema {} is not {DESCRIPTOR_SCHEMA}; regenerate it",
                self.schema
            ));
        }
        if self.abi_version != BUFFER_ABI_VERSION {
            return Err(format!(
                "descriptor claims buffer transport version {}, this build knows \
                 {BUFFER_ABI_VERSION}; a buffer wrapper generated for another \
                 transport must not be used",
                self.abi_version
            ));
        }
        if self.symbol.is_empty() || self.target.is_empty() {
            return Err("descriptor needs a non-empty symbol and target".to_string());
        }
        if self.returns == Scalar::Unit {
            return Err(format!(
                "`{}` returns Unit, which has no transport word; return a status \
                 scalar (for example Int64 zero) and record the unit in the caller",
                self.symbol
            ));
        }
        if self.params == 0 {
            return Err(format!(
                "`{}` declares no parameters, so it cannot carry a buffer",
                self.symbol
            ));
        }
        if !matches!(self.lengths, Scalar::I64 | Scalar::U64) {
            return Err(format!(
                "`{}` declares {} lengths: a length is a 64-bit parameter \
                 (Nat64 or Int64), so a 32-bit length cannot wrap on a buffer \
                 larger than 4 GiB",
                self.symbol,
                self.lengths.name()
            ));
        }
        if self.buffers.is_empty() {
            return Err(format!(
                "`{}` is a buffer adapter with no `buffers` line; a scalar-only \
                 call belongs in an `abi scalar` block",
                self.symbol
            ));
        }
        for buffer in &self.buffers {
            if buffer.buffer >= self.params || buffer.length >= self.params {
                return Err(format!(
                    "`{}` buffer {}->{} is out of range: it declares {} parameter(s), \
                     numbered 0..{}",
                    self.symbol,
                    buffer.buffer,
                    buffer.length,
                    self.params,
                    self.params.saturating_sub(1)
                ));
            }
            if buffer.buffer == buffer.length {
                return Err(format!(
                    "`{}` names parameter {} as both a buffer and its own length; \
                     the length is a separate parameter",
                    self.symbol, buffer.buffer
                ));
            }
        }
        for (i, buffer) in self.buffers.iter().enumerate() {
            if self.buffers[..i]
                .iter()
                .any(|earlier| earlier.buffer == buffer.buffer)
            {
                return Err(format!(
                    "`{}` describes parameter {} twice; one buffer parameter has one \
                     direction and one length",
                    self.symbol, buffer.buffer
                ));
            }
        }
        Ok(())
    }

    /// The canonical text of this descriptor, in the same field order the
    /// scalar blocks use.
    pub fn descriptor_text(&self) -> String {
        let buffers = self
            .buffers
            .iter()
            .map(|b| format!("{}:{}:{}", b.mode.name(), b.buffer, b.length))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "schema {}\nabi buffer {}\nsymbol {}\ntarget {}\nparams {}\nlengths {}\n\
             buffers {}\nempty {}\nreturns {}\n",
            self.schema,
            self.abi_version,
            self.symbol,
            self.target,
            self.params,
            self.lengths.name(),
            buffers,
            self.empty.name(),
            self.returns.name()
        )
    }

    /// Read every `abi buffer` block. `abi scalar` blocks are the scalar
    /// reader's and are skipped here, so one document can describe both.
    pub fn parse_descriptors(text: &str) -> Result<Vec<BufferAdapter>, String> {
        let mut adapters = Vec::new();
        let mut block = String::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                Self::finish(&block, &mut adapters)?;
                block.clear();
            } else {
                block.push_str(line);
                block.push('\n');
            }
        }
        Self::finish(&block, &mut adapters)?;
        Ok(adapters)
    }

    fn finish(block: &str, adapters: &mut Vec<BufferAdapter>) -> Result<(), String> {
        let has_fact = block.lines().any(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        });
        if !has_fact || !is_buffer_block(block) {
            return Ok(());
        }
        let adapter = BufferAdapter::parse_descriptor(block)?;
        if adapters.iter().any(|a| a.symbol == adapter.symbol) {
            return Err(format!(
                "duplicate buffer descriptor for `{}`; one call site gets one block",
                adapter.symbol
            ));
        }
        adapter.validate()?;
        adapters.push(adapter);
        Ok(())
    }

    pub fn parse_descriptor(text: &str) -> Result<BufferAdapter, String> {
        let mut schema = None;
        let mut abi_version = None;
        let mut symbol = None;
        let mut target = None;
        let mut params = None;
        let mut lengths = None;
        let mut buffers: Option<Vec<BufferParam>> = None;
        let mut empty = None;
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
                    let rest = value.strip_prefix("buffer ").ok_or_else(|| {
                        format!(
                            "`{line}` is not a buffer block (`abi buffer {BUFFER_ABI_VERSION}`)"
                        )
                    })?;
                    abi_version = Some(
                        rest.parse::<u32>()
                            .map_err(|_| format!("abi version `{rest}` is not a number"))?,
                    );
                }
                "symbol" => symbol = Some(value.to_string()),
                "target" => target = Some(value.to_string()),
                "params" => {
                    params = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| format!("parameter count `{value}` is not a number"))?,
                    )
                }
                "lengths" => lengths = Some(Scalar::parse(value)?),
                "buffers" => {
                    let mut parsed = Vec::new();
                    for entry in value.split(',').filter(|s| !s.is_empty()) {
                        let fields: Vec<&str> = entry.split(':').collect();
                        if fields.len() != 3 {
                            return Err(format!(
                                "buffer entry `{entry}` is not `mode:buffer:length`"
                            ));
                        }
                        let index = |what: &str, text: &str| {
                            text.parse::<usize>().map_err(|_| {
                                format!("buffer entry `{entry}`: {what} `{text}` is not a number")
                            })
                        };
                        parsed.push(BufferParam {
                            mode: BufferMode::parse(fields[0])?,
                            buffer: index("buffer index", fields[1])?,
                            length: index("length index", fields[2])?,
                        });
                    }
                    buffers = Some(parsed);
                }
                "empty" => empty = Some(EmptyConvention::parse(value)?),
                "returns" => returns = Some(Scalar::parse(value)?),
                other => return Err(format!("unknown buffer descriptor field `{other}`")),
            }
        }
        let adapter = BufferAdapter {
            schema: schema.ok_or("descriptor has no `schema` line")?,
            abi_version: abi_version.ok_or_else(|| {
                format!("descriptor has no `abi buffer {BUFFER_ABI_VERSION}` line")
            })?,
            symbol: symbol.ok_or("descriptor has no `symbol` line")?,
            target: target.ok_or("descriptor has no `target` line")?,
            params: params.ok_or("descriptor has no `params` line")?,
            lengths: lengths.ok_or("descriptor has no `lengths` line")?,
            buffers: buffers.ok_or("descriptor has no `buffers` line")?,
            empty: empty.ok_or("descriptor has no `empty` line")?,
            returns: returns.ok_or("descriptor has no `returns` line")?,
        };
        adapter.validate()?;
        Ok(adapter)
    }
}

fn is_buffer_block(block: &str) -> bool {
    block.lines().any(|line| {
        line.trim()
            .strip_prefix("abi ")
            .map(|rest| rest.starts_with("buffer "))
            .unwrap_or(false)
    })
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

/// Render the Rust wrapper an `abi buffer` descriptor calls for.
///
/// The wrapper takes the same number of 64-bit words as the signature has
/// parameters: a buffer word is the host pointer, a length word is the length.
/// Buffer transport version 1 defines a buffered signature as Nat8 buffers,
/// their 64-bit lengths and a scalar result, so the result is the only value
/// the wrapper encodes. Every buffer is checked before the call: length 0 is
/// the empty buffer and only then may the pointer be null; a non-zero length
/// with a null pointer, or a length that does not fit a signed 64-bit extent,
/// panics. Capacity is the declared length — the wrapper can check that a
/// length exists, not that the caller's allocation is that large.
pub fn render_buffer_wrapper(adapter: &BufferAdapter) -> Result<String, String> {
    adapter.validate()?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by umbrella from `{}` — do not edit.",
        adapter.symbol
    );
    let _ = writeln!(
        out,
        "// buffer encoding version {} (descriptor schema {})",
        adapter.abi_version, adapter.schema
    );
    let _ = writeln!(
        out,
        "use austral_cranelift_bridge::adapters::{{encode, ScalarType, ScalarValue}};"
    );
    let _ = writeln!(out);

    let length_ty = rust_scalar(adapter.lengths);
    let buffer_at: std::collections::HashMap<usize, &BufferParam> =
        adapter.buffers.iter().map(|b| (b.buffer, b)).collect();
    let pointer_ty = |mode: BufferMode| match mode {
        BufferMode::Read => "*const u8",
        BufferMode::Write | BufferMode::ReadWrite => "*mut u8",
    };
    let null = |mode: BufferMode| match mode {
        BufferMode::Read => "std::ptr::null()",
        BufferMode::Write | BufferMode::ReadWrite => "std::ptr::null_mut()",
    };
    let native: Vec<String> = (0..adapter.params)
        .map(|i| match buffer_at.get(&i) {
            Some(buffer) => format!("buf{i}: {}", pointer_ty(buffer.mode)),
            None => format!("len{i}: {length_ty}"),
        })
        .collect();
    let _ = writeln!(
        out,
        "extern \"C\" {{\n    fn {}({}) -> {};\n}}",
        adapter.target,
        native.join(", "),
        rust_scalar(adapter.returns)
    );
    let _ = writeln!(out);
    let words: Vec<String> = (0..adapter.params).map(|i| format!("w{i}: u64")).collect();
    let _ = writeln!(
        out,
        "#[no_mangle]\npub extern \"C\" fn {}({}) -> u64 {{",
        adapter.symbol,
        words.join(", ")
    );
    let length_indices: std::collections::BTreeSet<usize> =
        adapter.buffers.iter().map(|b| b.length).collect();
    for index in &length_indices {
        let _ = writeln!(out, "    let len{index} = w{index} as {length_ty};");
        let _ = writeln!(
            out,
            "    if (w{index} as i64) < 0 {{\n        panic!(\"{}: length {{}} does not fit a signed 64-bit extent\", w{index});\n    }}",
            adapter.symbol
        );
    }
    for buffer in &adapter.buffers {
        let (index, length) = (buffer.buffer, buffer.length);
        let _ = writeln!(
            out,
            "    let buf{index} = if w{length} == 0 {{\n        {}\n    }} else if w{index} == 0 {{\n        panic!(\"{}: buffer {index}: length {{}} with a null pointer\", w{length});\n    }} else {{\n        w{index} as {}\n    }};",
            null(buffer.mode),
            adapter.symbol,
            pointer_ty(buffer.mode)
        );
    }
    let args: Vec<String> = (0..adapter.params)
        .map(|i| match buffer_at.get(&i) {
            Some(buffer) => format!("buf{}", buffer.buffer),
            None => format!("len{i}"),
        })
        .collect();
    let _ = writeln!(
        out,
        "    let result = unsafe {{ {}({}) }};",
        adapter.target,
        args.join(", ")
    );
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

/// Transport version of the owned-value handle ABI (U13).
///
/// Version 1 defines exactly three things: a constructor that takes `params`
/// 64-bit words and returns a status word while writing the handle's two words
/// through an out pointer, a dispose that takes those two words and returns a
/// status word, and the status vocabulary `0` = done, non-zero = refused.
/// Nothing else crosses: no pointer, no integer that names a value.
pub const HANDLE_ABI_VERSION: u32 = 1;

/// One owned-value adapter: the module sees a handle, the provider keeps the
/// allocation, and this descriptor is what both sides agree on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandleAdapter {
    pub schema: u32,
    pub abi_version: u32,
    /// The symbol the module calls to create a value, `<target>__new`.
    pub symbol: String,
    /// The provider type's name; it names the generated module's registry and
    /// derives the dispose symbol `<target>__dispose`.
    pub target: String,
    /// The provider symbol that constructs the value.
    pub constructor: String,
    /// The provider symbol that frees the value. Called by the wrapper's
    /// registry, exactly once per destroyed value.
    pub destructor: String,
    /// The provider's accessor for the value itself: `unsafe extern "C" fn
    /// (*mut c_void) -> i64`. Optional, because a provider need not expose its
    /// value as a word; without it no `__use` wrapper is rendered, so a
    /// descriptor that omits it describes a handle that can be created and
    /// destroyed but not read.
    pub use_symbol: Option<String>,
    /// The object type name the registry compares handles against.
    pub type_name: String,
    /// How many 64-bit words the constructor takes.
    pub params: usize,
    /// The registry tag, assigned by the generator: unique inside one adapter
    /// crate, counted from 1. It is the owner the handle words carry, so a
    /// handle for one type cannot resolve in another type's registry.
    pub owner: u32,
}

impl HandleAdapter {
    pub fn dispose_symbol(&self) -> String {
        format!("{}__dispose", self.target)
    }

    /// The symbol of the reading wrapper, `<target>__use`, rendered only for a
    /// descriptor that names a provider accessor.
    pub fn use_symbol(&self) -> String {
        format!("{}__use", self.target)
    }

    /// The symbol of the scoped-borrow wrapper, `<target>__with`. It needs no
    /// provider symbol of its own: the callback is the caller's.
    pub fn with_symbol(&self) -> String {
        format!("{}__with", self.target)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != DESCRIPTOR_SCHEMA {
            return Err(format!(
                "descriptor schema {} is not {DESCRIPTOR_SCHEMA}; regenerate it",
                self.schema
            ));
        }
        if self.abi_version != HANDLE_ABI_VERSION {
            return Err(format!(
                "descriptor claims handle transport version {}, this build knows \
                 {HANDLE_ABI_VERSION}; a wrapper generated for another transport must \
                 not be used",
                self.abi_version
            ));
        }
        if self.symbol.is_empty()
            || self.target.is_empty()
            || self.constructor.is_empty()
            || self.destructor.is_empty()
            || self.type_name.is_empty()
        {
            return Err(
                "a handle descriptor needs a non-empty symbol, target, type, \
                 constructor and destructor"
                    .to_string(),
            );
        }
        if self.symbol != format!("{}__new", self.target) {
            return Err(format!(
                "`{}` is not `{}__new`: the constructor and the dispose symbol \
                 `<target>__dispose` are derived from one target name, so the module \
                 cannot call a dispose for a type whose constructor it never named",
                self.symbol, self.target
            ));
        }
        if self.owner == 0 {
            return Err(format!(
                "`{}` has registry tag 0, which is the null handle's owner; a live \
                 registry is tagged from 1",
                self.symbol
            ));
        }
        if !self
            .type_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!(
                "type name `{}` is not a plain identifier: it is rendered into the \
                 registry's type tag",
                self.type_name
            ));
        }
        if self.constructor == self.destructor {
            return Err(format!(
                "`{}` names `{}` as both constructor and destructor; one symbol \
                 cannot both allocate and free the value",
                self.symbol, self.constructor
            ));
        }
        if let Some(accessor) = &self.use_symbol {
            if accessor.is_empty()
                || !accessor
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(format!(
                    "`{accessor}` is not a plain identifier: a `use` line names the \
                     provider symbol the `__use` wrapper calls"
                ));
            }
            if *accessor == self.constructor || *accessor == self.destructor {
                return Err(format!(
                    "`{accessor}` is both an accessor and the constructor or \
                     destructor of `{}`; reading a value and creating or freeing it \
                     cannot share one provider symbol",
                    self.symbol
                ));
            }
        }
        Ok(())
    }

    /// Canonical text of this block, in the same field order the other kinds
    /// use. The registry tag is not part of the text: it is the generator's
    /// numbering, assigned when the wrappers are rendered into one crate.
    pub fn descriptor_text(&self) -> String {
        let use_line = match &self.use_symbol {
            Some(accessor) => format!("use {accessor}\n"),
            None => String::new(),
        };
        format!(
            "schema {}\nabi handle {}\nsymbol {}\ntarget {}\ntype {}\nconstructor {}\n\
             destructor {}\n{}params {}\n",
            self.schema,
            self.abi_version,
            self.symbol,
            self.target,
            self.type_name,
            self.constructor,
            self.destructor,
            use_line,
            self.params
        )
    }

    /// Read every `abi handle` block, numbering registries from 1. The other
    /// kinds' blocks are skipped, so one document can describe all three.
    pub fn parse_descriptors(text: &str) -> Result<Vec<HandleAdapter>, String> {
        let mut adapters: Vec<HandleAdapter> = Vec::new();
        let mut block = String::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                Self::finish(&block, &mut adapters)?;
                block.clear();
            } else {
                block.push_str(line);
                block.push('\n');
            }
        }
        Self::finish(&block, &mut adapters)?;
        Ok(adapters)
    }

    fn finish(block: &str, adapters: &mut Vec<HandleAdapter>) -> Result<(), String> {
        let has_fact = block.lines().any(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        });
        if !has_fact || !is_handle_block(block) {
            return Ok(());
        }
        let mut adapter = HandleAdapter::parse_descriptor(block)?;
        adapter.owner = u32::try_from(adapters.len() + 1).map_err(|_| {
            "more than 4294967295 handle registries in one document; the owner tag \
             is 32 bits"
                .to_string()
        })?;
        if adapters.iter().any(|a| a.target == adapter.target) {
            return Err(format!(
                "duplicate handle descriptor for `{}`; one type gets one registry",
                adapter.target
            ));
        }
        adapter.validate()?;
        adapters.push(adapter);
        Ok(())
    }

    pub fn parse_descriptor(text: &str) -> Result<HandleAdapter, String> {
        let mut schema = None;
        let mut abi_version = None;
        let mut symbol = None;
        let mut target = None;
        let mut type_name = None;
        let mut constructor = None;
        let mut destructor = None;
        let mut use_symbol = None;
        let mut params = None;
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
                    let rest = value.strip_prefix("handle ").ok_or_else(|| {
                        format!(
                            "`{line}` is not a handle block (`abi handle {HANDLE_ABI_VERSION}`)"
                        )
                    })?;
                    abi_version = Some(
                        rest.parse::<u32>()
                            .map_err(|_| format!("abi version `{rest}` is not a number"))?,
                    );
                }
                "symbol" => symbol = Some(value.to_string()),
                "target" => target = Some(value.to_string()),
                "type" => type_name = Some(value.to_string()),
                "constructor" => constructor = Some(value.to_string()),
                "destructor" => destructor = Some(value.to_string()),
                "use" => use_symbol = Some(value.to_string()),
                "params" => {
                    params = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| format!("parameter count `{value}` is not a number"))?,
                    )
                }
                other => return Err(format!("unknown handle descriptor field `{other}`")),
            }
        }
        let adapter = HandleAdapter {
            schema: schema.ok_or("descriptor has no `schema` line")?,
            abi_version: abi_version.ok_or_else(|| {
                format!("descriptor has no `abi handle {HANDLE_ABI_VERSION}` line")
            })?,
            symbol: symbol.ok_or("descriptor has no `symbol` line")?,
            target: target.ok_or("descriptor has no `target` line")?,
            type_name: type_name.ok_or("descriptor has no `type` line")?,
            constructor: constructor.ok_or("descriptor has no `constructor` line")?,
            destructor: destructor.ok_or("descriptor has no `destructor` line")?,
            use_symbol,
            params: params.ok_or("descriptor has no `params` line")?,
            // Filled in by `parse_descriptors`, which numbers the registries.
            owner: 1,
        };
        adapter.validate()?;
        Ok(adapter)
    }
}

fn is_handle_block(block: &str) -> bool {
    block.lines().any(|line| {
        line.trim()
            .strip_prefix("abi ")
            .map(|rest| rest.starts_with("handle "))
            .unwrap_or(false)
    })
}

/// Render the Rust wrappers an `abi handle` descriptor calls for: the
/// constructor (`<target>__new`), the destructor (`<target>__dispose`), the
/// scoped borrow (`<target>__with`), and — when the descriptor names a provider
/// accessor — the reader (`<target>__use`).
///
/// All of them go through one registry, so the words one wrapper mints are the
/// words the others resolve. The value itself never leaves the provider: the
/// constructor's pointer is wrapped in a `ProviderValue` whose `Drop` calls the
/// provider's destructor, so a refused insert frees the value and a destroyed
/// value is freed once. The status word is `0` for done and `1` for refused —
/// an unencodable handle, a null out pointer, a null constructor result, a
/// foreign owner, a previous provider generation or a stale slot.
///
/// This is generation 1's table. A reload of the provider renders the same
/// descriptor again at the next generation (see `render_handle_wrapper_at`), and
/// the two registries refuse each other's handles.
pub fn render_handle_wrapper(adapter: &HandleAdapter) -> Result<String, String> {
    render_handle_wrapper_at(adapter, 1)
}

/// The provider generation the next build renders at. The first build in a
/// process is generation 1 and a reload is 2; `build_adapters` allocates one
/// per build, so each build's table refuses the previous build's handles
/// instead of resolving a slot number that now names a different value.
/// Generation 0 is reserved (`render_handle_wrapper_at` refuses it), so a
/// counter that wraps skips it rather than minting an unrenderable build.
static PROVIDER_GENERATION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

pub fn next_provider_generation() -> u32 {
    loop {
        let generation = PROVIDER_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if generation != 0 {
            return generation;
        }
    }
}

/// Render the wrappers for one provider build. The generation is the build's,
/// not the descriptor's: every wrapper this process renders after the first
/// gets the next number, so a handle minted by an earlier build carries an
/// earlier generation and the new table refuses it as `StaleProvider` instead
/// of resolving a slot that now means something else.
pub fn render_handle_wrapper_at(
    adapter: &HandleAdapter,
    generation: u32,
) -> Result<String, String> {
    adapter.validate()?;
    if generation == 0 {
        return Err(format!(
            "provider generation 0 is reserved: `{}` must be rendered at 1 or later, \
             so a handle always names the build that minted it",
            adapter.symbol
        ));
    }
    let mut out = String::new();
    out.push_str(&format!(
        "// @generated by umbrella from `{}` — do not edit.\n",
        adapter.symbol
    ));
    out.push_str(&format!(
        "// handle encoding version {} (descriptor schema {}), registry tag {}\n",
        adapter.abi_version, adapter.schema, adapter.owner
    ));
    out.push_str(
        "use austral_cranelift_bridge::handles::{\n    HandleRegistry, Owner, ProviderValue, TypeTag,\n};\n\n",
    );
    let words: Vec<String> = (0..adapter.params).map(|i| format!("w{i}: i64")).collect();
    out.push_str(&format!(
        "extern \"C\" {{\n    fn {}({}) -> *mut core::ffi::c_void;\n    fn {}(ptr: *mut core::ffi::c_void);\n{}}}\n\n",
        adapter.constructor,
        words.join(", "),
        adapter.destructor,
        match &adapter.use_symbol {
            Some(accessor) => format!(
                "    // The provider's reader: one word out of the value it owns.\n    \
                 fn {accessor}(ptr: *mut core::ffi::c_void) -> i64;\n"
            ),
            None => String::new(),
        }
    ));
    out.push_str(&format!(
        "// The provider generation this table belongs to. A provider reload builds\n\
         // a new wrapper crate and so a new registry, which mints the next\n\
         // generation; a handle from the previous build carries the older one and\n\
         // is refused.\n\
         const PROVIDER_GENERATION: u32 = {generation};\n\n\
         static REGISTRY: std::sync::Mutex<HandleRegistry<ProviderValue>> =\n    std::sync::Mutex::new(HandleRegistry::new(\n        Owner::issued({}),\n        TypeTag::named(\"{}\"),\n        PROVIDER_GENERATION,\n    ));\n\n",
        adapter.owner, adapter.type_name
    ));
    let mut args = String::new();
    let mut params = words.join(", ");
    if !params.is_empty() {
        params.push_str(", ");
    }
    params.push_str("out: *mut i64");
    for i in 0..adapter.params {
        args.push_str(&format!("w{i}, "));
    }
    out.push_str(&format!(
        "#[no_mangle]\npub extern \"C\" fn {}({params}) -> i64 {{\n\
         \x20   if out.is_null() {{\n        return 1;\n    }}\n\
         \x20   let raw = unsafe {{ {}({args}) }};\n\
         \x20   if raw.is_null() {{\n        return 1;\n    }}\n\
         \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   match table.insert(ProviderValue::new(raw, {})) {{\n        Ok(handle) => {{\n            let (lo, hi) = handle.to_words();\n            unsafe {{\n                *out = lo as i64;\n                *out.add(1) = hi as i64;\n            }}\n            0\n        }}\n        // The refused value is dropped here, so its destructor runs once and\n        // the provider is not leaked by a table that refused it.\n        Err(_) => 1,\n    }}\n}}\n\n",
        adapter.symbol, adapter.constructor, adapter.destructor
    ));
    if let Some(accessor) = &adapter.use_symbol {
        out.push_str(&format!(
            "#[no_mangle]\npub extern \"C\" fn {}(lo: i64, hi: i64, out: *mut i64) -> i64 {{\n\
             \x20   if out.is_null() {{\n        return 1;\n    }}\n\
             \x20   let table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
             \x20   let handle = match table.handle_from_words(lo as u64, hi as u64) {{\n        Ok(handle) => handle,\n        Err(_) => return 1,\n    }};\n\
             \x20   let ptr = match table.get(handle) {{\n        Ok(value) => value.as_ptr(),\n        Err(_) => return 1,\n    }};\n\
             \x20   // The accessor is the provider's, called with the provider's own\n\
             \x20   // pointer while the table is locked, so the value cannot be\n\
             \x20   // destroyed underneath the read.\n\
             \x20   let value = unsafe {{ {}(ptr) }};\n\
             \x20   unsafe {{\n        *out = value;\n    }}\n\
             \x20   0\n}}\n\n",
            adapter.use_symbol(), accessor
        ));
    }
    out.push_str(&format!(
        "#[no_mangle]\npub extern \"C\" fn {}(lo: i64, hi: i64,\n\
         \x20   callback: Option<unsafe extern \"C\" fn(ptr: *mut core::ffi::c_void) -> i64>,\n\
         \x20   out: *mut i64,\n\
         ) -> i64 {{\n\
         \x20   if out.is_null() {{\n        return 1;\n    }}\n\
         \x20   let Some(callback) = callback else {{\n        return 1;\n    }};\n\
         \x20   let table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   let handle = match table.handle_from_words(lo as u64, hi as u64) {{\n        Ok(handle) => handle,\n        Err(_) => return 1,\n    }};\n\
         \x20   let ptr = match table.get(handle) {{\n        Ok(value) => value.as_ptr(),\n        Err(_) => return 1,\n    }};\n\
         \x20   // The table stays locked for the call, so the borrow is scoped to\n\
         \x20   // it: the value cannot be destroyed while the callback holds it,\n\
         \x20   // and the callback's word — not the value — is what the module\n\
         \x20   // receives. Calling back into this registry from the callback\n\
         \x20   // would deadlock on the same lock, which is the price of a borrow\n\
         \x20   // the other wrappers cannot invalidate.\n\
         \x20   let value = unsafe {{ callback(ptr) }};\n\
         \x20   unsafe {{\n        *out = value;\n    }}\n\
         \x20   0\n}}\n\n",
        adapter.with_symbol()
    ));
    out.push_str(&format!(
        "#[no_mangle]\npub extern \"C\" fn {}(lo: i64, hi: i64) -> i64 {{\n\
         \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   match table.destroy_words(lo as u64, hi as u64) {{\n        Ok(()) => 0,\n        Err(_) => 1,\n    }}\n}}\n",
        adapter.dispose_symbol()
    ));
    Ok(out)
}

/// One generated adapter, whichever transport its descriptor names. The
/// build stage renders every kind into the same crate, because the JIT
/// resolves `<name>__adapter` without knowing which descriptor produced it.
pub enum GeneratedAdapter<'a> {
    Scalar(&'a Adapter),
    Buffer(&'a BufferAdapter),
    Handle(&'a HandleAdapter),
}

impl<'a> GeneratedAdapter<'a> {
    pub fn symbol(&self) -> &'a str {
        match self {
            GeneratedAdapter::Scalar(a) => &a.symbol,
            GeneratedAdapter::Buffer(a) => &a.symbol,
            GeneratedAdapter::Handle(a) => &a.symbol,
        }
    }

    pub fn render(&self) -> Result<String, String> {
        self.render_at(1)
    }

    /// Render for one provider build. The generation only reaches the handle
    /// adapters: scalar and buffer wrappers carry no handle table, so a reload
    /// changes nothing they resolve.
    pub fn render_at(&self, generation: u32) -> Result<String, String> {
        match self {
            GeneratedAdapter::Scalar(a) => render_wrapper(a),
            GeneratedAdapter::Buffer(a) => render_buffer_wrapper(a),
            GeneratedAdapter::Handle(a) => render_handle_wrapper_at(a, generation),
        }
    }
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

    // ── U12: the buffer descriptor's conventions ───────────────────────────

    fn buffer_text(buffers: &str, lengths: &str, empty: &str) -> String {
        format!(
            "schema 1\nabi buffer 1\nsymbol rs_copy__adapter\ntarget rs_copy\n\
             params 3\nlengths {lengths}\nbuffers {buffers}\nempty {empty}\n\
             returns Int64\n"
        )
    }

    fn parse_buffer(text: &str) -> Result<Vec<BufferAdapter>, String> {
        BufferAdapter::parse_descriptors(text)
    }

    #[test]
    fn a_buffer_block_round_trips_through_its_canonical_text() {
        let text = buffer_text("read:0:2,write:1:2", "Nat64", "null-len0");
        let adapters = parse_buffer(&text).expect("parse");
        assert_eq!(adapters.len(), 1);
        let adapter = &adapters[0];
        assert_eq!(adapter.params, 3);
        assert_eq!(adapter.lengths, Scalar::U64);
        assert_eq!(adapter.empty, EmptyConvention::NullLen0);
        assert_eq!(
            adapter.buffers,
            vec![
                BufferParam {
                    mode: BufferMode::Read,
                    buffer: 0,
                    length: 2
                },
                BufferParam {
                    mode: BufferMode::Write,
                    buffer: 1,
                    length: 2
                },
            ]
        );
        assert_eq!(adapter.descriptor_text(), text);
    }

    #[test]
    fn scalar_and_buffer_blocks_share_one_document() {
        let text = format!(
            "{}\n{}",
            sample().descriptor_text(),
            buffer_text("read:0:2", "Int64", "null-len0")
        );
        assert_eq!(Adapter::parse_descriptors(&text).expect("scalars").len(), 1);
        assert_eq!(
            BufferAdapter::parse_descriptors(&text)
                .expect("buffers")
                .len(),
            1
        );
    }

    #[test]
    fn a_buffer_without_a_direction_or_length_is_refused() {
        let err = parse_buffer(&buffer_text("copy:0:2", "Nat64", "null-len0")).unwrap_err();
        assert!(err.contains("unknown buffer mode `copy`"), "{err}");
        let err = parse_buffer(&buffer_text("read:0", "Nat64", "null-len0")).unwrap_err();
        assert!(err.contains("mode:buffer:length"), "{err}");
        let err = parse_buffer(&buffer_text("read:0:2", "Int32", "null-len0")).unwrap_err();
        assert!(err.contains("64-bit"), "{err}");
    }

    #[test]
    fn a_buffer_length_must_be_its_own_parameter_inside_the_list() {
        let err = parse_buffer(&buffer_text("read:0:0", "Nat64", "null-len0")).unwrap_err();
        assert!(err.contains("both a buffer and its own length"), "{err}");
        let err = parse_buffer(&buffer_text("read:0:3", "Nat64", "null-len0")).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
        let err =
            parse_buffer(&buffer_text("read:0:2,write:0:2", "Nat64", "null-len0")).unwrap_err();
        assert!(err.contains("twice"), "{err}");
    }

    #[test]
    fn a_buffer_descriptor_renders_a_checked_wrapper() {
        let text = buffer_text("read:0:2,write:1:2", "Nat64", "null-len0");
        let adapter = &parse_buffer(&text).expect("parse")[0];
        let wrapper = render_buffer_wrapper(adapter).expect("wrapper");
        assert!(
            wrapper.contains("fn rs_copy(buf0: *const u8, buf1: *mut u8, len2: u64) -> i64;"),
            "{wrapper}"
        );
        assert!(
            wrapper
                .contains("pub extern \"C\" fn rs_copy__adapter(w0: u64, w1: u64, w2: u64) -> u64"),
            "{wrapper}"
        );
        assert_eq!(
            wrapper.matches("let len2 =").count(),
            1,
            "two buffers sharing one length bind it once: {wrapper}"
        );
        assert!(
            wrapper.contains("let buf0 = if w2 == 0 {\n        std::ptr::null()"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains(
                "panic!(\"rs_copy__adapter: buffer 0: length {} with a null pointer\", w2)"
            ),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("unsafe { rs_copy(buf0, buf1, len2) }"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("encode(ScalarType::I64, ScalarValue::I64(result))"),
            "{wrapper}"
        );
    }

    #[test]
    fn a_signed_length_is_carried_as_its_own_type() {
        let text = buffer_text("readwrite:0:2", "Int64", "null-len0");
        let adapter = &parse_buffer(&text).expect("parse")[0];
        let wrapper = render_buffer_wrapper(adapter).expect("wrapper");
        assert!(wrapper.contains("buf0: *mut u8"), "{wrapper}");
        assert!(wrapper.contains("len2: i64"), "{wrapper}");
        assert!(wrapper.contains("let len2 = w2 as i64;"), "{wrapper}");
        assert!(
            wrapper.contains("if (w2 as i64) < 0 {\n        panic!("),
            "{wrapper}"
        );
    }

    #[test]
    fn only_the_stated_empty_convention_is_accepted() {
        let err = parse_buffer(&buffer_text("read:0:2", "Nat64", "ptr-only")).unwrap_err();
        assert!(err.contains("null-len0"), "{err}");
        let err = parse_buffer(
            &buffer_text("read:0:2", "Nat64", "null-len0").replace("abi buffer 1", "abi buffer 2"),
        )
        .unwrap_err();
        assert!(err.contains("buffer transport version 2"), "{err}");
        let err = parse_buffer(
            &buffer_text("read:0:2", "Nat64", "null-len0").replace("empty null-len0\n", ""),
        )
        .unwrap_err();
        assert!(err.contains("no `empty` line"), "{err}");
    }

    // ── U13: the handle descriptor ──────────────────────────────────────────

    fn handle_text(target: &str) -> String {
        format!(
            "schema 1\nabi handle 1\nsymbol {target}__new\ntarget {target}\n\
             type widget\nconstructor {target}_ctor\ndestructor {target}_dtor\n\
             params 1\n"
        )
    }

    fn parse_handle(text: &str) -> Result<Vec<HandleAdapter>, String> {
        HandleAdapter::parse_descriptors(text)
    }

    #[test]
    fn a_handle_block_round_trips_and_is_numbered_from_one() {
        let text = handle_text("rs_widget");
        let adapters = parse_handle(&text).expect("parse");
        assert_eq!(adapters.len(), 1);
        let adapter = &adapters[0];
        assert_eq!(adapter.owner, 1, "the first registry is tagged 1");
        assert_eq!(adapter.symbol, "rs_widget__new");
        assert_eq!(adapter.dispose_symbol(), "rs_widget__dispose");
        assert_eq!(adapter.params, 1);
        assert_eq!(adapter.descriptor_text(), text);
    }

    #[test]
    fn two_handle_blocks_get_distinct_registry_tags() {
        let text = format!("{}\n{}", handle_text("rs_widget"), handle_text("rs_gadget"));
        let adapters = parse_handle(&text).expect("parse");
        assert_eq!(adapters.len(), 2);
        assert_eq!(adapters[0].owner, 1);
        assert_eq!(adapters[1].owner, 2);
    }

    #[test]
    fn handle_blocks_share_a_document_with_the_other_kinds() {
        let text = format!(
            "{}\n{}\n{}",
            sample().descriptor_text(),
            buffer_text("read:0:2", "Nat64", "null-len0"),
            handle_text("rs_widget")
        );
        assert_eq!(Adapter::parse_descriptors(&text).expect("scalars").len(), 1);
        assert_eq!(
            BufferAdapter::parse_descriptors(&text)
                .expect("buffers")
                .len(),
            1
        );
        assert_eq!(
            HandleAdapter::parse_descriptors(&text)
                .expect("handles")
                .len(),
            1
        );
    }

    #[test]
    fn a_handle_descriptor_refuses_what_it_cannot_derive() {
        let err = parse_handle(
            &handle_text("rs_widget").replace("symbol rs_widget__new", "symbol rs_new"),
        )
        .unwrap_err();
        assert!(err.contains("rs_widget__new"), "{err}");

        let err = parse_handle(&format!(
            "{}\n{}",
            handle_text("rs_widget"),
            handle_text("rs_widget")
        ))
        .unwrap_err();
        assert!(err.contains("duplicate handle descriptor"), "{err}");

        let err = parse_handle(&handle_text("rs_widget").replace("type widget", "type wi-dget"))
            .unwrap_err();
        assert!(err.contains("not a plain identifier"), "{err}");

        let err = parse_handle(&handle_text("rs_widget").replace("abi handle 1", "abi handle 2"))
            .unwrap_err();
        assert!(err.contains("handle transport version 2"), "{err}");

        let mut adapter = parse_handle(&handle_text("rs_widget"))
            .expect("parse")
            .remove(0);
        adapter.owner = 0;
        let err = adapter.validate().unwrap_err();
        assert!(err.contains("null handle's owner"), "{err}");

        let mut adapter = parse_handle(&handle_text("rs_widget"))
            .expect("parse")
            .remove(0);
        adapter.destructor = adapter.constructor.clone();
        let err = adapter.validate().unwrap_err();
        assert!(err.contains("both constructor and destructor"), "{err}");
    }

    #[test]
    fn a_handle_descriptor_renders_both_wrappers_through_one_registry() {
        let adapter = parse_handle(&handle_text("rs_widget"))
            .expect("parse")
            .remove(0);
        let wrapper = render_handle_wrapper(&adapter).expect("wrapper");
        assert!(
            wrapper.contains("fn rs_widget_ctor(w0: i64) -> *mut core::ffi::c_void;"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("fn rs_widget_dtor(ptr: *mut core::ffi::c_void);"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("Owner::issued(1)"),
            "the registry carries its tag: {wrapper}"
        );
        assert!(wrapper.contains("TypeTag::named(\"widget\")"), "{wrapper}");
        assert!(
            wrapper.contains("pub extern \"C\" fn rs_widget__new(w0: i64, out: *mut i64) -> i64"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("ProviderValue::new(raw, rs_widget_dtor)"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("*out.add(1) = hi as i64;"),
            "both handle words are written: {wrapper}"
        );
        assert!(
            wrapper.contains("pub extern \"C\" fn rs_widget__dispose(lo: i64, hi: i64) -> i64"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("table.destroy_words(lo as u64, hi as u64)"),
            "{wrapper}"
        );
        assert_eq!(
            wrapper.matches("static REGISTRY").count(),
            1,
            "one registry serves both wrappers: {wrapper}"
        );
    }

    fn handle_text_with_use(target: &str) -> String {
        handle_text(target).replace(
            "destructor rs_widget_dtor\n",
            "destructor rs_widget_dtor\nuse rs_widget_read\n",
        )
    }

    #[test]
    fn a_use_line_adds_a_reading_wrapper_and_round_trips() {
        let text = handle_text_with_use("rs_widget");
        let adapter = parse_handle(&text).expect("parse").remove(0);
        assert_eq!(adapter.use_symbol.as_deref(), Some("rs_widget_read"));
        assert_eq!(adapter.use_symbol(), "rs_widget__use");
        assert_eq!(
            adapter.descriptor_text(),
            text,
            "the optional line round-trips in place"
        );

        let wrapper = render_handle_wrapper(&adapter).expect("wrapper");
        assert!(
            wrapper.contains("fn rs_widget_read(ptr: *mut core::ffi::c_void) -> i64;"),
            "{wrapper}"
        );
        assert_eq!(adapter.with_symbol(), "rs_widget__with");
        assert!(
            wrapper.contains("pub extern \"C\" fn rs_widget__with("),
            "the scoped borrow is rendered beside the reader: {wrapper}"
        );
        assert!(
            wrapper.contains("let value = unsafe { callback(ptr) };"),
            "the callback is the caller's, called with the provider's pointer: {wrapper}"
        );
        assert!(
            wrapper.contains("let Some(callback) = callback else"),
            "a null callback is refused: {wrapper}"
        );
        assert!(
            wrapper.contains(
                "pub extern \"C\" fn rs_widget__use(lo: i64, hi: i64, out: *mut i64) -> i64"
            ),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("let value = unsafe { rs_widget_read(ptr) };"),
            "{wrapper}"
        );
        assert_eq!(
            wrapper.matches("static REGISTRY").count(),
            1,
            "the reader resolves the constructor's table: {wrapper}"
        );

        // A descriptor without the line renders no reader at all, but the
        // scoped borrow needs no provider symbol, so it is always there.
        let plain = parse_handle(&handle_text("rs_widget"))
            .expect("parse")
            .remove(0);
        let plain_wrapper = render_handle_wrapper(&plain).expect("wrapper");
        assert!(
            !plain_wrapper.contains("__use"),
            "a provider that names no reader gets no reader wrapper"
        );
        assert!(
            plain_wrapper.contains("pub extern \"C\" fn rs_widget__with("),
            "a scoped borrow needs only the registry: {plain_wrapper}"
        );
    }

    #[test]
    fn a_use_line_must_name_a_distinct_plain_identifier() {
        let err = parse_handle(&handle_text_with_use("rs_widget"))
            .expect("parse")
            .remove(0)
            .validate();
        assert!(err.is_ok(), "the sample is valid");

        let mut adapter = parse_handle(&handle_text_with_use("rs_widget"))
            .expect("parse")
            .remove(0);
        adapter.use_symbol = Some("rs_widget_dtor".to_string());
        let err = adapter.validate().unwrap_err();
        assert!(
            err.contains("both an accessor and the constructor"),
            "{err}"
        );

        let mut adapter = parse_handle(&handle_text_with_use("rs_widget"))
            .expect("parse")
            .remove(0);
        adapter.use_symbol = Some("rs read".to_string());
        let err = adapter.validate().unwrap_err();
        assert!(err.contains("not a plain identifier"), "{err}");
    }

    #[test]
    fn a_reload_renders_the_next_generation_and_generation_zero_is_refused() {
        let adapter = parse_handle(&handle_text_with_use("rs_widget"))
            .expect("parse")
            .remove(0);
        let first = render_handle_wrapper(&adapter).expect("wrapper");
        assert!(
            first.contains("const PROVIDER_GENERATION: u32 = 1;"),
            "{first}"
        );
        let second = render_handle_wrapper_at(&adapter, 2).expect("wrapper");
        assert!(
            second.contains("const PROVIDER_GENERATION: u32 = 2;"),
            "{second}"
        );
        let err = render_handle_wrapper_at(&adapter, 0).unwrap_err();
        assert!(err.contains("generation 0 is reserved"), "{err}");
    }

    #[test]
    fn a_handle_constructor_with_no_parameters_still_takes_the_out_pointer() {
        let adapter = parse_handle(&handle_text("rs_widget").replace("params 1", "params 0"))
            .expect("parse")
            .remove(0);
        let wrapper = render_handle_wrapper(&adapter).expect("wrapper");
        assert!(
            wrapper.contains("pub extern \"C\" fn rs_widget__new(out: *mut i64) -> i64"),
            "{wrapper}"
        );
        assert!(wrapper.contains("fn rs_widget_ctor() -> *mut"), "{wrapper}");
    }
}
