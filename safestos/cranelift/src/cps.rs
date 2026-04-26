/// CPS → Cranelift with Tail Call Support
///
/// Compiles CPS IR binary format to Cranelift IR with guaranteed O(1)
/// stack space via tail calls.
///
/// IR Format:
/// [magic: u32 = 0x43505331][functions: u32]
/// For each function:
///   [name_len: u32][name: u8*][params: u32][return_type: u8][body_len: u32][body: u8*]
///
/// Instructions:
/// 0x01: IntLit(value: i64)
/// 0x02: Var(name: string)
/// 0x03: Let(name, value, body)
/// 0x04: App(func, args...)
/// 0x05: Add(a, b)
/// 0x06: Sub(a, b)
/// 0x07: Return(value)

use cranelift::prelude::*;
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use std::collections::HashMap;

use cranelift_codegen::isa::CallConv;

const CPS_MAGIC: u32 = 0x43505331;

pub struct CompiledFunc {
    pub name: String,
    pub id: FuncId,
}

pub struct CompiledModule {
    pub functions: Vec<CompiledFunc>,
    pub name_map: HashMap<String, FuncId>,
}

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
        let bytes = &self.data[self.pos..self.pos + 4];
        self.pos += 4;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn read_u64(&mut self) -> Result<u64, String> {
        if self.pos + 8 > self.data.len() {
            return Err("EOF".to_string());
        }
        let bytes = &self.data[self.pos..self.pos + 8];
        self.pos += 8;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
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
        let bytes = &self.data[self.pos..self.pos + len];
        self.pos += len;
        String::from_utf8(bytes.to_vec()).map_err(|_| "Invalid UTF8".to_string())
    }

    pub fn peek_u8(&self) -> Option<u8> {
        if self.pos < self.data.len() {
            Some(self.data[self.pos])
        } else {
            None
        }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
}

struct FuncHeader {
    name: String,
    params: u32,
    param_names: Vec<String>,
    body_offset: usize,
    body_len: u32,
}

pub fn build_simple(jit: &mut JITModule) -> Result<CompiledFunc, String> {
    let mut sig = Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
    sig.returns.push(AbiParam::new(types::I64));

    let func_id = jit
        .declare_function("return_42", Linkage::Local, &sig)
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
            eprintln!("  [ACTION] return instruction emitted");
    builder.finalize();

    jit.define_function(func_id, &mut ctx)
        .map_err(|e| format!("Define: {:?}", e))?;

    Ok(CompiledFunc {
        name: "return_42".to_string(),
        id: func_id,
    })
}

/// Compile entire CPS IR module to Cranelift
///
/// Three-pass approach:
/// 1. Parse all function headers
/// 2. Declare all functions (so they can reference each other)
/// 3. Define all function bodies
pub fn compile_cps_to_clif(
    jit: &mut JITModule,
    ir_data: &[u8],
) -> Result<CompiledModule, String> {
    let mut reader = CpsReader::new(ir_data);

    if reader.read_u32()? != CPS_MAGIC {
        return Err("Invalid magic".to_string());
    }

    let func_count = reader.read_u32()?;
    if func_count == 0 {
        return Err("No functions".to_string());
    }

    // Pass 1: Parse headers
    let mut headers: Vec<FuncHeader> = Vec::new();
    for _ in 0..func_count {
        let name = reader.read_string()?;
        let params = reader.read_u32()?;
        let _return_type = reader.read_u8()?;
        let mut param_names = Vec::new();
        for _ in 0..params {
            param_names.push(reader.read_string()?);
        }
        let body_len = reader.read_u32()?;
        let body_offset = reader.pos;

        headers.push(FuncHeader {
            name,
            params,
            param_names,
            body_offset,
            body_len,
        });

        reader.pos = body_offset + body_len as usize;
    }

    // Pass 2: Declare all functions
    let mut name_map: HashMap<String, FuncId> = HashMap::new();
    let mut func_ids: Vec<FuncId> = Vec::new();

    for header in &headers {
        let mut sig =
            Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
        for _ in 0..header.params {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = jit
            .declare_function(&header.name, Linkage::Local, &sig)
            .map_err(|e| format!("Declare {:?}: {:?}", header.name, e))?;

        name_map.insert(header.name.clone(), func_id);
        func_ids.push(func_id);
    }

    // Also scan for external function references and declare them as imports
    // (We do this in a pre-scan of the body data)
    let mut import_names: HashMap<String, u32> = HashMap::new();
    for header in &headers {
        let body_data =
            &ir_data[header.body_offset..header.body_offset + header.body_len as usize];
        scan_for_calls(body_data, &name_map, &mut import_names);
    }

    // Declare imports
    let mut import_map: HashMap<String, FuncId> = HashMap::new();
    let mut stubs_to_define: Vec<(FuncId, String, u32)> = Vec::new();
    for (name, arg_count) in &import_names {
        let mut sig =
            Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
        for _ in 0..*arg_count {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = jit
            .declare_function(name, Linkage::Local, &sig)  // Local to avoid unresolved symbols
            .map_err(|e| format!("Declare import {:?}: {:?}", name, e))?;
        import_map.insert(name.clone(), func_id);
        stubs_to_define.push((func_id, name.clone(), *arg_count));
    }

    // Pass 3: Define all function bodies
    let mut compiled = Vec::new();

    for (i, header) in headers.iter().enumerate() {
        let func_id = func_ids[i];
        let body_data =
            &ir_data[header.body_offset..header.body_offset + header.body_len as usize];

        eprintln!("CPS: Defining function '{}' (params={}, body_len={})", header.name, header.params, header.body_len);

        define_function(
            jit,
            func_id,
            &header.name,
            header.params,
            &header.param_names,
            body_data,
            &name_map,
            &import_map,
        )?;

        compiled.push(CompiledFunc {
            name: header.name.clone(),
            id: func_id,
        });
    }


    // Define stub bodies for imported primitives
    for (func_id, name, arg_count) in stubs_to_define {
        define_stub_function(jit, func_id, &name, arg_count)?;
    }

    Ok(CompiledModule {
        functions: compiled,
        name_map,
    })
}

/// Scan body data for App instructions referencing external functions
fn scan_for_calls(
    body_data: &[u8],
    name_map: &HashMap<String, FuncId>,
    imports: &mut HashMap<String, u32>,
) {
    let mut reader = CpsReader::new(body_data);
    while reader.remaining() > 0 {
            println!("CPS DEBUG: peek at pos {}, remaining={}", reader.pos, reader.remaining());
        match reader.peek_u8() {
            None => break,
            Some(opcode) => {
                // Consume the opcode
                if reader.read_u8().is_err() {
                    break;
                }
                match opcode {
                    0x01 => {
                        // IntLit: skip 8 bytes
                        if reader.read_u64().is_err() {
                            break;
                        }
                    }
                    0x02 => {
                        // Var: skip string
                        if reader.read_string().is_err() {
                            break;
                        }
                    }
                    0x03 => {
                        // Let: skip name, then value and body continue
                        if reader.read_string().is_err() {
                            break;
                        }
                        // value and body are subsequent instructions, continue loop
                    }
                    0x04 => {
                        // App: read func name and arg count
                        let func_name = match reader.read_string() {
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        let arg_count = match reader.read_u32() {
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        if !name_map.contains_key(&func_name) && !imports.contains_key(&func_name)
                        {
                            imports.insert(func_name, arg_count);
                        }
                        // Args are subsequent instructions, continue loop
                        let _ = arg_count; // consumed above
                    }
                    0x05 | 0x06 => {
                        // Add/Sub: two sub-expressions follow
                    }
                    0x07 => {
                        // Return: one sub-expression follows
                    }
                    _ => break,
                }
            }
        }
    }
}

/// Define a single function body
fn define_function(
    jit: &mut JITModule,
    func_id: FuncId,
    _name: &str,
    params: u32,
    param_names: &[String],
    body_data: &[u8],
    name_map: &HashMap<String, FuncId>,
    import_map: &HashMap<String, FuncId>,
) -> Result<(), String> {
    let mut sig = Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
    for _ in 0..params {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));

    let mut ctx = cranelift::codegen::Context::new();
    ctx.func.signature = sig;

    let mut func_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);

    let entry_block = builder.create_block();
    println!("CPS DEBUG: define_function START entry_block created");
    builder.switch_to_block(entry_block);
    builder.seal_block(entry_block);

    let mut vars: HashMap<String, Value> = HashMap::new();

    for i in 0..params {
        let val = builder.append_block_param(entry_block, types::I64);
        let pname = param_names.get(i as usize)
            .cloned()
            .unwrap_or_else(|| format!("param_{}", i));
        vars.insert(pname, val);
    }

    let mut reader = CpsReader::new(body_data);
    let mut last_value: Option<Value> = None;

    while reader.remaining() > 0 {
        match reader.peek_u8() {
            Some(0x07) => {
                eprintln!("  define_function loop: found 0x07, will return");
                reader.read_u8()?;
                let val = emit_expr(jit, &mut reader, &mut builder, &mut vars, name_map, import_map)?;
                builder.ins().return_(&[val]);
            eprintln!("  [ACTION] return instruction emitted");
                last_value = Some(val);
                break;
            }
            Some(_) => {
                eprintln!("  define_function loop: other opcode");
                last_value = Some(emit_expr(
                    jit,
                    &mut reader,
                    &mut builder,
                    &mut vars,
                    name_map,
                    import_map,
                )?);
            }
            None => break,
        }
    }

    if last_value.is_none() {
        let zero = builder.ins().iconst(types::I64, 0);
        builder.ins().return_(&[zero]);
    }

    builder.seal_block(entry_block);
    builder.finalize();

    jit.define_function(func_id, &mut ctx)
        .map_err(|e| format!("Define {:?}: {:?}", _name, e))?;

    Ok(())
}

/// Emit a single expression
fn emit_expr(
    jit: &mut JITModule,
    reader: &mut CpsReader,
    builder: &mut FunctionBuilder,
    vars: &mut HashMap<String, Value>,
    name_map: &HashMap<String, FuncId>,
    import_map: &HashMap<String, FuncId>,
) -> Result<Value, String> {
    let opcode = reader.read_u8()?;
    eprintln!("  emit_expr: opcode={:#x} at pos={}", opcode, reader.pos - 1);

    match opcode {
        0x01 => {
            let val = reader.read_u64()? as i64;
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
            let value = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let old = vars.insert(name.clone(), value);
            let result = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
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
                args.push(emit_expr(jit, reader, builder, vars, name_map, import_map)?);
            }

            // Resolve function reference
            let func_ref = if let Some(&fid) = name_map.get(&func_name) {
                jit.declare_func_in_func(fid, builder.func)
            } else if let Some(&fid) = import_map.get(&func_name) {
                jit.declare_func_in_func(fid, builder.func)
            } else {
                // Unknown function — return 0 as stub
                eprintln!("CPS WARN: unknown function '{}', stubbing", func_name);
                return Ok(builder.ins().iconst(types::I64, 0));
            };

            let is_tail = matches!(reader.peek_u8(), Some(0x07));

            if is_tail {
                builder.ins().return_call(func_ref, &args);
                Ok(builder.ins().iconst(types::I64, 0))
            } else {
                let call = builder.ins().call(func_ref, &args);
                let results = builder.inst_results(call);
                if results.is_empty() {
                    // Void call — return 0
                    Ok(builder.ins().iconst(types::I64, 0))
                } else {
                    Ok(results[0])
                }
            }
        }

        0x08 => { 
            let cond = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let t = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let e = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let zero = builder.ins().iconst(types::I64, 0);
            let cond_bool = builder.ins().icmp(IntCC::NotEqual, cond, zero);
            Ok(builder.ins().select(cond_bool, t, e))
        },
        0x05 => {
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            Ok(builder.ins().iadd(a, b))
        }

        0x06 => {
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            Ok(builder.ins().isub(a, b))
        }

        0x10 => {
            // CmpLt: returns 1 if a < b, 0 otherwise
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let cmp = builder.ins().icmp(IntCC::SignedLessThan, a, b);
            Ok(builder.ins().uextend(types::I64, cmp))
        }

        0x13 => {
            // CmpGte: a >= b  <=>  !(a < b)
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let cmp = builder.ins().icmp(IntCC::SignedLessThan, a, b);
            let not_cmp = builder.ins().bxor_imm(cmp, 1);
            Ok(builder.ins().uextend(types::I64, not_cmp))
        }

        0x14 => {
            // CmpEq
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let cmp = builder.ins().icmp(IntCC::Equal, a, b);
            Ok(builder.ins().uextend(types::I64, cmp))
        }

        0x15 => {
            // CmpNeq
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let cmp = builder.ins().icmp(IntCC::NotEqual, a, b);
            Ok(builder.ins().uextend(types::I64, cmp))
        }

        0x16 => {
            // And
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            Ok(builder.ins().band(a, b))
        }

        0x17 => {
            // Or
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            Ok(builder.ins().bor(a, b))
        }

        0x18 => {
            // Mul
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            let b = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            Ok(builder.ins().imul(a, b))
        }

        0x19 => {
            // Not
            let a = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            Ok(builder.ins().bxor_imm(a, 1))
        }

        0x07 => {
            let val = emit_expr(jit, reader, builder, vars, name_map, import_map)?;
            builder.ins().return_(&[val]);
            eprintln!("  [ACTION] return instruction emitted");
            println!("CPS DEBUG: 0x07 RETURN with val {:?}", val);
            Ok(builder.ins().iconst(types::I64, 0))
        }

        _ => { println!("CPS DEBUG: Unknown opcode {:#x} at {}", opcode, reader.pos - 1); Err(format!("Unknown opcode: {:#x}", opcode)) }
    }
}

/// Define stub functions for imported primitives
fn define_stub_function(
    jit: &mut JITModule,
    func_id: FuncId,
    name: &str,
    arg_count: u32,
) -> Result<(), String> {
    let mut sig = Signature::new(CallConv::triple_default(&target_lexicon::Triple::host()));
    for _ in 0..arg_count {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));

    let mut ctx = cranelift::codegen::Context::new();
    ctx.func.signature = sig;

    let mut func_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);

    let block = builder.create_block();
    builder.switch_to_block(block);
    
    // Add params to block
    for _ in 0..arg_count {
        builder.append_block_param(block, types::I64);
    }
    builder.seal_block(block);

    // Known primitives with behavior
    let result = if name.contains("trappingAdd") {
        let a = builder.block_params(block)[0];
        let b = builder.block_params(block)[1];
        builder.ins().iadd(a, b)
    } else if name.contains("trappingSubtract") {
        let a = builder.block_params(block)[0];
        let b = builder.block_params(block)[1];
        builder.ins().isub(a, b)
    } else if name.contains("trappingMultiply") {
        let a = builder.block_params(block)[0];
        let b = builder.block_params(block)[1];
        builder.ins().imul(a, b)
    } else if name.contains("ExitSuccess") {
        builder.ins().iconst(types::I64, 0)
    } else if name.contains("__union_new") {
        // Placeholder for memory allocation
        builder.ins().iconst(types::I64, 0)
    } else {
        eprintln!("CPS WARNING: Unknown primitive '{}', stubbing to 0", name);
        builder.ins().iconst(types::I64, 0)
    };

    builder.ins().return_(&[result]);
    builder.finalize();

    jit.define_function(func_id, &mut ctx)
        .map_err(|e| format!("Define stub {:?}: {:?}", name, e))?;

    Ok(())
}
