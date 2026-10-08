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
- `setup.sh` — clones cloud-hypervisor, crosvm, and rust-vmm/vhost at pinned
  commits, **downloads** (does not vendor) the two SpectrumOS GPU-sharing patches,
  applies them, and builds both binaries — regenerating a working
  `cloud-hypervisor-build`-equivalent directory from source.

**Not here, and not meant to be committed anywhere** (regenerate with `setup.sh`
instead):
- The SpectrumOS patches themselves (`cloud-hypervisor.patch`, `vhost.patch`) — a
  third party's licensed work (`Apache-2.0 AND LicenseRef-BSD-3-Clause-Google`).
  `setup.sh` downloads them fresh from
  `https://spectrum-os.org/software/cloud-hypervisor/cloud-hypervisor-50.0-spectrum0-patches.tar.gz`
  each run (checksum-pinned — see `setup.sh`'s `PATCHES_SHA256`) rather than
  keeping a copy in this git history.
- Full `cloud-hypervisor`/`crosvm`/`vhost` upstream git checkouts (769MB for
  crosvm alone, each with their own extensive `third_party/` trees — these are
  complete other open-source projects with their own licensing/versioning, not
  something to fold into this repo's history).
- The built `cloud-hypervisor-spectrum_50.0-spectrum0_amd64.deb` (19MB binary) and
  the `deb_build/` dpkg staging tree (101MB) — build artifacts, not source. Not
  reproduced on NixOS: since the 2026-10-08 re-adaptation there is no `.deb`
  step at all (the release binaries + the flake's VM images are the artifacts;
  see `setup.sh`'s end-of-run message).
- `result` (a Nix build-output symlink) and `listjson` (a generated file).

## Regenerating the full build tree

```sh
./setup.sh                       # -> ../../cloud-hypervisor-build (sibling to this repo)
# or
./setup.sh /path/to/anywhere
```

This downloads the SpectrumOS patch tarball (checksum-verified), clones
cloud-hypervisor, crosvm, and vhost at the exact commits this recipe was last
verified against (see `setup.sh`'s pinned-commit header), and applies the two
patches with `git am`. It does **not** build the Nix VM image or launch anything
— see the script's own end-of-run message for those next (deliberately manual)
steps.

### NixOS notes (re-adaptation 2026-10-08)

The recipe originally targeted a Debian host (it produced a `.deb`); it now runs
on NixOS with no packaging step:

- Scripts use `#!/usr/bin/env bash` (NixOS has no `/bin/bash`).
- `setup.sh` patches minijail's hardcoded `/bin/echo` (absent on NixOS) right
  after the crosvm clone — idempotent.
- Build deps for a single `nix-shell` invocation are listed at the top of
  `setup.sh` (python3 for the seccomp compiler, `wayland-scanner` for crosvm's
  protocol codegen, wayland/mesa/libdrm/libcap/libxkbcommon for its display
  stack, `LIBCLANG_PATH` via `nix-build -A llvmPackages.libclang.lib` for
  bindgen).
- No `.deb` is built or needed: the two release binaries and the flake's
  `vm-perf`/`vm-sec` images are the deliverables.
- The launch scripts resolve NixOS locations at run time instead of
  hardcoding Debian ones: patched `cloud-hypervisor`/`crosvm` from the
  regenerated tree (or `CLOUD_HYPERVISOR=`/`CROSVM=`; when run straight from
  this recipe directory they fall back to `setup.sh`'s default output
  `../../cloud-hypervisor-build`), kernel + initrd from
  `/run/booted-system/{kernel,initrd}` (fallback: Debian's
  `/boot/vmlinuz-$(uname -r)`; override `KERNEL=`/`INITRD=`), and virtiofsd
  from PATH — **`nix develop` in this recipe provides it** (the flake's
  devShell). `DISK=` points `--disk` at an image built elsewhere (e.g.
  `unfer/unfer_nixvm`'s `nix build .#vm-perf-with-unfer`). Every missing
  dependency is a named error *before* any `sudo` runs.

## Relationship to `../../unfer/unfer_nixvm/`

`unfer_nixvm` (in the `unfer` repo, since its `packages.*` output is about the
unfer kernel) composes with **this directory directly** — since 2026-10-08 its
`path` flake input is `australVM/cloud_hypervisor_vm` itself (the tracked
recipe), so the composition evaluates from committed files only and does not
need a regenerated `cloud-hypervisor-build/` tree at all; the tree is
launch-only, carrying the two spectrum0-patched binaries. (It originally
pointed at whichever `cloud-hypervisor-build`-equivalent directory was on
 disk — the two `configuration.nix` copies are byte-identical, so the
composition never cared which one it got.) This split mirrors the earlier P11.20 placement
decision: `arctic_authority` lives here (in `australVM`) because its only consumer
is `australVM/safestos/cranelift`; this VM-build recipe lives here for the
analogous reason — it's runtime/module infrastructure, not kernel code — while the
Nix *package* of the kernel itself (`packages.x86_64-linux.unfer-ffi`) stays in
`unfer`.

## Licenses

- `flake.nix`/`configuration.nix`/`full-stack-vm-launch.sh`/`run-vm.sh`/`setup.sh`:
  original to this project (no upstream license header needed).
- The SpectrumOS patches (`Apache-2.0 AND LicenseRef-BSD-3-Clause-Google`) are
  **not vendored here** — `setup.sh` downloads them at run time from
  spectrum-os.org and applies them directly to the freshly-cloned
  cloud-hypervisor/vhost checkouts; their own `LICENSES/` files travel with the
  downloaded tarball, not with this repo.
- Regenerated via `setup.sh`: `cloud-hypervisor` (Apache-2.0), `crosvm` (BSD-3-Clause,
  Google), `vhost` (Apache-2.0, rust-vmm) — each stays under its own upstream
  license in its own regenerated checkout; none of that code is copied into this
  repo.
