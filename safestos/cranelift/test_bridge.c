#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <dlfcn.h>
#include <unistd.h>

void scheduler_dispatch(void) {
    fprintf(stderr, "scheduler_dispatch called (stub)\n");
    _exit(0);
}

typedef int32_t (*init_fn)(void);
typedef void*   (*compile_fn)(const uint8_t* ir, size_t len);
typedef int32_t (*ready_fn)(void);
typedef uint32_t(*version_fn)(void);
typedef void    (*shutdown_fn)(void);

int main() {
    const char* lib_path = "./target/release/libaustral_cranelift_bridge.so";
    void* lib = dlopen(lib_path, RTLD_NOW | RTLD_GLOBAL);
    
    if (!lib) {
        fprintf(stderr, "dlopen '%s' failed: %s\n", lib_path, dlerror());
        return 1;
    }

    init_fn init = (init_fn)dlsym(lib, "cranelift_init");
    compile_fn compile = (compile_fn)dlsym(lib, "compile_to_function");
    ready_fn ready = (ready_fn)dlsym(lib, "cranelift_is_ready");
    version_fn version = (version_fn)dlsym(lib, "cranelift_version");
    shutdown_fn shutdown = (shutdown_fn)dlsym(lib, "cranelift_shutdown");

    if (!init || !compile || !ready || !version || !shutdown) {
        fprintf(stderr, "dlsym failed: %s\n", dlerror());
        dlclose(lib);
        return 1;
    }

    printf("=== Cranelift Bridge Demo ===\n");
    printf("Version: 0x%x\n", version());
    printf("Ready before init: %d\n", ready());

    if (init() != 0) {
        fprintf(stderr, "Init failed\n");
        dlclose(lib);
        return 1;
    }
    printf("Ready after init: %d\n", ready());

    // Compile function (NULL = demo)
    void* fn_ptr = compile(NULL, 0);
    if (!fn_ptr) {
        fprintf(stderr, "Compile failed\n");
        dlclose(lib);
        return 1;
    }

    printf("Function compiled at: %p\n", fn_ptr);

    // Call it
    typedef int64_t (*demo_fn)(void);
    demo_fn fn = (demo_fn)fn_ptr;
    int64_t result = fn();
    printf("Result: %ld (expected 42)\n", (long)result);

    if (result == 42) {
        printf("\n✓ SUCCESS: Cranelift bridge works!\n");
        printf("  - Compiled from Rust\n");
        printf("  - Loaded at runtime\n");
        printf("  - Executed correctly\n");
    } else {
        printf("\n✗ FAILURE\n");
        dlclose(lib);
        return 1;
    }

    shutdown();
    dlclose(lib);
    return 0;
}
