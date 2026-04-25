/*
 * SafestOS VM Core Headers
 * 
 * This file defines the core VM structures and protocols.
 * All code is designed for zero-growth stack frames and linear type safety.
 */

#ifndef SAFESTOS_VM_H
#define SAFESTOS_VM_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdatomic.h>
#include <unistd.h>
#include <stdlib.h>
#include <stdio.h>
#include <dlfcn.h>

/* Forward declarations */
typedef struct Serializer Serializer;
typedef struct Deserializer Deserializer;
typedef struct CellDescriptor CellDescriptor;
typedef uint64_t CellId;  // Simple numeric cell identifier

/* ============================================================================
 * 1. Linear Types and Capabilities
 * ============================================================================
 */

// Linear type marker - prevents implicit copying
typedef struct {
    uint64_t id;
    uint64_t secret;
} LinearToken;

// Cap_Env: A linear capability that represents the execution environment
// Grants access to module loading and evaluation
typedef struct {
    LinearToken token;
    void* namespace;  // Opaque pointer to symbol table
} CapEnv;

// Capability types (bitflags)
#define CAP_ENV         (1ULL << 0)
#define CAP_FS          (1ULL << 1)
#define CAP_NET         (1ULL << 2)
#define CAP_TIME        (1ULL << 3)
#define CAP_PROC        (1ULL << 4)

// Cell capability requirements
typedef uint64_t CellCaps;

/* ============================================================================
 * 2. Cell Descriptor
 * ============================================================================
 */



// Cell step function - MUST be noreturn and use musttail
typedef void (*CellStep)(void* state);

// The descriptor that every cell exports
struct CellDescriptor {
    // Type information for subtyping check
    const char* type_hash;      // SHA256 of interface
    CellCaps   required_caps;   // Capabilities needed
    
    // Memory management
    void* (*alloc)(void* region, CapEnv* env);  // Create initial state
    void  (*drop)(void* state);                 // Destroy without save
    
    // Execution
    CellStep step;               // Main entry point
    
    // Serialization (for pausing)
    void  (*save)(void* state, Serializer* s);    
    void* (*restore)(Deserializer* d, void* region);
    
    // Migration (optional, for hot swap)
    void* (*migrate)(void* old_state, Deserializer* d);

    // JIT compilation support (Cranelift)
    void* _jit_fn_ptr;  // Raw function pointer from JIT (if compiled via Cranelift)
};

// Active cell in scheduler
struct ActiveCell {
    CellDescriptor* desc;
    void*           state;
};

/* ============================================================================
 * 3. Scheduler Queue
 * ============================================================================
 */

// Lock-free ring buffer for paused cells
#define PAUSE_QUEUE_SIZE 256

typedef struct {
    CellStep fn;
    void*    state;
} PauseEntry;

typedef struct {
    PauseEntry buffer[PAUSE_QUEUE_SIZE];
    atomic_int head;
    atomic_int tail;
    atomic_int size;
} PauseQueue;

/* ============================================================================
 * 4. Serialization Protocol
 * ============================================================================ */

struct Serializer {
    uint8_t* buffer;
    size_t   size;
    size_t   capacity;
    size_t   pos;
};

struct Deserializer {
    const uint8_t* buffer;
    size_t         size;
    size_t         pos;
};

// Basic operations
void ser_init(Serializer* s, uint8_t* buf, size_t cap);
void des_init(Deserializer* d, const uint8_t* buf, size_t size);

// Serialization primitives
void ser_u64(Serializer* s, uint64_t v);
uint64_t des_u64(Deserializer* d);

void ser_bytes(Serializer* s, const uint8_t* data, size_t len);
void des_bytes(Deserializer* d, uint8_t* out, size_t len);

// For linear types: consume and reconstruct
void ser_linear_token(Serializer* s, LinearToken* t);
LinearToken des_linear_token(Deserializer* d);

/* ============================================================================
 * 5. Scheduler API
 * ============================================================================
 */

// Global scheduler state
extern PauseQueue scheduler_queue;

// Enqueue a cell for pausing
void scheduler_enqueue(CellStep fn, void* state);

// Dequeue next runnable cell
bool scheduler_dequeue(CellStep* out_fn, void** out_state);

// Central dispatch - the tail-call trampoline
__attribute__((noreturn))
void scheduler_dispatch(void);

// Start the VM with first cell
__attribute__((noreturn))
void vm_start(CellStep first_step, void* first_state);

/* ============================================================================
 * 6. Cell Operations
 * ============================================================================
 */

// Load a cell from shared object or compile on-the-fly
CellDescriptor* cell_load(const char* name, CapEnv* env);

// Type-safe eval (compiled at runtime)
typedef enum {
    EVAL_OK,
    EVAL_PARSE_ERROR,
    EVAL_TYPE_ERROR,
    EVAL_LINK_ERROR
} EvalResultCode;

typedef struct {
    EvalResultCode code;
    union {
        CellDescriptor* cell;  // On success
        const char*     error; // On failure
    } data;
} EvalResult;

// typed_eval: compile string to cell descriptor
EvalResult typed_eval(const char* source, const char* expected_type, CapEnv* env);

// Hot-swap: replace old cell with new one
bool cell_swap(CellId old_id, CellDescriptor* new_desc);

/* ============================================================================
 * 7. Memory Management (Regions)
 * ============================================================================
 */

// Linear region allocator (arena)
typedef struct Region {
    uint8_t* memory;
    size_t   size;
    size_t   used;
} Region;

void* region_alloc(Region* r, size_t size);
void region_free(Region* r);

/* ============================================================================
 * 8. Utility Macros
 * ============================================================================
 */

// Helper for tail-call dispatch
#define TAIL_CALL(fn, state) \
    do { \
        __builtin_assume((fn) != NULL); \
        return; /* Compiler will use musttail */ \
    } while(0)

// Ensure a function is a tail-call dispatcher
#define DISPATCHABLE __attribute__((noreturn)) __attribute__((always_inline))

#endif // SAFESTOS_VM_H

/* ============================================================================
 * Exported Capability Operations
 * ============================================================================ */

extern CapEnv cap_env_create(void* namespace);
extern CapEnv cap_env_fork(CapEnv* parent, void* new_namespace);
extern void cap_drop(CapEnv* env);
extern bool cap_verify(CapEnv* env);
extern bool cell_can_replace(CellDescriptor* old, CellDescriptor* new);
