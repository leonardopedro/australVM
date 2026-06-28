/*
 * Cell V1: Simple counter
 * State: { uint64_t counter }
 * Step: counter++
 * This is the "old" version that will be hot-swapped to V2.
 */

#include "vm.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    uint64_t counter;
} CounterV1State;

static void* v1_alloc(void* region __attribute__((unused)), CapEnv* env __attribute__((unused))) {
    CounterV1State* st = malloc(sizeof(CounterV1State));
    st->counter = 0;
    return st;
}

static void v1_drop(void* state) {
    free(state);
}

static void v1_step(void* state) {
    CounterV1State* st = (CounterV1State*)state;
    st->counter++;
}

static void v1_save(void* state, Serializer* s) {
    CounterV1State* st = (CounterV1State*)state;
    ser_u64(s, st->counter);
}

static void* v1_restore(Deserializer* d, void* region __attribute__((unused))) {
    CounterV1State* st = malloc(sizeof(CounterV1State));
    st->counter = des_u64(d);
    return st;
}

static void* v1_migrate(void* old_state __attribute__((unused)), Deserializer* d) {
    /* V1 migrate = restore (same format) */
    return v1_restore(d, NULL);
}

static CellDescriptor counter_v1_descriptor = {
    .type_hash = "counter_cell",
    .required_caps = CAP_ENV,
    .alloc = v1_alloc,
    .drop = v1_drop,
    .step = v1_step,
    .save = v1_save,
    .restore = v1_restore,
    .migrate = v1_migrate,
    ._jit_fn_ptr = NULL,
};

__attribute__((visibility("default")))
CellDescriptor* get_cell_descriptor(void) {
    return &counter_v1_descriptor;
}
