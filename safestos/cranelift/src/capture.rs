//! The `.avm` cell capture backend (spec §3.4; Sprint 3, Task 3.2).
//!
//! Where the JIT backend publishes compiled code into executable pages,
//! this backend *captures* it: per function, the final machine-code bytes
//! (text + embedded constants) plus the relocation records that name the
//! functions they reference — exactly the raw material the `.avm` cell
//! layout needs for its text section and relocation table. It runs the
//! *same* CPS compiler as the JIT path (`cps::compile_cps_to_clif`, which
//! is generic over `cranelift_module::Module`), so a cell body and a
//! JIT'd function are the same compilation, published differently.
//!
//! The [`CaptureModule::encode_avce`] output — the `AVCE` container — is
//! consumed by `avm-exec`'s cell builder (spec §3.4 serializes it into the
//! `.avm` header + sections). No `unsafe` anywhere in this module.

use cranelift_codegen::binemit::Reloc;
use cranelift_codegen::ir;
use cranelift_codegen::isa;
use cranelift_codegen::settings::Flags;
use cranelift_codegen::Context;
use cranelift_control::ControlPlane;
use cranelift_module::{
    DataId, FuncId, Linkage, Module, ModuleDeclarations, ModuleError, ModuleReloc,
    ModuleRelocTarget, ModuleResult,
};
use std::collections::HashMap;
use std::sync::Arc;

/// One relocation: a patch site inside a function's own code bytes that
/// refers to a symbol **by name** (the cell linker resolves it later —
/// spec task 3.3).
#[derive(Debug, Clone)]
pub struct CapturedReloc {
    /// Byte offset within the defining function's code.
    pub offset: u64,
    /// Value to add to the symbol's address at that site.
    pub addend: i64,
    /// Cranelift `Reloc` variant name (e.g. `X86CallPCRel4`, `Abs8`).
    /// Stringly on purpose: the cell builder hard-errors on kinds it does
    /// not know how to apply — no silent mis-links.
    pub kind: String,
    /// Linkage name of a function declared in this compilation (defined
    /// here, or imported and left for the runtime/cell linker).
    pub symbol: String,
}

/// A compiled, defined function: text bytes + its relocation table.
#[derive(Debug, Clone)]
pub struct CapturedBody {
    pub bytes: Vec<u8>,
    pub relocs: Vec<CapturedReloc>,
}

/// A `cranelift_module::Module` that keeps compiled functions in plain
/// memory instead of publishing them anywhere.
pub struct CaptureModule {
    declarations: ModuleDeclarations,
    isa: Arc<dyn isa::TargetIsa>,
    bodies: HashMap<FuncId, CapturedBody>,
}

impl CaptureModule {
    pub fn new() -> Result<Self, String> {
        let target_builder =
            cranelift_native::builder().map_err(|e| format!("native builder: {e}"))?;
        let isa = target_builder
            .finish(Flags::new(cranelift_codegen::settings::builder()))
            .map_err(|e| format!("isa finish: {e}"))?;
        Ok(CaptureModule {
            declarations: ModuleDeclarations::default(),
            isa,
            bodies: HashMap::new(),
        })
    }

    /// Linkage name of any declared function (defined or imported).
    pub fn func_name(&self, id: FuncId) -> String {
        self.declarations
            .get_function_decl(id)
            .linkage_name(id)
            .into_owned()
    }

    pub fn body(&self, id: FuncId) -> Option<&CapturedBody> {
        self.bodies.get(&id)
    }

    /// Every defined function as `(name, body)`, in `FuncId` order —
    /// which is first-declaration order, so output is deterministic.
    pub fn defined(&self) -> Vec<(String, &CapturedBody)> {
        let mut ids: Vec<FuncId> = self.bodies.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .map(|id| (self.func_name(id), &self.bodies[&id]))
            .collect()
    }

    /// Declared-but-never-defined linkage names: the cell's external
    /// symbol table (runtime primitives, cross-module calls, other cells).
    pub fn imports(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .declarations
            .get_functions()
            .filter(|(id, d)| !d.linkage.is_definable() && !self.bodies.contains_key(id))
            .map(|(id, _)| self.func_name(id))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Host architecture string for the `.avm` header (spec §3.4 arch
    /// field), e.g. `x86_64`.
    pub fn arch_name(&self) -> String {
        self.isa.triple().architecture.to_string()
    }

    /// Encode the capture as the `AVCE` container — the wire format
    /// between this bridge and `avm-exec`'s cell builder:
    ///
    /// ```text
    /// magic   u32  = 0x41564345  ("AVCE" little-endian)
    /// version u16  = 1
    /// arch    u32 len + utf-8 bytes
    /// funcs   u32 count
    ///   per func:  name u32 len + bytes
    ///              body  u32 len + bytes
    ///              relocs u32 count, each:
    ///                offset u64, addend i64,
    ///                kind u32 len + bytes, symbol u32 len + bytes
    /// imports u32 count, each: name u32 len + bytes
    /// ```
    ///
    /// Endianness little; bounds-checked on parse (the cell builder
    /// rejects anything it does not fully consume).
    pub fn encode_avce(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0x4156_4345u32.to_le_bytes()); // "AVCE"
        b.extend_from_slice(&1u16.to_le_bytes());
        let arch = self.arch_name();
        b.extend_from_slice(&(arch.len() as u32).to_le_bytes());
        b.extend_from_slice(arch.as_bytes());
        let defined = self.defined();
        b.extend_from_slice(&(defined.len() as u32).to_le_bytes());
        for (name, body) in &defined {
            b.extend_from_slice(&(name.len() as u32).to_le_bytes());
            b.extend_from_slice(name.as_bytes());
            b.extend_from_slice(&(body.bytes.len() as u32).to_le_bytes());
            b.extend_from_slice(&body.bytes);
            b.extend_from_slice(&(body.relocs.len() as u32).to_le_bytes());
            for r in &body.relocs {
                b.extend_from_slice(&r.offset.to_le_bytes());
                b.extend_from_slice(&r.addend.to_le_bytes());
                b.extend_from_slice(&(r.kind.len() as u32).to_le_bytes());
                b.extend_from_slice(r.kind.as_bytes());
                b.extend_from_slice(&(r.symbol.len() as u32).to_le_bytes());
                b.extend_from_slice(r.symbol.as_bytes());
            }
        }
        let imports = self.imports();
        b.extend_from_slice(&(imports.len() as u32).to_le_bytes());
        for name in &imports {
            b.extend_from_slice(&(name.len() as u32).to_le_bytes());
            b.extend_from_slice(name.as_bytes());
        }
        b
    }
}

/// Reloc kind as its stable `Debug` name (parsed back by `avm-exec`).
fn reloc_kind_name(r: Reloc) -> String {
    format!("{r:?}")
}

/// Symbol name for a module reloc target: functions live in namespace 0
/// (see `Module::declare_func_in_func`), data objects in namespace 1.
fn symbol_string(name: &ModuleRelocTarget, decls: &ModuleDeclarations) -> String {
    match *name {
        ModuleRelocTarget::User { namespace: 0, index } => {
            let id = FuncId::from_u32(index);
            decls.get_function_decl(id).linkage_name(id).into_owned()
        }
        ModuleRelocTarget::User { namespace: 1, index } => {
            let id = DataId::from_u32(index);
            decls.get_data_decl(id).linkage_name(id).into_owned()
        }
        ModuleRelocTarget::User { namespace, index } => format!("user:{namespace}:{index}"),
        // LibCall and friends: keep the Debug name; the cell linker knows
        // runtime libcall names and hard-errors on anything else.
        ref other => format!("{other:?}"),
    }
}

impl Module for CaptureModule {
    fn isa(&self) -> &dyn isa::TargetIsa {
        &*self.isa
    }

    fn declarations(&self) -> &ModuleDeclarations {
        &self.declarations
    }

    fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        signature: &ir::Signature,
    ) -> ModuleResult<FuncId> {
        let (id, _linkage) = self
            .declarations
            .declare_function(name, linkage, signature)?;
        Ok(id)
    }

    fn declare_anonymous_function(&mut self, signature: &ir::Signature) -> ModuleResult<FuncId> {
        self.declarations.declare_anonymous_function(signature)
    }

    fn declare_data(
        &mut self,
        name: &str,
        linkage: Linkage,
        writable: bool,
        tls: bool,
    ) -> ModuleResult<DataId> {
        assert!(!tls, "cells do not model TLS");
        let (id, _linkage) = self
            .declarations
            .declare_data(name, linkage, writable, tls)?;
        Ok(id)
    }

    fn declare_anonymous_data(&mut self, writable: bool, tls: bool) -> ModuleResult<DataId> {
        assert!(!tls, "cells do not model TLS");
        self.declarations.declare_anonymous_data(writable, tls)
    }

    fn define_function_with_control_plane(
        &mut self,
        id: FuncId,
        ctx: &mut Context,
        ctrl_plane: &mut ControlPlane,
    ) -> ModuleResult<()> {
        let decl = self.declarations.get_function_decl(id);
        if !decl.linkage.is_definable() {
            return Err(ModuleError::InvalidImportDefinition(
                decl.linkage_name(id).into_owned(),
            ));
        }
        if self.bodies.contains_key(&id) {
            return Err(ModuleError::DuplicateDefinition(
                decl.linkage_name(id).into_owned(),
            ));
        }

        // Same two calls the JIT backend makes — then, instead of copying
        // into executable memory, keep the finalized bytes and relocs.
        let res = ctx.compile(&*self.isa, ctrl_plane)?;
        let _alignment = res.buffer.alignment;
        let compiled_code = ctx.compiled_code().ok_or_else(|| {
            ModuleError::Compilation(cranelift_codegen::CodegenError::Unsupported(
                "compiled code unavailable after compile".into(),
            ))
        })?;
        let bytes = compiled_code.code_buffer().to_vec();
        let relocs = compiled_code
            .buffer
            .relocs()
            .iter()
            .map(|r| {
                let m = ModuleReloc::from_mach_reloc(r, &ctx.func, id);
                CapturedReloc {
                    offset: u64::from(m.offset),
                    addend: m.addend,
                    kind: reloc_kind_name(m.kind),
                    symbol: symbol_string(&m.name, &self.declarations),
                }
            })
            .collect();
        self.bodies.insert(id, CapturedBody { bytes, relocs });
        Ok(())
    }

    fn define_function_bytes(
        &mut self,
        _func_id: FuncId,
        _alignment: u64,
        _bytes: &[u8],
        _relocs: &[ModuleReloc],
    ) -> ModuleResult<()> {
        Err(ModuleError::Compilation(
            cranelift_codegen::CodegenError::Unsupported(
                "capture backend: define_function_bytes not used by the CPS pipeline".into(),
            ),
        ))
    }

    fn define_data(
        &mut self,
        _data_id: DataId,
        _data: &cranelift_module::DataDescription,
    ) -> ModuleResult<()> {
        Err(ModuleError::Compilation(
            cranelift_codegen::CodegenError::Unsupported(
                "capture backend: data objects are not part of .avm cells yet".into(),
            ),
        ))
    }
}
