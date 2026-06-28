/*
 * Cell V2: Counter with bonus
 * State: { uint64_t counter; uint64_t bonus }
 * Step: counter += 10
 * Migrate: reads old counter from V1's serialized state, sets bonus = 100
 * Same type_hash as V1 ("counter_cell") — compatible for hot-swap.
 * required_caps = 0 (subset of V1's CAP_ENV) — caps downgrade is allowed.
 */

#include "vm.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    uint64_t counter;
    uint64_t bonus;
} CounterV2State;

static void* v2_alloc(void* region __attribute__((unused)), CapEnv* env __attribute__((unused))) {
    CounterV2State* st = malloc(sizeof(CounterV2State));
    st->counter = 0;
    st->bonus = 100;
    return st;
}

static void v2_drop(void* state) {
    free(state);
}

static void v2_step(void* state) {
    CounterV2State* st = (CounterV2State*)state;
    st->counter += 10;
}

static void v2_save(void* state, Serializer* s) {
    CounterV2State* st = (CounterV2State*)state;
    ser_u64(s, st->counter);
    ser_u64(s, st->bonus);
}

static void* v2_restore(Deserializer* d, void* region __attribute__((unused))) {
    CounterV2State* st = malloc(sizeof(CounterV2State));
    st->counter = des_u64(d);
    st->bonus = des_u64(d);
    return st;
}

static void* v2_migrate(void* old_state __attribute__((unused)), Deserializer* d) {
    /* Migrate from V1: read the old counter, set bonus to 100 (new field) */
    CounterV2State* st = malloc(sizeof(CounterV2State));
    st->counter = des_u64(d);  /* Read V1's counter */
    st->bonus = 100;           /* New field — default value */
    printf("[V2 migrate] Migrated counter=%lu, set bonus=%lu\n",
           (unsigned long)st->counter, (unsigned long)st->bonus);
    return st;
}

static CellDescriptor counter_v2_descriptor = {
    .type_hash = "counter_cell",  /* Same as V1 — compatible */
    .required_caps = 0,           /* Subset of V1's CAP_ENV */
    .alloc = v2_alloc,
    .drop = v2_drop,
    .step = v2_step,
    .save = v2_save,
    .restore = v2_restore,
    .migrate = v2_migrate,
    ._jit_fn_ptr = NULL,
};

__attribute__((visibility("default")))
CellDescriptor* get_cell_descriptor(void) {
    return &counter_v2_descriptor;
}
