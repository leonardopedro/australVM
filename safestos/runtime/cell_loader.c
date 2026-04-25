/*
 * SafestOS Cell Loader
 * 
 * Dynamic loading of cells and hot-swap mechanism.
 */

#include "vm.h"
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

// Map from cell ID to descriptor
typedef struct {
    CellId id;
    CellDescriptor* desc;
    void* handle;
} CellEntry;

#define MAX_CELLS 64
static CellEntry cell_table[MAX_CELLS];
static int cell_count = 0;

// Load cell from shared object
CellDescriptor* cell_load(const char* name, CapEnv* env __attribute__((unused))) {
    char path[512];
    snprintf(path, sizeof(path), "./cells/%s.so", name);
    
    printf("[Loader] Loading cell: %s\n", path);
    
    void* handle = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (!handle) {
        const char* error = dlerror();
        printf("[Loader] dlopen failed: %s\n", error);
        
        // Fallback: try to compile on-the-fly
        // In production, this would call typed_eval with file contents
        return NULL;
    }
    
    // Get descriptor
    CellDescriptor* (*get_desc)(void) = dlsym(handle, "get_cell_descriptor");
    if (!get_desc) {
        dlclose(handle);
        return NULL;
    }
    
    CellDescriptor* desc = get_desc();
    
    // Register in table
    if (cell_count < MAX_CELLS) {
        cell_table[cell_count].id = cell_count + 1; // Simple ID assignment
        cell_table[cell_count].desc = desc;
        cell_table[cell_count].handle = handle;
        cell_count++;
    }
    
    printf("[Loader] Cell loaded successfully, id=%d\n", cell_count);
    return desc;
}

// Type check: verify compatibility
bool cell_can_replace(CellDescriptor* old, CellDescriptor* new) {
    if (!old || !new) return false;
    
    // Check type hash (structural subtyping)
    if (strcmp(old->type_hash, new->type_hash) != 0) {
        printf("[Loader] Type hash mismatch: %s != %s\n", old->type_hash, new->type_hash);
        return false;
    }
    
    // Check capabilities: new cell requires fewer or equal
    if ((new->required_caps & old->required_caps) != new->required_caps) {
        printf("[Loader] Capability requirement mismatch\n");
        return false;
    }
    
    return true;
}

// Hot-swap procedure
bool cell_swap(CellId old_id, CellDescriptor* new_desc) {
    if (old_id < 1 || (int)old_id > cell_count) return false;
    
    CellEntry* old_entry = &cell_table[old_id - 1];
    CellDescriptor* old_desc = old_entry->desc;
    
    // Verify compatibility
    if (!cell_can_replace(old_desc, new_desc)) {
        return false;
    }
    
    printf("[Loader] Hot-swapping cell %ld\n", old_id);
    
    // We assume the old cell is paused and serialized
    
    // Step 1: Pause the old cell (orchestrated by mod_mgmt)
    // - The old cell returns Pause from its step
    // - Scheduler serializes its state
    // - State buffer is saved
    
    // Step 2: Optionally call migration function
    if (new_desc->migrate) {
        printf("[Loader] Migration function present\n");
        // In real implementation: restore old state with new migrator
    }
    
    // Step 3: Replace descriptor
    old_entry->desc = new_desc;
    
    // Step 4: If old cell had a DSO handle, we might close it
    // but for now, we keep it loaded
    
    printf("[Loader] Swap complete\n");
    return true;
}

// Cleanup
void cell_cleanup(void) {
    for (int i = 0; i < cell_count; i++) {
        if (cell_table[i].handle) {
            dlclose(cell_table[i].handle);
        }
    }
}
