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
    /// `status checked`: the compiler selected the checked wrapper for this
    /// import, so the build path renders `<symbol>__checked` beside the payload
    /// wrapper and the module's call site expects the Option-tagged result.
    pub checked: bool,
    /// `(failure tag, success tag)` of the declared Option union, written only
    /// when `checked`. The wrapper allocates the union itself: tag at offset 0,
    /// the encoded payload at offset 8.
    pub checked_tags: Option<(u32, u32)>,
    /// Written only for a checked union whose *failure* case also carries a
    /// payload (`Either[L, R]`). The provider then writes `[tag, payload]`
    /// into an out pair, because the payload word alone cannot say whether it
    /// belongs to the failure or the success case.
    pub checked_union: Option<CheckedUnion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedUnion {
    pub union_name: String,
    pub failure_case: String,
    pub success_case: String,
    /// The failure case's payload type. The success case's is `Adapter::returns`.
    pub failure_payload: Scalar,
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
            checked: false,
            checked_tags: None,
            checked_union: None,
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
        match (self.checked, self.checked_tags) {
            (false, Some(_)) => {
                return Err("descriptor has `failure`/`success` tags without `status checked`; \
                            the payload wrapper does not carry a tag"
                    .to_string())
            }
            (true, None) => {
                return Err("`status checked` needs both a `failure` and a `success` tag line, \
                            so the wrapper cannot guess the union's case order"
                    .to_string())
            }
            (true, Some((failure, success))) if failure == success => {
                return Err(format!(
                    "`status checked` names the same tag ({failure}) for failure and success"
                ))
            }
            (true, Some((failure, success))) if failure > 1 || success > 1 => {
                return Err(format!(
                    "`status checked` tags ({failure}, {success}) are not indices of a \
                     two-case Option union (0 and 1)"
                ))
            }
            _ => {}
        }
        if let Some(union) = &self.checked_union {
            if self.checked_tags.is_none() {
                return Err("a `union` block needs `status checked` and its tags".to_string());
            }
            if union.union_name.is_empty()
                || union.failure_case.is_empty()
                || union.success_case.is_empty()
            {
                return Err("a `union` block needs a name and both case names".to_string());
            }
            if union.failure_case == union.success_case {
                return Err(format!(
                    "`union` names the same case `{}` for failure and success",
                    union.failure_case
                ));
            }
            if union.failure_payload == Scalar::Unit {
                return Err(
                    "`failure-payload` cannot be Unit: a payload-less failure case is the \
                     `Option` shape, which has no `union` block"
                        .to_string(),
                );
            }
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
        if let Some((failure, success)) = self.checked_tags {
            let _ = writeln!(out, "status checked");
            let _ = writeln!(out, "failure {failure}");
            let _ = writeln!(out, "success {success}");
        }
        if let Some(union) = &self.checked_union {
            let _ = writeln!(out, "union {}", union.union_name);
            let _ = writeln!(out, "failure-case {}", union.failure_case);
            let _ = writeln!(out, "success-case {}", union.success_case);
            let _ = writeln!(out, "failure-payload {}", union.failure_payload.name());
        }
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
        let mut checked = false;
        let mut failure = None;
        let mut success = None;
        let mut union_name = None;
        let mut failure_case = None;
        let mut success_case = None;
        let mut failure_payload = None;
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
                "status" => {
                    if value != "checked" {
                        return Err(format!(
                            "unknown status `{value}`; only `checked` is defined"
                        ));
                    }
                    checked = true;
                }
                "failure" => {
                    failure = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| format!("failure tag `{value}` is not a number"))?,
                    )
                }
                "success" => {
                    success = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| format!("success tag `{value}` is not a number"))?,
                    )
                }
                "union" => union_name = Some(value.to_string()),
                "failure-case" => failure_case = Some(value.to_string()),
                "success-case" => success_case = Some(value.to_string()),
                "failure-payload" => failure_payload = Some(Scalar::parse(value)?),
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
            checked,
            checked_tags: match (failure, success) {
                (None, None) => None,
                (Some(failure), Some(success)) => Some((failure, success)),
                _ => {
                    return Err(
                        "`status checked` needs both a `failure` and a `success` tag line"
                            .to_string(),
                    )
                }
            },
            checked_union: match (union_name, failure_case, success_case, failure_payload) {
                (None, None, None, None) => None,
                (Some(union_name), Some(failure_case), Some(success_case), Some(failure_payload)) => {
                    Some(CheckedUnion {
                        union_name,
                        failure_case,
                        success_case,
                        failure_payload,
                    })
                }
                _ => {
                    return Err(
                        "a two-payload union needs `union`, `failure-case`, \
                         `success-case` and `failure-payload` together"
                            .to_string(),
                    )
                }
            },
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
// The emitter writes these blocks: `lib/Compiler_cps.ml` `adapter_block`,
// admitted by the identical rule in `lib/TypingPass.ml`. Whatever it writes is
// checked here before a wrapper is generated.
//
// Version 2 adds 64-bit integer *words* beside a buffer — the GPU service's
// `(context, buffer, offset, address, length)` shape, where the handles and the
// offset are not lengths. They are carried as `scalars`, named by index, so a
// wrapper reads a length only from a parameter a `buffers` entry names. A
// version-1 block keeps its old meaning exactly (every non-buffer parameter is
// a length), which is why the emitter writes version 1 unless a word is present.

/// ABI version of the buffer transport this build writes. It also *reads*
/// every version up to this one: version 1 is still a valid description, and
/// refusing it would break a descriptor that is still correct. Separate from
/// the scalar ABI: a buffer wrapper and a scalar wrapper are not
/// interchangeable.
pub const BUFFER_ABI_VERSION: u32 = 2;

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

/// One 64-bit integer word beside a buffer: which parameter it is, and its
/// type. Version 2 only; the pointer and length parameters are named by
/// `buffers` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarParam {
    pub index: usize,
    pub ty: Scalar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferAdapter {
    pub schema: u32,
    pub abi_version: u32,
    pub symbol: String,
    pub target: String,
    /// How many parameters the wrapper takes in total (buffers, lengths and
    /// scalars together); the three lines below index into this list.
    pub params: usize,
    /// The type of every length parameter. 32-bit lengths are refused: a
    /// buffer larger than 4 GiB must not wrap, and the transport word is 64-bit
    /// anyway.
    pub lengths: Scalar,
    pub buffers: Vec<BufferParam>,
    /// Words carried unchanged beside the buffers. Empty for version 1, whose
    /// non-buffer parameters are all lengths.
    pub scalars: Vec<ScalarParam>,
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
        if self.abi_version == 0 || self.abi_version > BUFFER_ABI_VERSION {
            return Err(format!(
                "descriptor claims buffer transport version {}, this build knows 1 \
                 through {BUFFER_ABI_VERSION}; a buffer wrapper generated for \
                 another transport must not be used",
                self.abi_version
            ));
        }
        if self.abi_version == 1 && !self.scalars.is_empty() {
            return Err(format!(
                "`{}` declares `scalars` in a buffer transport version 1 block; a \
                 64-bit word beside a buffer is version {BUFFER_ABI_VERSION}",
                self.symbol
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
        // Version 2: every parameter is exactly one of a buffer, a buffer's
        // length, or a declared word. A parameter nothing names is a parameter
        // the wrapper would have to guess a type for, so it is refused here.
        // Version 1 keeps its published meaning (every non-buffer parameter is
        // a length), because a format we still accept is not tightened under
        // its old number.
        let mut claimed: Vec<Option<&'static str>> = vec![None; self.params];
        for buffer in &self.buffers {
            claimed[buffer.buffer] = Some("a buffer");
            claimed[buffer.length] = Some("a length");
        }
        for scalar in &self.scalars {
            if scalar.index >= self.params {
                return Err(format!(
                    "`{}` names parameter {} as a scalar: it declares {} \
                     parameter(s), numbered 0..{}",
                    self.symbol,
                    scalar.index,
                    self.params,
                    self.params.saturating_sub(1)
                ));
            }
            if !matches!(scalar.ty, Scalar::I64 | Scalar::U64) {
                return Err(format!(
                    "`{}` declares parameter {} as a {} scalar; a word beside a \
                     buffer is a 64-bit integer (Int64 or Nat64), because any \
                     narrower value needs marshalling the buffer descriptor has \
                     no field for",
                    self.symbol,
                    scalar.index,
                    scalar.ty.name()
                ));
            }
            if let Some(what) = claimed[scalar.index] {
                return Err(format!(
                    "`{}` names parameter {} twice: it is both a scalar and {}",
                    self.symbol, scalar.index, what
                ));
            }
            claimed[scalar.index] = Some("a scalar");
        }
        if self.abi_version >= 2 {
            if let Some(index) = claimed.iter().position(|what| what.is_none()) {
                return Err(format!(
                    "`{}` leaves parameter {} unexplained: every parameter is a \
                     buffer, a buffer's length, or a declared scalar, because the \
                     wrapper has no type to give anything else",
                    self.symbol, index
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
        let scalars = if self.scalars.is_empty() {
            String::new()
        } else {
            format!(
                "scalars {}\n",
                self.scalars
                    .iter()
                    .map(|s| format!("{}:{}", s.index, s.ty.name()))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        format!(
            "schema {}\nabi buffer {}\nsymbol {}\ntarget {}\nparams {}\nlengths {}\n\
             buffers {}\n{}empty {}\nreturns {}\n",
            self.schema,
            self.abi_version,
            self.symbol,
            self.target,
            self.params,
            self.lengths.name(),
            buffers,
            scalars,
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
        let mut scalars: Option<Vec<ScalarParam>> = None;
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
                "scalars" => {
                    let mut parsed = Vec::new();
                    for entry in value.split(',').filter(|s| !s.is_empty()) {
                        let (index, ty) = entry.split_once(':').ok_or_else(|| {
                            format!("scalar entry `{entry}` is not `index:Type`")
                        })?;
                        parsed.push(ScalarParam {
                            index: index.parse::<usize>().map_err(|_| {
                                format!("scalar entry `{entry}`: index `{index}` is not a number")
                            })?,
                            ty: Scalar::parse(ty)?,
                        });
                    }
                    scalars = Some(parsed);
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
            scalars: scalars.unwrap_or_default(),
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

/// Render the checked form of a scalar wrapper: the same provider call and the
/// same argument words, but the payload goes through `out` and the return is a
/// status word. A word that does not decode or a provider panic is then a value
/// the caller can branch on, instead of the abort the payload form makes it.
/// The payload wrapper keeps its ABI, so existing modules are untouched; a
/// module opts in by importing `<symbol>__checked` explicitly.
pub fn render_wrapper_checked(adapter: &Adapter) -> Result<String, String> {
    adapter.validate()?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by umbrella from `{}` — do not edit.",
        adapter.symbol
    );
    let _ = writeln!(
        out,
        "// checked scalar encoding version {} (descriptor schema {})",
        adapter.abi_version, adapter.schema
    );
    let _ = writeln!(
        out,
        "use austral_cranelift_bridge::adapters::{{decode, encode, ScalarType, ScalarValue}};"
    );
    let _ = writeln!(
        out,
        "use austral_cranelift_bridge::handles::{{STATUS_OK, STATUS_PANIC, STATUS_REFUSED}};"
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
            "extern \"C-unwind\" {{\n    fn {}() -> {};\n}}",
            adapter.target,
            rust_scalar(adapter.returns)
        );
    } else {
        let _ = writeln!(
            out,
            "extern \"C-unwind\" {{\n    fn {}({}) -> {};\n}}",
            adapter.target,
            native_params.join(", "),
            rust_scalar(adapter.returns)
        );
    }
    let _ = writeln!(out);
    let mut params: Vec<String> = (0..adapter.params.len())
        .map(|i| format!("word{i}: u64"))
        .collect();
    params.push("out: *mut u64".to_string());
    let _ = writeln!(
        out,
        "#[no_mangle]\npub extern \"C\" fn {}__checked({}) -> i64 {{",
        adapter.symbol,
        params.join(", ")
    );
    let _ = writeln!(
        out,
        "    if out.is_null() {{\n        return STATUS_REFUSED;\n    }}"
    );
    let mut arg_names = Vec::new();
    for (i, param) in adapter.params.iter().enumerate() {
        let _ = writeln!(
            out,
            "    let a{i} = match decode({}, word{i}) {{\n        Ok({}(v)) => v,\n        Ok(_) => return STATUS_REFUSED,\n        Err(_) => return STATUS_REFUSED,\n    }};",
            scalar_type_path(*param),
            scalar_value_ctor(*param)
        );
        arg_names.push(format!("a{i}"));
    }
    let call = format!("{}({})", adapter.target, arg_names.join(", "));
    let _ = writeln!(
        out,
        "    let encoded = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {{ {call} }})) {{
        Ok(result) => encode({}, {}(result))
            .expect(\"{}: result is not its declared type\"),
        Err(_) => return STATUS_PANIC,
    }};",
        scalar_type_path(adapter.returns),
        scalar_value_ctor(adapter.returns),
        adapter.symbol
    );
    let _ = writeln!(out, "    unsafe {{ *out = encoded; }}");
    let _ = writeln!(out, "    STATUS_OK");
    let _ = writeln!(out, "}}");
    Ok(out)
}

/// Render the Option-tagged checked form the compiler selects for an ordinary
/// `rs_*` import whose declared result is `Option[T]`.
///
/// The ABI is one word in and one pointer out: the wrapper builds the union the
/// JIT already represents (`tag` at offset 0, the encoded payload at offset 8)
/// and returns its address. A word that does not decode, a provider panic, or a
/// payload that does not encode all return the failure tag, so a refusal never
/// reaches the module as the payload word. The payload wrapper is still
/// rendered beside it, so a caller that names the plain symbol is unchanged.
pub fn render_wrapper_checked_option(adapter: &Adapter) -> Result<String, String> {
    adapter.validate()?;
    let (failure, success) = adapter.checked_tags.ok_or_else(|| {
        format!(
            "`{}` is not `status checked`; the Option wrapper needs its failure and \
             success tags",
            adapter.symbol
        )
    })?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by umbrella from `{}` — do not edit.",
        adapter.symbol
    );
    let _ = writeln!(
        out,
        "// Option-tagged checked scalar, encoding version {} (descriptor schema {})\n\
         // tag {failure} = failure, tag {success} = success; tag at offset 0, payload at 8",
        adapter.abi_version, adapter.schema
    );
    let _ = writeln!(
        out,
        "use austral_cranelift_bridge::adapters::{{decode, encode, ScalarType, ScalarValue}};"
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "// The failure value is shared: union values are immutable in this model."
    );
    let _ = writeln!(out, "static OPTION_FAILURE: [u64; 2] = [{failure}, 0];");
    let _ = writeln!(
        out,
        "fn option_failure() -> *mut u64 {{\n    OPTION_FAILURE.as_ptr() as *mut u64\n}}"
    );
    let _ = writeln!(out);
    let native_params: Vec<String> = adapter
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| format!("a{i}: {}", rust_scalar(*p)))
        .collect();
    let _ = writeln!(
        out,
        "extern \"C-unwind\" {{\n    fn {}({}) -> {};\n}}",
        adapter.target,
        native_params.join(", "),
        rust_scalar(adapter.returns)
    );
    let _ = writeln!(out);
    let params: Vec<String> = (0..adapter.params.len())
        .map(|i| format!("word{i}: u64"))
        .collect();
    let _ = writeln!(
        out,
        "#[no_mangle]\npub extern \"C\" fn {}__checked({}) -> *mut u64 {{",
        adapter.symbol,
        params.join(", ")
    );
    let mut arg_names = Vec::new();
    for (i, param) in adapter.params.iter().enumerate() {
        let _ = writeln!(
            out,
            "    let a{i} = match decode({}, word{i}) {{\n        Ok({}(v)) => v,\n        _ => return option_failure(),\n    }};",
            scalar_type_path(*param),
            scalar_value_ctor(*param)
        );
        arg_names.push(format!("a{i}"));
    }
    let call = format!("{}({})", adapter.target, arg_names.join(", "));
    let _ = writeln!(
        out,
        "    let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {{ {call} }})) {{\n        Ok(result) => result,\n        Err(_) => return option_failure(),\n    }};"
    );
    let _ = writeln!(
        out,
        "    let encoded = match encode({}, {}(result)) {{\n        Ok(word) => word,\n        Err(_) => return option_failure(),\n    }};",
        scalar_type_path(adapter.returns),
        scalar_value_ctor(adapter.returns)
    );
    let _ = writeln!(
        out,
        "    Box::into_raw(Box::new([{success}, encoded])) as *mut u64"
    );
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Release a union returned by `{}__checked`. The failure value is a\n\
         /// static, so it is not freed; every success value is a one-allocation Box.\n\
         #[no_mangle]\npub extern \"C\" fn {}__free(value: *mut u64) {{\n    \
         if value.is_null() || value == option_failure() {{\n        return;\n    }}\n    \
         unsafe {{ drop(Box::from_raw(value as *mut [u64; 2])); }}\n}}",
        adapter.symbol, adapter.symbol
    );
    Ok(out)
}

/// Render the checked wrapper for a union whose failure case also carries a
/// payload (`Either[L, R]`).
///
/// One pointer out remains: the wrapper returns the address of `[tag,
/// payload]`, the union the JIT represents. But one word in is not enough for
/// the provider to say which case it chose, so this ABI adds an out pair: the
/// provider writes `[tag, payload]` (the payload encoded at its declared
/// scalar type) and returns a status, `0` meaning filled. A non-zero status, a
/// panic, an unknown tag, or a payload that does not decode all return the
/// failure case with payload `0`, so no unchecked word reaches the module.
pub fn render_wrapper_checked_union(adapter: &Adapter) -> Result<String, String> {
    adapter.validate()?;
    let (failure, success) = adapter.checked_tags.ok_or_else(|| {
        format!(
            "`{}` is not `status checked`; the union wrapper needs its tags",
            adapter.symbol
        )
    })?;
    let union = adapter.checked_union.as_ref().ok_or_else(|| {
        format!(
            "`{}` has no `union` block; the payload-carrying failure case needs \
             its name and payload type",
            adapter.symbol
        )
    })?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by umbrella from `{}` — do not edit.",
        adapter.symbol
    );
    let _ = writeln!(
        out,
        "// checked {} union, encoding version {} (descriptor schema {})\n\
         // tag {failure} = {}, tag {success} = {}; provider writes [tag, payload]",
        union.union_name,
        adapter.abi_version,
        adapter.schema,
        union.failure_case,
        union.success_case
    );
    let _ = writeln!(
        out,
        "use austral_cranelift_bridge::adapters::{{decode, encode, ScalarType, ScalarValue}};"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "static REFUSAL: [u64; 2] = [{failure}, 0];");
    let _ = writeln!(
        out,
        "fn refusal() -> *mut u64 {{\n    REFUSAL.as_ptr() as *mut u64\n}}"
    );
    let _ = writeln!(out);
    let mut native_params: Vec<String> = adapter
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| format!("a{i}: {}", rust_scalar(*p)))
        .collect();
    native_params.push("out: *mut u64".to_string());
    let _ = writeln!(
        out,
        "extern \"C-unwind\" {{\n    fn {}({}) -> i64;\n}}",
        adapter.target,
        native_params.join(", ")
    );
    let _ = writeln!(out);
    let params: Vec<String> = (0..adapter.params.len())
        .map(|i| format!("word{i}: u64"))
        .collect();
    let _ = writeln!(
        out,
        "#[no_mangle]\npub extern \"C\" fn {}__checked({}) -> *mut u64 {{",
        adapter.symbol,
        params.join(", ")
    );
    let mut arg_names = Vec::new();
    for (i, param) in adapter.params.iter().enumerate() {
        let _ = writeln!(
            out,
            "    let a{i} = match decode({}, word{i}) {{\n        Ok({}(v)) => v,\n        _ => return refusal(),\n    }};",
            scalar_type_path(*param),
            scalar_value_ctor(*param)
        );
        arg_names.push(format!("a{i}"));
    }
    arg_names.push("pair.as_mut_ptr()".to_string());
    let call = format!("{}({})", adapter.target, arg_names.join(", "));
    let _ = writeln!(out, "    let mut pair = [0u64; 2];");
    let _ = writeln!(
        out,
        "    let status = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {{ {call} }})) {{\n        Ok(status) => status,\n        Err(_) => return refusal(),\n    }};"
    );
    let _ = writeln!(
        out,
        "    if status != 0 {{\n        return refusal();\n    }}"
    );
    let _ = writeln!(out, "    let tag = pair[0];");
    let _ = writeln!(
        out,
        "    let payload = if tag == {failure} {{\n        match decode({}, pair[1]) {{\n            Ok({}(v)) => match encode({}, {}(v)) {{\n                Ok(word) => word,\n                Err(_) => return refusal(),\n            }},\n            Err(_) => return refusal(),\n            _ => return refusal(),\n        }}\n    }} else if tag == {success} {{\n        match decode({}, pair[1]) {{\n            Ok({}(v)) => match encode({}, {}(v)) {{\n                Ok(word) => word,\n                Err(_) => return refusal(),\n            }},\n            Err(_) => return refusal(),\n            _ => return refusal(),\n        }}\n    }} else {{\n        return refusal();\n    }};",
        scalar_type_path(union.failure_payload),
        scalar_value_ctor(union.failure_payload),
        scalar_type_path(union.failure_payload),
        scalar_value_ctor(union.failure_payload),
        scalar_type_path(adapter.returns),
        scalar_value_ctor(adapter.returns),
        scalar_type_path(adapter.returns),
        scalar_value_ctor(adapter.returns)
    );
    let _ = writeln!(out, "    Box::into_raw(Box::new([tag, payload])) as *mut u64");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Release a union returned by `{}__checked`. The refusal value is a\n\
         /// static, so it is not freed; every answered call is a one-allocation Box.\n\
         #[no_mangle]\npub extern \"C\" fn {}__free(value: *mut u64) {{\n    \
         if value.is_null() || value == refusal() {{\n        return;\n    }}\n    \
         unsafe {{ drop(Box::from_raw(value as *mut [u64; 2])); }}\n}}",
        adapter.symbol, adapter.symbol
    );
    Ok(out)
}

/// Render the Rust wrapper an `abi buffer` descriptor calls for.
///
/// The wrapper takes the same number of 64-bit words as the signature has
/// parameters: a buffer word is the host pointer, a length word is the length,
/// and a scalar word (version 2) is carried unchanged at its declared type.
/// Buffer transport version 1 defines a buffered signature as Nat8 buffers,
/// their 64-bit lengths and a scalar result; version 2 adds 64-bit words. The
/// result is the only value the wrapper encodes. Every buffer is checked before the call: length 0 is
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
    let scalar_at: std::collections::HashMap<usize, Scalar> =
        adapter.scalars.iter().map(|s| (s.index, s.ty)).collect();
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
            None => match scalar_at.get(&i) {
                Some(ty) => format!("s{i}: {}", rust_scalar(*ty)),
                None => format!("len{i}: {length_ty}"),
            },
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
            None => match scalar_at.get(&i) {
                Some(ty) => format!("w{i} as {}", rust_scalar(*ty)),
                None => format!("len{i}"),
            },
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

/// Render the checked form of a buffer wrapper: the same buffers, lengths and
/// scalar words, but a status word out and the encoded result through `out`. A
/// negative length, a null pointer with a non-zero length, or a provider panic
/// is refused as a status instead of aborting the worker. The payload wrapper
/// keeps its ABI; a module opts in by importing `<symbol>__checked`.
pub fn render_buffer_wrapper_checked(adapter: &BufferAdapter) -> Result<String, String> {
    adapter.validate()?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by umbrella from `{}` — do not edit.",
        adapter.symbol
    );
    let _ = writeln!(
        out,
        "// checked buffer encoding version {} (descriptor schema {})",
        adapter.abi_version, adapter.schema
    );
    let _ = writeln!(
        out,
        "use austral_cranelift_bridge::adapters::{{encode, ScalarType, ScalarValue}};"
    );
    let _ = writeln!(
        out,
        "use austral_cranelift_bridge::handles::{{STATUS_OK, STATUS_PANIC, STATUS_REFUSED}};"
    );
    let _ = writeln!(out);

    let length_ty = rust_scalar(adapter.lengths);
    let buffer_at: std::collections::HashMap<usize, &BufferParam> =
        adapter.buffers.iter().map(|b| (b.buffer, b)).collect();
    let scalar_at: std::collections::HashMap<usize, Scalar> =
        adapter.scalars.iter().map(|s| (s.index, s.ty)).collect();
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
            None => match scalar_at.get(&i) {
                Some(ty) => format!("s{i}: {}", rust_scalar(*ty)),
                None => format!("len{i}: {length_ty}"),
            },
        })
        .collect();
    let _ = writeln!(
        out,
        "extern \"C-unwind\" {{\n    fn {}({}) -> {};\n}}",
        adapter.target,
        native.join(", "),
        rust_scalar(adapter.returns)
    );
    let _ = writeln!(out);
    let mut params: Vec<String> = (0..adapter.params).map(|i| format!("w{i}: u64")).collect();
    params.push("out: *mut u64".to_string());
    let _ = writeln!(
        out,
        "#[no_mangle]\npub extern \"C\" fn {}__checked({}) -> i64 {{",
        adapter.symbol,
        params.join(", ")
    );
    let _ = writeln!(
        out,
        "    if out.is_null() {{\n        return STATUS_REFUSED;\n    }}"
    );
    let length_indices: std::collections::BTreeSet<usize> =
        adapter.buffers.iter().map(|b| b.length).collect();
    for index in &length_indices {
        let _ = writeln!(out, "    let len{index} = w{index} as {length_ty};");
        let _ = writeln!(
            out,
            "    if (w{index} as i64) < 0 {{\n        return STATUS_REFUSED;\n    }}"
        );
    }
    for buffer in &adapter.buffers {
        let (index, length) = (buffer.buffer, buffer.length);
        let _ = writeln!(
            out,
            "    let buf{index} = if w{length} == 0 {{\n        {}\n    }} else if w{index} == 0 {{\n        return STATUS_REFUSED;\n    }} else {{\n        w{index} as {}\n    }};",
            null(buffer.mode),
            pointer_ty(buffer.mode)
        );
    }
    let args: Vec<String> = (0..adapter.params)
        .map(|i| match buffer_at.get(&i) {
            Some(buffer) => format!("buf{}", buffer.buffer),
            None => match scalar_at.get(&i) {
                Some(ty) => format!("w{i} as {}", rust_scalar(*ty)),
                None => format!("len{i}"),
            },
        })
        .collect();
    let call = format!("{}({})", adapter.target, args.join(", "));
    let _ = writeln!(
        out,
        "    let encoded = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {{ {call} }})) {{
        Ok(result) => encode({}, {}(result))
            .expect(\"{}: result is not its declared type\"),
        Err(_) => return STATUS_PANIC,
    }};",
        scalar_type_path(adapter.returns),
        scalar_value_ctor(adapter.returns),
        adapter.symbol
    );
    let _ = writeln!(out, "    unsafe {{ *out = encoded; }}");
    let _ = writeln!(out, "    STATUS_OK");
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
/// Which scope the retained API may open. `Call` means the descriptor says
/// callbacks are call-scoped: no `__retain` is rendered, so a provider cannot
/// keep the callback past the call. `Instance` is the retained lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackLifetime {
    Call,
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackThread {
    Caller,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackReentrancy {
    Forbidden,
    Nested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackErrors {
    Status,
    Retire,
}

/// The `callback` line: what the descriptor chooses, field by field. An absent
/// field keeps the wrapper's default, which is why each one is optional — the
/// canonical text then names only what the author decided, and a wrapper
/// rendered from an older descriptor keeps the old policy exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CallbackPolicySpec {
    pub lifetime: Option<CallbackLifetime>,
    pub thread: Option<CallbackThread>,
    pub reentrancy: Option<CallbackReentrancy>,
    pub error: Option<CallbackErrors>,
}

impl CallbackPolicySpec {
    /// Parse one `callback` line's value: space-separated `key=value` pairs.
    /// A duplicate key is refused rather than last-wins, so a descriptor
    /// cannot silently disagree with itself.
    pub fn parse(value: &str) -> Result<CallbackPolicySpec, String> {
        let mut spec = CallbackPolicySpec::default();
        for token in value.split_whitespace() {
            let (key, raw) = token.split_once('=').ok_or_else(|| {
                format!("`{token}` is not a `key=value` callback field")
            })?;
            match key {
                "lifetime" => {
                    if spec.lifetime.is_some() {
                        return Err("the callback `lifetime` field is repeated".to_string());
                    }
                    spec.lifetime = Some(match raw {
                        "call" => CallbackLifetime::Call,
                        "instance" => CallbackLifetime::Instance,
                        other => return Err(format!(
                            "unknown callback lifetime `{other}`; use `call` or `instance`"
                        )),
                    });
                }
                "thread" => {
                    if spec.thread.is_some() {
                        return Err("the callback `thread` field is repeated".to_string());
                    }
                    spec.thread = Some(match raw {
                        "caller" => CallbackThread::Caller,
                        "any" => CallbackThread::Any,
                        other => return Err(format!(
                            "unknown callback thread `{other}`; use `caller` or `any`"
                        )),
                    });
                }
                "reentrancy" => {
                    if spec.reentrancy.is_some() {
                        return Err("the callback `reentrancy` field is repeated".to_string());
                    }
                    spec.reentrancy = Some(match raw {
                        "forbidden" => CallbackReentrancy::Forbidden,
                        "nested" => CallbackReentrancy::Nested,
                        other => return Err(format!(
                            "unknown callback reentrancy `{other}`; use `forbidden` or `nested`"
                        )),
                    });
                }
                "error" => {
                    if spec.error.is_some() {
                        return Err("the callback `error` field is repeated".to_string());
                    }
                    spec.error = Some(match raw {
                        "status" => CallbackErrors::Status,
                        "retire" => CallbackErrors::Retire,
                        other => return Err(format!(
                            "unknown callback error policy `{other}`; use `status` or `retire`"
                        )),
                    });
                }
                other => return Err(format!("unknown callback field `{other}`")),
            }
        }
        if spec.is_default() {
            return Err("a `callback` line must name at least one field".to_string());
        }
        Ok(spec)
    }

    pub fn is_default(&self) -> bool {
        *self == CallbackPolicySpec::default()
    }

    /// The canonical trailing fragment, in fixed field order, naming only the
    /// fields the descriptor set.
    pub fn descriptor_fields(&self) -> Vec<String> {
        let mut fields = Vec::new();
        if let Some(lifetime) = self.lifetime {
            fields.push(format!(
                "lifetime={}",
                match lifetime {
                    CallbackLifetime::Call => "call",
                    CallbackLifetime::Instance => "instance",
                }
            ));
        }
        if let Some(thread) = self.thread {
            fields.push(format!(
                "thread={}",
                match thread {
                    CallbackThread::Caller => "caller",
                    CallbackThread::Any => "any",
                }
            ));
        }
        if let Some(reentrancy) = self.reentrancy {
            fields.push(format!(
                "reentrancy={}",
                match reentrancy {
                    CallbackReentrancy::Forbidden => "forbidden",
                    CallbackReentrancy::Nested => "nested",
                }
            ));
        }
        if let Some(error) = self.error {
            fields.push(format!(
                "error={}",
                match error {
                    CallbackErrors::Status => "status",
                    CallbackErrors::Retire => "retire",
                }
            ));
        }
        fields
    }
}

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
    /// The provider's writer for a mutable held view. Optional: without it a
    /// module can hold a read-only view, but no `__borrow_mut`/`__write`
    /// wrapper is rendered, so a mutation the provider cannot perform is
    /// never implied.
    pub write_symbol: Option<String>,
    /// The provider's subscription entry point for a retained callback:
    /// `unsafe extern "C-unwind" fn(owner: *mut c_void, dispatch: DispatchFn,
    /// slot: *mut c_void) -> i64` (0 accepted). Optional: without it `__retain`
    /// keeps the callback in the wrapper only, and no native code can invoke
    /// it. With it, the provider owns the trampoline for the instance and
    /// must stop calling it after the wrapper's retirement hook runs.
    pub subscribe_symbol: Option<String>,
    /// The `callback` policy line. Every field is optional; an absent field is
    /// the wrapper's default, so a descriptor written before this line existed
    /// renders exactly the wrappers it used to.
    pub callback: CallbackPolicySpec,
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

    /// The symbol of the callback-scoped borrow, `<target>__with`. It needs no
    /// provider symbol of its own: the callback is the caller's.
    pub fn with_symbol(&self) -> String {
        format!("{}__with", self.target)
    }

    /// The module-held view: `<target>__borrow` mints the token that
    /// `<target>__view` reads through and `<target>__release` discharges.
    /// Unlike `__with`, the view outlives the call, so the module holds it
    /// across statements and the registry refuses `__dispose` meanwhile.
    pub fn borrow_symbol(&self) -> String {
        format!("{}__borrow", self.target)
    }

    pub fn view_symbol(&self) -> String {
        format!("{}__view", self.target)
    }

    pub fn release_symbol(&self) -> String {
        format!("{}__release", self.target)
    }

    /// The exclusive-view wrappers: `<target>__borrow_mut` mints the token and
    /// `<target>__write` mutates through it, both rendered only when the
    /// descriptor names a provider writer.
    pub fn borrow_mut_symbol(&self) -> String {
        format!("{}__borrow_mut", self.target)
    }

    pub fn write_symbol(&self) -> String {
        format!("{}__write", self.target)
    }

    /// The host's binding point, rendered into this adapter's wrapper module.
    /// The build the JIT resolved answers it; a reload calls it with the next
    /// build's implementation for the same forwarder.
    pub fn bind_symbol(&self) -> String {
        let suffix: String = self
            .target
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!("umbrella_bind_{suffix}")
    }

    /// Every forwarder this adapter renders, in the order the wrappers appear.
    pub fn forwarded_symbols(&self) -> Vec<String> {
        let mut symbols = vec![self.symbol.clone()];
        if self.use_symbol.is_some() {
            symbols.push(self.use_symbol());
        }
        symbols.push(self.borrow_symbol());
        if self.use_symbol.is_some() {
            symbols.push(self.view_symbol());
        }
        if self.write_symbol.is_some() {
            symbols.push(self.borrow_mut_symbol());
            symbols.push(self.write_symbol());
        }
        symbols.push(self.release_symbol());
        symbols.push(self.with_symbol());
        symbols.push(self.dispose_symbol());
        symbols
    }

    /// The export name of the implementation a generation renders for
    /// `forwarder`. Unique per build, so two generations can be mapped at once.
    pub fn implementation_symbol(forwarder: &str, generation: u32) -> String {
        format!("{forwarder}__impl_{generation}")
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
        if self.callback.lifetime == Some(CallbackLifetime::Call) && self.subscribe_symbol.is_some() {
            return Err(format!(
                "`{}` names a `subscribe` symbol while `callback lifetime=call`: a \
                 call-scoped callback cannot be handed to the provider, and the \
                 retained wrappers are not rendered",
                self.symbol
            ));
        }
        if let Some(subscribe) = &self.subscribe_symbol {
            if subscribe.is_empty()
                || !subscribe
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(format!(
                    "`{subscribe}` is not a plain identifier: a `subscribe` line names \
                     the provider symbol that receives the retained dispatch"
                ));
            }
            if *subscribe == self.constructor
                || *subscribe == self.destructor
                || self.use_symbol.as_deref() == Some(subscribe.as_str())
                || self.write_symbol.as_deref() == Some(subscribe.as_str())
            {
                return Err(format!(
                    "`{subscribe}` is also the constructor, destructor, accessor or \
                     writer of `{}`; a subscription is a provider symbol of its own",
                    self.symbol
                ));
            }
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
        if let Some(writer) = &self.write_symbol {
            if writer.is_empty()
                || !writer
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(format!(
                    "`{writer}` is not a plain identifier: a `write` line names the \
                     provider symbol the `__write` wrapper calls"
                ));
            }
            if *writer == self.constructor
                || *writer == self.destructor
                || self.use_symbol.as_deref() == Some(writer.as_str())
            {
                return Err(format!(
                    "`{writer}` is also the constructor, destructor or accessor of \
                     `{}`; a writer is a provider symbol of its own",
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
        let write_line = match &self.write_symbol {
            Some(writer) => format!("write {writer}\n"),
            None => String::new(),
        };
        let subscribe_line = match &self.subscribe_symbol {
            Some(subscribe) => format!("subscribe {subscribe}\n"),
            None => String::new(),
        };
        let callback_line = if self.callback.is_default() {
            String::new()
        } else {
            format!("callback {}\n", self.callback.descriptor_fields().join(" "))
        };
        format!(
            "schema {}\nabi handle {}\nsymbol {}\ntarget {}\ntype {}\nconstructor {}\n\
             destructor {}\n{}{}{}{}params {}\n",
            self.schema,
            self.abi_version,
            self.symbol,
            self.target,
            self.type_name,
            self.constructor,
            self.destructor,
            use_line,
            write_line,
            subscribe_line,
            callback_line,
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
        let mut write_symbol = None;
        let mut subscribe_symbol = None;
        let mut callback = CallbackPolicySpec::default();
        let mut callback_seen = false;
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
                "write" => write_symbol = Some(value.to_string()),
                "subscribe" => subscribe_symbol = Some(value.to_string()),
                "callback" => {
                    if callback_seen {
                        return Err("a handle block has more than one `callback` line".to_string());
                    }
                    callback_seen = true;
                    callback = CallbackPolicySpec::parse(value)?;
                }
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
            write_symbol,
            subscribe_symbol,
            callback,
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
/// callback-scoped borrow (`<target>__with`), the module-held view
/// (`<target>__borrow`/`__view`/`__release`), and — when the descriptor names a
/// provider accessor — the reader (`<target>__use`).
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

/// Move the counter past a generation that arrived from outside this process —
/// a cached adapter crate. A later build must mint a generation the reused
/// tables refuse, so the counter can never issue one they already used.
pub fn reserve_generation(generation: u32) {
    let mut current = PROVIDER_GENERATION.load(std::sync::atomic::Ordering::Relaxed);
    while current <= generation {
        match PROVIDER_GENERATION.compare_exchange(
            current,
            generation.saturating_add(1),
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
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
        "use austral_cranelift_bridge::handles::{\n    HandleRegistry, Owner, ProviderValue, STATUS_OK, STATUS_PANIC, STATUS_REFUSED,\n    TypeTag,\n};\n\n",
    );
    let words: Vec<String> = (0..adapter.params).map(|i| format!("w{i}: i64")).collect();
    let mut provider_calls = String::new();
    if let Some(accessor) = &adapter.use_symbol {
        provider_calls.push_str(&format!(
            "    // The provider's reader: one word out of the value it owns.\n    \
             fn {accessor}(ptr: *mut core::ffi::c_void) -> i64;\n"
        ));
    }
    if let Some(writer) = &adapter.write_symbol {
        provider_calls.push_str(&format!(
            "    // The provider's writer: one word into the value it owns.\n    \
             fn {writer}(ptr: *mut core::ffi::c_void, value: i64);\n"
        ));
    }
    out.push_str(&format!(
        "// `C-unwind`: a provider built with `panic=unwind` may unwind into the\n\
         // wrapper, which catches it and returns `STATUS_PANIC`. A\n\
         // `panic=abort` provider aborts the process — a worker crash, not a\n\
         // catchable status.\n\
         extern \"C-unwind\" {{\n    fn {}({}) -> *mut core::ffi::c_void;\n{}}}\n\n\
         // The destructor is called by `ProviderValue::drop`, whose signature\n\
         // is the plain C ABI; a panicking destructor is not caught here and\n\
         // is documented as a provider fault, not a status.\n\
         extern \"C\" {{\n    fn {}(ptr: *mut core::ffi::c_void);\n}}\n\n",
        adapter.constructor,
        words.join(", "),
        provider_calls,
        adapter.destructor
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
    // The live-reload seam. These forwarders are the symbols the JIT resolved
    // when it compiled the module, so a reload must not rename them: the host
    // binds them to the next build's implementations instead, and every later
    // call through the compiled module reaches the current table. A slot that
    // was never bound runs this build's own body, which is what the first
    // build does.
    let bind_symbol = adapter.bind_symbol();
    let dispatch_symbols = adapter.forwarded_symbols();
    out.push_str(&format!(
        "// The reload's indirection: the forwarders below stay the symbols the JIT\n\
         // resolved, and a reload binds them to the next build's implementation.\n\
         mod dispatch {{\n    \
         use std::sync::Mutex;\n\n    \
         static BINDINGS: Mutex<Vec<(&'static str, usize)>> = Mutex::new(Vec::new());\n\n    \
         pub fn lookup(name: &str) -> Option<usize> {{\n        \
         let table = match BINDINGS.lock() {{\n            Ok(table) => table,\n            \
         Err(poisoned) => poisoned.into_inner(),\n        }};\n        \
         // Last binding wins, so a second reload supersedes the first.\n        \
         table.iter().rev().find(|(n, _)| *n == name).map(|(_, p)| *p)\n    }}\n\n    \
         pub fn set(name: &'static str, ptr: usize) {{\n        \
         match BINDINGS.lock() {{\n            Ok(mut table) => table.push((name, ptr)),\n            \
         Err(poisoned) => poisoned.into_inner().push((name, ptr)),\n        }}\n    }}\n}}\n\n\
         // The host's binding point for this adapter: it names a forwarder this\n\
         // build rendered and the implementation a later build exports. An\n\
         // unknown name is refused — binding something nothing calls would look\n\
         // like a reload.\n\
         #[no_mangle]\npub extern \"C\" fn {bind_symbol}(\n    \
         name: *const u8,\n    len: usize,\n    ptr: *mut core::ffi::c_void,\n) -> i64 {{\n    \
         if name.is_null() || ptr.is_null() || len == 0 {{\n        return 1;\n    }}\n    \
         let bytes = unsafe {{ std::slice::from_raw_parts(name, len) }};\n    \
         let Ok(text) = std::str::from_utf8(bytes) else {{\n        return 1;\n    }};\n    \
         match text {{\n{}\n        _ => 1,\n    }}\n}}\n\n",
        dispatch_symbols
            .iter()
            .map(|symbol| format!(
                "        \"{symbol}\" => {{\n            dispatch::set(\"{symbol}\", ptr as usize);\n            0\n        }}"
            ))
            .collect::<Vec<_>>()
            .join("\n")
    ));
    // A wrapper is two functions: the generation-suffixed implementation and
    // the forwarder that finds it. `symbol` is the forwarder (the name the JIT
    // resolved), `impl_name` the local Rust name of the body.
    let forwarder = |symbol: &str, impl_name: &str, named: &str, types: &str, args: &str| {
        format!(
            "#[no_mangle]\npub extern \"C\" fn {symbol}({named}) -> i64 {{\n    \
             // Until a reload binds this slot, the build's own body answers.\n    \
             match dispatch::lookup(\"{symbol}\") {{\n        \
             Some(target) => {{\n            \
             let f: extern \"C\" fn({types}) -> i64 = unsafe {{ std::mem::transmute(target) }};\n            \
             f({args})\n        }}\n        \
             None => {impl_name}({args}),\n    }}\n}}\n\n"
        )
    };
    let mut args = String::new();
    let mut params = words.join(", ");
    if !params.is_empty() {
        params.push_str(", ");
    }
    params.push_str("out: *mut i64");
    for i in 0..adapter.params {
        args.push_str(&format!("w{i}, "));
    }
    let symbol = adapter.symbol.clone();
    let new_impl = format!("{}__impl", adapter.symbol);
    let mut fn_types: Vec<String> = (0..adapter.params).map(|_| "i64".to_string()).collect();
    fn_types.push("*mut i64".to_string());
    let new_call: Vec<String> = (0..adapter.params)
        .map(|i| format!("w{i}"))
        .chain(["out".to_string()])
        .collect();
    out.push_str(&format!(
        "#[export_name = \"{symbol}__impl_{generation}\"]\nextern \"C\" fn {new_impl}({params}) -> i64 {{\n\
         \x20   if out.is_null() {{\n        return STATUS_REFUSED;\n    }}\n\
         \x20   // A panicking constructor is caught before it crosses the ABI, so\n\
         \x20   // it is a status word, not a process-wide fault. Nothing was\n\
         \x20   // allocated that this table owns, so the out slot stays untouched.\n\
         \x20   let raw = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {{ {}({args}) }})) {{\n\
         \x20       Ok(raw) => raw,\n\
         \x20       Err(_) => return STATUS_PANIC,\n\
         \x20   }};\n\
         \x20   if raw.is_null() {{\n        return STATUS_REFUSED;\n    }}\n\
         \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   match table.insert(ProviderValue::new(raw, {})) {{\n        Ok(handle) => {{\n            let (lo, hi) = handle.to_words();\n            unsafe {{\n                *out = lo as i64;\n                *out.add(1) = hi as i64;\n            }}\n            STATUS_OK\n        }}\n        // The refused value is dropped here, so its destructor runs once and\n        // the provider is not leaked by a table that refused it.\n        Err(_) => STATUS_REFUSED,\n    }}\n}}\n\n",
        adapter.constructor, adapter.destructor
    ));
    out.push_str(&forwarder(
        &symbol,
        &new_impl,
        &params,
        &fn_types.join(", "),
        &new_call.join(", "),
    ));
    let use_symbol = adapter.use_symbol();
    let use_impl = format!("{use_symbol}__impl");
    let with_symbol = adapter.with_symbol();
    let with_impl = format!("{with_symbol}__impl");
    let dispose_symbol = adapter.dispose_symbol();
    let dispose_impl = format!("{dispose_symbol}__impl");
    if let Some(accessor) = &adapter.use_symbol {
        out.push_str(&format!(
            "#[export_name = \"{use_symbol}__impl_{generation}\"]\nextern \"C\" fn {use_impl}(lo: i64, hi: i64, out: *mut i64) -> i64 {{\n\
             \x20   if out.is_null() {{\n        return STATUS_REFUSED;\n    }}\n\
             \x20   let table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
             \x20   let handle = match table.handle_from_words(lo as u64, hi as u64) {{\n        Ok(handle) => handle,\n        Err(_) => return STATUS_REFUSED,\n    }};\n\
             \x20   let ptr = match table.get(handle) {{\n        Ok(value) => value.as_ptr(),\n        Err(_) => return STATUS_REFUSED,\n    }};\n\
             \x20   // The accessor is the provider's, called with the provider's own\n\
             \x20   // pointer while the table is locked, so the value cannot be\n\
             \x20   // destroyed underneath the read. A panicking reader is caught\n\
             \x20   // before it crosses the ABI and writes nothing.\n\
             \x20   let value = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {{ {}(ptr) }})) {{\n\
             \x20       Ok(value) => value,\n\
             \x20       Err(_) => return STATUS_PANIC,\n\
             \x20   }};\n\
             \x20   unsafe {{\n        *out = value;\n    }}\n\
             \x20   STATUS_OK\n}}\n\n",
            accessor
        ));
        out.push_str(&forwarder(
            &use_symbol,
            &use_impl,
            "lo: i64, hi: i64, out: *mut i64",
            "i64, i64, *mut i64",
            "lo, hi, out",
        ));
    }
    // The module-held view: `__borrow` mints a claim and writes its two-word
    // token, `__view` reads the provider's word through that token, and
    // `__release` discharges it. The claim outlives each call, so `__dispose`
    // is refused while the module holds the view.
    let borrow_symbol = adapter.borrow_symbol();
    let borrow_impl = format!("{borrow_symbol}__impl");
    out.push_str(&format!(
        "#[export_name = \"{borrow_symbol}__impl_{generation}\"]\nextern \"C\" fn {borrow_impl}(lo: i64, hi: i64, out: *mut i64) -> i64 {{\n\
         \x20   if out.is_null() {{\n        return STATUS_REFUSED;\n    }}\n\
         \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   match table.borrow_to_words(lo as u64, hi as u64) {{\n        Ok((w0, w1)) => {{\n            unsafe {{\n                *out = w0 as i64;\n                *out.add(1) = w1 as i64;\n            }}\n            STATUS_OK\n        }}\n        Err(_) => STATUS_REFUSED,\n    }}\n}}\n\n",
    ));
    out.push_str(&forwarder(
        &borrow_symbol,
        &borrow_impl,
        "lo: i64, hi: i64, out: *mut i64",
        "i64, i64, *mut i64",
        "lo, hi, out",
    ));
    // The exclusive view: only rendered when the descriptor names a writer, so
    // a provider that cannot write is never given a mutation wrapper.
    if let Some(writer) = &adapter.write_symbol {
        let borrow_mut_symbol = adapter.borrow_mut_symbol();
        let borrow_mut_impl = format!("{borrow_mut_symbol}__impl");
        out.push_str(&format!(
            "#[export_name = \"{borrow_mut_symbol}__impl_{generation}\"]\nextern \"C\" fn {borrow_mut_impl}(lo: i64, hi: i64, out: *mut i64) -> i64 {{\n\
             \x20   if out.is_null() {{\n        return STATUS_REFUSED;\n    }}\n\
             \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
             \x20   match table.borrow_mut_to_words(lo as u64, hi as u64) {{\n        Ok((w0, w1)) => {{\n            unsafe {{\n                *out = w0 as i64;\n                *out.add(1) = w1 as i64;\n            }}\n            STATUS_OK\n        }}\n        Err(_) => STATUS_REFUSED,\n    }}\n}}\n\n",
        ));
        out.push_str(&forwarder(
            &borrow_mut_symbol,
            &borrow_mut_impl,
            "lo: i64, hi: i64, out: *mut i64",
            "i64, i64, *mut i64",
            "lo, hi, out",
        ));
        let write_symbol = adapter.write_symbol();
        let write_impl = format!("{write_symbol}__impl");
        out.push_str(&format!(
            "#[export_name = \"{write_symbol}__impl_{generation}\"]\nextern \"C\" fn {write_impl}(w0: i64, w1: i64, value: i64) -> i64 {{\n\
             \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
             \x20   let ptr = match table.get_words_view_mut(w0 as u64, w1 as u64) {{\n        Ok(value) => value.as_ptr(),\n        Err(_) => return STATUS_REFUSED,\n    }};\n\
             \x20   // The writer runs under the claim the module holds, with the\n\
             \x20   // provider's own pointer, and its panic is caught before it\n\
             \x20   // crosses the ABI.\n\
             \x20   match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {{ {}(ptr, value) }})) {{\n\
             \x20       Ok(()) => STATUS_OK,\n\
             \x20       Err(_) => STATUS_PANIC,\n\
             \x20   }}\n}}\n\n",
            writer
        ));
        out.push_str(&forwarder(
            &write_symbol,
            &write_impl,
            "w0: i64, w1: i64, value: i64",
            "i64, i64, i64",
            "w0, w1, value",
        ));
    }
    let view_symbol = adapter.view_symbol();
    let view_impl = format!("{view_symbol}__impl");
    let release_symbol = adapter.release_symbol();
    let release_impl = format!("{release_symbol}__impl");
    if let Some(accessor) = &adapter.use_symbol {
        out.push_str(&format!(
            "#[export_name = \"{view_symbol}__impl_{generation}\"]\nextern \"C\" fn {view_impl}(w0: i64, w1: i64, out: *mut i64) -> i64 {{\n\
             \x20   if out.is_null() {{\n        return STATUS_REFUSED;\n    }}\n\
             \x20   let table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
             \x20   let ptr = match table.get_words_view(w0 as u64, w1 as u64) {{\n        Ok(value) => value.as_ptr(),\n        Err(_) => return STATUS_REFUSED,\n    }};\n\
             \x20   let value = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {{ {}(ptr) }})) {{\n\
             \x20       Ok(value) => value,\n\
             \x20       Err(_) => return STATUS_PANIC,\n\
             \x20   }};\n\
             \x20   unsafe {{\n        *out = value;\n    }}\n\
             \x20   STATUS_OK\n}}\n\n",
            accessor
        ));
        out.push_str(&forwarder(
            &view_symbol,
            &view_impl,
            "w0: i64, w1: i64, out: *mut i64",
            "i64, i64, *mut i64",
            "w0, w1, out",
        ));
    }
    out.push_str(&format!(
        "#[export_name = \"{release_symbol}__impl_{generation}\"]\nextern \"C\" fn {release_impl}(w0: i64, w1: i64) -> i64 {{\n\
         \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   match table.release_words(w0 as u64, w1 as u64) {{\n        Ok(()) => STATUS_OK,\n        Err(_) => STATUS_REFUSED,\n    }}\n}}\n\n",
    ));
    out.push_str(&forwarder(
        &release_symbol,
        &release_impl,
        "w0: i64, w1: i64",
        "i64, i64",
        "w0, w1",
    ));
    // The descriptor's `callback` policy, with each wrapper's own default for
    // a field it does not name. `__with` is always call-scoped; only the
    // retained API can be `instance`, which is why `lifetime` gates that block
    // below rather than changing this one.
    let with_thread = match adapter.callback.thread {
        Some(CallbackThread::Any) => "ThreadAffinity::AnyThread",
        _ => "ThreadAffinity::CallerThread",
    };
    let with_reentrancy = match adapter.callback.reentrancy {
        Some(CallbackReentrancy::Nested) => "Reentrancy::Nested",
        _ => "Reentrancy::Forbidden",
    };
    let with_error = match adapter.callback.error {
        Some(CallbackErrors::Retire) => "ErrorPolicy::RetireOnPanic",
        _ => "ErrorPolicy::StatusWord",
    };
    out.push_str(&format!(
        "static CALLBACKS: std::sync::OnceLock<\
         austral_cranelift_bridge::callbacks::CallbackRegistry> =\n\
         \x20   std::sync::OnceLock::new();\n\n\
         fn callbacks() -> &'static austral_cranelift_bridge::callbacks::CallbackRegistry {{\n\
         \x20   CALLBACKS.get_or_init(austral_cranelift_bridge::callbacks::CallbackRegistry::new)\n}}\n\n\
         #[export_name = \"{with_symbol}__impl_{generation}\"]\nextern \"C-unwind\" fn {with_impl}(lo: i64, hi: i64,\n\
         \x20   callback: Option<unsafe extern \"C-unwind\" fn(ptr: *mut core::ffi::c_void) -> i64>,\n\
         \x20   out: *mut i64,\n\
         ) -> i64 {{\n\
         \x20   use austral_cranelift_bridge::callbacks::{{\n        CallbackError, CallbackPolicy, ErrorPolicy, Lifetime, Reentrancy, ThreadAffinity,\n    }};\n\
         \x20   if out.is_null() {{\n        return STATUS_REFUSED;\n    }}\n\
         \x20   let Some(callback) = callback else {{\n        return STATUS_REFUSED;\n    }};\n\
         \x20   // The value is *claimed*, not locked: `borrow_to_words` takes an\n\
         \x20   // immutable claim that every mutating or destroying wrapper refuses,\n\
         \x20   // and the table's mutex is dropped before the callback runs. A nested\n\
         \x20   // call into this registry therefore reaches the claim check instead of\n\
         \x20   // deadlocking, and a callback that would mutate the lent value is\n\
         \x20   // refused by the claim. The callback itself runs through a checked\n\
         \x20   // token: call-scoped, one-shot, caller-thread, no reentrancy, and a\n\
         \x20   // panic is a status word rather than an unwind across the ABI.\n\
         \x20   let (ptr, view_id, owner_tag) = {{\n        let mut table = match REGISTRY.lock() {{\n            Ok(table) => table,\n            Err(poisoned) => poisoned.into_inner(),\n        }};\n\
         \x20       let handle = match table.handle_from_words(lo as u64, hi as u64) {{\n            Ok(handle) => handle,\n            Err(_) => return STATUS_REFUSED,\n        }};\n\
         \x20       let ptr = match table.get(handle) {{\n            Ok(value) => value.as_ptr(),\n            Err(_) => return STATUS_REFUSED,\n        }};\n\
         \x20       let (view_id, owner_tag) = match table.borrow_to_words(lo as u64, hi as u64) {{\n            Ok(words) => words,\n            Err(_) => return STATUS_REFUSED,\n        }};\n        \
         (ptr, view_id, owner_tag)\n    }};\n\
         \x20   let scope = match callbacks().open(CallbackPolicy {{\n        lifetime: Lifetime::Call,\n        generation: PROVIDER_GENERATION,\n        thread: {with_thread},\n        reentrancy: {with_reentrancy},\n        error: {with_error},\n    }}) {{\n\
         \x20       Ok(scope) => scope,\n\
         \x20       Err(_) => {{\n            let mut table = match REGISTRY.lock() {{\n                Ok(table) => table,\n                Err(poisoned) => poisoned.into_inner(),\n            }};\n            \
         let _ = table.release_words(view_id, owner_tag);\n            return STATUS_REFUSED;\n        }}\n    }};\n\
         \x20   let status = match scope.register(callback, ptr) {{\n        Ok(token) => match scope.invoke(token) {{\n            Ok(value) => {{\n                unsafe {{\n                    *out = value;\n                }}\n                STATUS_OK\n            }}\n            Err(CallbackError::Panicked) => STATUS_PANIC,\n            Err(_) => STATUS_REFUSED,\n        }},\n        Err(_) => STATUS_REFUSED,\n    }};\n\
         \x20   drop(scope);\n\
         \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   let _ = table.release_words(view_id, owner_tag);\n\
         \x20   status\n}}\n\n",
    ));
    out.push_str(&forwarder(
        &with_symbol,
        &with_impl,
        "lo: i64, hi: i64, callback: Option<unsafe extern \"C-unwind\" fn(ptr: *mut core::ffi::c_void) -> i64>, out: *mut i64",
        "i64, i64, Option<unsafe extern \"C-unwind\" fn(*mut core::ffi::c_void) -> i64>, *mut i64",
        "lo, hi, callback, out",
    ));
    if adapter.callback.lifetime != Some(CallbackLifetime::Call) {
    let retain_thread = match adapter.callback.thread {
        Some(CallbackThread::Any) => "ThreadAffinity::AnyThread",
        _ => "ThreadAffinity::CallerThread",
    };
    let retain_reentrancy = match adapter.callback.reentrancy {
        Some(CallbackReentrancy::Forbidden) => "Reentrancy::Forbidden",
        _ => "Reentrancy::Nested",
    };
    let retain_error = match adapter.callback.error {
        Some(CallbackErrors::Status) => "ErrorPolicy::StatusWord",
        _ => "ErrorPolicy::RetireOnPanic",
    };
    // Retained callbacks: a provider that keeps the function pointer for the
    // life of the instance. `__retain` opens an *instance* scope at this
    // build's generation and registers the callback. With a `subscribe`
    // symbol in the descriptor it also hands the provider a dispatch
    // trampoline plus an opaque slot; the provider owns that pair for the
    // instance and calls it when the event arrives. `__invoke_retained` drives
    // the same trampoline from the host, and `__retire_callbacks` is the
    // reload hook: a retired generation closes the scope, so even a provider
    // that still holds the slot is refused before the callback runs.
    let retain_symbol = format!("{with_symbol}__retain");
    let retain_impl = format!("{retain_symbol}__impl");
    let invoke_symbol = format!("{with_symbol}__invoke_retained");
    let invoke_impl = format!("{invoke_symbol}__impl");
    let retire_symbol = format!("{with_symbol}__retire_callbacks");
    let retire_impl = format!("{retire_symbol}__impl");
    let (subscribe_decl, subscribe_call) = match &adapter.subscribe_symbol {
        Some(subscribe) => (
            format!(
                "extern \"C-unwind\" {{\n    // The provider's subscription entry point: it receives this build's\n    // dispatch trampoline and the opaque slot, and owns both for the\n    // instance. It must stop calling after the retirement hook runs; the\n    // slot is an id, so even a stale call is refused instead of jumping\n    // into replaced code.\n    fn {subscribe}(owner: *mut core::ffi::c_void, dispatch: Dispatch, slot: *mut core::ffi::c_void) -> i64;\n}}\n"
            ),
            r#"    let status = unsafe { @SUBSCRIBE@(ptr, dispatch, slot as usize as *mut core::ffi::c_void) };
    if status != 0 {
        // The provider refused the subscription: drop the entry and close the
        // scope, so no token survives a failed handoff.
        let mut table = match RETAINED.lock() {
            Ok(table) => table,
            Err(poisoned) => poisoned.into_inner(),
        };
        table.retain(|entry| entry.0 != slot);
        drop(table);
        scope.close();
        return STATUS_REFUSED;
    }
    STATUS_OK
"#
            .replace("@SUBSCRIBE@", subscribe),
        ),
        None => (String::new(), "    STATUS_OK\n".to_string()),
    };
    let retained = r#"
type Retained = (u64, u64, u64, std::sync::Arc<austral_cranelift_bridge::callbacks::CallbackScope>, austral_cranelift_bridge::callbacks::CallbackToken);

static RETAINED: std::sync::Mutex<Vec<Retained>> = std::sync::Mutex::new(Vec::new());
static NEXT_SLOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

// The native side calls this with the opaque slot the subscription handed it.
// The slot is an id, never a pointer into this crate's memory, so a provider
// that keeps it after retirement can call it safely and be refused.
type Dispatch = unsafe extern "C-unwind" fn(slot: *mut core::ffi::c_void, out: *mut i64) -> i64;

fn dispatch_slot(slot: u64, out: *mut i64) -> i64 {
    if out.is_null() {
        return STATUS_REFUSED;
    }
    let entry = {
        let table = match RETAINED.lock() {
            Ok(table) => table,
            Err(poisoned) => poisoned.into_inner(),
        };
        table
            .iter()
            .find(|entry| entry.0 == slot)
            .map(|entry| (std::sync::Arc::clone(&entry.3), entry.4))
    };
    let Some((scope, token)) = entry else {
        return STATUS_REFUSED;
    };
    match scope.invoke(token) {
        Ok(value) => {
            unsafe {
                *out = value;
            }
            STATUS_OK
        }
        Err(austral_cranelift_bridge::callbacks::CallbackError::Panicked) => STATUS_PANIC,
        Err(_) => STATUS_REFUSED,
    }
}

#[allow(dead_code)]
unsafe extern "C-unwind" fn dispatch(slot: *mut core::ffi::c_void, out: *mut i64) -> i64 {
    dispatch_slot(slot as usize as u64, out)
}

@SUBSCRIBE_DECL@
#[export_name = "@RETAIN@"]
pub extern "C-unwind" fn @RETAIN_IMPL@(lo: i64, hi: i64,
    callback: Option<unsafe extern "C-unwind" fn(ptr: *mut core::ffi::c_void) -> i64>,
) -> i64 {
    use austral_cranelift_bridge::callbacks::{
        CallbackPolicy, ErrorPolicy, Lifetime, Reentrancy, ThreadAffinity,
    };
    let Some(callback) = callback else {
        return STATUS_REFUSED;
    };
    // The callback is retained for the instance, so the pointer it receives
    // must outlive the call: it is the provider's value, not a stack
    // temporary. The registry is only borrowed to resolve it.
    let ptr = {
        let table = match REGISTRY.lock() {
            Ok(table) => table,
            Err(poisoned) => poisoned.into_inner(),
        };
        let handle = match table.handle_from_words(lo as u64, hi as u64) {
            Ok(handle) => handle,
            Err(_) => return STATUS_REFUSED,
        };
        match table.get(handle) {
            Ok(value) => value.as_ptr(),
            Err(_) => return STATUS_REFUSED,
        }
    };
    let scope = match callbacks().open(CallbackPolicy {
        lifetime: Lifetime::Instance,
        generation: PROVIDER_GENERATION,
        thread: @RETAIN_THREAD@,
        reentrancy: @RETAIN_REENTRANCY@,
        error: @RETAIN_ERROR@,
    }) {
        Ok(scope) => std::sync::Arc::new(scope),
        Err(_) => return STATUS_REFUSED,
    };
    let token = match scope.register(callback, ptr) {
        Ok(token) => token,
        Err(_) => return STATUS_REFUSED,
    };
    let slot = NEXT_SLOT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    {
        let mut table = match RETAINED.lock() {
            Ok(table) => table,
            Err(poisoned) => poisoned.into_inner(),
        };
        table.push((slot, lo as u64, hi as u64, std::sync::Arc::clone(&scope), token));
    }
@SUBSCRIBE_CALL@}

#[export_name = "@INVOKE@"]
pub extern "C-unwind" fn @INVOKE_IMPL@(lo: i64, hi: i64, out: *mut i64) -> i64 {
    if out.is_null() {
        return STATUS_REFUSED;
    }
    let slot = {
        let table = match RETAINED.lock() {
            Ok(table) => table,
            Err(poisoned) => poisoned.into_inner(),
        };
        table
            .iter()
            .find(|entry| entry.1 == lo as u64 && entry.2 == hi as u64)
            .map(|entry| entry.0)
    };
    match slot {
        Some(slot) => dispatch_slot(slot, out),
        None => STATUS_REFUSED,
    }
}

// The reload hook: close every retained scope of this generation or older and
// retire the generation in the registry, so a provider that kept the slot
// cannot call into replaced JIT code.
#[export_name = "@RETIRE@"]
pub extern "C" fn @RETIRE_IMPL@(generation: u32) -> i64 {
    let mut table = match RETAINED.lock() {
        Ok(table) => table,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut kept = Vec::with_capacity(table.len());
    for entry in table.drain(..) {
        if entry.3.generation() <= generation {
            entry.3.close();
        } else {
            kept.push(entry);
        }
    }
    *table = kept;
    drop(table);
    callbacks().retire_generations_through(generation);
    STATUS_OK
}
"#
    .replace("@SUBSCRIBE_DECL@", &subscribe_decl)
    .replace("@SUBSCRIBE_CALL@", &subscribe_call)
    .replace("@RETAIN_IMPL@", &retain_impl)
    .replace("@INVOKE_IMPL@", &invoke_impl)
    .replace("@RETIRE_IMPL@", &retire_impl)
    .replace("@RETAIN@", &retain_symbol)
    .replace("@INVOKE@", &invoke_symbol)
    .replace("@RETIRE@", &retire_symbol)
    .replace("@RETAIN_THREAD@", retain_thread)
    .replace("@RETAIN_REENTRANCY@", retain_reentrancy)
    .replace("@RETAIN_ERROR@", retain_error);
    out.push_str(&retained);
    }
    out.push_str(&format!(
        "#[export_name = \"{dispose_symbol}__impl_{generation}\"]\nextern \"C\" fn {dispose_impl}(lo: i64, hi: i64) -> i64 {{\n\
         \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   match table.destroy_words(lo as u64, hi as u64) {{\n        Ok(()) => STATUS_OK,\n        Err(_) => STATUS_REFUSED,\n    }}\n}}\n",
    ));
    out.push_str(&forwarder(
        &dispose_symbol,
        &dispose_impl,
        "lo: i64, hi: i64",
        "i64, i64",
        "lo, hi",
    ));
    // The generation-replacement hook. A reload builds a new table and the old
    // one is never unmapped, so without this every value the old table still
    // holds would stay owned for the life of the process. Draining runs the
    // provider's destructors; a live view refuses the drain, so the reload
    // fails instead of freeing a value a module can still read.
    let drain_symbol = format!("{}__drain", adapter.target);
    let drain_impl = format!("{drain_symbol}__impl");
    out.push_str(&format!(
        "// Destroy every value this build's table still holds. Called once, by the
\
         // reload, before the previous generation is retired.\n\
         #[export_name = \"{drain_symbol}\"]\n\
         pub extern \"C\" fn {drain_impl}() -> i64 {{\n\
         \x20   let mut table = match REGISTRY.lock() {{\n        Ok(table) => table,\n        Err(poisoned) => poisoned.into_inner(),\n    }};\n\
         \x20   match table.drain() {{\n        Ok(_) => STATUS_OK,\n        Err(_) => STATUS_REFUSED,\n    }}\n}}\n",
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

    /// Every symbol this adapter's rendered crate exports. A checked scalar
    /// adapter renders the payload wrapper *and* `<symbol>__checked`, and the
    /// JIT can resolve either, so a local map must register both.
    pub fn exported_symbols(&self) -> Vec<String> {
        match self {
            GeneratedAdapter::Scalar(a) if a.checked_union.is_some() => {
                vec![
                    format!("{}__checked", a.symbol),
                    format!("{}__free", a.symbol),
                ]
            }
            GeneratedAdapter::Scalar(a) if a.checked => {
                vec![
                    a.symbol.clone(),
                    format!("{}__checked", a.symbol),
                    format!("{}__free", a.symbol),
                ]
            }
            _ => vec![self.symbol().to_string()],
        }
    }

    /// Symbols a reload calls on the *previous* build. A handle adapter keeps
    /// retained callbacks; its retirement hook closes them so a provider that
    /// kept a pointer cannot call into JIT code that is gone. Every handle
    /// adapter also exposes its table's drain hook, so the values the replaced
    /// generation still owns are destroyed there instead of leaked.
    pub fn retirement_symbols(&self) -> Vec<String> {
        match self {
            GeneratedAdapter::Handle(a) if a.callback.lifetime != Some(CallbackLifetime::Call) => {
                vec![format!("{}__retire_callbacks", a.with_symbol())]
            }
            _ => Vec::new(),
        }
    }

    /// Symbols a reload calls on the *previous* build so its live values run
    /// their provider destructors. Empty for scalar and buffer adapters, which
    /// hold no table.
    pub fn drain_symbols(&self) -> Vec<String> {
        match self {
            GeneratedAdapter::Handle(a) => vec![format!("{}__drain", a.target)],
            _ => Vec::new(),
        }
    }

    /// Render for one provider build. The generation only reaches the handle
    /// adapters: scalar and buffer wrappers carry no handle table, so a reload
    /// changes nothing they resolve.
    pub fn render_at(&self, generation: u32) -> Result<String, String> {
        match self {
            GeneratedAdapter::Scalar(a) if a.checked_union.is_some() => {
                // A two-payload union has no payload-only wrapper: its
                // provider writes the tag as well, so the descriptor names
                // only `__checked`.
                render_wrapper_checked_union(a)
            }
            GeneratedAdapter::Scalar(a) if a.checked => {
                // Each wrapper brings its own `use` list and its own `extern`
                // declaration of the provider symbol, so they cannot share a
                // module: two checked adapters would also redeclare each
                // other's imports.
                let payload = render_wrapper(a)?;
                let checked = render_wrapper_checked_option(a)?;
                Ok(format!(
                    "mod payload {{\n{payload}\n}}\nmod checked {{\n{checked}\n}}"
                ))
            }
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
            &buffer_text("read:0:2", "Nat64", "null-len0").replace("abi buffer 1", "abi buffer 3"),
        )
        .unwrap_err();
        assert!(err.contains("buffer transport version 3"), "{err}");
        let err = parse_buffer(
            &buffer_text("read:0:2", "Nat64", "null-len0").replace("empty null-len0\n", ""),
        )
        .unwrap_err();
        assert!(err.contains("no `empty` line"), "{err}");
    }

    // ── U12 version 2: word scalars beside a buffer ────────────────────────

    fn buffer_scalar_text() -> String {
        "schema 1\nabi buffer 2\nsymbol rs_gpu_write__adapter\ntarget rs_gpu_write\n\
         params 5\nlengths Int64\nbuffers readwrite:3:4\n\
         scalars 0:Int64,1:Int64,2:Int64\nempty null-len0\nreturns Int64\n"
            .to_string()
    }

    #[test]
    fn a_version_2_block_round_trips_with_its_scalars() {
        let text = buffer_scalar_text();
        let adapters = parse_buffer(&text).expect("parse");
        assert_eq!(adapters.len(), 1);
        let adapter = &adapters[0];
        assert_eq!(adapter.abi_version, 2);
        assert_eq!(
            adapter.scalars,
            vec![
                ScalarParam {
                    index: 0,
                    ty: Scalar::I64
                },
                ScalarParam {
                    index: 1,
                    ty: Scalar::I64
                },
                ScalarParam {
                    index: 2,
                    ty: Scalar::I64
                },
            ]
        );
        assert_eq!(adapter.descriptor_text(), text);
    }

    #[test]
    fn a_scalar_is_passed_through_and_never_read_as_a_length() {
        let text = buffer_scalar_text();
        let adapter = &parse_buffer(&text).expect("parse")[0];
        let wrapper = render_buffer_wrapper(adapter).expect("wrapper");
        assert!(
            wrapper.contains("fn rs_gpu_write(s0: i64, s1: i64, s2: i64, buf3: *mut u8, len4: i64) -> i64;"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("unsafe { rs_gpu_write(w0 as i64, w1 as i64, w2 as i64, buf3, len4) }"),
            "{wrapper}"
        );
        assert!(
            !wrapper.contains("let len0") && !wrapper.contains("w0 < 0"),
            "a scalar must not acquire a length check: {wrapper}"
        );
        assert!(wrapper.contains("let len4 = w4 as i64;"), "{wrapper}");
    }

    #[test]
    fn a_scalar_in_a_version_1_block_is_refused() {
        let text = buffer_scalar_text().replace("abi buffer 2", "abi buffer 1");
        let err = parse_buffer(&text).unwrap_err();
        assert!(err.contains("version 2"), "{err}");
    }

    #[test]
    fn every_version_2_parameter_must_be_accounted_for() {
        let text = buffer_scalar_text().replace("scalars 0:Int64,1:Int64,2:Int64", "scalars 0:Int64");
        let err = parse_buffer(&text).unwrap_err();
        assert!(err.contains("leaves parameter 1 unexplained"), "{err}");
    }

    #[test]
    fn a_narrow_scalar_beside_a_buffer_is_refused() {
        let text = buffer_scalar_text().replace("scalars 0:Int64,1:Int64,2:Int64", "scalars 0:Int32,1:Int64,2:Int64");
        let err = parse_buffer(&text).unwrap_err();
        assert!(err.contains("64-bit integer"), "{err}");
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
        assert!(
            wrapper.contains(
                "pub extern \"C\" fn rs_widget__borrow(lo: i64, hi: i64, out: *mut i64) -> i64"
            ),
            "the module can hold a view: {wrapper}"
        );
        assert!(
            wrapper.contains("table.borrow_to_words(lo as u64, hi as u64)"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("pub extern \"C\" fn rs_widget__release(w0: i64, w1: i64) -> i64"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains("table.release_words(w0 as u64, w1 as u64)"),
            "{wrapper}"
        );
        assert!(
            !wrapper.contains("__view"),
            "a descriptor without a reader renders no view wrapper"
        );
        // U15: the status is a named word, and every provider call is caught.
        assert!(
            wrapper.contains("extern \"C-unwind\""),
            "a panicking provider must be able to unwind into the wrapper: {wrapper}"
        );
        assert!(
            wrapper.contains("std::panic::catch_unwind"),
            "provider calls are caught before the ABI: {wrapper}"
        );
        assert!(
            wrapper.contains("return STATUS_PANIC,"),
            "a caught panic has its own status: {wrapper}"
        );
        assert!(
            wrapper.contains("return STATUS_REFUSED;"),
            "host refusals use the checked status words: {wrapper}"
        );
        assert!(
            wrapper.contains("STATUS_OK"),
            "success is a named status, not a bare zero: {wrapper}"
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
            wrapper.contains("scope.register(callback, ptr)"),
            "the callback is registered behind a checked token: {wrapper}"
        );
        assert!(
            wrapper.contains("table.borrow_to_words(lo as u64, hi as u64)"),
            "the callback runs under an immutable claim, not the table lock: {wrapper}"
        );
        assert!(
            wrapper.contains("let _ = table.release_words(view_id, owner_tag);"),
            "the claim is released after the callback, on every path: {wrapper}"
        );
        assert!(
            wrapper.find("let scope = match callbacks().open(").expect("scope")
                > wrapper.find("borrow_to_words").expect("claim"),
            "the table lock is dropped before the callback scope opens: {wrapper}"
        );
        assert!(
            wrapper
                .rfind("release_words(view_id, owner_tag)")
                .expect("release")
                > wrapper.find("scope.invoke(token)").expect("invoke"),
            "the claim is released after the callback has run: {wrapper}"
        );
        assert!(
            wrapper.contains("lifetime: Lifetime::Call"),
            "the borrow is call-scoped: {wrapper}"
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
            wrapper.contains("AssertUnwindSafe(|| unsafe { rs_widget_read(ptr) })"),
            "the reader is called under the panic boundary: {wrapper}"
        );
        assert!(
            wrapper.contains(
                "pub extern \"C\" fn rs_widget__view(w0: i64, w1: i64, out: *mut i64) -> i64"
            ),
            "reading through a held view is rendered beside the reader: {wrapper}"
        );
        assert!(
            wrapper.contains("table.get_words_view(w0 as u64, w1 as u64)"),
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

    fn handle_text_with_write(target: &str) -> String {
        handle_text(target).replace(
            "destructor rs_widget_dtor\n",
            "destructor rs_widget_dtor\nuse rs_widget_read\nwrite rs_widget_write\n",
        )
    }

    #[test]
    fn a_write_line_adds_the_exclusive_view_wrappers() {
        let text = handle_text_with_write("rs_widget");
        let adapter = parse_handle(&text).expect("parse").remove(0);
        assert_eq!(adapter.write_symbol.as_deref(), Some("rs_widget_write"));
        assert_eq!(adapter.write_symbol(), "rs_widget__write");
        assert_eq!(
            adapter.descriptor_text(),
            text,
            "the writer round-trips in place, after `use`"
        );

        let wrapper = render_handle_wrapper(&adapter).expect("wrapper");
        assert!(
            wrapper.contains("fn rs_widget_write(ptr: *mut core::ffi::c_void, value: i64);"),
            "the provider's writer is declared: {wrapper}"
        );
        assert!(
            wrapper.contains(
                "pub extern \"C\" fn rs_widget__borrow_mut(lo: i64, hi: i64, out: *mut i64) -> i64"
            ),
            "the module can mint an exclusive token: {wrapper}"
        );
        assert!(
            wrapper.contains("table.borrow_mut_to_words(lo as u64, hi as u64)"),
            "{wrapper}"
        );
        assert!(
            wrapper.contains(
                "pub extern \"C\" fn rs_widget__write(w0: i64, w1: i64, value: i64) -> i64"
            ),
            "the module can write through its token: {wrapper}"
        );
        assert!(
            wrapper.contains("table.get_words_view_mut(w0 as u64, w1 as u64)"),
            "only a mutable token reaches the mutation path: {wrapper}"
        );
        assert!(
            wrapper.contains("AssertUnwindSafe(|| unsafe { rs_widget_write(ptr, value) })"),
            "a panicking writer is caught: {wrapper}"
        );

        // A descriptor without a writer renders no mutation wrappers, so a
        // provider that cannot write is never given a way to be asked.
        let read_only = parse_handle(&handle_text_with_use("rs_widget"))
            .expect("parse")
            .remove(0);
        let read_only_wrapper = render_handle_wrapper(&read_only).expect("wrapper");
        assert!(!read_only_wrapper.contains("__borrow_mut"));
        assert!(!read_only_wrapper.contains("fn rs_widget__write"));
    }

    #[test]
    fn a_write_line_must_be_a_distinct_provider_symbol() {
        let mut adapter = parse_handle(&handle_text_with_write("rs_widget"))
            .expect("parse")
            .remove(0);
        adapter.write_symbol = Some("rs_widget_read".to_string());
        let err = adapter.validate().unwrap_err();
        assert!(
            err.contains("also the constructor, destructor or accessor"),
            "{err}"
        );

        let mut adapter = parse_handle(&handle_text_with_write("rs_widget"))
            .expect("parse")
            .remove(0);
        adapter.write_symbol = Some("rs write".to_string());
        let err = adapter.validate().unwrap_err();
        assert!(err.contains("not a plain identifier"), "{err}");
    }

    #[test]
    fn a_checked_option_descriptor_renders_a_tagged_union_wrapper() {
        let text = "schema 1\nabi scalar 1\nsymbol rs_lookup__adapter\ntarget \
                    rs_lookup\nparams Int64\nreturns Int64\nstatus checked\nfailure 0\n\
                    success 1\n";
        let adapter = Adapter::parse_descriptors(text).expect("parse").remove(0);
        assert!(adapter.checked);
        assert_eq!(adapter.checked_tags, Some((0, 1)));
        assert_eq!(
            adapter.descriptor_text(),
            text,
            "the checked lines round-trip in canonical order"
        );

        let wrapper = render_wrapper_checked_option(&adapter).expect("render");
        assert!(
            wrapper.contains("static OPTION_FAILURE: [u64; 2] = [0, 0];"),
            "the failure tag is the declared one: {wrapper}"
        );
        assert!(
            wrapper.contains(
                "pub extern \"C\" fn rs_lookup__adapter__checked(word0: u64) -> *mut u64"
            ),
            "the checked wrapper returns the union pointer: {wrapper}"
        );
        assert!(
            wrapper.contains("Box::into_raw(Box::new([1, encoded])) as *mut u64"),
            "success writes the success tag and the payload: {wrapper}"
        );
        assert!(
            wrapper.contains("_ => return option_failure(),"),
            "a bad word fails as the tagged None: {wrapper}"
        );
        assert!(
            wrapper.matches("Err(_) => return option_failure(),").count() >= 2,
            "a panic and an unencodable payload both fail as the tagged None: {wrapper}"
        );

        let both = GeneratedAdapter::Scalar(&adapter)
            .render()
            .expect("render both");
        assert!(
            both.contains("fn rs_lookup__adapter(word0: u64) -> u64"),
            "the payload wrapper stays beside the checked one: {both}"
        );
        assert!(
            both.contains("fn rs_lookup__adapter__checked(word0: u64) -> *mut u64"),
            "the checked wrapper is added when the compiler selects it: {both}"
        );

        for bad in [
            text.replace("failure 0\nsuccess 1\n", ""),
            text.replace("failure 0\nsuccess 1\n", "failure 1\nsuccess 1\n"),
            text.replace("status checked", "status unchecked"),
            text.replace("failure 0\n", ""),
            text.replace("failure 0", "failure 2"),
        ] {
            assert!(
                Adapter::parse_descriptors(&bad).is_err(),
                "a checked descriptor without distinct tags is refused: {bad}"
            );
        }
    }

    #[test]
    fn checked_wrappers_report_a_status_and_write_the_payload_out() {
        let checked = render_wrapper_checked(&Adapter::new(
            "rs_scale__adapter",
            "rs_scale",
            vec![Scalar::F64, Scalar::I64],
            Scalar::F64,
        ))
        .expect("render");
        assert!(
            checked.contains(
                "fn rs_scale__adapter__checked(word0: u64, word1: u64, out: *mut u64) -> i64"
            ),
            "{checked}"
        );
        assert!(checked.contains(
            "use austral_cranelift_bridge::handles::{STATUS_OK, STATUS_PANIC, STATUS_REFUSED}"
        ));
        assert!(
            checked.contains("return STATUS_REFUSED"),
            "a refused word is a status"
        );
        assert!(checked.contains("Err(_) => return STATUS_PANIC"));
        assert!(checked.contains("unsafe { *out = encoded; }"));
        assert!(checked.contains("extern \"C-unwind\" {"));
        // The payload wrapper keeps its ABI and its abort semantics.
        let payload = render_wrapper(&Adapter::new(
            "rs_scale__adapter",
            "rs_scale",
            vec![Scalar::F64, Scalar::I64],
            Scalar::F64,
        ))
        .expect("render");
        assert!(!payload.contains("__checked"));
        assert!(payload.contains("extern \"C\" {"));
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
