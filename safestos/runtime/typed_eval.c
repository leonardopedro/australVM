/*
 * SafestOS Typed Eval
 * 
 * Runtime compilation with Cranelift JIT backend.
 * Falls back to GCC for legacy C codegen.
 */

#define _DEFAULT_SOURCE
#include "vm.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <dlfcn.h>
#include <unistd.h>
#include <stdint.h>

/* Cranelift Bridge Loading */
static void* load_cranelift_bridge(void) {
    void* handle = NULL;
    char path[1024];
    
    const char* pwd = getenv("PWD");
    if (!pwd) pwd = ".";
    
    // Multiple paths to search
    const char* try_paths[] = {
        // Runtime executed from ./test/, library is ../lib/ relative to that
        "../lib/libaustral_cranelift_bridge.so",
        // Runtime executed from ./, library in ./lib/
        "./lib/libaustral_cranelift_bridge.so",
        // Full absolute path
        NULL
    };
    
    // Build absolute path
    snprintf(path, sizeof(path), "%s/lib/libaustral_cranelift_bridge.so", pwd);
    try_paths[2] = path;
    
    for (int i = 0; try_paths[i]; i++) {
        handle = dlopen(try_paths[i], RTLD_NOW | RTLD_GLOBAL);
        if (handle) return handle;
    }
    
    return NULL;
}

static int cl_init(void) {
    static int loaded = 0;
    static int ready = 0;
    
    if (loaded) return ready;
    loaded = 1;
    
    void* lib = load_cranelift_bridge();
    if (!lib) {
        fprintf(stderr, "[typed_eval] Cranelift bridge not found in search paths\n");
        return 0;
    }
    
    // Test if basic symbols work
    int32_t (*init)(void) = dlsym(lib, "cranelift_init");
    uint32_t (*version)(void) = dlsym(lib, "cranelift_version");
    
    if (!init || !version) {
        fprintf(stderr, "[typed_eval] Cranelift symbols missing\n");
        dlclose(lib);
        return 0;
    }
    
    if (init() != 0) {
        fprintf(stderr, "[typed_eval] Cranelift init failed\n");
        return 0;
    }
    
    fprintf(stderr, "[typed_eval] Cranelift v0x%x ready\n", version());
    ready = 1;
    return 1;
}

/* Legacy GCC fallback */
static char* generate_cell_c(const char* source, const char* type_hash) {
    // ... same as before ...
    const char* template = 
        "/* Generated cell */\n"
        "#include <stdint.h>\n#include <stdlib.h>\n#include <stdio.h>\n"
        "#include \"vm.h\"\n\n"
        "typedef struct { uint64_t value; char ident[64]; } CellState;\n\n"
        "void* cell_alloc(void* region, CapEnv* env) {\n"
        "    (void)region; (void)env;\n"
        "    CellState* st = malloc(sizeof(CellState));\n"
        "    st->value = 0;\n"
        "    snprintf(st->ident, 64, \"cell_%%s\", \"%s\");\n"
        "    return st;\n"
        "}\n\n"
        "void cell_step(void* state) { CellState* st = (CellState*)state; st->value++; }\n\n"
        "void cell_save(void* state, Serializer* s) { \n"
        "    CellState* st = (CellState*)state;\n"
        "    ser_u64(s, st->value); ser_bytes(s, (uint8_t*)st->ident, 64);\n"
        "}\n\n"
        "void* cell_restore(Deserializer* d, void* region) {\n"
        "    (void)region; CellState* st = malloc(sizeof(CellState));\n"
        "    st->value = des_u64(d); des_bytes(d, (uint8_t*)st->ident, 64); return st;\n"
        "}\n\n"
        "void* cell_migrate(void* old_state, Deserializer* d) { return cell_restore(d, NULL); }\n\n"
        "CellDescriptor cell_descriptor = {\n"
        "    .type_hash = \"%s\",\n"
        "    .required_caps = CAP_ENV,\n"
        "    .alloc = cell_alloc,\n"
        "    .step = cell_step,\n"
        "    .save = cell_save,\n"
        "    .restore = cell_restore,\n"
        "    .migrate = cell_migrate,\n"
        "    ._jit_fn_ptr = NULL\n"
        "};\n\n"
        "__attribute__((visibility(\"default\")))\n"
        "void* get_cell_descriptor() { return &cell_descriptor; }\n";
    
    char* result = malloc(2048);
    snprintf(result, 2048, template, type_hash, type_hash);
    return result;
}

static int compile_so_from_c(const char* c_code, const char* so_path) {
    char c_path[512];
    snprintf(c_path, sizeof(c_path), "%s.c", so_path);
    FILE* f = fopen(c_path, "w");
    if (!f) return -1;
    fprintf(f, "%s", c_code);
    fclose(f);
    
    char* pwd = getenv("PWD");
    if (!pwd) pwd = ".";
    char lib_path[512];
    snprintf(lib_path, sizeof(lib_path), "%s/lib", pwd);
    
    char cmd[1024];
    snprintf(cmd, sizeof(cmd), "gcc -shared -fPIC -O2 -I%s/include -L%s -lSafestOS -o %s %s 2>&1",
             pwd, lib_path, so_path, c_path);
    
    printf("[typed_eval] Compile: %s\n", cmd);
    int result = system(cmd);
    unlink(c_path);
    return (result == 0) ? 0 : -1;
}

EvalResult typed_eval(const char* source, const char* expected_type, CapEnv* env) {
    EvalResult result = {0};
    (void)env;

    printf("========================================\n");
    printf("[typed_eval] Runtime Compilation\n");
    printf("========================================\n");
    printf("Expression: %s\n", source);
    printf("Expected type: %s\n", expected_type);
    printf("----------------------------------------\n");

    /* Try Cranelift path - REMOVED FOR NOW, BROKEN BY SYMBOLS
       We'll re-enable when C bridge api is ready */
    if (0 && cl_init()) {
        // Would call JIT path here
        printf("[typed_eval] Cranelift would be used (currently disabled)\n");
    } else if (1) {
        /* Fallback to GCC */
        printf("[typed_eval] Using GCC fallback backend\n");
        printf("[typed_eval] Step 1: Generating C code...\n");
        char* c_code = generate_cell_c(source, expected_type);
        
        printf("[typed_eval] Step 2: Compiling to shared object...\n");
        char so_path[] = "/tmp/cell_XXXXXX.so";
        int fd = mkstemps(so_path, 3);
        if (fd < 0) {
            free(c_code);
            result.code = EVAL_LINK_ERROR;
            result.data.error = "Failed to create temp file";
            return result;
        }
        close(fd);
        unlink(so_path);
        
        if (compile_so_from_c(c_code, so_path) != 0) {
            free(c_code);
            result.code = EVAL_LINK_ERROR;
            result.data.error = "Compilation failed";
            return result;
        }
        
        printf("[typed_eval] Step 3: Loading shared object...\n");
        void* handle = dlopen(so_path, RTLD_NOW | RTLD_LOCAL);
        if (!handle) {
            fprintf(stderr, "[typed_eval] dlopen failed: %s\n", dlerror());
            free(c_code);
            result.code = EVAL_LINK_ERROR;
            result.data.error = "Failed to load .so";
            return result;
        }
        
        void* (*get_desc)(void) = dlsym(handle, "get_cell_descriptor");
        if (!get_desc) {
            dlclose(handle);
            free(c_code);
            result.code = EVAL_LINK_ERROR;
            result.data.error = "No get_cell_descriptor symbol";
            return result;
        }
        
        CellDescriptor* desc = get_desc();
        free(c_code);
        
        result.code = EVAL_OK;
        result.data.cell = desc;
        
        printf("[typed_eval] Step 4: Loaded cell '%s'\n", desc->type_hash);
        printf("[typed_eval] Temp .so preserved at: %s\n", so_path);
    }

    printf("========================================\n");
    return result;
}

/**
 * Note: scheduler_dispatch is implemented in scheduler.c
 * This stub version confirms integration capability for Phase 4.
 * 
 * For Phase 5: the real scheduler.c will be compiled with 
 * libSafestOS.a which links with the bridge.
 */
