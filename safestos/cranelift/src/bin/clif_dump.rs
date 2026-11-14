//! `clif-dump` — what Cranelift IR does the CPS lowering actually produce?
//!
//! `scripts/clif2mm0` turns this measurement into `mm0/clif.mm0` (W9.2): one
//! declaration per instruction form the bridge really emits, so the *shape* of
//! the lowering's output is a measured fact rather than a reading of the
//! Cranelift manual. The object digest says "the bytes did not change"; this
//! says "these are the instruction forms those bytes were made of".
//!
//! The measurement is taken where the lowering runs: `cps::compile_cps_to_clif`
//! with the [`CaptureModule`] backend, i.e. the same parser and the same CLIF
//! the JIT consumes, with nothing executed and no code published. The wrapping
//! module only intercepts `Context::func` at define time, before Cranelift's
//! backend lowers it to machine code.
//!
//! Output is line-oriented, not CLIF text: parsing a *displayed* function would
//! make this tool depend on Cranelift's pretty-printer, and the printer is not
//! what proves anything. Each instruction is one line —
//!
//! ```text
//! inst iadd 2 1 i64,i64 -> i64
//! inst brif 1 0 i8 -> none
//! ```
//!
//! (mnemonic, operand count, result count, operand types `->` result types) —
//! which is exactly what a shape model needs, and nothing that depends on SSA
//! numbering, so the output is deterministic across runs and across renames.
//!
//! Usage: `clif-dump [--allow-all] <module.cps>...`
//! Exit: 0 measured at least one instruction; 1 inconsistent input; 2 could not
//! run. A module that compiles to no instruction is a failure, not an empty
//! measurement — the same rule `isa-census` applies to opcode reads.

use austral_cranelift_bridge::capture::CaptureModule;
use austral_cranelift_bridge::cps;
use cranelift_codegen::ir;
use cranelift_codegen::isa;
use cranelift_codegen::Context;
use cranelift_control::ControlPlane;
use cranelift_module::{
    DataId, FuncId, Linkage, Module, ModuleDeclarations, ModuleReloc, ModuleResult,
};
use std::path::PathBuf;

/// A capture backend that also records each function's CLIF instruction shapes
/// at the moment it is defined. Every other method is the capture backend's,
/// unchanged — this is a measuring instrument, not a second compiler.
struct MeasuringModule {
    inner: CaptureModule,
    lines: Vec<String>,
}

impl MeasuringModule {
    fn new() -> Result<Self, String> {
        Ok(MeasuringModule {
            inner: CaptureModule::new()?,
            lines: Vec::new(),
        })
    }

    fn measure(&mut self, id: FuncId, func: &ir::Function) {
        let name = self.inner.func_name(id);
        let mut n_insts = 0usize;
        for block in func.layout.blocks() {
            for inst in func.layout.block_insts(block) {
                let mnemonic = func.dfg.insts[inst].opcode().to_string();
                let args = func.dfg.inst_args(inst);
                let results = func.dfg.inst_results(inst);
                let arg_types: Vec<String> = args
                    .iter()
                    .map(|v| func.dfg.value_type(*v).to_string())
                    .collect();
                let result_types: Vec<String> = results
                    .iter()
                    .map(|v| func.dfg.value_type(*v).to_string())
                    .collect();
                let fmt = |ts: &[String]| {
                    if ts.is_empty() {
                        "none".to_string()
                    } else {
                        ts.join(",")
                    }
                };
                self.lines.push(format!(
                    "inst {mnemonic} {} {} {} -> {}",
                    args.len(),
                    results.len(),
                    fmt(&arg_types),
                    fmt(&result_types)
                ));
                n_insts += 1;
            }
        }
        self.lines.push(format!("func {name} {n_insts}"));
    }
}

impl Module for MeasuringModule {
    fn isa(&self) -> &dyn isa::TargetIsa {
        self.inner.isa()
    }

    fn declarations(&self) -> &ModuleDeclarations {
        self.inner.declarations()
    }

    fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        signature: &ir::Signature,
    ) -> ModuleResult<FuncId> {
        self.inner.declare_function(name, linkage, signature)
    }

    fn declare_anonymous_function(&mut self, signature: &ir::Signature) -> ModuleResult<FuncId> {
        self.inner.declare_anonymous_function(signature)
    }

    fn declare_data(
        &mut self,
        name: &str,
        linkage: Linkage,
        writable: bool,
        tls: bool,
    ) -> ModuleResult<DataId> {
        self.inner.declare_data(name, linkage, writable, tls)
    }

    fn declare_anonymous_data(&mut self, writable: bool, tls: bool) -> ModuleResult<DataId> {
        self.inner.declare_anonymous_data(writable, tls)
    }

    fn define_function_with_control_plane(
        &mut self,
        id: FuncId,
        ctx: &mut Context,
        ctrl_plane: &mut ControlPlane,
    ) -> ModuleResult<()> {
        // Before the backend runs: after `compile`, the CLIF in `ctx.func` is
        // no longer the shape the lowering produced.
        self.measure(id, &ctx.func);
        self.inner
            .define_function_with_control_plane(id, ctx, ctrl_plane)
    }

    fn define_function_bytes(
        &mut self,
        id: FuncId,
        alignment: u64,
        bytes: &[u8],
        relocs: &[ModuleReloc],
    ) -> ModuleResult<()> {
        self.inner
            .define_function_bytes(id, alignment, bytes, relocs)
    }

    fn define_data(&mut self, id: DataId, data: &cranelift_module::DataDescription) -> ModuleResult<()> {
        self.inner.define_data(id, data)
    }
}

fn main() {
    match run() {
        Ok(report) => {
            print!("{report}");
        }
        Err(e) => {
            eprintln!("clif-dump: {e}");
            std::process::exit(2);
        }
    }
}

fn run() -> Result<String, String> {
    let usage = "usage: clif-dump [--allow-all] <module.cps>...";
    let mut modules: Vec<PathBuf> = Vec::new();
    let mut allow_all = false;
    for arg in std::env::args_os().skip(1) {
        let s = arg.to_string_lossy().into_owned();
        if s == "--allow-all" {
            allow_all = true;
        } else if s.starts_with("--") {
            return Err(format!("unknown flag {s}; {usage}"));
        } else {
            modules.push(PathBuf::from(arg));
        }
    }
    if modules.is_empty() {
        return Err(usage.into());
    }
    if allow_all {
        austral_cranelift_bridge::auth::set_allow_all();
    }

    let mut out = String::new();
    for path in &modules {
        let ir = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut module = MeasuringModule::new()?;
        cps::compile_cps_to_clif(&mut module, &ir)
            .map_err(|e| format!("cps compile {}: {e}", path.display()))?;
        let insts = module.lines.iter().filter(|l| l.starts_with("inst ")).count();
        if insts == 0 {
            return Err(format!(
                "{}: the compile produced no Cranelift instruction — nothing was measured",
                path.display()
            ));
        }
        out.push_str(&format!("module {}\n", path.display()));
        for line in &module.lines {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}
