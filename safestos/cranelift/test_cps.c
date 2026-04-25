/// Test CPS IR binary generation and compilation
/// This generates a minimal IR blob and tests the full pipeline

#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <dlfcn.h>
#include <string.h>

// Cranelift function pointer types
typedef int32_t (*init_fn)(void);
typedef void*   (*compile_fn)(const uint8_t* ir, size_t len);
typedef int32_t (*ready_fn)(void);

// IR Builder helpers
typedef struct {
    uint8_t* data;
    size_t capacity;
    size_t len;
} IRBuilder;

void ir_init(IRBuilder* b, size_t cap) {
    b->data = malloc(cap);
    b->capacity = cap;
    b->len = 0;
}

void ir_append_u8(IRBuilder* b, uint8_t v) {
    b->data[b->len++] = v;
}

void ir_append_u32(IRBuilder* b, uint32_t v) {
    uint8_t bytes[4] = {v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff};
    memcpy(&b->data[b->len], bytes, 4);
    b->len += 4;
}

void ir_append_i64(IRBuilder* b, int64_t v) {
    uint8_t bytes[8];
    memcpy(bytes, &v, 8);
    memcpy(&b->data[b->len], bytes, 8);
    b->len += 8;
}

void ir_append_string(IRBuilder* b, const char* s) {
    uint32_t len = strlen(s);
    ir_append_u32(b, len);
    memcpy(&b->data[b->len], s, len);
    b->len += len;
}

int main() {
    // Load Cranelift
    const char* paths[] = {
        "./lib/libaustral_cranelift_bridge.so",
        "../lib/libaustral_cranelift_bridge.so",
        "target/release/libaustral_cranelift_bridge.so",
        NULL
    };
    void* lib = NULL;
    for (int i = 0; paths[i]; i++) {
        lib = dlopen(paths[i], RTLD_NOW);
        if (lib) break;
    }
    if (!lib) {
        fprintf(stderr, "dlopen failed (tried all paths): %s\n", dlerror());
        return 1;
    }

    compile_fn compile = (compile_fn)dlsym(lib, "compile_to_function");
    init_fn init = (init_fn)dlsym(lib, "cranelift_init");
    ready_fn ready = (ready_fn)dlsym(lib, "cranelift_is_ready");

    if (!compile || !init) return 1;
    init();

    printf("=== CPS IR Compilation Test ===\n");

    // Test 1: Simple function that returns 42
    // IR: func "simple"() -> I64 { return 42; }
    IRBuilder b1;
    ir_init(&b1, 256);

    ir_append_u32(&b1, 0x43505331);  // Magic "CPS1"
    ir_append_u32(&b1, 1);           // 1 function

    // Function: "simple"(), returns I64
    ir_append_string(&b1, "simple");
    ir_append_u32(&b1, 0);           // 0 params
    ir_append_u8(&b1, 3);            // Return type: I64 = 3

    // Body length
    ir_append_u32(&b1, 6);           // Tag + value

    // Instructions: IntLit(42)
    ir_append_u8(&b1, 0x01);         // Tag 0x01 = IntLit
    ir_append_i64(&b1, 42);

    printf("IR bytes: %zu\n", b1.len);

    void* fn = compile(b1.data, b1.len);
    if (!fn) {
        printf("Compilation failed!\n");
        return 1;
    }

    typedef int64_t (*test_fn)(void);
    test_fn f = (test_fn)fn;
    int64_t result = f();
    printf("Result from IR: %ld\n", (long)result);

    if (result != 42) {
        printf("FAIL: Expected 42, got %ld\n", result);
        return 1;
    }
    printf("PASS: Simple IR compiled correctly\n\n");

    // Test 2: Binary operation (10 + 32 = 42)
    IRBuilder b2;
    ir_init(&b2, 256);

    ir_append_u32(&b2, 0x43505331);
    ir_append_u32(&b2, 1);
    ir_append_string(&b2, "add_test");
    ir_append_u32(&b2, 0);
    ir_append_u8(&b2, 3);            // I64

    // Body: (binop Add (IntLit 10) (IntLit 32))
    // Binary ops need 2 args + all sub-instructions
    // For sub-instructions, we don't encode their length - just stream them
    ir_append_u32(&b2, 1 + 1 + 9 + 1 + 9);  // tag + op + lit + tag + lit
    ir_append_u8(&b2, 0x06);         // BinOp
    ir_append_u8(&b2, 0);            // Add
    ir_append_u8(&b2, 0x01);         // Left: IntLit
    ir_append_i64(&b2, 10);
    ir_append_u8(&b2, 0x01);         // Right: IntLit
    ir_append_i64(&b2, 32);

    printf("IR bytes: %zu\n", b2.len);

    void* fn2 = compile(b2.data, b2.len);
    test_fn f2 = (test_fn)fn2;
    int64_t result2 = f2();
    printf("Result from add IR: %ld\n", (long)result2);

    if (result2 != 42) {
        printf("FAIL: Expected 42, got %ld\n", result2);
        return 1;
    }
    printf("PASS: Binary op IR compiled correctly\n\n");

    printf("=== All CPS IR Tests Passed ===\n");
    free(b1.data);
    free(b2.data);
    dlclose(lib);
    return 0;
}

// Stub for cranelift to link with
void scheduler_dispatch(void) { __builtin_unreachable(); }
