/*
 * SafestOS Region Allocator
 * 
 * Simple arena allocator for linear memory management.
 * No deallocation, just reset.
 */

#include "vm.h"
#include <stdlib.h>
#include <string.h>

#define REGION_DEFAULT_SIZE (1024 * 1024) // 1MB

Region* region_create(void) {
    Region* r = malloc(sizeof(Region));
    r->memory = malloc(REGION_DEFAULT_SIZE);
    r->size = REGION_DEFAULT_SIZE;
    r->used = 0;
    return r;
}

void* region_alloc(Region* r, size_t size) {
    // Simple alignment to 8 bytes
    size_t aligned = (size + 7) & ~7;
    
    if (r->used + aligned > r->size) {
        return NULL; // Out of memory
    }
    
    void* ptr = r->memory + r->used;
    r->used += aligned;
    return ptr;
}

void region_free(Region* r) {
    if (r) {
        if (r->memory) free(r->memory);
        free(r);
    }
}

void region_reset(Region* r) {
    if (r) {
        r->used = 0;
    }
}
