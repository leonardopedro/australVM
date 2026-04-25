/// CPS → Cranelift with Tail Call Support (Structural)

use cranelift::prelude::*;
use cranelift_codegen::isa::CallConv;
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

const CPS_MAGIC: u32 = 0x43505331;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u8)]
pub enum TypeTag {
    I64 = 3,
    // Others omitted for brevity
}

impl TypeTag {
    pub fn to_cranelift(&self) -> Type {
        match self {
            TypeTag::I64 => types::I64,
            _ => types::I64,
        }
    }
}

pub struct CompiledFunc {
    pub name: String,
    pub id: FuncId,
}

/// Simple demo: fn() -> i64 { return 42; }
pub fn build_simple(
    jit: &mut cranelift_jit::JITModule,
) -> Result<CompiledFunc, String> {
    let call_conv = CallConv::triple_default(&target_lexicon::Triple::host());
    let mut sig = Signature::new(call_conv);
    sig.returns.push(AbiParam::new(types::I64));

    let func_id = jit.declare_function("return_42", Linkage::Local, &sig)
        .map_err(|e| format!("Declare: {:?}", e))?;

    let mut ctx = codegen::Context::new();
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
