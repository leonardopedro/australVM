/// CPS → Cranelift with Tail Call Support
/// 
/// This module compiles a simple CPS IR format to Cranelift IR with
/// guaranteed O(1) stack space via tail calls.

use cranelift::prelude::*;
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use std::collections::HashMap;

// CallConv is not in prelude in 0.131
// It's in cranelift_codegen::isa
use cranelift_codegen::isa::CallConv;

/// Magic bytes to identify valid CPS IR
const CPS_MAGIC: u32 = 0x43505331;

/// Compiled function handle
pub struct CompiledFunc {
    pub name: String,
    pub id: FuncId,
}

/// Binary CPS IR reader
pub struct CpsReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CpsReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn read_u32(&mut self) -> Result<u32, String> {
        if self.pos + 4 > self.data.len() {
            return Err("EOF".to_string());
        }
        let bytes = &self.data[self.pos..self.pos+4];
        self.pos += 4;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn read_u8(&mut self) -> Result<u8, String> {
        if self.pos >= self.data.len() {
            return Err("EOF".to_string());
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    pub fn read_string(&mut self) -> Result<String, String> {
        let len = self.read_u32()? as usize;
        if self.pos + len > self.data.len() {
            return Err("EOF".to_string());
        }
        let bytes = &self.data[self.pos..self.pos+len];
        self.pos += len;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| "Invalid UTF8".to_string())
    }

    pub fn peek_u8(&self) -> Option<u8> {
        if self.pos < self.data.len() {
            Some(self.data[self.pos])
        } else {
            None
        }
    }
}

/// Simple demo: fn() -> i64 { return 42; }
pub fn build_simple(
    jit: &mut JITModule,
) -> Result<CompiledFunc, String> {
    let mut sig = Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
    sig.returns.push(AbiParam::new(types::I64));

    let func_id = jit.declare_function("return_42", Linkage::Local, &sig)
        .map_err(|e| format!("Declare: {:?}", e))?;

    let mut ctx = cranelift::codegen::Context::new();
    ctx.func.signature = sig;
    let mut func_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);

    let block = builder.create_block();
    builder.switch_to_block(block);
    builder.seal_block(block);
    let val = builder.ins().iconst(types::I64, 42);
    builder.ins().return_(&[val]);
    builder.finalize();

    jit.define_function(func_id, &mut ctx)
        .map_err(|e| format!("Define: {:?}", e))?;

    Ok(CompiledFunc { name: "return_42".to_string(), id: func_id })
}

/// Compile CPS IR to Cranelift
/// 
/// IR format:
/// [magic: u32 = 0x43505331][functions: u32]
/// For each function:
///   [name_len][name][params][return_type][body_len][body...]
/// 
/// Instructions:
/// 0x01: IntLit(value: i64)
/// 0x02: Var(name: string)
/// 0x03: Let(name, value, body)
/// 0x04: App(func, args...) [followed by 0x07 for tail call]
/// 0x05: Add(a, b)
/// 0x06: Sub(a, b)
/// 0x07: Return(value)
pub fn compile_cps_to_clif(
    jit: &mut JITModule,
    ir_data: &[u8],
) -> Result<CompiledFunc, String> {
    let mut reader = CpsReader::new(ir_data);
    
    if reader.read_u32()? != CPS_MAGIC {
        return Err("Invalid magic".to_string());
    }
    
    let func_count = reader.read_u32()?;
    if func_count == 0 {
        return Err("No functions".to_string());
    }
    
    // For now, compile just the first function
    compile_function(jit, &mut reader)
}

fn compile_function(
    jit: &mut JITModule,
    reader: &mut CpsReader,
) -> Result<CompiledFunc, String> {
    let name = reader.read_string()?;
    let params = reader.read_u32()?;
    let _return_type = reader.read_u8()?; // Currently always I64
    
    // Signature
    let mut sig = Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
    for _ in 0..params {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));
    
    let func_id = jit.declare_function(&name, Linkage::Local, &sig)
        .map_err(|e| format!("Declare {:?}: {:?}", name, e))?;
    
    let mut ctx = cranelift::codegen::Context::new();
    ctx.func.signature = sig;
    let mut func_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
    
    // Entry block
    let block = builder.create_block();
    builder.switch_to_block(block);
    
    // Parameters
    let mut vars = HashMap::new();
    for i in 0..params {
        let val = builder.block_params(block)[i as usize];
        vars.insert(format!("param_{}", i), val);
    }
    
    // Body
    let body_len = reader.read_u32()?;
    let body_start = reader.pos;
    
    // Emit till we hit return or end
    while reader.pos < body_start + body_len as usize {
        let result = emit_instruction(jit, reader, &mut builder, block, &mut vars)?;
        let is_return = matches!(reader.peek_u8(), Some(0x07));
        
        if is_return {
            // Handled by 0x07 case
            break;
        } else {
            // Non-terminating - need a value but nothing to do with it
            // This is CPS, so values flow via variables
        }
    }
    
    builder.seal_block(block);
    builder.finalize();
    
    jit.define_function(func_id, &mut ctx)
        .map_err(|e| format!("Define: {:?}", e))?;
    
    Ok(CompiledFunc { name, id: func_id })
}

fn emit_instruction(
    jit: &mut JITModule,
    reader: &mut CpsReader,
    builder: &mut FunctionBuilder,
    block: cranelift::codegen::ir::Block,
    vars: &mut HashMap<String, cranelift::codegen::ir::Value>,
) -> Result<cranelift::codegen::ir::Value, String> {
    let opcode = reader.read_u8()?;
    
    match opcode {
        0x01 => {
            let val = reader.read_u32()? as i64;
            Ok(builder.ins().iconst(types::I64, val))
        }
        
        0x02 => {
            let name = reader.read_string()?;
            vars.get(&name)
                .cloned()
                .ok_or_else(|| format!("Undefined var: {}", name))
        }
        
        0x03 => {
            let name = reader.read_string()?;
            let value = emit_instruction(jit, reader, builder, block, vars)?;
            let old = vars.insert(name.clone(), value);
            let result = emit_instruction(jit, reader, builder, block, vars)?;
            if let Some(v) = old {
                vars.insert(name, v);
            } else {
                vars.remove(&name);
            }
            Ok(result)
        }
        
        0x04 => {
            let func_name = reader.read_string()?;
            let arg_count = reader.read_u32()?;
            let mut args = Vec::new();
            
            for _ in 0..arg_count {
                args.push(emit_instruction(jit, reader, builder, block, vars)?);
            }
            
            // Check if next is Return = tail call
            let is_tail = matches!(reader.peek_u8(), Some(0x07));
            
            // Declare/call function
            let mut sig = Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
            for _ in 0..arg_count {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            
            let func_ref = jit.declare_function(&func_name, Linkage::Local, &sig)
                .map_err(|e| format!("Declare {:?}: {:?}", func_name, e))?;
            let imported = jit.declare_func_in_func(func_ref, &mut builder.func);
            
            if is_tail {
                // THIS IS THE KEY: return_call = O(1) stack!
                builder.ins().return_call(imported, &args);
                Ok(builder.ins().iconst(types::I64, 0)) // Won't execute
            } else {
                let call = builder.ins().call(imported, &args);
                Ok(builder.inst_results(call)[0])
            }
        }
        
        0x05 => {
            let a = emit_instruction(jit, reader, builder, block, vars)?;
            let b = emit_instruction(jit, reader, builder, block, vars)?;
            Ok(builder.ins().iadd(a, b))
        }
        
        0x06 => {
            let a = emit_instruction(jit, reader, builder, block, vars)?;
            let b = emit_instruction(jit, reader, builder, block, vars)?;
            Ok(builder.ins().isub(a, b))
        }
        
        0x07 => {
            let val = emit_instruction(jit, reader, builder, block, vars)?;
            builder.ins().return_(&[val]);
            Ok(builder.ins().iconst(types::I64, 0))
        }
        
        _ => Err(format!("Unknown opcode: {:#x}", opcode)),
    }
}

/// Helper to declare functions for recursive calls
pub fn declare_func(
    jit: &mut JITModule,
    name: &str,
    params: u32,
) -> Result<cranelift::codegen::ir::FuncRef, String> {
    let mut sig = Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
    for _ in 0..params {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));
    
    let func_id = jit.declare_function(name, Linkage::Local, &sig)
        .map_err(|e| format!("Declare {:?}: {:?}", name, e))?;
    
    Ok(jit.declare_func_in_func(func_id, &mut cranelift::codegen::Context::new().func))
}
