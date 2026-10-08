#!/usr/bin/env bash
# setup.sh — regenerate a cloud-hypervisor-build-equivalent working directory from
# upstream sources + the SpectrumOS GPU-sharing patches, so this small recipe (a few
# hand-authored Nix/shell files, all committed to this repo) is enough to reproduce
# the full local build tree without vendoring it.
#
# The SpectrumOS patches themselves are fetched from spectrum-os.org at run time
# (not committed to this repo) — they're a third party's licensed work
# (Apache-2.0 AND LicenseRef-BSD-3-Clause-Google; see the LICENSES/ files inside
# the downloaded tarball), so this script downloads them fresh each time rather
# than vendoring a copy into this git history.
#
# What this script does NOT do: build the Nix VM images themselves (that's
# `nix build .#vm-perf`/`.#vm-sec` via flake.nix, once configuration.nix is in place),
# or launch any VM (full-stack-vm-launch.sh / run-vm.sh — both require sudo and real
# GPU/network device access, deliberately not invoked here).
#
# Usage:
#   ./setup.sh [target-dir]     (defaults to ./cloud-hypervisor-build, sibling to this repo)
#
# NixOS notes (this recipe was re-adapted from a Debian-package build 2026-10-08;
# no .deb is produced or needed here — the binaries and the Nix VM images are):
#   * There is no /bin/bash: every script here uses #!/usr/bin/env bash.
#   * crosvm's seccomp step needs python3 on PATH, minijail's Makefile
#     hardcodes /bin/echo (patched below after the clone — idempotent), and
#     wayland-scanner is its own nixpkgs package (not in `wayland`).
#   * A package set + env that works for the whole script:
#       nix-shell -p pkg-config systemd protobuf llvmPackages.libclang openssl \
#         git curl file python3 wayland-scanner wayland wayland-protocols \
#         libdrm mesa libxkbcommon libcap \
#         --run 'export LIBCLANG_PATH="$(nix-build "<nixpkgs>" -A llvmPackages.libclang.lib --no-out-link)/lib"; ./setup.sh'

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET_DIR="${1:-$SCRIPT_DIR/../../cloud-hypervisor-build}"

# Pinned upstream commits (as last verified against a real, working local build —
# 2026-07-01). Bump these deliberately, re-testing the patches still apply, rather
# than always tracking each project's HEAD.
CLOUD_HYPERVISOR_REPO="https://github.com/cloud-hypervisor/cloud-hypervisor.git"
CLOUD_HYPERVISOR_COMMIT="9a24680abdfedf014e496bced49e11b26dda582c"

CROSVM_REPO="https://github.com/google/crosvm"
CROSVM_COMMIT="e36cf699c2253a2901bdda7c8be6a24683e408ee"

VHOST_REPO="https://github.com/rust-vmm/vhost.git"
VHOST_COMMIT="eae4f737781af92d306115368937089e429dde18"

# The SpectrumOS GPU-sharing patches. Pin the sha256 alongside the URL so a
# tampered or silently-changed download is caught rather than applied blindly.
PATCHES_URL="https://spectrum-os.org/software/cloud-hypervisor/cloud-hypervisor-50.0-spectrum0-patches.tar.gz"
PATCHES_SHA256="b495216f460e34acd26a83890086d3912672a368325c48e1a6febdb3afac98bc"

PATCHES_DIR="$(mktemp -d)"
trap 'rm -rf "$PATCHES_DIR"' EXIT

echo "=== Downloading SpectrumOS GPU-sharing patches ==="
curl -fsSL "$PATCHES_URL" -o "$PATCHES_DIR/patches.tar.gz"
echo "$PATCHES_SHA256  $PATCHES_DIR/patches.tar.gz" | sha256sum -c -
tar -xzf "$PATCHES_DIR/patches.tar.gz" -C "$PATCHES_DIR"
PATCHES_SRC="$PATCHES_DIR/cloud-hypervisor-50.0-spectrum0-patches"

mkdir -p "$TARGET_DIR"
cd "$TARGET_DIR"

echo "=== Cloning rust-vmm/vhost @ $VHOST_COMMIT ==="
[ -d vhost ] || git clone "$VHOST_REPO" vhost
git -C vhost checkout "$VHOST_COMMIT"
echo "Applying vhost.patch (SpectrumOS, Apache-2.0 AND LicenseRef-BSD-3-Clause-Google)..."
git -C vhost am --3way "$PATCHES_SRC/vhost.patch"

echo "=== Cloning cloud-hypervisor @ $CLOUD_HYPERVISOR_COMMIT ==="
[ -d cloud-hypervisor ] || git clone "$CLOUD_HYPERVISOR_REPO" cloud-hypervisor
git -C cloud-hypervisor checkout "$CLOUD_HYPERVISOR_COMMIT"
echo "Applying cloud-hypervisor.patch ('build: use local vhost', points Cargo.toml at ../vhost)..."
git -C cloud-hypervisor am --3way "$PATCHES_SRC/cloud-hypervisor.patch"

echo "=== Cloning crosvm @ $CROSVM_COMMIT (GPU device backend, vhost-user-gpu) ==="
[ -d crosvm ] || git clone --recurse-submodules "$CROSVM_REPO" crosvm
git -C crosvm checkout "$CROSVM_COMMIT"

echo "=== NixOS adaptation: drop /bin/echo hardcodes in minijail's Makefile ==="
# Debian build hosts had /bin/echo; NixOS does not, so the GEN rules fail with
# "sh: /bin/echo: No such file or directory" (make Error 127). Plain `echo`
# (shell builtin) is equivalent here. Idempotent, and survives `git -C crosvm
# checkout` above since submodules aren't reset by it.
sed -i 's|/bin/echo|echo|g' \
   crosvm/third_party/minijail/Makefile crosvm/third_party/minijail/common.mk

echo "=== Building cloud-hypervisor (release) ==="
( cd cloud-hypervisor && cargo build --release )

echo "=== Building crosvm (release) ==="
( cd crosvm && cargo build --release )

echo "=== Copying recipe files into place ==="
cp "$SCRIPT_DIR/flake.nix" "$SCRIPT_DIR/configuration.nix" \
   "$SCRIPT_DIR/full-stack-vm-launch.sh" "$SCRIPT_DIR/run-vm.sh" "$TARGET_DIR/"
chmod +x "$TARGET_DIR/full-stack-vm-launch.sh" "$TARGET_DIR/run-vm.sh"

cat <<EOF

Done. $TARGET_DIR now has:
  cloud-hypervisor/   (built release binary at cloud-hypervisor/target/release/cloud-hypervisor)
  crosvm/             (built release binary at crosvm/target/release/crosvm)
  vhost/              (local path dependency cloud-hypervisor's Cargo.toml now points at)
  flake.nix, configuration.nix, full-stack-vm-launch.sh, run-vm.sh

Next steps (NOT run by this script — real host-level actions, do these by hand):
  nix build .#vm-perf   # or .#vm-sec — builds the NixOS guest image
  sudo ./full-stack-vm-launch.sh --strategy perf   # actually boots the VM

No .deb is built on this machine: the original Debian host produced
cloud-hypervisor-spectrum_50.0-spectrum0_amd64.deb (cloud-hypervisor + crosvm
under /usr/bin), but on NixOS the two release binaries above and the flake's
vm-perf/vm-sec images are the deliverables — install via Nix, not dpkg.
EOF
