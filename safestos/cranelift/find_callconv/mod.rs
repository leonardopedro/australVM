// Check what module CallConv is in
use cranelift::prelude::*;
// Signature works from prelude

// Now find where CallConv is:
// Attempt 1: cranelift_codegen::ir::CallConv
// Attempt 2: cranelift::isa::CallConv

fn _functions() {
    // Just need to find the right module
    let _host_triple = target_lexicon::Triple::host();
    // The triple_default function is on CallConv
}
