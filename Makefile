BIN := austral
SRC := lib/*.ml lib/*.mli lib/*.mll lib/*.mly lib/dune bin/dune bin/austral.ml lib/BuiltInModules.ml
PREFIX ?= /usr/local

# Cranelift bridge location: relative to repo root
BRIDGE_DIR ?= safestos/cranelift/target/release

.PHONY: all
all: $(BIN)

lib/BuiltInModules.ml: lib/builtin/*.aui lib/builtin/*.aum lib/prelude.h lib/prelude.c
	python3 concat_builtins.py

$(BIN): $(SRC)
	dune build lib/ bin/
	cp _build/default/bin/austral.exe $(BIN)

# ── Bridge rebuild ──────────────────────────────────────────────────
# Builds the Rust cranelift bridge, redeploys the .so, and rebuilds the
# OCaml binary. Run after touching safestos/cranelift/src/*.rs or after
# pulling a new unfer commit (the bridge statically links unfer_ffi).
.PHONY: bridge
bridge:
	cargo build --release --manifest-path safestos/cranelift/Cargo.toml
	AUSTRAL_BRIDGE_DIR=$(BRIDGE_DIR) dune build lib/ bin/
	cp _build/default/bin/austral.exe $(BIN)
	@echo "--- Bridge rebuilt: $(BRIDGE_DIR)/libaustral_cranelift_bridge.so"

.PHONY: test
test: $(BIN)
	dune runtest

.PHONY: install
install: $(BIN)
	install -D -m 755 austral $(PREFIX)/bin/austral

.PHONY: uninstall
uninstall:
	sudo rm $(PREFIX)/bin/austral

.PHONY: clean
clean:
	rm -f $(BIN); rm -rf _build; rm -f lib/BuiltInModules.ml
