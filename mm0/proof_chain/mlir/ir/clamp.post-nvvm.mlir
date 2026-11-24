gpu.module @clamp [#nvvm.target<chip = "sm_70">]  {
  llvm.func @main(%arg0: !llvm.ptr, %arg1: !llvm.ptr, %arg2: i64, %arg3: i64, %arg4: i64, %arg5: !llvm.ptr, %arg6: !llvm.ptr, %arg7: i64, %arg8: i64, %arg9: i64) attributes {gpu.kernel, gpu.known_block_size = array<i32: 64, 1, 1>, nvvm.kernel} {
    %0 = llvm.mlir.undef : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)>
    %1 = llvm.insertvalue %arg5, %0[0] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %2 = llvm.insertvalue %arg6, %1[1] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %3 = llvm.insertvalue %arg7, %2[2] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %4 = llvm.insertvalue %arg8, %3[3, 0] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %5 = llvm.insertvalue %arg9, %4[4, 0] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %6 = builtin.unrealized_conversion_cast %5 : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> to memref<65xi32>
    %7 = builtin.unrealized_conversion_cast %6 : memref<65xi32> to !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)>
    %8 = llvm.mlir.undef : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)>
    %9 = llvm.insertvalue %arg0, %8[0] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %10 = llvm.insertvalue %arg1, %9[1] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %11 = llvm.insertvalue %arg2, %10[2] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %12 = llvm.insertvalue %arg3, %11[3, 0] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %13 = llvm.insertvalue %arg4, %12[4, 0] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %14 = builtin.unrealized_conversion_cast %13 : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> to memref<65xi32>
    %15 = builtin.unrealized_conversion_cast %14 : memref<65xi32> to !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)>
    %16 = llvm.mlir.constant(100 : i32) : i32
    %17 = llvm.mlir.constant(65 : index) : i64
    %18 = nvvm.read.ptx.sreg.tid.x : i32
    %19 = llvm.sext %18 : i32 to i64
    %20 = builtin.unrealized_conversion_cast %19 : i64 to index
    %21 = builtin.unrealized_conversion_cast %20 : index to i64
    %22 = nvvm.read.ptx.sreg.ctaid.x : i32
    %23 = llvm.sext %22 : i32 to i64
    %24 = builtin.unrealized_conversion_cast %23 : i64 to index
    %25 = builtin.unrealized_conversion_cast %24 : index to i64
    %26 = nvvm.read.ptx.sreg.ntid.x : i32
    %27 = llvm.sext %26 : i32 to i64
    %28 = builtin.unrealized_conversion_cast %27 : i64 to index
    %29 = builtin.unrealized_conversion_cast %28 : index to i64
    %30 = llvm.mul %25, %29 : i64
    %31 = llvm.add %30, %21 : i64
    %32 = builtin.unrealized_conversion_cast %31 : i64 to index
    %33 = llvm.icmp "ult" %31, %17 : i64
    llvm.cond_br %33, ^bb1, ^bb5
  ^bb1:  // pred: ^bb0
    %34 = llvm.extractvalue %15[1] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %35 = llvm.getelementptr %34[%31] : (!llvm.ptr, i64) -> !llvm.ptr, i32
    %36 = llvm.load %35 : !llvm.ptr -> i32
    %37 = llvm.icmp "ugt" %36, %16 : i32
    llvm.cond_br %37, ^bb2(%16 : i32), ^bb2(%36 : i32)
  ^bb2(%38: i32):  // 2 preds: ^bb1, ^bb1
    llvm.br ^bb3
  ^bb3:  // pred: ^bb2
    llvm.br ^bb4
  ^bb4:  // pred: ^bb3
    %39 = llvm.extractvalue %7[1] : !llvm.struct<(ptr, ptr, i64, array<1 x i64>, array<1 x i64>)> 
    %40 = llvm.getelementptr %39[%31] : (!llvm.ptr, i64) -> !llvm.ptr, i32
    llvm.store %38, %40 : i32, !llvm.ptr
    llvm.br ^bb5
  ^bb5:  // 2 preds: ^bb0, ^bb4
    llvm.return
  }
}

