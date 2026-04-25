#include <stdlib.h>
#include <stdio.h>
#include <stdnoreturn.h>

// Mock scheduler for testing
noreturn void scheduler_dispatch() {
    printf("scheduler_dispatch called - exiting\n");
    exit(0);
}

// Linking test
void test_link() {
    printf("Mock scheduler links correctly\n");
}
