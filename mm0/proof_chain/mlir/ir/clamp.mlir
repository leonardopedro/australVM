// Generated from typed KernelIR by the Austral GPU backend (G20a). Do not edit.
// kernelir-sha256: 48caf04ea1714fba539ab596cd83d4f7ab537f3eea7837a5b129ccbecb9d8288
// numeric-profile: u32-modular-v1
// extent: 65
// abi: g19a-pending (memref per operand; not verified by an MLIR toolchain)
module attributes {austral.extent = 65 : i64, austral.numeric_profile = "u32-modular-v1", austral.abi = "g19a-pending"} {
  gpu.module @clamp {
    gpu.func @main(%arg0: memref<65xi32>, %arg1: memref<65xi32>) kernel attributes {gpu.known_block_size = array<i32: 64, 1, 1>} {
      %c_extent = arith.constant 65 : index
      %tid = gpu.thread_id x
      %bid = gpu.block_id x
      %bdim = gpu.block_dim x
      %bid_base = arith.muli %bid, %bdim : index
      %gid = arith.addi %bid_base, %tid : index
      %in_bounds = arith.cmpi ult, %gid, %c_extent : index
      scf.if %in_bounds {
        %arg0_v = memref.load %arg0[%gid] : memref<65xi32>
        %v0 = arith.constant 100 : i32
        %v1 = arith.cmpi ugt, %arg0_v, %v0 : i32
        %v3 = scf.if %v1 -> (i32) {
          %v2 = arith.constant 100 : i32
          scf.yield %v2 : i32
        } else {
          scf.yield %arg0_v : i32
        }
        memref.store %v3, %arg1[%gid] : memref<65xi32>
      }
      gpu.return
    }
  }
}
