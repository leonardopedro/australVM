module attributes {austral.abi = "g19a-pending", austral.extent = 65 : i64, austral.numeric_profile = "u32-modular-v1"} {
  gpu.module @clamp {
    gpu.func @main(%arg0: memref<65xi32>, %arg1: memref<65xi32>) kernel attributes {gpu.known_block_size = array<i32: 64, 1, 1>} {
      %c100_i32 = arith.constant 100 : i32
      %c65 = arith.constant 65 : index
      %thread_id_x = gpu.thread_id  x
      %0 = arith.cmpi ult, %thread_id_x, %c65 : index
      scf.if %0 {
        %1 = memref.load %arg0[%thread_id_x] : memref<65xi32>
        %2 = arith.cmpi ugt, %1, %c100_i32 : i32
        %3 = arith.select %2, %c100_i32, %1 : i32
        memref.store %3, %arg1[%thread_id_x] : memref<65xi32>
      }
      gpu.return
    }
  }
}

