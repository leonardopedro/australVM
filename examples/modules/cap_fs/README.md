# cap_fs — a compiled capability workflow (U22)

This example has two measured paths:

1. **`run.sh`** — the real module runtime path added 2026-11-27. `CapFs.aum` is
   compiled by the stock front end to `module.cps`; `modhost` loads that CPS
   under `module.toml`, installs the instance's capability set and invokes
   `compute` through the JIT. The module calls `rs_cap_open_root`,
   `rs_cap_read` and `rs_cap_write` itself.
2. **`capfsdemo`** — the earlier host-level start
   (`safestos/cranelift/src/bin/capfsdemo.rs`). It drives the same ABI from
   Rust and is kept because it measures the raw capability boundary without the
   module runtime in the way.

## The compiled path

```
./run.sh
granted: read data/records.txt, wrote out/result.txt (23 bytes) matching the reference
denied traversal: ../outside/sentinel.txt refused while the module ran; sentinel unchanged
perturbation: changed input produced the changed reference output (31 bytes)
denied: without the rs_cap_write grant the load is refused (UK-4001) and no file is created
cap_fs workflow: granted read/write matched the reference, traversal refused, perturbation tracked, ungranted call denied
```

The staged instance root is a scratch copy; its `.state/` holds `data/` (granted
`read`, root index 1) and `out/` (granted `create,write`, root index 2). The
state root itself keeps no rights (index 0 is refused). `module.toml` is the
authority: the `kernel` grants name the three verbs and the `fs` grants name the
two roots. The outside sentinel lives at `.state/outside/sentinel.txt`, inside
the instance but outside both approved roots.

What each check rules out:

- **Granted read + write**: the output is compared with `reference.rs`, a
  separate Rust program with ambient filesystem access, compiled by `rustc` on
  every run. A truncated, padded or reordered buffer fails.
- **Denied traversal**: the module attempts `../outside/sentinel.txt` while it
  runs and returns a failure code if the read answers; `run.sh` also re-reads
  the sentinel and requires it byte-identical.
- **Perturbation**: the input is replaced and the module re-run; the output must
  match the new reference and must differ from the first output, so a canned
  result cannot survive.
- **Denied grant**: a copy of the manifest without `rs_cap_write` must fail the
  load with `UK-4001` naming the verb, and must create no `result.txt`.

Exit 0 means all four held; 1 prints the failed check; 2 a required tool is
absent. `KEEP=1` retains the scratch directory and prints its path.
`AVM_FEATURES` overrides the reduced bridge feature set used to build
`modhost` (`--no-default-features --features cedar,tier1-rs` by default while
the sibling `unfer` checkout is mid-merge).

`gen_cps.sh` regenerates `module.cps` from `CapFs.aum` with the ordinary front
end; the CPS is committed so a fresh checkout can run the example without
building OCaml.

## The host-level path (`capfsdemo`)

```
cargo run --quiet --offline --no-default-features --features cedar,tier1-rs \
    --bin capfsdemo -- examples/modules/cap_fs
```

That binary builds the grant itself and compares a granted read to a direct
`std::fs::read`, requires `../outside/sentinel.txt` to return exactly
`CAP_ERR_REFUSED`, and checks the outside sentinel is unchanged. It covers the
capability boundary without the module runtime.

## Remaining gaps

- **The transform is a copy.** `CapFs.aum` could read and write bytes but not
  alter them: the memory builtins that would reverse or rewrite a buffer
  (`positiveOffset`/`load`/`store`) lower to an unresolved `__embed$2` in the
  JIT, so they cannot be called from a compiled module yet. The copy is still
  input-dependent (perturbation is measured), but it is not a transformation.
- **No scoped callback.** U22's card also asks for a callback into the module
  from the adapter; that path is U16's runtime, not wired into this example.
- **No loopback network call.** U20 measures the network capability; this
  example does not yet dial from the module.
- **No umbrella project document.** This runs through `modhost` and a module
  manifest. The umbrella's project-document path (`capabilities` in JSON) is
  exercised separately by U19/U20 tests, not by this example.
- `examples/modules/rust_kv` remains the unimplemented status demo guarded by
  `scripts/test-umbrella --case HAR02`; this example does not replace or relax
  that guard.
