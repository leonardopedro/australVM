module attributes {austral.abi = "g19a-pending", austral.extent = 65 : i64, austral.numeric_profile = "u32-modular-v1"} {
  gpu.module @clamp {
    gpu.func @main(%arg0: memref<65xi32>, %arg1: memref<65xi32>) kernel attributes {gpu.known_block_size = array<i32: 64, 1, 1>} {
      %0 = llvm.mlir.constant(100 : i32) : i32
      %1 = llvm.mlir.constant(65 : index) : i64
      %thread_id_x = gpu.thread_id  x
      %2 = builtin.unrealized_conversion_cast %thread_id_x : index to i64
      %block_id_x = gpu.block_id  x
      %3 = builtin.unrealized_conversion_cast %block_id_x : index to i64
      %block_dim_x = gpu.block_dim  x
      %4 = builtin.unrealized_conversion_cast %block_dim_x : index to i64
      %5 = llvm.mul %3, %4 : i64
      %6 = llvm.add %5, %2 : i64
      %7 = builtin.unrealized_conversion_cast %6 : i64 to index
      %8 = llvm.icmp "ult" %6, %1 : i64
      llvm.cond_br %8, ^bb1, ^bb5
    ^bb1:  // pred: ^bb0
      %9 = memref.load %arg0[%7] : memref<65xi32>
      %10 = llvm.icmp "ugt" %9, %0 : i32
      llvm.cond_br %10, ^bb2(%0 : i32), ^bb2(%9 : i32)
    ^bb2(%11: i32):  // 2 preds: ^bb1, ^bb1
      llvm.br ^bb3
    ^bb3:  // pred: ^bb2
      llvm.br ^bb4
    ^bb4:  // pred: ^bb3
      memref.store %11, %arg1[%7] : memref<65xi32>
      llvm.br ^bb5
    ^bb5:  // 2 preds: ^bb0, ^bb4
      gpu.return
    }
  }
}


