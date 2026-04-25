// Just verify the Cranelift module compiles and API exists
#include <stdio.h>
#include <dlfcn.h>

int main() {
    const char* path = "../lib/libaustral_cranelift_bridge.so";
    void* lib = dlopen(path, RTLD_LAZY);
    if (!lib) {
        printf("Could not load: %s\n", dlerror());
        return 1;
    }

    void* compile = dlsym(lib, "compile_to_function");
    void* init = dlsym(lib, "cranelift_init");
    void* ready = dlsym(lib, "cranelift_is_ready");
    
    printf("lib: %p\n", lib);
    printf("compile_to_function: %p\n", compile);
    printf("cranelift_init: %p\n", init);
    printf("cranelift_is_ready: %p\n", ready);
    
    if (init) {
        printf("Calling init...\n");
        int result = ((int(*)(void))init)();
        printf("init returned: %d\n", result);
    }

    if (ready) {
        printf("is_ready: %d\n", ((int(*)(void))ready)());
    }

    dlclose(lib);
    return 0;
}
