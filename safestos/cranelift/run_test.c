#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>

// Stub that crash if called, or do nothing
void scheduler_dispatch() {
    fprintf(stderr, "ERROR: scheduler_dispatch was called unexpectedly\n");
    exit(1);
}

// Cranelift init needs this symbol - register it
__attribute__((constructor))
void register_symbols() {
    dlopen(NULL, RTLD_NOW | RTLD_GLOBAL); // Self
}

int main() {
    // Compile with simulator
    printf("Testing bridge at: %s\n", "./target/release/libaustral_cranelift_bridge.so");
    
    // Since scheduler_dispatch is required by init, we provide it
    void* handle = dlopen("./target/release/libaustral_cranelift_bridge.so", RTLD_NOW | RTLD_GLOBAL);
    if (!handle) {
        fprintf(stderr, "Load failed: %s\n", dlerror());
        return 1;
    }
    
    // Get versions to confirm load
    unsigned int (*version)() = dlsym(handle, "cranelift_version");
    int (*ready)() = dlsym(handle, "cranelift_is_ready");
    int (*init)() = dlsym(handle, "cranelift_init");
    void* (*compile)(const unsigned char*, size_t) = dlsym(handle, "compile_to_function");
    
    if (!version || !ready || !init || !compile) {
        fprintf(stderr, "Missing symbols: %s\n", dlerror());
        return 1;
    }
    
    printf("Version: 0x%x\n", version());
    printf("Ready (before init): %d\n", ready());
    printf("Init: %d\n", init());
    printf("Ready (after init): %d\n", ready());
    
    // Test demo mode
    void* fn = compile(0, 0);
    if (!fn) {
        fprintf(stderr, "Compilation failed\n");
        return 1;
    }
    
    typedef long (*func_t)();
    long result = ((func_t)fn)();
    printf("Result: %ld\n", result);
    
    if (result == 42) {
        printf("✓ PASS: Bridge compiles and runs!\n");
        return 0;
    } else {
        printf("✗ FAIL: Expected 42, got %ld\n", result);
        return 1;
    }
}
