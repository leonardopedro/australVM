use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_codegen::settings::Configurable;
use std::collections::HashMap;
use cranelift_codegen::ir::FuncRef;

pub struct CpsModule {
    pub name_map: HashMap<String, FuncId>,
}

pub struct CpsReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CpsReader<'a> {
    pub fn new(data: &'a [u8]) -> Self { Self { data, pos: 0 } }
    pub fn remaining(&self) -> usize { self.data.len() - self.pos }
    pub fn read_u8(&mut self) -> Result<u8, String> {
        if self.pos >= self.data.len() { return Err("EOF".to_string()); }
        let val = self.data[self.pos];
        self.pos += 1;
        Ok(val)
    }
    pub fn peek_u8(&self) -> Option<u8> {
        if self.pos >= self.data.len() { None }
        else { Some(self.data[self.pos]) }
    }
    pub fn read_u32(&mut self) -> Result<u32, String> {
        if self.pos + 4 > self.data.len() { return Err("EOF".to_string()); }
        let val = u32::from_le_bytes(self.data[self.pos..self.pos+4].try_into().unwrap());
        self.pos += 4;
        Ok(val)
    }
    pub fn read_i64(&mut self) -> Result<i64, String> {
        if self.pos + 8 > self.data.len() { return Err("EOF".to_string()); }
        let val = i64::from_le_bytes(self.data[self.pos..self.pos+8].try_into().unwrap());
        self.pos += 8;
        Ok(val)
    }
    pub fn read_string(&mut self) -> Result<String, String> {
        let len = self.read_u32()? as usize;
        if self.pos + len > self.data.len() { return Err("EOF".to_string()); }
        let s = String::from_utf8_lossy(&self.data[self.pos..self.pos+len]).to_string();
        self.pos += len;
        Ok(s)
    }
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], String> {
        if self.pos + len > self.data.len() { return Err("EOF".to_string()); }
        let data = &self.data[self.pos..self.pos+len];
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

fn emit_expr(
    jit: &mut JITModule,
    reader: &mut CpsReader,
    mgr: &mut BlockManager,
    vars: &mut HashMap<String, Variable>,
    name_map: &HashMap<String, FuncId>,
    import_map: &HashMap<String, FuncId>,
) -> Result<Value, String> {
    let opcode = reader.read_u8()?;
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
                args.push(emit_expr(jit, reader, mgr, vars, name_map, import_map)?);
            }
            
            if func_name == "__slot_get" || func_name == "__ptr_slot_get" {
                // (ptr, offset) -> val
                let ptr = args[0];
                let offset_val = args[1];
                let addr = mgr.builder.ins().iadd(ptr, offset_val);
                return Ok(mgr.builder.ins().load(types::I64, MemFlags::new(), addr, 0));
            } else if func_name == "__record_new" {
                // (size, field1, field2, ...) -> ptr
                let size_val = args[0];
                let alloc_fid = jit.declare_function("au_alloc", cranelift_module::Linkage::Import, &{
                    let mut sig = jit.make_signature();
                    sig.params.push(AbiParam::new(types::I64));
                    sig.returns.push(AbiParam::new(types::I64));
                    sig
                }).map_err(|e| format!("Failed to declare au_alloc: {}", e))?;
                let alloc_fref = jit.declare_func_in_func(alloc_fid, mgr.builder.func);
                let call = mgr.builder.ins().call(alloc_fref, &[size_val]);
                let ptr = mgr.builder.inst_results(call)[0];
                for i in 1..args.len() {
                    let field_val = args[i];
                    let offset = (i - 1) * 8;
                    mgr.builder.ins().store(MemFlags::new(), field_val, ptr, offset as i32);
                }
                return Ok(ptr);
            } else if func_name == "__union_new" {
                    // (size, tag, field1, ...) -> ptr
                    let size_val = args[0];
                    let tag_val = args[1];
                    let alloc_fid = jit.declare_function("au_alloc", cranelift_module::Linkage::Import, &{
                        let mut sig = jit.make_signature();
                        sig.params.push(AbiParam::new(types::I64));
                        sig.returns.push(AbiParam::new(types::I64));
                        sig
                    }).map_err(|e| format!("Failed to declare au_alloc: {}", e))?;
                    let alloc_fref = jit.declare_func_in_func(alloc_fid, mgr.builder.func);
                    let call = mgr.builder.ins().call(alloc_fref, &[size_val]);
                    let ptr = mgr.builder.inst_results(call)[0];
                    
                    // Store tag at offset 0
                    mgr.builder.ins().store(MemFlags::new(), tag_val, ptr, 0);
                    
                    // Store fields starting at offset 8
                    for i in 2..args.len() {
                        let field_val = args[i];
                        let offset = (i - 1) * 8;
                        mgr.builder.ins().store(MemFlags::new(), field_val, ptr, offset as i32);
                    }
                return Ok(ptr);
            }

            let func_ref = if let Some(&fid) = name_map.get(&func_name) {
                jit.declare_func_in_func(fid, mgr.builder.func)
            } else if let Some(&fid) = import_map.get(&func_name) {
                jit.declare_func_in_func(fid, mgr.builder.func)
            } else if func_name.starts_with("__") || func_name.starts_with("au_") {
                // Auto-declare external/internal builtin
                let mut sig = jit.make_signature();
                for _ in 0..args.len() {
                    sig.params.push(AbiParam::new(types::I64));
                }
                // Most return i64, some might be void but i64 is safe for now
                sig.returns.push(AbiParam::new(types::I64));
                let fid = jit.declare_function(&func_name, cranelift_module::Linkage::Import, &sig)
                    .map_err(|e| format!("Failed to declare builtin {}: {}", func_name, e))?;
                jit.declare_func_in_func(fid, mgr.builder.func)
            } else {
                return Err(format!("Call to unknown function: {}", func_name));
            };
            let call = mgr.builder.ins().call(func_ref, &args);
            let results = mgr.builder.inst_results(call);
            Ok(if results.is_empty() { mgr.builder.ins().iconst(types::I64, 0) } else { results[0] })
        }
        0x05 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            Ok(mgr.builder.ins().iadd(a, b))
        }
        0x06 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            Ok(mgr.builder.ins().isub(a, b))
        }
        0x10 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let cmp = mgr.builder.ins().icmp(IntCC::SignedLessThan, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x11 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let cmp = mgr.builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x12 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let cmp = mgr.builder.ins().icmp(IntCC::SignedLessThanOrEqual, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x13 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let cmp = mgr.builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x14 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let cmp = mgr.builder.ins().icmp(IntCC::Equal, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x15 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let cmp = mgr.builder.ins().icmp(IntCC::NotEqual, a, b);
            Ok(mgr.builder.ins().uextend(types::I64, cmp))
        }
        0x18 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            Ok(mgr.builder.ins().imul(a, b))
        }
        0x19 => {
            let a = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            Ok(mgr.builder.ins().bnot(a))
        }
        0x20 => {
            let ptr = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            Ok(mgr.builder.ins().load(types::I64, MemFlags::new(), ptr, 0))
        }
        _ => Err(format!("Unknown opcode: 0x{:02x}", opcode)),
    }
}

fn emit_stmt_list(
    jit: &mut JITModule,
    reader: &mut CpsReader,
    mgr: &mut BlockManager,
    vars: &mut HashMap<String, Variable>,
    name_map: &HashMap<String, FuncId>,
    import_map: &HashMap<String, FuncId>,
) -> Result<(), String> {
    while reader.remaining() > 0 {
        if mgr.terminated { return Ok(()); }
        match reader.peek_u8() {
            Some(0x07) => {
                reader.read_u8()?;
                if reader.peek_u8() == Some(0x04) {
                    reader.read_u8()?; // App
                    let func_name = reader.read_string()?;
                    let arg_count = reader.read_u32()?;
                    let mut args = Vec::new();
                    for _ in 0..arg_count {
                        args.push(emit_expr(jit, reader, mgr, vars, name_map, import_map)?);
                    }

                    if func_name == "__slot_get" || func_name == "__ptr_slot_get" {
                        let ptr = args[0];
                        let offset_val = args[1];
                        let addr = mgr.builder.ins().iadd(ptr, offset_val);
                        let val = mgr.builder.ins().load(types::I64, MemFlags::new(), addr, 0);
                        mgr.builder.ins().return_(&[val]);
                        mgr.terminated = true;
                        return Ok(());
                    } else if func_name == "__record_new" {
                        let size_val = args[0];
                        let alloc_fid = jit.declare_function("au_alloc", cranelift_module::Linkage::Import, &{
                            let mut sig = jit.make_signature();
                            sig.params.push(AbiParam::new(types::I64));
                            sig.returns.push(AbiParam::new(types::I64));
                            sig
                        }).map_err(|e| format!("Failed to declare au_alloc: {}", e))?;
                        let alloc_fref = jit.declare_func_in_func(alloc_fid, mgr.builder.func);
                        let call = mgr.builder.ins().call(alloc_fref, &[size_val]);
                        let ptr = mgr.builder.inst_results(call)[0];
                        for i in 1..args.len() {
                            let field_val = args[i];
                            let offset = (i - 1) * 8;
                            mgr.builder.ins().store(MemFlags::new(), field_val, ptr, offset as i32);
                        }
                        mgr.builder.ins().return_(&[ptr]);
                        mgr.terminated = true;
                        return Ok(());
                    }

                    let func_ref = if let Some(&fid) = name_map.get(&func_name) {
                        jit.declare_func_in_func(fid, mgr.builder.func)
                    } else if let Some(&fid) = import_map.get(&func_name) {
                        jit.declare_func_in_func(fid, mgr.builder.func)
                    } else if func_name.starts_with("__") || func_name.starts_with("au_") {
                        let mut sig = jit.make_signature();
                        for _ in 0..args.len() {
                            sig.params.push(AbiParam::new(types::I64));
                        }
                        sig.returns.push(AbiParam::new(types::I64));
                        let fid = jit.declare_function(&func_name, cranelift_module::Linkage::Import, &sig)
                            .map_err(|e| format!("Failed to declare builtin {}: {}", func_name, e))?;
                        jit.declare_func_in_func(fid, mgr.builder.func)
                    } else {
                        return Err(format!("Tail call to unknown function: {}", func_name));
                    };
                    mgr.emit_return_call(func_ref, &args);
                } else {
                    let val = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
                    mgr.emit_return(&[val]);
                }
                return Ok(());
            }
            Some(0x08) => {
                reader.read_u8()?;
                let cond = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
                let zero = mgr.builder.ins().iconst(types::I64, 0);
                let cond_bool = mgr.builder.ins().icmp(IntCC::NotEqual, cond, zero);
                
                let then_len = reader.read_u32()?;
                let then_data = reader.read_bytes(then_len as usize)?;
                let else_len = reader.read_u32()?;
                let else_data = reader.read_bytes(else_len as usize)?;

                let then_block = mgr.create_block();
                let else_block = mgr.create_block();
                let merge_block = mgr.create_block();
                mgr.builder.ins().brif(cond_bool, then_block, &[], else_block, &[]);
                
                // Then branch
                mgr.switch_to_block(then_block);
                let mut then_reader = CpsReader::new(then_data);
                emit_stmt_list(jit, &mut then_reader, mgr, vars, name_map, import_map)?;
                mgr.ensure_terminated(Some(merge_block));
                
                // Else branch
                mgr.switch_to_block(else_block);
                let mut else_reader = CpsReader::new(else_data);
                emit_stmt_list(jit, &mut else_reader, mgr, vars, name_map, import_map)?;
                mgr.ensure_terminated(Some(merge_block));
                
                // Merge
                mgr.switch_to_block(merge_block);
            }
            Some(0x09) => {
                reader.read_u8()?;
                
                let header_block = mgr.create_block();
                let body_block = mgr.create_block();
                let exit_block = mgr.create_block();
                
                mgr.emit_jump(header_block);
                
                // Header: check condition
                mgr.switch_to_block(header_block);
                let cond = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
                let zero = mgr.builder.ins().iconst(types::I64, 0);
                let cond_bool = mgr.builder.ins().icmp(IntCC::NotEqual, cond, zero);
                
                let body_len = reader.read_u32()?;
                let body_data = reader.read_bytes(body_len as usize)?;
                
                mgr.builder.ins().brif(cond_bool, body_block, &[], exit_block, &[]);
                
                // Body
                mgr.switch_to_block(body_block);
                let mut body_reader = CpsReader::new(body_data);
                emit_stmt_list(jit, &mut body_reader, mgr, vars, name_map, import_map)?;
                mgr.emit_jump(header_block);
                
                // Seal header only after back-edge is added
                // Wait, mgr.seal_all() handles sealing all blocks at the end.
                // But for loops, sealing early helps optimizations.
                // However, since we seal everything at the end, it's fine.
                
                // Exit
                mgr.switch_to_block(exit_block);
            }
            Some(0x0A) => {
                reader.read_u8()?;
                let cond = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
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
                    
                    mgr.builder.ins().brif(is_match, match_block, &[], next_case_block, &[]);
                    
                    // Match branch
                    mgr.switch_to_block(match_block);
                    let mut body_reader = CpsReader::new(body_data);
                    emit_stmt_list(jit, &mut body_reader, mgr, vars, name_map, import_map)?;
                    mgr.ensure_terminated(Some(merge_block));
                    
                    // Continue to next case
                    mgr.switch_to_block(next_case_block);
                }
                
                // Default branch (current block is now the 'last' next_case_block)
                let def_len = reader.read_u32()?;
                let def_data = reader.read_bytes(def_len as usize)?;
                let mut def_reader = CpsReader::new(def_data);
                emit_stmt_list(jit, &mut def_reader, mgr, vars, name_map, import_map)?;
                mgr.ensure_terminated(Some(merge_block));
                
                // Merge
                mgr.switch_to_block(merge_block);
            }
            Some(0x03) => {
                reader.read_u8()?;
                let name = reader.read_string()?;
                let val = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
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
                reader.read_u8()?;
                let ptr = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
                let val = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
                mgr.builder.ins().store(MemFlags::new(), val, ptr, 0);
            }
            Some(_) => {
                let _ = emit_expr(jit, reader, mgr, vars, name_map, import_map)?;
            }
            None => break,
        }
    }
    Ok(())
}

pub fn compile_cps_to_clif(jit: &mut JITModule, data: &[u8]) -> Result<CpsModule, String> {
    let mut reader = CpsReader::new(data);
    let magic = reader.read_u32()?;
    if magic != 0x43505331 { return Err(format!("Invalid magic: 0x{:08x}", magic)); }
    
    let func_count = reader.read_u32()?;
    let mut name_map = HashMap::new();
    let import_map = HashMap::new();
    
    let mut func_headers = Vec::new();
    for _ in 0..func_count {
        let name = reader.read_string()?;
        let param_count = reader.read_u32()?;
        let mut params = Vec::new();
        for _ in 0..param_count {
            params.push(reader.read_string()?);
        }
        let _ret_type = reader.read_u8()?;
        let body_len = reader.read_u32()?;
        let body_data = reader.read_bytes(body_len as usize)?;
        
        let mut sig = jit.make_signature();
        for _ in 0..param_count {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        
        let func_id = jit.declare_function(&name, Linkage::Export, &sig)
            .map_err(|e| format!("Declare {}: {:?}", name, e))?;
        
        name_map.insert(name.clone(), func_id);
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
        mgr.builder.append_block_params_for_function_params(entry_block);
        
        let mut vars = HashMap::new();
        for i in 0..param_count {
            let val = mgr.builder.block_params(entry_block)[i as usize];
            let var = mgr.builder.declare_var(types::I64);
            mgr.builder.def_var(var, val);
            let pname = param_names.get(i as usize).cloned().unwrap_or_else(|| format!("p{}", i));
            vars.insert(pname, var);
        }
        
        let mut body_reader = CpsReader::new(body_data);
        emit_stmt_list(jit, &mut body_reader, &mut mgr, &mut vars, &name_map, &import_map)?;
        
        mgr.ensure_terminated(None);
            mgr.seal_all();
        }
        builder.finalize();
        
        let flags = cranelift_codegen::settings::Flags::new(cranelift_codegen::settings::builder());
        if let Err(errors) = cranelift_codegen::verify_function(&ctx.func, &flags) {
            return Err(format!("Verifier failed for function '{}':\n{}", name, errors));
        }
        
        jit.define_function(func_id, &mut ctx)
            .map_err(|e| format!("Define {}: {:?}", name, e))?;
    }
    
    Ok(CpsModule { name_map })
}
