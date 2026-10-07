BIN := austral
SRC := lib/*.ml lib/*.mli lib/*.mll lib/*.mly lib/dune bin/dune bin/austral.ml lib/BuiltInModules.ml
PREFIX ?= /usr/local

# Cranelift bridge location: relative to repo root.
#
# Deliberately NOT named `BRIDGE_DIR`: that name belongs to the `test` staging
# directory below (`$(HOME)/.local/lib`), and sharing one name for two
# directories is what silently broke this target — `AUSTRAL_BRIDGE_DIR`
# expanded to `<repo>//$(HOME)/.local/lib` and ld could not find
# `-laustral_cranelift_bridge`. Two directories, two names.
BRIDGE_CARGO_DIR = safestos/cranelift/target/release

.PHONY: all
all: $(BIN)

# ── Bridge freshness gate (A15) ─────────────────────────────────────────────
# The OCaml side LINKS the release artifact (lib/dune: -laustral_cranelift_bridge),
# so staleness is not a performance detail: a stale lib silently misses the
# symbols added since the last `make bridge` — the failure mode AGENTS.md
# names as "may miss uk_* symbols". This gate runs before anything that links
# it and turns that silence into an actionable error.
#
# Sources checked: the bridge crate itself (src/*.rs, Cargo.toml, Cargo.lock).
# The statically linked unfer_ffi is deliberately out of scope — ../unfer's
# tree moves on independently, and "rebuild after pulling a new unfer commit"
# stays the documented `make bridge` step (see the comment above that target).
BRIDGE_RUST_SRC := $(wildcard safestos/cranelift/src/*.rs safestos/cranelift/src/*/*.rs) \
	safestos/cranelift/Cargo.toml safestos/cranelift/Cargo.lock

.PHONY: check-bridge-fresh
check-bridge-fresh:
	@test -f "$(BRIDGE_SRC)" || { \
	  echo "error: $(BRIDGE_SRC) does not exist." >&2; \
	  echo "       The OCaml side links it; build it first: make bridge" >&2; \
	  exit 1; \
	}
	@stale=$$(find $(BRIDGE_RUST_SRC) -newer "$(BRIDGE_SRC)" 2>/dev/null | head -n 1); \
	if [ -n "$$stale" ]; then \
	  echo "error: stale bridge — $$stale is newer than $(BRIDGE_SRC)." >&2; \
	  echo "       Linking it would silently miss symbols (the uk_* drift AGENTS.md names)." >&2; \
	  echo "       Rebuild it: make bridge" >&2; \
	  exit 1; \
	fi

lib/BuiltInModules.ml: lib/builtin/*.aui lib/builtin/*.aum lib/prelude.h lib/prelude.c
	python3 concat_builtins.py

# Order-only: the freshness gate must RUN whenever the binary is considered,
# but a phony check must never make the binary rebuild by itself.
$(BIN): $(SRC) | check-bridge-fresh
	dune build lib/ bin/
	cp _build/default/bin/austral.exe $(BIN)

# ── Bridge rebuild ──────────────────────────────────────────────────
# Builds the Rust cranelift bridge, redeploys the .so, and rebuilds the
# OCaml binary. Run after touching safestos/cranelift/src/*.rs or after
# pulling a new unfer commit (the bridge statically links unfer_ffi).
.PHONY: bridge
bridge: lib/BuiltInModules.ml
	cargo build --release --manifest-path safestos/cranelift/Cargo.toml
	AUSTRAL_BRIDGE_DIR=$(CURDIR)/$(BRIDGE_CARGO_DIR) dune build lib/ bin/
	cp _build/default/bin/austral.exe $(BIN)
	@echo "--- Bridge rebuilt: $(CURDIR)/$(BRIDGE_CARGO_DIR)/libaustral_cranelift_bridge.so"

# X4(b): one entry point that works out how to load the bridge, instead of every
# caller having to know.
#
# Two situations, and they need opposite answers:
#
#   * the bridge was built by the *nix* toolchain -> its dependencies resolve from
#     the store, so LD_LIBRARY_PATH is enough;
#   * the bridge was built by *host rustup* against a newer glibc than the nix
#     dev-shell provides -> the dev-shell loader refuses it, and the test binary
#     has to be launched by the host loader instead.
#
# Detecting which one you have is the whole point. Getting it wrong produces
# `error while loading shared libraries: libaustral_cranelift_bridge.so`, which
# reads like a missing build rather than a loader mismatch.
BRIDGE_SRC = safestos/cranelift/target/release/libaustral_cranelift_bridge.so
BRIDGE_DIR = $(HOME)/.local/lib
HOST_LOADER = /lib64/ld-linux-x86-64.so.2

.PHONY: test
test: $(BIN) | check-bridge-fresh
	@mkdir -p $(BRIDGE_DIR)
	@cp -f $(BRIDGE_SRC) $(BRIDGE_DIR)/
	@LD_LIBRARY_PATH=$(BRIDGE_DIR) dune build @runtest
	@if LD_LIBRARY_PATH=$(BRIDGE_DIR) ./_build/default/test/JitTest.exe >/dev/null 2>&1; then \
	  echo "loader: dev-shell loader can load the bridge (LD_LIBRARY_PATH)"; \
	  LD_LIBRARY_PATH=$(BRIDGE_DIR) dune runtest; \
	elif [ -x "$(HOST_LOADER)" ]; then \
	  echo "loader: dev-shell loader refused the bridge; re-execing under $(HOST_LOADER)"; \
	  echo "        (the .so was built against a newer glibc than this shell provides)"; \
	  failed=0; \
	  for t in ./_build/default/test/*.exe; do \
	    if LD_LIBRARY_PATH=$(BRIDGE_DIR) $(HOST_LOADER) "$$t" >/dev/null 2>&1; then \
	      echo "  ok   $$(basename $$t)"; \
	    else \
	      echo "  FAIL $$(basename $$t)"; \
	      failed=1; \
	    fi; \
	  done; \
	  exit $$failed; \
	else \
	  echo "loader: the dev-shell loader refused the bridge and no host loader was found." >&2; \
	  echo "        Rebuild the bridge with the nix toolchain: make -C safestos/cranelift" >&2; \
	  exit 1; \
	fi

.PHONY: loader
loader:
	@echo "bridge:  $(BRIDGE_SRC)"
	@echo "staged:  $(BRIDGE_DIR)"
	@echo -n "dev-shell loader can load it: "
	@LD_LIBRARY_PATH=$(BRIDGE_DIR) ./_build/default/test/JitTest.exe >/dev/null 2>&1 \
	  && echo yes || echo no
	@echo -n "host loader present:          "
	@[ -x "$(HOST_LOADER)" ] && echo "$(HOST_LOADER)" || echo no

.PHONY: install
install: $(BIN)
	install -D -m 755 austral $(PREFIX)/bin/austral

.PHONY: uninstall
uninstall:
	sudo rm $(PREFIX)/bin/austral

.PHONY: clean
clean:
	rm -f $(BIN); rm -rf _build; rm -f lib/BuiltInModules.ml
