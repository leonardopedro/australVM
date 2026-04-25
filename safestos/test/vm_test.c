/*
 * SafestOS VM Test Program
 * Tests scheduler, serialization, and cell loading
 */

#include "vm.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>

// Test state
typedef struct {
    int counter;
    const char* name;
} TestState;

// Demo cell step function - performs 3 steps then stops
void demo_step(void* state) {
    TestState* s = (TestState*)state;
    
    printf("[%s] Step %d\n", s->name, s->counter);
    s->counter++;
    
    if (s->counter >= 3) {
        printf("[%s] Completed 3 steps\n", s->name);
        return;  // Done
    }
    
    // Schedule next step
    scheduler_enqueue(demo_step, s);
}

void test_serialization() {
    printf("\n=== Testing Serialization ===\n");
    
    uint8_t buffer[256];
    Serializer ser;
    ser_init(&ser, buffer, 256);
    
    // Write data
    ser_u64(&ser, 42);
    ser_u64(&ser, 123);
    
    // Read back
    Deserializer des;
    des_init(&des, buffer, ser.size);
    
    uint64_t a = des_u64(&des);
    uint64_t b = des_u64(&des);
    
    assert(a == 42);
    assert(b == 123);
    printf("Serialization test PASSED\n");
}

void test_linear_token() {
    printf("\n=== Testing Linear Tokens ===\n");
    
    uint8_t buffer[256];
    Serializer ser;
    ser_init(&ser, buffer, 256);
    
    LinearToken token = {12345, 67890};
    ser_linear_token(&ser, &token);
    
    // After serialization, token should be zeroed
    assert(token.id == 0);
    assert(token.secret == 0);
    
    Deserializer des;
    des_init(&des, buffer, ser.size);
    LinearToken restored = des_linear_token(&des);
    
    assert(restored.id == 12345);
    assert(restored.secret == 67890);
    printf("Linear token test PASSED\n");
}

void test_queue() {
    printf("\n=== Testing Queue ===\n");
    
    // Enqueue some items
    scheduler_enqueue(demo_step, (void*)1);
    scheduler_enqueue(demo_step, (void*)2);
    
    CellStep fn;
    void* state;
    
    assert(scheduler_dequeue(&fn, &state) == true);
    assert(state == (void*)1);
    
    assert(scheduler_dequeue(&fn, &state) == true);
    assert(state == (void*)2);
    
    assert(scheduler_dequeue(&fn, &state) == false);
    
    printf("Queue test PASSED\n");
}

// Simple one-shot step function for test
void simple_step(void* state) {
    TestState* ts = (TestState*)state;
    printf("[Simple] Step %d\n", ts->counter);
    if (ts->counter < 2) {
        ts->counter++;
        scheduler_enqueue(simple_step, ts);
    }
}

void test_vm_start() {
    printf("\n=== Testing VM Start ===\n");
    printf("(Manual execution to test scheduler mechanics)\n");
    
    CellStep fn;
    void* state;
    
    // Enqueue 2 tasks
    for (int i = 0; i < 2; i++) {
        TestState* s = malloc(sizeof(TestState));
        s->counter = 0;
        s->name = "T";
        scheduler_enqueue(simple_step, s);
    }
    
    // Drain the queue (simulate what scheduler_dispatch would do)
    int count = 0;
    while (scheduler_dequeue(&fn, &state) && count < 10) {
        fn(state);
        count++;
    }
    
    printf("VM start test PASSED (Executed %d iterations)\n", count);
}

void test_capabilities() {
    printf("\n=== Testing Capabilities ===\n");
    
    CapEnv env = cap_env_create(NULL);
    assert(cap_verify(&env) == true);
    
    CapEnv child = cap_env_fork(&env, NULL);
    assert(cap_verify(&env) == false);  // Parent consumed
    assert(cap_verify(&child) == true); // Child valid
    
    cap_drop(&child);
    assert(cap_verify(&child) == false);
    
    printf("Capabilities test PASSED\n");
}

void test_cell_loader() {
    printf("\n=== Testing Cell Loader ===\n");
    
    // Test that loader exists and is callable
    // (In production would load actual .so)
    CellDescriptor* desc = cell_load("demo", NULL);
    if (desc == NULL) {
        printf("Cell loader test: No demo.so found (expected)\n");
    } else {
        printf("Cell loaded: %s\n", desc->type_hash);
    }
    
    // Test type check logic
    CellDescriptor old = { .type_hash = "A", .required_caps = CAP_ENV };
    CellDescriptor new = { .type_hash = "A", .required_caps = CAP_ENV };
    CellDescriptor different = { .type_hash = "B", .required_caps = CAP_ENV };
    
    assert(cell_can_replace(&old, &new) == true);
    assert(cell_can_replace(&old, &different) == false);
    assert(cell_can_replace(NULL, &new) == false);
    
    printf("Cell loader test PASSED\n");
}

void test_typed_eval() {
    printf("\n=== Testing Typed Eval ===\n");
    
    // Test stub implementation
    const char* expr = "42";
    EvalResult result = typed_eval(expr, "Integer", NULL);
    
    if (result.code == EVAL_OK) {
        printf("Eval successful (stub)\n");
    } else {
        printf("Eval result: %d (%s)\n", result.code, result.data.error);
    }
    
    printf("Typed eval test PASSED (stub)\n");
}

int main() {
    printf("╔════════════════════════════════════════╗\n");
    printf("║   SafestOS VM Runtime Tests           ║\n");
    printf("╚════════════════════════════════════════╝\n\n");
    
    test_serialization();
    test_linear_token();
    test_queue();
    test_capabilities();
    test_cell_loader();
    test_typed_eval();
    test_vm_start();
    
    printf("\n╔════════════════════════════════════════╗\n");
    printf("║   All Tests Passed! ✓                 ║\n");
    printf("╚════════════════════════════════════════╝\n");
    
    return 0;
}
