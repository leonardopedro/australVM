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

# dune is not installed on the host: it comes from the flake's `ocaml`
# devShell (`devShells.default` carries why3/alt-ergo and cannot be built on an
# aarch64 host — see AGENTS.md). Running `make` outside that shell used to fail
# with a bare `dune: command not found` *after* `make bridge` had spent five
# minutes in cargo, so the check runs first and names the command to use.
.PHONY: check-dune
check-dune:
	@command -v dune >/dev/null 2>&1 || { \
	  echo "error: dune is not on PATH." >&2; \
	  echo "       dune is provided by the flake's ocaml shell, not by this host:" >&2; \
	  echo "       nix develop \"git+file://\$$PWD#ocaml\" -c make $(MAKECMDGOALS)" >&2; \
	  exit 1; \
	}

# Order-only: the freshness gate must RUN whenever the binary is considered,
# but a phony check must never make the binary rebuild by itself.
$(BIN): $(SRC) | check-bridge-fresh check-dune
	dune build lib/ bin/
	cp _build/default/bin/austral.exe $(BIN)

# ── Bridge rebuild ──────────────────────────────────────────────────
# Builds the Rust cranelift bridge, redeploys the .so, and rebuilds the
# OCaml binary. Run after touching safestos/cranelift/src/*.rs or after
# pulling a new unfer commit (the bridge statically links unfer_ffi).
#
# `--offline` because a plain build first refreshes the crates.io index, and
# on this host that fetch never completes: the build prints nothing for tens
# of minutes (it looks like linking, since output is buffered) and eventually
# dies, while the same build offline finishes in ~7 min. Every dependency is
# either in the cargo cache or a path dep into ../unfer, so the network is not
# needed; if a *new* crate is ever added, run `make bridge CARGO_FLAGS=` once.
CARGO_FLAGS ?= --offline
.PHONY: bridge
bridge: lib/BuiltInModules.ml | check-dune
	cargo build --release $(CARGO_FLAGS) --manifest-path safestos/cranelift/Cargo.toml
# `check-bridge-fresh` compares mtimes, but cargo compares fingerprints: after a
# no-op rebuild it leaves the `.so` older than a source whose bytes it has
# already seen, and the gate then refuses a link that is in fact current. The
# touch records what actually happened — a bridge build ran against these
# sources — instead of asking the `.so` to lie about its age.
	@touch $(BRIDGE_SRC)
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

# The whole-gate drift controls. Each one builds a scratch copy, drifts exactly
# one claim in it, runs `scripts/verify-invariants` against that copy and
# requires the gate to refuse the drift — so each control costs one full gate
# run (~4.5 min here), and this is the long half of `make test`. They are here as
# a target of their own so they can be run alone (`make drift-controls`) without
# rebuilding the compiler, and so their cost is visible rather than hidden
# inside the test target. `scripts/w3-drift-controls all` covers the checks the
# census classified `copy`; the `*-drift-control` scripts cover the rest.
#
# Both halves run `DRIFT_JOBS` gate runs at a time (default: one per core).
# Concurrency is sound because the controls are independent: each writes only
# its own scratch copy, takes its own `TMPDIR` for the gate's `mktemp -d` roots,
# and reads the repository without writing it. The exception is
# `obj-drift-control`, which edits the tree and rebuilds the compiler, so it
# stays last and serial. Measured 2026-11-18 on a two-core host, everything
# holding either way: the 14 `w3-drift-controls` cases take 62m43s at -j 1 and
# 44m9s at -j 2, the five read-only controls 22m44s serial and 17m22s at -P 2 —
# 1h25m27s against 1h01m31s for the two halves, 23m56s (28%) off. Each core is
# worth ~1.4x, not 2x, because a gate run is itself mostly serial. The first half
# writes a row per run to `docs/data/w3/drift-timings.tsv` when
# `W3_DRIFT_RECORD` names it, so those numbers are quoted rather than retyped.
DRIFT_JOBS ?= $(shell nproc 2>/dev/null || echo 1)
READ_ONLY_CONTROLS = scripts/w3-emitter-drift-control \
                     scripts/w93-composition-drift-control \
                     scripts/w93-closure-drift-control \
                     scripts/w93-certification-drift-control \
                     scripts/w3-approval-state-drift-control

# ── Emitter byte-identity gate (docs/lean4mm0.md §13.39) ─────────────────────
# Every rewrite since §13.30 is justified by "the artifacts do not move", and
# each was proved once, by hand, with `cmp`. This gate is that proof as a check:
# FockPair in its three modes, sizes and sha256 against
# `scripts/lean-typing-proof.golden.tsv`. It needs the corpus, and `mm0/exports/`
# is gitignored, so a corpus-less checkout gets an explained gap and exit 0 —
# never a silent pass. Re-record deliberately after an intended byte change:
# `make check-typing-proof RECORD=1`.
.PHONY: check-typing-proof
check-typing-proof:
	@python3 scripts/check-lean-typing-proof $(if $(RECORD),--record,)

.PHONY: drift-controls
drift-controls: | check-bridge-fresh
	@echo "--- drift controls: one full gate run each, $(DRIFT_JOBS) at a time"
	@bash scripts/w3-drift-controls -j $(DRIFT_JOBS) all
	@printf '%s\n' $(READ_ONLY_CONTROLS) \
	  | xargs -P $(DRIFT_JOBS) -I{} bash -c 'echo "--- {}"; bash {}'
	@echo "--- scripts/obj-drift-control (edits the tree, rebuilds: serial)"
	@bash scripts/obj-drift-control

.PHONY: test
test: $(BIN) drift-controls | check-bridge-fresh
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

# ── W9: the external ISA model (lean shell) ─────────────────────────
# W9.1 re-converts the two pinned RV64 models and W9.3 refutes the
# "the JIT's memory op lowers to execute_LOAD/execute_STORE" shortcut. Both
# need one PATH carrying lean, lean4export, mm0 and python3 with the model
# trees on it, i.e. the flake's `lean` shell — and *entering* that shell builds
# the derivations, the Sail one taking ~28 min cold (store-cached afterwards).
# That cost is why `scripts/verify-invariants` reports both as `[gap]` when it
# runs anywhere else, and why this target exists: the gap says "not checked
# here", which is only honest if some entry point does check it.
#
# The acceptance lines are asserted, not just the exit codes: a converter that
# silently stops emitting an opcode (`0 not covered`), or a lowering that grows
# the model's signature so the refutation no longer holds (`REFUTED as
# stated`), has to fail here instead of reading as a documented gap.
.PHONY: w9
w9:
	nix develop "git+file://$(CURDIR)#lean" -c bash -c '\
	  set -o pipefail; \
	  scripts/rv64-model-mm0 check 2>&1 | tee /tmp/w9-rv64.log; \
	  grep -q "check: 2 stem(s) match" /tmp/w9-rv64.log \
	    || { echo "w9: expected both stems (ALU + memory) to be checked" >&2; exit 1; }; \
	  grep -q "0 not covered" /tmp/w9-rv64.log \
	    || { echo "w9: ISA_REGISTRY coverage regressed (an opcode is not covered)" >&2; exit 1; }; \
	  scripts/mem-refines 2>&1 | tee /tmp/w9-mem.log; \
	  grep -q "REFUTED as stated" /tmp/w9-mem.log \
	    || { echo "w9: load/store now agree with the model — re-argue the correspondence" >&2; exit 1; }'

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
