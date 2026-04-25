/*
 * rust_bridge.c
 * C stub for linking Rust library with OCaml
 * Provides scheduler_dispatch symbol for linking
 */

#include <stdint.h>

/* Forward declaration from Rust bridge */
extern int64_t rust_bridge_compile(const unsigned char* data, int len);
extern int rust_bridge_init(void);
extern int rust_bridge_ready(void);

/* OCaml's scheduler_dispatch for linker symbol resolution */
void scheduler_dispatch(void (*func)(void*), void* state) {
    /* This is called by compiled cells to enqueue work */
    /* Real implementation is in runtime/scheduler.c */
    /* This stub ensures linking succeeds */
    if (func) {
        func(state);
    }
}

/* Support function for OCaml FFI */
int64_t compile_to_function(const unsigned char* ir_data, int ir_len) {
    return rust_bridge_compile(ir_data, ir_len);
}

/* Initialize bridge */
int initialize_bridge(void) {
    return rust_bridge_init();
}

/* Check if ready */
int bridge_is_ready(void) {
    return rust_bridge_ready();
}
