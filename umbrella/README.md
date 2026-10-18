# umbrella

PLAN_valen §5 **P3** — an australVM-as-a-service host that lives inside
australVM as a plain subfolder (`australVM/umbrella`; integrated out of
its old submodule form on 2026-10-18) and
runs the whole extern-rust pipeline end to end:

1. `cargo build --release --message-format=json` builds
   [`modules/rscrate`](modules/rscrate) (arbitrary crate; the artifact
   path comes from cargo's JSON stream, nothing about target layout is
   assumed);
2. `dlopen(RTLD_NOW | RTLD_GLOBAL)` registers the cdylib — the bridge's
   default import lookup (`dlsym(RTLD_DEFAULT)`) resolves the crate's
   `rs_*` symbols with **zero australVM changes**;
3. the stock **`austral` binary** is invoked as a subprocess
   (`compile ... --use-cps-jit --allow-all --target-type=tc
   --emit-cps=...`, the same flags modhost's `spawn_jit_server` uses) to
   serialize [`modules/Demo.aum`](modules/Demo.aum) to CPS;
4. the **Cranelift bridge, linked as a plain path dependency**, loads
   [`module.toml`](module.toml) (`--manifest`, or explicit DenyAll
   without it), compiles the CPS with the manifest gate live, and calls
   the entry function in-process.

australVM contains no umbrella-specific code: it is consumed exactly as
the plan drew the boundary — library + front-end binary, while the
umbrella owns cargo and rustc.

## Run

```sh
# granted path: Result should be RESULT -1781872834555808882
cargo run --release -- \
    --manifest module.toml \
    modules/Demo.aum

# denied path: exits 1 before execution; stderr names UK-4001
cargo run --release -- modules/Demo.aum
```

Flags: `--manifest <toml>` grant list; `--entry <name>` (default
`compute`) CPS function to execute; `--austral <bin>` front-end path
(defaults to `../austral`, the dune-built binary at the australVM root,
also
overridable via `AUSTRAL_BIN`); `--cratedir <dir>` crate to build
(default `modules/rscrate`); `--expect <i64>` assert on the result.

## What a run proves

- **granted** — `RESULT <hash>` where the hash is the
  `rs_demo_checksum` FNV-1a over the `"au!"` buffer the Austral module
  created in its own code and passed across the `extern rust` boundary
  (borrowed pointer + borrowed length in, owned scalar out, per
  PLAN_valen §4 Tier 1). The symbol is in **neither** the CPS binary nor
  the bridge's baked rs_* table; it is found only because this project
  built a cdylib and dlopened it.
- **denied** — without the manifest the compile fails with `UK-4001`
  and the process exits 1; no foreign code runs. The gate is checked
  against the manifest's grants, so `rs_*` sits on the kernel boundary
  exactly like `uk_*`/`uz_*`.

## Limits (honest scope)

- single-process, single-module runs — no multi-module linking, no
  versioned deployment dir like `deploy_host`;
- one artifact per cargo invocation (no workspace of crates), rebuilt
  on every run (cargo incremental makes this cheap);
- the demo crate's extern is a plain scalar-out buffer call; no struct
  or linear-type values cross the boundary here (covered by the
  bridge-side tests in australVM instead).