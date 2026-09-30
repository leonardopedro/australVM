# `mathed_kernel` — the granted kernel module behind mathed `\kernel`

The sample hosted module for mathed's kernel segments (velysterm
`PLAN_mathed_full_vision.md` N11, `unfer/docs/PROTOCOL.md`
`kernel_exec`). It is the **generalization of Jupyter kernels to the
plugin system**: a Jupyter kernel is a language runtime trusted after
container quarantine; here safety comes from **grants, not
per-kernel isolation** — the manifest's `[grants] kernel = [uk_*]`
capabilities (checked deny-by-default via Cedar policy, UK-4001) plus
the worker-side `MATHED_EXEC_GRANTS` / `MATHED_KERNEL_LANGS`
allowlists.

## Layout

- `module.toml` — `haskell_effect` archetype; the uk_* capability
  list a `\kernel` segment's grants map onto; `effects = ["Kernel"]`;
  `[limits] max_ms`.
- `haskell/MathedKernel.hs` — the entrypoint `main` implementing the
  wire convention kernel_client uses: one line of
  `{"module", "language", "code"}` JSON on stdin → one line of
  `{"outputs": [<KernelOutput>...]}` JSON on stdout, where
  `KernelOutput` mirrors the Jupyter message content (`stream` /
  `execute_result` / `error`).
- `run_demo.sh` — dev-machine acceptance (fock_match-style): compiles
  the module with the GHC env, proves the wire contract answers
  Jupyter-shaped outputs, and — when modhost is built — proves the
  grant-denial path (a manifest without the requested uk_* grant is
  refused, UK-4001, deny-by-default).

Run: `./run_demo.sh` (GHC env from the unfer flake, read-only use).
The module is dependency-free, so any GHC works: override the env
when the pinned store path is absent, e.g.
`GHC_ENV=$(nix build github:NixOS/nixpkgs/<rev>#ghc --print-out-paths)`
or point it at an installed `ghc` wrapper (`bin/ghc` must be on
`$GHC_ENV/bin`).
velysterm's worker spawns this module as its `MATHED_KERNEL_BIN`
backend; a real Jupyter kernel over the stdio transport attaches
through the same `kernel_exec` op (`kernel_client::jupyter_stdio`).
