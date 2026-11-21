use cranelift::prelude::*;
use cranelift_codegen::ir::FuncRef;
use cranelift_module::{FuncId, Linkage, Module};
use std::cell::RefCell;
use std::collections::HashMap;

// The restart block and parameter names of the function currently being
// compiled, for self-tail-call elimination. `emit_stmt_list`/`emit_expr` are
// threaded through many recursive call sites, so the target lives here instead
// of in every signature; it is set for one function at a time.
thread_local! {
    static SELF_TAIL: RefCell<Option<(Block, Vec<String>)>> = const { RefCell::new(None) };
}

fn self_tail_target(caller: &str, callee: &str, argc: usize) -> Option<Block> {
    if caller != callee {
        return None;
    }
    SELF_TAIL.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|(block, params)| (params.len() == argc).then_some(*block))
    })
}

/// Does this CPS body contain `return <self>(…)`?
///
/// The restart block must exist *before* the body is emitted, but creating it
/// for every function would add a jump to every compiled module and change
/// the bytes the certificates and the CLIF census are pinned against. The
/// tail-call encoding is `0x07 0x04 <u32 len> <name>`, so the exact byte
/// sequence is searched for; a string payload that happens to contain it only
/// adds an unused (harmless) restart block. A false negative is not possible,
/// because the real call is encoded exactly this way.
fn body_has_self_tail(body: &[u8], name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let name = name.as_bytes();
    let len = (name.len() as u32).to_le_bytes();
    let mut pattern = Vec::with_capacity(6 + name.len());
    pattern.extend_from_slice(&[0x07, 0x04]);
    pattern.extend_from_slice(&len);
    pattern.extend_from_slice(name);
    body.windows(pattern.len()).any(|w| w == pattern.as_slice())
}

fn self_tail_params() -> Vec<String> {
    SELF_TAIL.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|(_, params)| params.clone())
            .unwrap_or_default()
    })
}

fn check_call_permission(caller: &str, module_name: &str, callee: &str) -> Result<(), String> {
    if callee.starts_with("__") || callee.starts_with("au_") || caller == callee {
        return Ok(());
    }
    // Only kernel-boundary calls (`uk_*`/`uz_*` FFI) and tier-1 extern rust
    // calls (`rs_*`, PLAN_valen §4) are gated against the manifest grant
    // list. Internal Austral calls — intra- or cross-module (e.g. a library
    // wrapper like `kernelModelCreateStr` calling back into the same compiled
    // program) — are part of the module's own code, not kernel calls, and
    // must never be denied here. The namespaces are independent grants: a
    // manifest granting `uk_version` does not grant `rs_checksum`.
    if !callee.starts_with("uk_") && !callee.starts_with("uz_") && !callee.starts_with("rs_") {
        return Ok(());
    }
    // Prefer the deployment principal (the manifest's module name, e.g.
    // "demo_module") over the CPS header's Austral module name, falling back to
    // the caller name for legacy binaries that lack module headers.
    let principal = crate::auth::deployment_principal()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| {
            if module_name.is_empty() {
                caller.to_string()
            } else {
                module_name.to_string()
            }
        });
    crate::auth::check(&principal, "Call", callee)
}

/// The external symbol an *unlowered* `@embed` call declares.
///
/// The JIT cannot execute the C text of an arbitrary embed, so the call is a
/// link-time import. `cranelift_module::ModuleDeclarations` keys imports by
/// name and refuses a second declaration of one name at a different signature,
/// which made a module containing embeds of two shapes (`load` takes one word,
/// `store`/an allocating embed two) fail to compile at all with "incompatible
/// with previous declaration". The arity is therefore part of the symbol: each
/// shape declares its own import, and if one is ever actually reached the
/// resolution error names the shape that had no lowering.
fn embed_symbol(argc: usize) -> String {
    format!("__embed${}", argc)
}

/// Declare `name` as an external import with `argc` i64 params returning i64,
/// returning its `FuncId`. Used for runtime/kernel builtins and for cross-module
/// Austral calls (resolved by name to a definition in the persistent JITModule).
fn declare_import<M: Module>(jit: &mut M, name: &str, argc: usize) -> Result<FuncId, String> {
    // Unknown embeds are keyed by shape (see `embed_symbol`); every other
    // import keeps the name the runtime or the other module exports.
    let name = if name == "__embed" {
        embed_symbol(argc)
    } else {
        name.to_string()
    };
    let mut sig = jit.make_signature();
    for _ in 0..argc {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));
    jit.declare_function(&name, Linkage::Import, &sig)
        .map_err(|e| format!("Failed to declare import {}: {}", name, e))
}

/// Emit the checked form of the `sub`/`mul` instruction.
///
/// The language's `-`/`*` trap on overflow (`BuiltInModules.ml`), so the opcodes
/// must trap too: now that the lowering emits them, a wrapping `isub`/`imul`
/// would silently change program meaning. The overflow branch calls the very
/// helper the call-based lowering used, so both paths abort with the same
/// message and cannot drift.
fn emit_trapping_arith<M: Module>(
    jit: &mut M,
    mgr: &mut BlockManager,
    a: Value,
    b: Value,
    op: char,
) -> Result<Value, String> {
    let (res, overflow) = match op {
        '-' => mgr.builder.ins().ssub_overflow(a, b),
        _ => mgr.builder.ins().smul_overflow(a, b),
    };
    let ok_block = mgr.create_block();
    let overflow_block = mgr.create_block();
    mgr.builder
        .ins()
        .brif(overflow, overflow_block, &[], ok_block, &[]);
    mgr.switch_to_block(overflow_block);
    let symbol = if op == '-' {
        "Austral.Pervasive::trappingSubtract"
    } else {
        "Austral.Pervasive::trappingMultiply"
    };
    let fid = declare_import(jit, symbol, 2)?;
    let fref = jit.declare_func_in_func(fid, mgr.builder.func);
    mgr.builder.ins().call(fref, &[a, b]);
    mgr.emit_jump(ok_block);
    mgr.switch_to_block(ok_block);
    Ok(res)
}

pub struct CpsModule {
    pub name_map: HashMap<String, FuncId>,
    pub module_name: String,
    /// The signature each function *declares* in the CPS header: parameter
    /// count and return-type byte. The CLIF signature is always
    /// `(i64*) -> i64` (the transport is 64-bit words), so this is the only
    /// place the declared type survives; entry admission compares against it
    /// before a caller invokes the pointer.
    pub signatures: HashMap<String, (u32, u8)>,
}

pub struct CpsReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CpsReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub fn read_u8(&mut self) -> Result<u8, String> {
        if self.pos >= self.data.len() {
            return Err("EOF".to_string());
        }
        let val = self.data[self.pos];
        self.pos += 1;
        Ok(val)
    }
    pub fn peek_u8(&self) -> Option<u8> {
        if self.pos >= self.data.len() {
            None
        } else {
            Some(self.data[self.pos])
        }
    }
    pub fn read_u32(&mut self) -> Result<u32, String> {
        if self.pos + 4 > self.data.len() {
            return Err("EOF".to_string());
        }
        let val = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(val)
    }
    pub fn read_i64(&mut self) -> Result<i64, String> {
        if self.pos + 8 > self.data.len() {
            return Err("EOF".to_string());
        }
        let val = i64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(val)
    }
    pub fn read_string(&mut self) -> Result<String, String> {
        let len = self.read_u32()? as usize;
        if self.pos + len > self.data.len() {
            return Err("EOF".to_string());
        }
        let s = String::from_utf8_lossy(&self.data[self.pos..self.pos + len]).to_string();
        self.pos += len;
        Ok(s)
    }
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], String> {
        if self.pos + len > self.data.len() {
            return Err("EOF".to_string());
        }
        let data = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(data)
    }
}

/// Robust Block Manager for Cranelift
struct BlockManager<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    all_blocks: Vec<Block>,
    terminated: bool,
}

impl<'a, 'b> BlockManager<'a, 'b> {
    fn new(builder: &'a mut FunctionBuilder<'b>) -> Self {
        Self {
            builder,
            all_blocks: Vec::new(),
            terminated: false,
        }
    }

    fn create_block(&mut self) -> Block {
        let block = self.builder.create_block();
        self.all_blocks.push(block);
        block
    }

    fn seal_all(&mut self) {
        for &block in &self.all_blocks {
            self.builder.seal_block(block);
        }
    }

    fn switch_to_block(&mut self, block: Block) {
        self.builder.switch_to_block(block);
        self.terminated = false;
    }

    fn emit_return(&mut self, vals: &[Value]) {
        if !self.terminated {
            self.builder.ins().return_(vals);
            self.terminated = true;
        }
    }

    fn emit_return_call(&mut self, func: FuncRef, args: &[Value]) {
        if !self.terminated {
            let call = self.builder.ins().call(func, args);
            let results = self.builder.inst_results(call).to_vec();
            self.builder.ins().return_(&results);
            self.terminated = true;
        }
    }

    fn emit_jump(&mut self, target: Block) {
        if !self.terminated {
            self.builder.ins().jump(target, &[]);
            self.terminated = true;
        }
    }

    fn ensure_terminated(&mut self, merge_block: Option<Block>) {
        if !self.terminated {
            if let Some(target) = merge_block {
                self.emit_jump(target);
            } else {
                let zero = self.builder.ins().iconst(types::I64, 0);
                self.emit_return(&[zero]);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)] // threaded through the whole CPS emit chain
fn emit_expr<M: Module>(
    jit: &mut M,
    reader: &mut CpsReader,
    mgr: &mut BlockManager,
    vars: &mut HashMap<String, Variable>,
    name_map: &HashMap<String, FuncId>,
    import_map: &HashMap<String, FuncId>,
    caller_name: &str,
    module_name: &str,
) -> Result<Value, String> {
    let opcode = reader.read_u8()?;
    crate::isa::note(opcode);
    match opcode {
        0x01 => {
            let val = reader.read_i64()?;
            Ok(mgr.builder.ins().iconst(types::I64, val))
        }
        0x02 => {
            let name = reader.read_string()?;
            if let Some(&var) = vars.get(&name) {
                Ok(mgr.builder.use_var(var))
            } else {
                Err(format!("Undefined variable: {}", name))
            }
        }
        0x04 => {
            let func_name = reader.read_string()?;
            let arg_count = reader.read_u32()?;
            let mut args = Vec::new();
            for _ in 0..arg_count {
                args.push(emit_expr(
                    jit,
                    reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?);
            }

            // Authorization check (module_name threaded through call chain)
            check_call_permission(caller_name, module_name, &func_name)?;

            if func_name == "__slot_get" {
                // (ptr, offset) -> val
                let ptr = args[0];
                let offset_val = args[1];
                let addr = mgr.builder.ins().iadd(ptr, offset_val);
                return Ok(mgr.builder.ins().load(types::I64, MemFlags::new(), addr, 0));
            } else if func_name == "__ptr_slot_get" {
                // (ptr, offset) -> address of the slot. `MPointerSlotAccessor`
                // denotes the *address* (the C backend wraps it in `AddressOf`)
                // and the lowering applies `Deref` to it, so this must not load:
                // doing so made every `record->field` read a load of the field
                // as an address.
                let ptr = args[0];
                let offset_val = args[1];
                return Ok(mgr.builder.ins().iadd(ptr, offset_val));
            } else if func_name == "__deref" {
                // (ptr) -> val: `MDeref` lowers to this. Every value in this JIT is
                // one 64-bit word, so a dereference is a load of the pointee.
                return Ok(mgr
                    .builder
                    .ins()
                    .load(types::I64, MemFlags::new(), args[0], 0));
            } else if func_name == "__record_new" {
                // (size, field1, field2, ...) -> ptr
                let size_val = args[0];
                let alloc_fid = jit
                    .declare_function("au_alloc", cranelift_module::Linkage::Import, &{
                        let mut sig = jit.make_signature();
                        sig.params.push(AbiParam::new(types::I64));
                        sig.returns.push(AbiParam::new(types::I64));
                        sig
                    })
                    .map_err(|e| format!("Failed to declare au_alloc: {}", e))?;
                let alloc_fref = jit.declare_func_in_func(alloc_fid, mgr.builder.func);
                let call = mgr.builder.ins().call(alloc_fref, &[size_val]);
                let ptr = mgr.builder.inst_results(call)[0];
                for (i, field_val) in args.iter().enumerate().skip(1) {
                    let offset = (i - 1) * 8;
                    mgr.builder
                        .ins()
                        .store(MemFlags::new(), *field_val, ptr, offset as i32);
                }
                return Ok(ptr);
            } else if func_name == "__union_new" {
                // (size, tag, field1, ...) -> ptr
                let size_val = args[0];
                let tag_val = args[1];
                let alloc_fid = jit
                    .declare_function("au_alloc", cranelift_module::Linkage::Import, &{
                        let mut sig = jit.make_signature();
                        sig.params.push(AbiParam::new(types::I64));
                        sig.returns.push(AbiParam::new(types::I64));
                        sig
                    })
                    .map_err(|e| format!("Failed to declare au_alloc: {}", e))?;
                let alloc_fref = jit.declare_func_in_func(alloc_fid, mgr.builder.func);
                let call = mgr.builder.ins().call(alloc_fref, &[size_val]);
                let ptr = mgr.builder.inst_results(call)[0];

                // Store tag at offset 0
                mgr.builder.ins().store(MemFlags::new(), tag_val, ptr, 0);

                // Store fields starting at offset 8
                for (i, field_val) in args.iter().enumerate().skip(2) {
                    let offset = (i - 1) * 8;
                    mgr.builder
                        .ins()
                        .store(MemFlags::new(), *field_val, ptr, offset as i32);
                }
                return Ok(ptr);
            }

            let func_ref = if let Some(&fid) = name_map.get(&func_name) {
                jit.declare_func_in_func(fid, mgr.builder.func)
            } else if let Some(&fid) = import_map.get(&func_name) {
                jit.declare_func_in_func(fid, mgr.builder.func)
            } else {
                // Not defined in this module's table. Declare it as an external
                // import and let the linker resolve it by name. This covers both
                // (a) runtime/kernel builtins (au_/uk_/__ registered as native
                // symbols) and (b) cross-module Austral calls: the callee was
                // defined (Linkage::Export) when its own module was compiled into
                // this same persistent JITModule, so an Import of the same name
                // links to that definition at finalize time.
                let fid = declare_import(jit, &func_name, args.len())?;
                jit.declare_func_in_func(fid, mgr.builder.func)
            };
            let call = mgr.builder.ins().call(func_ref, &args);
            let results = mgr.builder.inst_results(call);
            Ok(if results.is_empty() {
                mgr.builder.ins().iconst(types::I64, 0)
            } else {
                results[0]
            })
        }
        0x05 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            Ok(mgr.builder.ins().iadd(a, b))
        }
        0x06 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            emit_trapping_arith(jit, mgr, a, b, '-')
        }
        0x10 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let cmp = mgr.builder.ins().icmp(IntCC::SignedLessThan, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x11 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let cmp = mgr.builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x12 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let cmp = mgr.builder.ins().icmp(IntCC::SignedLessThanOrEqual, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x13 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let cmp = mgr
                .builder
                .ins()
                .icmp(IntCC::SignedGreaterThanOrEqual, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x14 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let cmp = mgr.builder.ins().icmp(IntCC::Equal, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x15 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let cmp = mgr.builder.ins().icmp(IntCC::NotEqual, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        // 0x16 / 0x17: boolean And / Or. `And`/`Or` in the source lower to these
        // opcodes (lib/CpsGen.ml), and their operands are already 0/1 i64 — the
        // comparisons above widen with `uextend`. Implemented as a bitwise
        // `band`/`bor` on those, which is the truth table for 0/1 inputs. 0x19
        // is the 0/1 complement the serializer emits for `not`, so it is a
        // `bxor` with 1: a bitwise `bnot` would turn 1 into -2, a truthy value.
        // Without these two arms any module that conjoins two
        // conditions — including every kernel `main` that checks its own
        // obligations — was rejected as "Unknown opcode: 0x16".
        0x16 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            Ok(mgr.builder.ins().band(a, b))
        }
        0x17 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            Ok(mgr.builder.ins().bor(a, b))
        }
        0x18 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let b = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            emit_trapping_arith(jit, mgr, a, b, '*')
        }
        0x19 => {
            let a = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            let one = mgr.builder.ins().iconst(types::I64, 1);
            Ok(mgr.builder.ins().bxor(a, one))
        }
        0x20 => {
            let ptr = emit_expr(
                jit,
                reader,
                mgr,
                vars,
                name_map,
                import_map,
                caller_name,
                module_name,
            )?;
            Ok(mgr.builder.ins().load(types::I64, MemFlags::new(), ptr, 0))
        }
        0x21 => {
            // String/byte constant. Embed the bytes in heap memory and yield a
            // Span value, represented as a pointer to a heap {data@0, size@8}
            // struct (so `@embed "$1.data"`/`"$1.size"` are slot-0/slot-8 loads,
            // and the kernel C ABI receives a real (ptr, len) pair after the
            // module decomposes the span). This sidesteps the by-value 16-byte
            // au_span_t aggregate, which the i64-centric JIT cannot pass.
            let len = reader.read_u32()? as usize;
            let bytes = reader.read_bytes(len)?.to_vec();
            let alloc_fref = {
                let mut sig = jit.make_signature();
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(types::I64));
                let fid = jit
                    .declare_function("au_alloc", Linkage::Import, &sig)
                    .map_err(|e| format!("Failed to declare au_alloc: {}", e))?;
                jit.declare_func_in_func(fid, mgr.builder.func)
            };
            // Data buffer (at least 1 byte so au_alloc never sees 0).
            let data_size = mgr.builder.ins().iconst(types::I64, len.max(1) as i64);
            let data_call = mgr.builder.ins().call(alloc_fref, &[data_size]);
            let data_ptr = mgr.builder.inst_results(data_call)[0];
            for (i, &b) in bytes.iter().enumerate() {
                let bv = mgr.builder.ins().iconst(types::I8, b as i64);
                mgr.builder
                    .ins()
                    .store(MemFlags::new(), bv, data_ptr, i as i32);
            }
            // Span struct {data, size}.
            let span_size = mgr.builder.ins().iconst(types::I64, 16);
            let span_call = mgr.builder.ins().call(alloc_fref, &[span_size]);
            let span_ptr = mgr.builder.inst_results(span_call)[0];
            let len_val = mgr.builder.ins().iconst(types::I64, len as i64);
            mgr.builder
                .ins()
                .store(MemFlags::new(), data_ptr, span_ptr, 0);
            mgr.builder
                .ins()
                .store(MemFlags::new(), len_val, span_ptr, 8);
            Ok(span_ptr)
        }
        _ => Err(format!("Unknown opcode: 0x{:02x}", opcode)),
    }
}

#[allow(clippy::too_many_arguments)] // threaded through the whole CPS emit chain
fn emit_stmt_list<M: Module>(
    jit: &mut M,
    reader: &mut CpsReader,
    mgr: &mut BlockManager,
    vars: &mut HashMap<String, Variable>,
    name_map: &HashMap<String, FuncId>,
    import_map: &HashMap<String, FuncId>,
    caller_name: &str,
    module_name: &str,
) -> Result<(), String> {
    while reader.remaining() > 0 {
        if mgr.terminated {
            return Ok(());
        }
        match reader.peek_u8() {
            Some(0x07) => {
                crate::isa::note(reader.read_u8()?);
                if reader.peek_u8() == Some(0x04) {
                    crate::isa::note(reader.read_u8()?); // App
                    let func_name = reader.read_string()?;
                    let arg_count = reader.read_u32()?;
                    let mut args = Vec::new();
                    for _ in 0..arg_count {
                        args.push(emit_expr(
                            jit,
                            reader,
                            mgr,
                            vars,
                            name_map,
                            import_map,
                            caller_name,
                            module_name,
                        )?);
                    }

                    // Authorization check
                    check_call_permission(caller_name, module_name, &func_name)?;

                    if func_name == "__slot_get" || func_name == "__ptr_slot_get" {
                        let ptr = args[0];
                        let offset_val = args[1];
                        let addr = mgr.builder.ins().iadd(ptr, offset_val);
                        // `__ptr_slot_get` yields the address; `__slot_get` the
                        // loaded value (see the matching branch in `emit_expr`).
                        let val = if func_name == "__ptr_slot_get" {
                            addr
                        } else {
                            mgr.builder.ins().load(types::I64, MemFlags::new(), addr, 0)
                        };
                        mgr.builder.ins().return_(&[val]);
                        mgr.terminated = true;
                        return Ok(());
                    } else if func_name == "__deref" {
                        // Same intrinsic as the emit_expr arm; see the comment there.
                        let val = mgr
                            .builder
                            .ins()
                            .load(types::I64, MemFlags::new(), args[0], 0);
                        mgr.builder.ins().return_(&[val]);
                        mgr.terminated = true;
                        return Ok(());
                    } else if func_name == "__record_new" {
                        let size_val = args[0];
                        let alloc_fid = jit
                            .declare_function("au_alloc", cranelift_module::Linkage::Import, &{
                                let mut sig = jit.make_signature();
                                sig.params.push(AbiParam::new(types::I64));
                                sig.returns.push(AbiParam::new(types::I64));
                                sig
                            })
                            .map_err(|e| format!("Failed to declare au_alloc: {}", e))?;
                        let alloc_fref = jit.declare_func_in_func(alloc_fid, mgr.builder.func);
                        let call = mgr.builder.ins().call(alloc_fref, &[size_val]);
                        let ptr = mgr.builder.inst_results(call)[0];
                        for (i, field_val) in args.iter().enumerate().skip(1) {
                            let offset = (i - 1) * 8;
                            mgr.builder.ins().store(
                                MemFlags::new(),
                                *field_val,
                                ptr,
                                offset as i32,
                            );
                        }
                        mgr.builder.ins().return_(&[ptr]);
                        mgr.terminated = true;
                        return Ok(());
                    } else if func_name == "__union_new" {
                        // A union constructed in *tail position* (`return Case(…)`)
                        // reaches this path, not `emit_expr`. Lower it the same
                        // way: allocate, store the tag at 0 and the fields from
                        // offset 8. Without this arm it was treated as an
                        // external import whose arity is the number of fields,
                        // so two different cases produced two incompatible
                        // declarations of `__union_new`.
                        let size_val = args[0];
                        let tag_val = args[1];
                        let alloc_fid = jit
                            .declare_function("au_alloc", cranelift_module::Linkage::Import, &{
                                let mut sig = jit.make_signature();
                                sig.params.push(AbiParam::new(types::I64));
                                sig.returns.push(AbiParam::new(types::I64));
                                sig
                            })
                            .map_err(|e| format!("Failed to declare au_alloc: {}", e))?;
                        let alloc_fref = jit.declare_func_in_func(alloc_fid, mgr.builder.func);
                        let call = mgr.builder.ins().call(alloc_fref, &[size_val]);
                        let ptr = mgr.builder.inst_results(call)[0];
                        mgr.builder.ins().store(MemFlags::new(), tag_val, ptr, 0);
                        for (i, field_val) in args.iter().enumerate().skip(2) {
                            let offset = (i - 1) * 8;
                            mgr.builder.ins().store(
                                MemFlags::new(),
                                *field_val,
                                ptr,
                                offset as i32,
                            );
                        }
                        mgr.builder.ins().return_(&[ptr]);
                        mgr.terminated = true;
                        return Ok(());
                    }

                    // `return f(...)` where `f` is the function being compiled:
                    // assign the parameters and jump back to the restart block.
                    // Without this, a linear state machine's polling loop (the
                    // shape `Waiting` token -> poll -> `Waiting` again) recurses
                    // once per poll and overflows the stack whenever completion
                    // takes longer than the recursion budget.
                    if let Some(loop_block) =
                        self_tail_target(caller_name, &func_name, args.len())
                    {
                        for (pname, val) in self_tail_params().iter().zip(args.iter()) {
                            if let Some(&var) = vars.get(pname) {
                                mgr.builder.def_var(var, *val);
                            }
                        }
                        mgr.builder.ins().jump(loop_block, &[]);
                        mgr.terminated = true;
                        return Ok(());
                    }

                    let func_ref = if let Some(&fid) = name_map.get(&func_name) {
                        jit.declare_func_in_func(fid, mgr.builder.func)
                    } else if let Some(&fid) = import_map.get(&func_name) {
                        jit.declare_func_in_func(fid, mgr.builder.func)
                    } else {
                        // External import: builtin or cross-module Austral call
                        // (see the matching branch in emit_expr).
                        let fid = declare_import(jit, &func_name, args.len())?;
                        jit.declare_func_in_func(fid, mgr.builder.func)
                    };
                    mgr.emit_return_call(func_ref, &args);
                } else {
                    let val = emit_expr(
                        jit,
                        reader,
                        mgr,
                        vars,
                        name_map,
                        import_map,
                        caller_name,
                        module_name,
                    )?;
                    mgr.emit_return(&[val]);
                }
                return Ok(());
            }
            Some(0x08) => {
                crate::isa::note(reader.read_u8()?);
                let cond = emit_expr(
                    jit,
                    reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                let zero = mgr.builder.ins().iconst(types::I64, 0);
                let cond_bool = mgr.builder.ins().icmp(IntCC::NotEqual, cond, zero);

                let then_len = reader.read_u32()?;
                let then_data = reader.read_bytes(then_len as usize)?;
                let else_len = reader.read_u32()?;
                let else_data = reader.read_bytes(else_len as usize)?;

                let then_block = mgr.create_block();
                let else_block = mgr.create_block();
                let merge_block = mgr.create_block();
                mgr.builder
                    .ins()
                    .brif(cond_bool, then_block, &[], else_block, &[]);

                // Then branch
                mgr.switch_to_block(then_block);
                let mut then_reader = CpsReader::new(then_data);
                emit_stmt_list(
                    jit,
                    &mut then_reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                mgr.ensure_terminated(Some(merge_block));

                // Else branch
                mgr.switch_to_block(else_block);
                let mut else_reader = CpsReader::new(else_data);
                emit_stmt_list(
                    jit,
                    &mut else_reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                mgr.ensure_terminated(Some(merge_block));

                // Merge
                mgr.switch_to_block(merge_block);
            }
            Some(0x09) => {
                crate::isa::note(reader.read_u8()?);

                let header_block = mgr.create_block();
                let body_block = mgr.create_block();
                let exit_block = mgr.create_block();

                mgr.emit_jump(header_block);

                // Header: check condition
                mgr.switch_to_block(header_block);
                let cond = emit_expr(
                    jit,
                    reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                let zero = mgr.builder.ins().iconst(types::I64, 0);
                let cond_bool = mgr.builder.ins().icmp(IntCC::NotEqual, cond, zero);

                let body_len = reader.read_u32()?;
                let body_data = reader.read_bytes(body_len as usize)?;

                mgr.builder
                    .ins()
                    .brif(cond_bool, body_block, &[], exit_block, &[]);

                // Body
                mgr.switch_to_block(body_block);
                let mut body_reader = CpsReader::new(body_data);
                emit_stmt_list(
                    jit,
                    &mut body_reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                mgr.emit_jump(header_block);

                // Seal header only after back-edge is added
                // Wait, mgr.seal_all() handles sealing all blocks at the end.
                // But for loops, sealing early helps optimizations.
                // However, since we seal everything at the end, it's fine.

                // Exit
                mgr.switch_to_block(exit_block);
            }
            Some(0x0A) => {
                crate::isa::note(reader.read_u8()?);
                let cond = emit_expr(
                    jit,
                    reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                let case_count = reader.read_u32()?;

                let merge_block = mgr.create_block();

                for _ in 0..case_count {
                    let val_const = reader.read_i64()?;
                    let body_len = reader.read_u32()?;
                    let body_data = reader.read_bytes(body_len as usize)?;

                    let val_v = mgr.builder.ins().iconst(types::I64, val_const);
                    let is_match = mgr.builder.ins().icmp(IntCC::Equal, cond, val_v);

                    let match_block = mgr.create_block();
                    let next_case_block = mgr.create_block();

                    mgr.builder
                        .ins()
                        .brif(is_match, match_block, &[], next_case_block, &[]);

                    // Match branch
                    mgr.switch_to_block(match_block);
                    let mut body_reader = CpsReader::new(body_data);
                    emit_stmt_list(
                        jit,
                        &mut body_reader,
                        mgr,
                        vars,
                        name_map,
                        import_map,
                        caller_name,
                        module_name,
                    )?;
                    mgr.ensure_terminated(Some(merge_block));

                    // Continue to next case
                    mgr.switch_to_block(next_case_block);
                }

                // Default branch (current block is now the 'last' next_case_block)
                let def_len = reader.read_u32()?;
                let def_data = reader.read_bytes(def_len as usize)?;
                let mut def_reader = CpsReader::new(def_data);
                emit_stmt_list(
                    jit,
                    &mut def_reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                mgr.ensure_terminated(Some(merge_block));

                // Merge
                mgr.switch_to_block(merge_block);
            }
            Some(0x03) => {
                crate::isa::note(reader.read_u8()?);
                let name = reader.read_string()?;
                let val = emit_expr(
                    jit,
                    reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                let var = if let Some(&v) = vars.get(&name) {
                    v
                } else {
                    let v = mgr.builder.declare_var(types::I64);
                    vars.insert(name.clone(), v);
                    v
                };
                mgr.builder.def_var(var, val);
            }
            Some(0x30) => {
                crate::isa::note(reader.read_u8()?);
                let ptr = emit_expr(
                    jit,
                    reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                let val = emit_expr(
                    jit,
                    reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
                mgr.builder.ins().store(MemFlags::new(), val, ptr, 0);
            }
            Some(_) => {
                let _ = emit_expr(
                    jit,
                    reader,
                    mgr,
                    vars,
                    name_map,
                    import_map,
                    caller_name,
                    module_name,
                )?;
            }
            None => break,
        }
    }
    Ok(())
}

pub fn compile_cps_to_clif<M: Module>(jit: &mut M, data: &[u8]) -> Result<CpsModule, String> {
    let mut reader = CpsReader::new(data);
    let magic = reader.read_u32()?;

    // Module name: v1 (magic 0x43505331) uses the first function's name
    // as the module principal; v2+ (magic 0x43505332) has an explicit
    // module name field after the magic.
    let module_name = match magic {
        0x43505331 => String::new(), // backwards compat — no module header
        0x43505332 => reader.read_string()?,
        _ => return Err(format!("Invalid magic: 0x{:08x}", magic)),
    };

    let func_count = reader.read_u32()?;
    let mut name_map = HashMap::new();
    let mut signatures = HashMap::new();
    let import_map = HashMap::new();

    let mut func_headers = Vec::new();
    for _ in 0..func_count {
        let name = reader.read_string()?;
        let param_count = reader.read_u32()?;
        let mut params = Vec::new();
        for _ in 0..param_count {
            params.push(reader.read_string()?);
        }
        let ret_type = reader.read_u8()?;
        let body_len = reader.read_u32()?;
        let body_data = reader.read_bytes(body_len as usize)?;

        let mut sig = jit.make_signature();
        for _ in 0..param_count {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));

        // X4a: declare under a unique symbol name if this thread's engine has
        // already defined this one, but keep `name_map` keyed by the caller's
        // name. Without this, two compiles in one thread that both export `run`
        // fail on the engine's single symbol namespace -- which is what made
        // `JitTest` red for reasons unrelated to the code under test.
        //
        // Imports are still resolved by the *original* name, so cross-module
        // linking keeps working against the first definition.
        let symbol = crate::claim_export_symbol(&name);
        let func_id = jit
            .declare_function(&symbol, Linkage::Export, &sig)
            .map_err(|e| format!("Declare {}: {:?}", symbol, e))?;

        name_map.insert(name.clone(), func_id);
        signatures.insert(name.clone(), (param_count, ret_type));
        func_headers.push((name, param_count, params, body_data, func_id, sig));
    }

    for (name, param_count, param_names, body_data, func_id, sig) in func_headers {
        let mut ctx = cranelift::codegen::Context::new();
        ctx.func.signature = sig;
        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
        {
            let mut mgr = BlockManager::new(&mut builder);
            let entry_block = mgr.create_block();
            mgr.switch_to_block(entry_block);
            mgr.builder
                .append_block_params_for_function_params(entry_block);

            let mut vars = HashMap::new();
            for i in 0..param_count {
                let val = mgr.builder.block_params(entry_block)[i as usize];
                let var = mgr.builder.declare_var(types::I64);
                mgr.builder.def_var(var, val);
                let pname = param_names
                    .get(i as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("p{}", i));
                vars.insert(pname, var);
            }

            // A restart block so a tail self call can jump back instead of
            // recursing (see `self_tail_target`). Only when the body really
            // contains one: every other function keeps its exact previous
            // code, which the pinned digests and the CLIF census depend on.
            let restart_block = if body_has_self_tail(body_data, &name) {
                let block = mgr.create_block();
                mgr.builder.ins().jump(block, &[]);
                mgr.switch_to_block(block);
                SELF_TAIL.with(|slot| {
                    *slot.borrow_mut() = Some((block, param_names.clone()))
                });
                Some(block)
            } else {
                None
            };

            let mut body_reader = CpsReader::new(body_data);
            let emitted = emit_stmt_list(
                jit,
                &mut body_reader,
                &mut mgr,
                &mut vars,
                &name_map,
                &import_map,
                &name,
                &module_name,
            );
            if restart_block.is_some() {
                SELF_TAIL.with(|slot| *slot.borrow_mut() = None);
            }
            emitted?;

            mgr.ensure_terminated(None);
            mgr.seal_all();
        }
        builder.finalize();

        let flags = cranelift_codegen::settings::Flags::new(cranelift_codegen::settings::builder());
        if let Err(errors) = cranelift_codegen::verify_function(&ctx.func, &flags) {
            return Err(format!(
                "Verifier failed for function '{}':\n{}",
                name, errors
            ));
        }

        jit.define_function(func_id, &mut ctx)
            .map_err(|e| format!("Define {}: {:?}", name, e))?;
    }

    Ok(CpsModule {
        name_map,
        module_name,
        signatures,
    })
}

#[cfg(test)]
mod a10_live_grant_tests {
    use super::check_call_permission;
    use crate::auth;

    /// A10: the LIVE JIT gate — the function the CPS emitter itself calls at
    /// compile time, not `modhost authorize` (which the module e2e jobs
    /// already prove) — must attribute a kernel call to the **module** the
    /// manifest grants by, and deny anything that module does not grant.
    ///
    /// One test, run sequentially on purpose: the auth engine and the
    /// deployment principal are process-global, so concurrent tests in this
    /// binary would race each other's engines.
    #[test]
    fn live_jit_denies_uk_calls_the_module_manifest_does_not_grant() {
        let engine = auth::ManifestAuthEngine::from_toml_str(
            "[module]\nname = \"a10_widget\"\n\n[grants]\nkernel = [\"uk_version\"]\n",
        )
        .expect("manifest parses");
        auth::set_auth_engine(Box::new(engine));
        auth::set_deployment_principal("a10_widget".to_string());

        // Granted: the manifest names uk_version for this module.
        assert!(
            check_call_permission("run", "", "uk_version").is_ok(),
            "a granted kernel call must pass"
        );

        // Denied: not granted to this module — UK-4001, and the principal in
        // the message is the MODULE, not the calling function ("run"), which
        // is exactly the A10 distinction the old extension point described.
        for callee in ["uk_evolve", "uk_model_free"] {
            let err = check_call_permission("run", "", callee).unwrap_err();
            assert!(
                err.contains("UK-4001"),
                "expected UK-4001 for {callee}, got: {err}"
            );
            assert!(
                err.contains("'a10_widget'") && !err.contains("'run'"),
                "the principal must be the module, not the caller, got: {err}"
            );
        }

        // Non-kernel calls are never gated: internal/cross-module Austral
        // code is the module's own, not a kernel capability.
        assert!(check_call_permission("run", "", "kernelModelCreateStr").is_ok());
        assert!(check_call_permission("run", "", "__slot_get").is_ok());
        assert!(check_call_permission("run", "au_alloc", "au_alloc").is_ok());

        // PLAN_valen §4 Tier 1: the rs_ namespace is gated too, and its
        // grants are independent of the kernel namespace — a manifest that
        // grants `uk_version` does NOT thereby grant `rs_checksum`.
        let engine = auth::ManifestAuthEngine::from_toml_str(
            "[module]\nname = \"a10_widget\"\n\n[grants]\nkernel = [\"uk_version\", \"rs_checksum\"]\n",
        )
        .expect("manifest parses");
        auth::set_auth_engine(Box::new(engine));
        auth::set_deployment_principal("a10_widget".to_string());
        assert!(
            check_call_permission("run", "", "rs_checksum").is_ok(),
            "a grant naming rs_checksum must pass an rs_ call"
        );
        let engine = auth::ManifestAuthEngine::from_toml_str(
            "[module]\nname = \"a10_widget\"\n\n[grants]\nkernel = [\"uk_version\"]\n",
        )
        .expect("manifest parses");
        auth::set_auth_engine(Box::new(engine));
        auth::set_deployment_principal("a10_widget".to_string());
        let err = check_call_permission("run", "", "rs_checksum").unwrap_err();
        assert!(
            err.contains("UK-4001"),
            "a uk-only manifest must deny rs_ calls with UK-4001, got: {err}"
        );
        assert!(
            err.contains("'a10_widget'") && !err.contains("'run'"),
            "the rs_ denial's principal must be the module, got: {err}"
        );

        // No deployment principal (empty = unset) -> the CPS header's module
        // name is the principal: same grants, keyed by header name.
        auth::set_deployment_principal(String::new());
        assert!(
            check_call_permission("run", "a10_widget", "uk_version").is_ok(),
            "header module name must carry the manifest's grants"
        );
        let err = check_call_permission("run", "a10_other", "uk_version").unwrap_err();
        assert!(
            err.contains("UK-4001"),
            "a different header module must NOT inherit the grants, got: {err}"
        );

        // Neither principal present -> the caller name is the legacy fallback
        // for binaries without module headers: grants keyed by that name work,
        // any other name does not.
        assert!(check_call_permission("a10_widget", "", "uk_version").is_ok());
        let err = check_call_permission("some_legacy_fn", "", "uk_version").unwrap_err();
        assert!(err.contains("UK-4001"), "got: {err}");

        // Restore the documented default (no manifest -> DenyAll) so this
        // test leaves the process-global state it found.
        auth::set_auth_engine(Box::new(auth::DenyAll));
        auth::set_deployment_principal(String::new());
    }
}

#[cfg(test)]
mod embed_shape_tests {
    use super::{declare_import, embed_symbol};
    use cranelift_jit::{JITBuilder, JITModule};
    use cranelift_module::default_libcall_names;

    /// REGRESSION: cranelift-module keys imports by name, so one module that
    /// contained an unlowered `__embed` of two shapes (one word, as `load` is;
    /// two, as an allocating embed is) failed at the *second* declaration —
    /// "Failed to declare import __embed: … incompatible with previous
    /// declaration" — and the compiler emitted no C at all. The shape is part
    /// of the symbol now, so both declarations succeed.
    #[test]
    fn one_module_can_declare_embeds_of_two_arities() {
        let mut jit = JITModule::new(JITBuilder::new(default_libcall_names()).unwrap());
        assert_ne!(embed_symbol(1), embed_symbol(2));
        let one = declare_import(&mut jit, "__embed", 1).expect("one-word embed declares");
        let two = declare_import(&mut jit, "__embed", 2).expect("two-word embed declares");
        assert_ne!(one, two, "each shape must be its own declaration");
        // A second call of the same shape names the same import, so two
        // `load`s in one function do not multiply the table.
        let again = declare_import(&mut jit, "__embed", 1).expect("re-declaration is accepted");
        assert_eq!(one, again);
    }
}
