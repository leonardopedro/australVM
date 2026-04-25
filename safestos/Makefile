# SafestOS Runtime Makefile

CC = gcc
CFLAGS = -std=c11 -Wall -Wextra -fPIC -O2 -Wno-attributes -Wno-unused-result
LDFLAGS = -shared -ldl -pthread
INCLUDE = -I./include

CRANELIFT_DIR = cranelift
CRANELIFT_SO = $(CRANELIFT_DIR)/target/release/libaustral_cranelift_bridge.so

RUNTIME_SRCS = runtime/scheduler.c \
               runtime/serialize.c \
               runtime/region.c \
               runtime/typed_eval.c \
               runtime/cell_loader.c \
               runtime/capabilities.c

RUNTIME_OBJS = $(RUNTIME_SRCS:.c=.o)

.PHONY: all test clean install-headers compiler tools cranelift cranelift-test

# Default target
all: lib/libSafestOS.so cranelift test/vm_test

# Build runtime library
lib/libSafestOS.a: $(RUNTIME_OBJS)
	@mkdir -p lib
	ar rcs $@ $^
	@echo "Built $@"

lib/libSafestOS.so: $(RUNTIME_SRCS)
	@mkdir -p lib
	$(CC) $(CFLAGS) $(INCLUDE) -shared -o $@ $(RUNTIME_SRCS) $(LDFLAGS)
	@echo "Built $@"

runtime/%.o: runtime/%.c include/vm.h
	$(CC) $(CFLAGS) $(INCLUDE) -c $< -o $@

# Cranelift Rust bridge
cranelift: $(CRANELIFT_SO)

$(CRANELIFT_SO):
	cd $(CRANELIFT_DIR) && cargo build --release
	@echo "Built Cranelift bridge"

cranelift-test: $(CRANELIFT_SO)
	cd $(CRANELIFT_DIR) && gcc -rdynamic -o test_bridge test_bridge.c -ldl && ./test_bridge
	@echo "Cranelift bridge test complete"

# Cell compiler tool
compiler/compile_cell: compiler/compile_cell.c lib/libSafestOS.so
	@mkdir -p compiler
	$(CC) $(CFLAGS) $(INCLUDE) -o $@ compiler/compile_cell.c
	@echo "Built cell compiler tool"

# Compile sample cells using the compiler tool
cells/nano_core.so: compiler/compile_cell
	@mkdir -p cells
	@echo "Compiling nano_core.aum..."
	$(if $(wildcard cells/nano_core.aum), \
		./compiler/compile_cell cells/nano_core.aum cells/nano_core.so, \
		@echo "Error: cells/nano_core.aum not found, creating template..." && \
		echo "module NanoCore is end module." > cells/nano_core.aum && \
		./compiler/compile_cell cells/nano_core.aum cells/nano_core.so)

# To build actual austral cells (once compiler is integrated):
cells/%.so: cells/%.aum compiler/compile_cell
	./compiler/compile_cell $< $@

# Test executable
test/vm_test: test/vm_test.c lib/libSafestOS.a
	@mkdir -p test
	$(CC) $(CFLAGS) $(INCLUDE) -o $@ test/vm_test.c -L./lib -lSafestOS -ldl -lpthread
	@echo "Built test executable"

# Full integration test
test/integration: test/vm_test cells/nano_core.so
	@echo "Running integration tests..."
	@echo "Note: Integration tests require full Austral compiler chain"
	@echo "Current: C runtime + Cranelift bridge + compiler tool ready"

# Install headers
install-headers: include/vm.h
	@mkdir -p /usr/local/include/safestos
	cp include/vm.h /usr/local/include/safestos/
	@echo "Headers installed"

# Run all tests
test: test/vm_test cranelift-test
	@echo "Running C runtime tests..."
	LD_LIBRARY_PATH=./lib:$$LD_LIBRARY_PATH ./test/vm_test
	@echo ""
	@echo "All tests passed! (C runtime + Cranelift bridge)"

# Clean
clean:
	rm -rf $(RUNTIME_OBJS) lib/*.a lib/*.so
	rm -rf test/vm_test
	rm -rf compiler/compile_cell
	rm -rf cells/*.so
	cd $(CRANELIFT_DIR) && cargo clean 2>/dev/null || true
	rm -f $(CRANELIFT_DIR)/test_bridge
	@echo "Cleaned"
