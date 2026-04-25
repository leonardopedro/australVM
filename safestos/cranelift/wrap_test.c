#include <dlfcn.h>
#include <stdio.h>

// Stub scheduler
void scheduler_dispatch() { 
    printf("scheduler_dispatch (stub)\n"); 
}

// We load the lib ourselves so the symbol is available
int main() {
    // Load in current process where scheduler is defined
    void* handle = dlopen("./target/release/libaustral_cranelift_bridge.so", RTLD_NOW | RTLD_GLOBAL);
    if (!handle) {
        printf("Error: %s\n", dlerror());
        return 1;
    }
    
    // Get functions
    int (*init)() = dlsym(handle, "cranelift_init");
    void* (*compile)(const unsigned char*, size_t) = dlsym(handle, "compile_to_function");
    unsigned int (*version)() = dlsym(handle, "cranelift_version");
    int (*ready)() = dlsym(handle, "cranelift_is_ready");
    
    if (!init || !compile || !version) {
        printf("Missing symbols: %s\n", dlerror());
        return 1;
    }
    
    printf("Bridge version: 0x%x\n", version());
    printf("Init result: %d\n", init());
    printf("Ready: %d\n", ready());
    
    // Test compile
    void* fn = compile(0, 0); // Demo mode
    if (!fn) {
        printf("✗ Compilation failed\n");
        return 1;
    }
    
    typedef long (*func_t)();
    long result = ((func_t)fn)();
    printf("Result: %ld (expected 42)\n", result);
    
    if (result == 42) {
        printf("✓ SUCCESS!\n");
        return 0;
    }
    return 1;
}
