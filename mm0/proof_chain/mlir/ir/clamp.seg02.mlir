module attributes {austral.abi = "g19a-pending", austral.extent = 65 : i64, austral.numeric_profile = "u32-modular-v1"} {
  gpu.module @clamp {
    gpu.func @main(%arg0: memref<65xi32>, %arg1: memref<65xi32>) kernel attributes {gpu.known_block_size = array<i32: 64, 1, 1>} {
      %c65 = arith.constant 65 : index
      %thread_id_x = gpu.thread_id  x
      %block_id_x = gpu.block_id  x
      %block_dim_x = gpu.block_dim  x
      %0 = arith.muli %block_id_x, %block_dim_x : index
      %1 = arith.addi %0, %thread_id_x : index
      %2 = arith.cmpi ult, %1, %c65 : index
      scf.if %2 {
        %3 = memref.load %arg0[%1] : memref<65xi32>
        %c100_i32 = arith.constant 100 : i32
        %4 = arith.cmpi ugt, %3, %c100_i32 : i32
        %5 = scf.if %4 -> (i32) {
          %c100_i32_0 = arith.constant 100 : i32
          scf.yield %c100_i32_0 : i32
        } else {
          scf.yield %3 : i32
        }
        memref.store %5, %arg1[%1] : memref<65xi32>
      }
      gpu.return
    }
  }
}


