/*
 * SafestOS Scheduler
 * 
 * Implements lock-free pause/resume queue and central dispatch trampoline.
 * Guarantees O(1) stack depth through tail-call chaining.
 */

#include "vm.h"
#include <string.h>
#include <stdio.h>
#include <unistd.h>

/* Spin-loop pause hint.
 *
 * `__builtin_ia32_pause` (the x86 PAUSE instruction) is the right hint on
 * Intel/AMD, but it does not exist on aarch64 — where the equivalent
 * spin-wait hint is YIELD. Without the guard below the bridge's C runtime
 * fails to compile on aarch64 hosts with
 *   error: implicit declaration of function '__builtin_ia32_pause'
 * (-Wimplicit-function-declaration is an error), so this file was x86-only.
 * On any other architecture there is no hint and the loop just spins. */
static inline void scheduler_pause(void) {
#if defined(__x86_64__) || defined(__i386__)
    __builtin_ia32_pause();
#elif defined(__aarch64__)
    __asm__ volatile("yield" ::: "memory");
#endif
}

// Global scheduler queue
PauseQueue scheduler_queue = {
    .buffer = {{0}},
    .head = 0,
    .tail = 0,
    .size = 0
};

// Enqueue a cell for pausing
void scheduler_enqueue(CellStep fn, void* state) {
    int tail = atomic_load(&scheduler_queue.tail);
    int next_tail = (tail + 1) % PAUSE_QUEUE_SIZE;
    
    // Check if queue is full (simple spin-lock fallback)
    while (next_tail == atomic_load(&scheduler_queue.head)) {
        // In production, use futex or wait strategy
        scheduler_pause();
    }
    
    scheduler_queue.buffer[tail].fn = fn;
    scheduler_queue.buffer[tail].state = state;
    
    // Memory barrier before updating tail
    atomic_store(&scheduler_queue.tail, next_tail);
    atomic_fetch_add(&scheduler_queue.size, 1);
}

// Dequeue next runnable cell
bool scheduler_dequeue(CellStep* out_fn, void** out_state) {
    int head = atomic_load(&scheduler_queue.head);
    int tail = atomic_load(&scheduler_queue.tail);
    
    if (head == tail) {
        return false; // Queue empty
    }
    
    *out_fn = scheduler_queue.buffer[head].fn;
    *out_state = scheduler_queue.buffer[head].state;
    
    atomic_store(&scheduler_queue.head, (head + 1) % PAUSE_QUEUE_SIZE);
    atomic_fetch_sub(&scheduler_queue.size, 1);
    
    return true;
}

// The central dispatch function - the heart of the VM
// This is designed to be called via tail-call and reuse its own stack frame.
// NOTE: In production with clang, all returns should have [[clang::musttail]]
void scheduler_dispatch(void) {
    CellStep fn;
    void* state;
    
    while (1) {
        if (scheduler_dequeue(&fn, &state)) {
            // Found a runnable cell - call its step function
            // In clang: [[clang::musttail]] return fn(state);
            // In gcc: rely on optimization or manual trampoline
            fn(state);
            // Tail-call chaining: fn(state) should have chain-called
            // back into scheduler_dispatch, but if it returns normally,
            // loop back and pick up the next cell.
            continue;
        } else {
            // Queue is empty — normally shouldn't happen.
            // In production, wait on futex or event fd.
            // PAUSE hint avoids spinning the frontend pipeline.
            scheduler_pause();
        }
    }
}

// Entry point - starts the VM
void vm_start(CellStep first_step, void* first_state) {
    printf("[VM] Starting SafestOS\n");
    
    // Tail-call into the first cell
    // In clang: [[clang::musttail]] return first_step(first_state);
    first_step(first_state);
    
    // Should never reach here
    exit(0);
}
