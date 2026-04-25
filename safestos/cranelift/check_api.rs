// Quick test to see what jump actually needs
use cranelift::prelude::*;
use cranelift_frontend::FunctionBuilder;

fn test_jump() {
    let mut func = cranelift::codegen::ir::Function::new();
    let mut fbc = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut func, &mut fbc);
    
    let block1 = builder.create_block();
    let block2 = builder.create_block();
    
    builder.append_block_param(block2, types::I64);
    
    let val = builder.ins().iconst(types::I64, 42);
    
    // This should work:
    // builder.ins().jump(block2, &[val]);
    
    println!("Check what BlockArg is");
}
