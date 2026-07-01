# cloud_hypervisor_vm

The small, hand-authored recipe behind `$ROOT/cloud-hypervisor-build` (used by
`../../unfer/unfer_nixvm/`, P11.23) — committed here so it's version-controlled,
without vendoring the ~930MB of upstream checkouts and build output that recipe
produces.

## What's here vs. what isn't

**Here (all authored/adapted, all small):**
- `flake.nix` / `configuration.nix` — the Nix side: two NixOS image variants
  (`vm-perf`, `vm-sec`) via `nixos-generators`, and the guest module (virtiofs `/nix`
  share, GPU passthrough, the `agent` user, sshd).
- `full-stack-vm-launch.sh` / `run-vm.sh` — the launch side: crosvm GPU backend +
  virtiofsd exporters + tap networking + the actual `cloud-hypervisor` invocation.
- `patches/cloud-hypervisor.patch`, `patches/vhost.patch` — the two SpectrumOS
  GPU-sharing patches (Alyssa Ross / Unikie, `Apache-2.0 AND
  LicenseRef-BSD-3-Clause-Google`, see `patches/LICENSES/`) that make
  `vhost-user-gpu` sharing work against this exact cloud-hypervisor/vhost version
  pair.
- `setup.sh` — clones cloud-hypervisor, crosvm, and rust-vmm/vhost at pinned
  commits, applies the two patches, and builds both binaries — regenerating a
  working `cloud-hypervisor-build`-equivalent directory from source.

**Not here, and not meant to be committed anywhere** (regenerate with `setup.sh`
instead):
- Full `cloud-hypervisor`/`crosvm`/`vhost` upstream git checkouts (769MB for
  crosvm alone, each with their own extensive `third_party/` trees — these are
  complete other open-source projects with their own licensing/versioning, not
  something to fold into this repo's history).
- The built `cloud-hypervisor-spectrum_50.0-spectrum0_amd64.deb` (19MB binary) and
  the `deb_build/` dpkg staging tree (101MB) — build artifacts, not source.
- `result` (a Nix build-output symlink) and `listjson` (a generated file).

## Regenerating the full build tree

```sh
./setup.sh                       # -> ../../cloud-hypervisor-build (sibling to this repo)
# or
./setup.sh /path/to/anywhere
```

This clones and builds cloud-hypervisor, crosvm, and vhost at the exact commits
this recipe was last verified against (see `setup.sh`'s pinned-commit header) and
applies the two patches with `git am`. It does **not** build the Nix VM image or
launch anything — see the script's own end-of-run message for those next
(deliberately manual) steps.

## Relationship to `../../unfer/unfer_nixvm/`

`unfer_nixvm` (in the `unfer` repo, since its `packages.*` output is about the
unfer kernel) composes with whichever `cloud-hypervisor-build`-equivalent
directory's `configuration.nix` is on disk, via a `path` flake input — it doesn't
care whether that directory came from running this repo's `setup.sh` or from an
existing hand-built checkout. This split mirrors the earlier P11.20 placement
decision: `arctic_authority` lives here (in `australVM`) because its only consumer
is `australVM/safestos/cranelift`; this VM-build recipe lives here for the
analogous reason — it's runtime/module infrastructure, not kernel code — while the
Nix *package* of the kernel itself (`packages.x86_64-linux.unfer-ffi`) stays in
`unfer`.

## Licenses

- `flake.nix`/`configuration.nix`/`full-stack-vm-launch.sh`/`run-vm.sh`/`setup.sh`:
  original to this project (no upstream license header needed).
- `patches/*.patch`: SpectrumOS, `Apache-2.0 AND LicenseRef-BSD-3-Clause-Google`
  (see `patches/LICENSES/`) — copied verbatim, not modified.
- Regenerated via `setup.sh`: `cloud-hypervisor` (Apache-2.0), `crosvm` (BSD-3-Clause,
  Google), `vhost` (Apache-2.0, rust-vmm) — each stays under its own upstream
  license in its own regenerated checkout; none of that code is copied into this
  repo.
