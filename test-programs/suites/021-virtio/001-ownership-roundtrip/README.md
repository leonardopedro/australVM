Ownership roundtrip through stdlib/sys/Virtio.aui: the compiled program
allocates a 64-byte DMA buffer, submits it (linear consumption), completes
it (ownership restored), reads the length through a borrow, and frees it
exactly once. Printing `64` proves the value survived the two ownership
transfers.