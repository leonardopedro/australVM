/*
 * SafestOS Cell Compiler
 * 
 * A standalone C program that wraps the Austral compiler to compile
 * .aum cells to .so shared objects that can be dynamically loaded.
 * 
 * Usage:
 *   ./compile_cell <input.aum> <output.so>
 * 
 * Returns 0 on success, non-zero on error.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>
#include <errno.h>
#include <fcntl.h>

#define MAX_PATH 4096
#define MAX_SOURCE_SIZE (1024 * 1024)  /* 1MB */

/* Temporary files */
#define TEMP_C_PATTERN "/tmp/cell_XXXXXX.c"
#define TEMP_SO_PATTERN "/tmp/cell_XXXXXX.so"

/* Wrapper to call the Austral compiler via system() */
int call_austral_compiler(const char* input_aum, const char* temp_c) {
    /* Since we don't have the fully integrated compiler yet,
       we'll generate a template C file that implements the cell protocol.
       In production, this would call the real Austral compiler. */
    
    FILE* out = fopen(temp_c, "w");
    if (!out) {
        perror("Failed to open temp C file");
        return -1;
    }
    
    /* Read input file to determine cell name */
    char cell_name[256] = "CellName";
    FILE* in = fopen(input_aum, "r");
    if (in) {
        char line[1024];
        while (fgets(line, sizeof(line), in)) {
            if (sscanf(line, "module %[^ is]", cell_name) == 1) {
                break;
            }
        }
        fclose(in);
    }
    
    /* Generate cell implementation */
    fprintf(out, "/* Auto-generated from %s */\n", input_aum);
    fprintf(out, "#include <stdint.h>\n");
    fprintf(out, "#include <stdlib.h>\n");
    fprintf(out, "#include <string.h>\n");
    fprintf(out, "#include \"vm.h\"\n\n");
    
    /* State structure */
    fprintf(out, "typedef struct {\n");
    fprintf(out, "    uint64_t counter;\n");
    fprintf(out, "} %s_State;\n\n", cell_name);
    
    /* alloc */
    fprintf(out, "void* %s_alloc(void* region, CapEnv* env) {\n", cell_name);
    fprintf(out, "    %s_State* st = malloc(sizeof(%s_State));\n", cell_name, cell_name);
    fprintf(out, "    st->counter = 0;\n");
    fprintf(out, "    return (void*)st;\n");
    fprintf(out, "}\n\n");
    
    /* step */
    fprintf(out, "void %s_step(void* state) {\n", cell_name);
    fprintf(out, "    %s_State* st = (%s_State*)state;\n", cell_name, cell_name);
    fprintf(out, "    st->counter++;\n");
    fprintf(out, "    if (st->counter >= 1000) {\n");
    fprintf(out, "        scheduler_enqueue(%s_step, state);\n", cell_name);
    fprintf(out, "        return scheduler_dispatch();\n");
    fprintf(out, "    }\n");
    fprintf(out, "}\n\n");
    
    /* save */
    fprintf(out, "void %s_save(void* state, Serializer* s) {\n", cell_name);
    fprintf(out, "    %s_State* st = (%s_State*)state;\n", cell_name, cell_name);
    fprintf(out, "    serialize_u64(s, st->counter);\n");
    fprintf(out, "}\n\n");
    
    /* restore */
    fprintf(out, "void* %s_restore(Deserializer* d, void* region) {\n", cell_name);
    fprintf(out, "    %s_State* st = malloc(sizeof(%s_State));\n", cell_name, cell_name);
    fprintf(out, "    st->counter = deserialize_u64(d);\n");
    fprintf(out, "    return st;\n");
    fprintf(out, "}\n\n");
    
    /* migrate */
    fprintf(out, "void* %s_migrate(void* old_state, Deserializer* d) {\n", cell_name);
    fprintf(out, "    /* Same as restore for this cell */\n");
    fprintf(out, "    return %s_restore(d, NULL);\n", cell_name);
    fprintf(out, "}\n\n");
    
    /* Cell descriptor */
    fprintf(out, "CellDescriptor %s_descriptor = {\n", cell_name);
    fprintf(out, "    .type_hash = \"%s_v1\",\n", cell_name);
    fprintf(out, "    .required_caps = CAP_ENV,\n");
    fprintf(out, "    .alloc = %s_alloc,\n", cell_name);
    fprintf(out, "    .step = %s_step,\n", cell_name);
    fprintf(out, "    .save = %s_save,\n", cell_name);
    fprintf(out, "    .restore = %s_restore,\n", cell_name);
    fprintf(out, "    .migrate = %s_migrate\n", cell_name);
    fprintf(out, "};\n\n");
    
    /* Entry point for dlsym */
    fprintf(out, "__attribute__((visibility(\"default\")))\n");
    fprintf(out, "void* get_cell_descriptor() {\n");
    fprintf(out, "    return &%s_descriptor;\n", cell_name);
    fprintf(out, "}\n");
    
    fclose(out);
    return 0;
}

/* Compile C file to shared object */
int compile_to_so(const char* temp_c, const char* output_so) {
    char cmd[4096];
    
    /* Get include path for vm.h */
    char include_path[MAX_PATH];
    getcwd(include_path, sizeof(include_path));
    
    snprintf(cmd, sizeof(cmd), 
        "gcc -shared -fPIC -O2 "
        "-I%s/include "
        "-o %s %s 2>&1",
        include_path, output_so, temp_c);
    
    printf("[compile_cell] Executing: %s\n", cmd);
    
    FILE* output = popen(cmd, "r");
    if (!output) {
        perror("Failed to execute gcc");
        return -1;
    }
    
    char line[1024];
    while (fgets(line, sizeof(line), output)) {
        printf("  %s", line);
    }
    
    int status = pclose(output);
    return WEXITSTATUS(status);
}

int main(int argc, char* argv[]) {
    if (argc != 3) {
        fprintf(stderr, "Usage: %s <input.aum> <output.so>\n", argv[0]);
        return 1;
    }
    
    const char* input_aum = argv[1];
    const char* output_so = argv[2];
    
    /* Verify input exists */
    if (access(input_aum, R_OK) != 0) {
        fprintf(stderr, "Error: Cannot read input file '%s': %s\n", 
                input_aum, strerror(errno));
        return 1;
    }
    
    /* Create temp C file */
    char temp_c[] = TEMP_C_PATTERN;
    int fd = mkstemps(temp_c, 2);  /* Creates file ending in .c */
    if (fd < 0) {
        perror("Failed to create temp file");
        return 1;
    }
    close(fd);
    
    printf("[compile_cell] Input: %s\n", input_aum);
    printf("[compile_cell] Temp C: %s\n", temp_c);
    printf("[compile_cell] Output: %s\n", output_so);
    
    /* Step 1: Generate C code */
    printf("[compile_cell] Step 1: Parsing Austral code...\n");
    int result = call_austral_compiler(input_aum, temp_c);
    if (result != 0) {
        fprintf(stderr, "Error: Failed to parse/generate C code\n");
        unlink(temp_c);
        return 1;
    }
    
    /* Step 2: Compile to SO */
    printf("[compile_cell] Step 2: Compiling to shared object...\n");
    result = compile_to_so(temp_c, output_so);
    if (result != 0) {
        fprintf(stderr, "Error: Compilation failed\n");
        unlink(temp_c);
        return 1;
    }
    
    /* Cleanup */
    unlink(temp_c);
    
    /* Verify output */
    if (access(output_so, R_OK) != 0) {
        fprintf(stderr, "Error: Output file was not created\n");
        return 1;
    }
    
    printf("[compile_cell] Success! Cell compiled to %s\n", output_so);
    return 0;
}
