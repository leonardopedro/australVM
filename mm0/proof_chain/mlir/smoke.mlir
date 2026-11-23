// smoke.mlir — G19a's handwritten toolchain fixture.
//
// Deliberately NOT produced by the Austral compiler: it exists to pin a
// toolchain round trip (verify → lower → PTX) independently of `Gpu_mlir`, so a
// failure in the probe is a fact about the tools rather than about the emitter.
// The shape mirrors the v1 contract: two `memref<65xi32>` operands, one static
// extent, a dominating tail guard, one element per thread.
module attributes {austral.purpose = "g19a-smoke"} {
  gpu.module @copy_smoke {
    gpu.func @copy(%in: memref<65xi32>, %out: memref<65xi32>) kernel
        attributes {gpu.known_block_size = array<i32: 64, 1, 1>} {
      %c65 = arith.constant 65 : index
      %tid = gpu.thread_id x
      %in_bounds = arith.cmpi ult, %tid, %c65 : index
      scf.if %in_bounds {
        %v = memref.load %in[%tid] : memref<65xi32>
        memref.store %v, %out[%tid] : memref<65xi32>
      }
      gpu.return
    }
  }
}
