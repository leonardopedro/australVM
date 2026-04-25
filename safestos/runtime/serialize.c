/*
 * SafestOS Serialization
 * 
 * Linear type-aware serialization for pause/resume and hot-swap.
 */

#include "vm.h"
#include <string.h>
#include <assert.h>

// Initialize serializer
void ser_init(Serializer* s, uint8_t* buf, size_t cap) {
    s->buffer = buf;
    s->capacity = cap;
    s->size = 0;
    s->pos = 0;
}

// Initialize deserializer
void des_init(Deserializer* d, const uint8_t* buf, size_t size) {
    d->buffer = buf;
    d->size = size;
    d->pos = 0;
}

// Write uint64_t
void ser_u64(Serializer* s, uint64_t v) {
    assert(s->pos + 8 <= s->capacity);
    memcpy(s->buffer + s->pos, &v, 8);
    s->pos += 8;
    if (s->pos > s->size) s->size = s->pos;
}

// Read uint64_t
uint64_t des_u64(Deserializer* d) {
    assert(d->pos + 8 <= d->size);
    uint64_t v;
    memcpy(&v, d->buffer + d->pos, 8);
    d->pos += 8;
    return v;
}

// Write bytes
void ser_bytes(Serializer* s, const uint8_t* data, size_t len) {
    assert(s->pos + len + 8 <= s->capacity);
    ser_u64(s, len);
    memcpy(s->buffer + s->pos, data, len);
    s->pos += len;
    if (s->pos > s->size) s->size = s->pos;
}

// Read bytes
void des_bytes(Deserializer* d, uint8_t* out, size_t len) {
    uint64_t actual_len = des_u64(d);
    assert(actual_len == len);
    assert(d->pos + len <= d->size);
    memcpy(out, d->buffer + d->pos, len);
    d->pos += len;
}

// Linear token serialization (linear: consumed on write)
void ser_linear_token(Serializer* s, LinearToken* t) {
    ser_u64(s, t->id);
    ser_u64(s, t->secret);
    // Zero out the token to prevent reuse
    t->id = 0;
    t->secret = 0;
}

// Deserialization reconstructs the linear token
LinearToken des_linear_token(Deserializer* d) {
    LinearToken t;
    t.id = des_u64(d);
    t.secret = des_u64(d);
    return t;
}

// Capability environment serialization
void ser_cap_env(Serializer* s, CapEnv* env) {
    ser_linear_token(s, &(env->token));
    // Note: namespace pointer is not serialized, must be reconstructed
    ser_u64(s, (uint64_t)(uintptr_t)env->namespace);
}

CapEnv des_cap_env(Deserializer* d) {
    CapEnv env;
    env.token = des_linear_token(d);
    env.namespace = (void*)(uintptr_t)des_u64(d);
    return env;
}
