/*
 * SafestOS Capabilities
 * 
 * Linear capability token management.
 */

#include <stdlib.h>
#include <stdio.h>
#include <string.h>

// Forward declarations
#include "vm.h"

// Capability token generation
static uint64_t token_counter = 1;

// Allocate a fresh environment capability
CapEnv cap_env_create(void* namespace) {
    CapEnv env;
    env.token.id = token_counter++;
    env.token.secret = rand(); // In production: use proper RNG
    env.namespace = namespace;
    return env;
}

// Fork a capability (linear: original is consumed)
CapEnv cap_env_fork(CapEnv* parent, void* new_namespace) {
    // Verify parent is valid
    if (parent->token.id == 0) {
        fprintf(stderr, "Attempt to fork consumed capability\n");
        exit(1);
    }
    
    CapEnv child;
    child.token.id = parent->token.id;
    child.token.secret = parent->token.secret ^ rand(); // Derive new secret
    child.namespace = new_namespace;
    
    // Consume parent
    parent->token.id = 0;
    parent->token.secret = 0;
    parent->namespace = NULL;
    
    return child;
}

// Drop a capability (finalize)
void cap_drop(CapEnv* env) {
    env->token.id = 0;
    env->token.secret = 0;
    env->namespace = NULL;
}

// Verify capability is valid
bool cap_verify(CapEnv* env) {
    return env->token.id != 0 && env->token.secret != 0;
}

// Capability check: does cell have required caps?
bool cell_has_caps(CellDescriptor* desc, CellCaps required) {
    return (desc->required_caps & required) == required;
}

// Capability granting (for delegation)
CellCaps cap_grant(CellCaps base, CellCaps grant) {
    return base | grant;
}

// Capability revocation
CellCaps cap_revoke(CellCaps base, CellCaps revoke) {
    return base & ~revoke;
}
