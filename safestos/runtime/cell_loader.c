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
    void* state;  // Active cell state (allocated via desc->alloc)
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
        cell_table[cell_count].state = NULL;  // Not yet allocated
        cell_count++;
        printf("[Loader] Cell loaded successfully, id=%d\n", cell_count);
    } else {
        dlclose(handle);
        return NULL;
    }
    return desc;
}

// Allocate state for a loaded cell
void* cell_alloc_state(CellId id, void* region, CapEnv* env) {
    if (id < 1 || (int)id > cell_count) return NULL;
    CellEntry* entry = &cell_table[id - 1];
    if (!entry->desc || !entry->desc->alloc) return NULL;
    if (entry->state) return entry->state;  // Already allocated
    entry->state = entry->desc->alloc(region, env);
    return entry->state;
}

// Run one step of a loaded cell
void cell_run_step(CellId id) {
    if (id < 1 || (int)id > cell_count) return;
    CellEntry* entry = &cell_table[id - 1];
    if (!entry->desc || !entry->desc->step || !entry->state) return;
    entry->desc->step(entry->state);
}

// Get the state pointer for a loaded cell (for inspection)
void* cell_get_state(CellId id) {
    if (id < 1 || (int)id > cell_count) return NULL;
    return cell_table[id - 1].state;
}

// Get the descriptor for a loaded cell
CellDescriptor* cell_get_descriptor(CellId id) {
    if (id < 1 || (int)id > cell_count) return NULL;
    return cell_table[id - 1].desc;
}

// Get the number of loaded cells
int cell_count_loaded(void) {
    return cell_count;
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
    
    printf("[Loader] Hot-swapping cell %ld\n", (long)old_id);
    
    // Step 1: Save the old cell's state to a serialization buffer
    void* new_state = NULL;
    if (old_entry->state && old_desc->save && new_desc->migrate) {
        // Serialize old state
        uint8_t ser_buf[4096];
        Serializer ser;
        ser_init(&ser, ser_buf, sizeof(ser_buf));
        old_desc->save(old_entry->state, &ser);
        
        // Deserialize into new state via migrate
        Deserializer des;
        des_init(&des, ser_buf, ser.size);
        new_state = new_desc->migrate(old_entry->state, &des);
        printf("[Loader] State migrated (%zu bytes serialized)\n", ser.size);
        
        // Drop old state
        if (old_desc->drop) {
            old_desc->drop(old_entry->state);
        } else {
            free(old_entry->state);
        }
    } else if (old_entry->state && new_desc->migrate) {
        // No save function — migrate gets the raw old_state
        new_state = new_desc->migrate(old_entry->state, NULL);
        printf("[Loader] State migrated (raw pointer)\n");
        
        if (old_desc->drop) {
            old_desc->drop(old_entry->state);
        } else {
            free(old_entry->state);
        }
    } else {
        // No state or no migrate — new cell starts fresh
        printf("[Loader] No migration needed (no state or no migrate fn)\n");
    }
    
    // Step 2: Replace descriptor and state
    old_entry->desc = new_desc;
    old_entry->state = new_state;
    
    // Step 3: Close the old shared object handle (if the new descriptor
    // came from a different .so). We keep the handle if old and new are
    // from the same .so (e.g. in-process test).
    // Note: in production, the scheduler ensures the old cell is paused
    // before we reach here. The new descriptor's step function will be
    // called on the next scheduler tick.
    
    printf("[Loader] Swap complete\n");
    return true;
}

// JIT function pointer setter (replaces fragile hardcoded offset in Rust).
#include <assert.h>
static_assert(
    offsetof(struct CellDescriptor, _jit_fn_ptr) > 0,
    "CellDescriptor._jit_fn_ptr must exist in the struct"
);

void cell_set_jit_fn_ptr(CellDescriptor* desc, void* ptr) {
    if (desc) {
        desc->_jit_fn_ptr = ptr;
    }
}

// Cleanup
void cell_cleanup(void) {
    for (int i = 0; i < cell_count; i++) {
        if (cell_table[i].handle) {
            dlclose(cell_table[i].handle);
        }
    }
}
