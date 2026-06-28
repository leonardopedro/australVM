/*
 * End-to-end hot-swap test (P6 B3)
 *
 * Loads counter_v1.so → allocates state → steps 3 times (counter=3)
 * → loads counter_v2.so → swaps cell 1 to V2 (migrates state)
 * → steps V2 (counter = 3 + 10 = 13, bonus = 100)
 * → verifies the counter was preserved across the swap.
 *
 * Build: make test/hotswap_e2e
 * Run:    LD_LIBRARY_PATH=./lib ./test/hotswap_e2e
 *
 * Exits 0 on success, 1 on failure.
 */

#include "vm.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>
#include <stdint.h>

/* Mirrors CounterV1State (used to inspect state before swap) */
typedef struct {
    uint64_t counter;
} V1State;

/* Mirrors CounterV2State (used to inspect state after swap) */
typedef struct {
    uint64_t counter;
    uint64_t bonus;
} V2State;

int main(void) {
    printf("=== End-to-End Hot-Swap Test ===\n\n");

    /* Step 1: Load V1 */
    printf("[Test] Loading counter_v1...\n");
    CellDescriptor* v1_desc = cell_load("counter_v1", NULL);
    if (!v1_desc) {
        fprintf(stderr, "FAIL: Could not load counter_v1.so\n");
        return 1;
    }
    printf("[Test] V1 loaded: type_hash=\"%s\", caps=%lu\n",
           v1_desc->type_hash, (unsigned long)v1_desc->required_caps);
    assert(cell_count_loaded() == 1);

    /* Step 2: Allocate V1's state */
    printf("[Test] Allocating V1 state...\n");
    void* state = cell_alloc_state(1, NULL, NULL);
    if (!state) {
        fprintf(stderr, "FAIL: Could not allocate V1 state\n");
        return 1;
    }

    /* Step 3: Step V1 three times (counter → 3) */
    printf("[Test] Stepping V1 three times...\n");
    cell_run_step(1);
    cell_run_step(1);
    cell_run_step(1);
    V1State* v1s = (V1State*)cell_get_state(1);
    printf("[Test] V1 counter = %lu\n", (unsigned long)v1s->counter);
    assert(v1s->counter == 3);

    /* Step 4: Load V2 */
    printf("[Test] Loading counter_v2...\n");
    CellDescriptor* v2_desc = cell_load("counter_v2", NULL);
    if (!v2_desc) {
        fprintf(stderr, "FAIL: Could not load counter_v2.so\n");
        return 1;
    }
    printf("[Test] V2 loaded: type_hash=\"%s\", caps=%lu\n",
           v2_desc->type_hash, (unsigned long)v2_desc->required_caps);
    assert(cell_count_loaded() == 2);

    /* Verify compatibility */
    assert(cell_can_replace(v1_desc, v2_desc));

    /* Step 5: Hot-swap cell 1 to V2 */
    printf("[Test] Swapping cell 1 from V1 to V2...\n");
    bool ok = cell_swap(1, v2_desc);
    if (!ok) {
        fprintf(stderr, "FAIL: cell_swap returned false\n");
        return 1;
    }

    /* Step 6: Verify migrated state */
    V2State* v2s = (V2State*)cell_get_state(1);
    printf("[Test] After swap: counter=%lu, bonus=%lu\n",
           (unsigned long)v2s->counter, (unsigned long)v2s->bonus);
    assert(v2s->counter == 3);   /* Preserved from V1 */
    assert(v2s->bonus == 100);   /* New field, set by migrate */

    /* Step 7: Step V2 (counter += 10 → 13) */
    printf("[Test] Stepping V2...\n");
    cell_run_step(1);
    v2s = (V2State*)cell_get_state(1);
    printf("[Test] After V2 step: counter=%lu, bonus=%lu\n",
           (unsigned long)v2s->counter, (unsigned long)v2s->bonus);
    assert(v2s->counter == 13);  /* 3 + 10 */
    assert(v2s->bonus == 100);

    /* Step 8: Step V2 again (counter += 10 → 23) */
    cell_run_step(1);
    v2s = (V2State*)cell_get_state(1);
    assert(v2s->counter == 23);  /* 13 + 10 */

    /* Cleanup */
    cell_cleanup();

    printf("\n=== Hot-Swap Test PASSED ===\n");
    printf("V1 counter=3 → migrated → V2 counter=3 → stepped to 23\n");
    return 0;
}
