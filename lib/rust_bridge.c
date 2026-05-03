/*
 * rust_bridge.c
 * C stub for linking Rust library with OCaml
 */

#include <stdint.h>
#include <stdio.h>
#include <caml/mlvalues.h>
#include <caml/memory.h>
#include <caml/alloc.h>

/* Forward declaration from Rust bridge */
extern int64_t compile_to_function(const unsigned char* data, int len);
extern int64_t cranelift_init(void);
extern int64_t cranelift_is_ready(void);
extern int64_t execute_function(void* ptr);
extern int64_t execute_function_1(void* ptr, int64_t arg1);
extern int64_t execute_function_2(void* ptr, int64_t arg1, int64_t arg2);
extern const char* cranelift_last_error(void);
extern void cranelift_clear_error(void);
extern int64_t au_cedar_load_policy(const char* policy);
extern int64_t au_cedar_check_runtime(const char* p, const char* a, const char* r);
extern void au_set_cell_jit_ptr(void* desc, void* jit);
extern int au_cell_swap(uint64_t old_id, void* new_desc);
extern void scheduler_dispatch(void (*func)(void*), void* state);

/* OCaml's scheduler_dispatch for linker symbol resolution */
void scheduler_dispatch(void (*func)(void*), void* state) {
    if (func) {
        func(state);
    }
}

/* Support function for OCaml FFI */
CAMLprim value ocaml_compile_to_function(value ir_data, value ir_len) {
    CAMLparam2(ir_data, ir_len);
    int64_t ptr = compile_to_function((const unsigned char*)String_val(ir_data), Int_val(ir_len));
    CAMLreturn(caml_copy_int64(ptr));
}

/* Initialize bridge */
CAMLprim value ocaml_initialize_bridge(value unit) {
    CAMLparam1(unit);
    int64_t res = cranelift_init();
    CAMLreturn(caml_copy_int64(res));
}

/* Check if ready */
CAMLprim value ocaml_bridge_ready(value unit) {
    CAMLparam1(unit);
    int64_t res = cranelift_is_ready();
    CAMLreturn(caml_copy_int64(res));
}

/* Execute function */
CAMLprim value ocaml_execute_function(value ptr) {
    CAMLparam1(ptr);
    int64_t res = execute_function((void*)Int64_val(ptr));
    CAMLreturn(caml_copy_int64(res));
}
/* Runtime primitives for JIT-compiled code */
void au_print_int(int64_t i) {
    printf("%ld\n", i);
    fflush(stdout);
}

void au_exit(int64_t code) {
    printf("Austral: Exit with code %ld\n", code);
}

void* au_alloc(int64_t size) {
    return malloc((size_t)size);
}

void au_free(void* ptr) {
    free(ptr);
}

CAMLprim value ocaml_execute_function_1(value ptr, value arg1) {
    CAMLparam2(ptr, arg1);
    int64_t res = execute_function_1((void*)Int64_val(ptr), Int64_val(arg1));
    CAMLreturn(caml_copy_int64(res));
}

CAMLprim value ocaml_execute_function_2(value ptr, value arg1, value arg2) {
    CAMLparam3(ptr, arg1, arg2);
    int64_t res = execute_function_2((void*)Int64_val(ptr), Int64_val(arg1), Int64_val(arg2));
    CAMLreturn(caml_copy_int64(res));
}

/* Retrieve last JIT error as an OCaml string option */
CAMLprim value ocaml_cranelift_last_error(value unit) {
    CAMLparam1(unit);
    CAMLlocal2(some, str);
    const char* err = cranelift_last_error();
    if (err == NULL) {
        CAMLreturn(Val_int(0)); /* None */
    }
    str = caml_copy_string(err);
    cranelift_clear_error();
    some = caml_alloc(1, 0);
    Store_field(some, 0, str);
    CAMLreturn(some); /* Some err_string */
}
/* Cedar Policy Management */
CAMLprim value ocaml_cedar_load_policy(value policy_str) {
    CAMLparam1(policy_str);
    int64_t res = au_cedar_load_policy(String_val(policy_str));
    CAMLreturn(caml_copy_int64(res));
}

CAMLprim value ocaml_cedar_check_runtime(value p, value a, value r) {
    CAMLparam3(p, a, r);
    int64_t res = au_cedar_check_runtime(String_val(p), String_val(a), String_val(r));
    CAMLreturn(caml_copy_int64(res));
}

CAMLprim value ocaml_set_cell_jit_ptr(value desc, value jit) {
    CAMLparam2(desc, jit);
    au_set_cell_jit_ptr((void*)Int64_val(desc), (void*)Int64_val(jit));
    CAMLreturn(Val_unit);
}

CAMLprim value ocaml_cell_swap(value old_id, value new_desc) {
    CAMLparam2(old_id, new_desc);
    int res = au_cell_swap(Int64_val(old_id), (void*)Int64_val(new_desc));
    CAMLreturn(Val_int(res));
}

CAMLprim value ocaml_scheduler_dispatch(value unit) {
    CAMLparam1(unit);
    CAMLreturn(Val_unit);
}

CAMLprim value ocaml_au_alloc(value size) {
    CAMLparam1(size);
    void* ptr = malloc(Int64_val(size));
    CAMLreturn(caml_copy_int64((int64_t)ptr));
}

CAMLprim value ocaml_load(value ptr) {
    CAMLparam1(ptr);
    int64_t val = *(int64_t*)Int64_val(ptr);
    CAMLreturn(caml_copy_int64(val));
}

CAMLprim value ocaml_store(value ptr, value val) {
    CAMLparam2(ptr, val);
    *(int64_t*)Int64_val(ptr) = Int64_val(val);
    CAMLreturn(Val_unit);
}
