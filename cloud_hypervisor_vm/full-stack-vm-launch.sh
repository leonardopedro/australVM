#!/usr/bin/env bash
# full-stack-vm-launch.sh
# Usage:
#   ./full-stack-vm-launch.sh                    (defaults to perf)
#   ./full-stack-vm-launch.sh --strategy perf
#   ./full-stack-vm-launch.sh --strategy sec

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
STRATEGY="${STRATEGY:-perf}"

# --- NixOS adaptations (2026-10-08) -----------------------------------------
# The original targeted Debian; on NixOS none of `/usr/libexec/virtiofsd`,
# `/boot/vmlinuz-$(uname -r)`, nor `cloud-hypervisor` on PATH exist. Every
# lookup below is env-overridable (CLOUD_HYPERVISOR, CROSVM, VIRTIOFSD,
# KERNEL, INITRD); the defaults are the NixOS-native locations.
#
# cloud-hypervisor MUST be the spectrum0-patched build from the tree this
# script ships in (setup.sh's output) — upstream cloud-hypervisor has no
# `--gpu` device, which is the entire point of the SpectrumOS patch.
# Binary root: this script's own dir (when it lives in the regenerated tree),
# else setup.sh's default output ../.. /cloud-hypervisor-build (when it is run
# straight from this recipe directory).
TREE="$SCRIPT_DIR"
if [[ ! -x "$TREE/cloud-hypervisor/target/release/cloud-hypervisor" && -x "$SCRIPT_DIR/../../cloud-hypervisor-build/cloud-hypervisor/target/release/cloud-hypervisor" ]]; then
  TREE="$SCRIPT_DIR/../../cloud-hypervisor-build"
fi
CH_BIN="${CLOUD_HYPERVISOR:-$TREE/cloud-hypervisor/target/release/cloud-hypervisor}"
# crosvm: the vhost-user-gpu backend, same regenerated tree.
CROSVM_BIN="${CROSVM:-$TREE/crosvm/target/release/crosvm}"
# virtiofsd: Debian ships it at /usr/libexec; on NixOS it comes from PATH
# (`nix shell nixpkgs#virtiofsd`, or `nix develop` in this recipe — the
# flake's devShell provides it).
VIRTIOFSD_BIN="${VIRTIOFSD:-/usr/libexec/virtiofsd}"
if [[ ! -x "$VIRTIOFSD_BIN" ]]; then
  VIRTIOFSD_BIN="$(command -v virtiofsd || true)"
fi
# Kernel/initrd: Debian /boot/vmlinuz-$(uname -r) first (original host), else
# NixOS /run/booted-system/{kernel,initrd} — symlinks into the booted
# system's store paths, which cloud-hypervisor reads through fine.
KERNEL_IMG="${KERNEL:-}"
INITRD_IMG="${INITRD:-}"
if [[ -z "$KERNEL_IMG" ]]; then
  if [[ -e "/boot/vmlinuz-$(uname -r)" ]]; then
    KERNEL_IMG="/boot/vmlinuz-$(uname -r)"
    INITRD_IMG="/boot/initrd.img-$(uname -r)"
  elif [[ -e /run/booted-system/kernel ]]; then
    KERNEL_IMG="/run/booted-system/kernel"
    INITRD_IMG="/run/booted-system/initrd"
  else
    echo "No kernel found (tried /boot/vmlinuz-$(uname -r), /run/booted-system/kernel); set KERNEL=/path/to/kernel" >&2
    exit 1
  fi
fi
[[ -x "$CH_BIN" ]] || { echo "cloud-hypervisor not at $CH_BIN — run australVM/cloud_hypervisor_vm/setup.sh to build the spectrum0-patched binary (or set CLOUD_HYPERVISOR=)" >&2; exit 1; }
[[ -x "$CROSVM_BIN" ]] || { echo "crosvm not at $CROSVM_BIN — run australVM/cloud_hypervisor_vm/setup.sh (or set CROSVM=)" >&2; exit 1; }
[[ -n "$VIRTIOFSD_BIN" && -x "$VIRTIOFSD_BIN" ]] || { echo "virtiofsd not found — 'nix shell nixpkgs#virtiofsd' or 'nix develop' in this recipe (or set VIRTIOFSD=)" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --strategy)
      STRATEGY="$2"
      shift 2
      ;;
    *)
      echo "Usage: $0 [--strategy perf|sec]"
      exit 1
      ;;
  esac
done

if [[ "$STRATEGY" != "perf" && "$STRATEGY" != "sec" ]]; then
  echo "Error: strategy must be 'perf' or 'sec', got '$STRATEGY'"
  exit 1
fi

echo "=== Launching with strategy: $STRATEGY ==="

# Clean up stale sockets
sudo rm -f /tmp/vgpu.sock /tmp/vm-nix-virtiofs.sock /tmp/vm-nix-virtiofs.sock.pid
sudo rm -f /tmp/vm-ssh-virtiofs.sock /tmp/vm-ssh-virtiofs.sock.pid

# 1. Start GPU backend
echo "Starting GPU backend..."
"$CROSVM_BIN" device gpu \
  --socket-path /tmp/vgpu.sock \
  --wayland-sock /run/user/$(id -u)/wayland-0 &
GPU_PID=$!

# 2. Start Nix Store exporter (always needed — git lives in /nix/store)
#
# NOTE (P11.23, unfer_nixvm): this must be the host's real, absolute /nix —
# not a path relative to $SCRIPT_DIR (previously "../nix", which silently
# resolved against the *caller's* cwd instead of $SCRIPT_DIR and pointed at
# a nonexistent directory unless this script happened to be invoked from
# one level above $SCRIPT_DIR). Content-addressed store paths only transfer
# with no copy between host and guest — the mechanism this whole flake
# depends on — when the guest's virtiofs mount is backed by the host's
# actual /nix/store, not a project-local decoy.
echo "Starting Nix Store exporter..."
sudo "$VIRTIOFSD_BIN" \
  --socket-path /tmp/vm-nix-virtiofs.sock \
  --shared-dir /nix \
  --sandbox namespace 2>/dev/null &
VIRTIO_PID=$!

# 3. SSH agent socket exporter (sec strategy only)
SSH_VIRTIO_PID=""
if [[ "$STRATEGY" == "sec" ]]; then
  SSH_AUTH_SOCK="${SSH_AUTH_SOCK:-/run/user/1000/gnupg/S.gpg-agent.ssh}"
  SSH_SOCK_DIR="$(dirname "$SSH_AUTH_SOCK")"
  echo "Forwarding SSH agent socket from: $SSH_AUTH_SOCK"
  sudo "$VIRTIOFSD_BIN" \
    --socket-path /tmp/vm-ssh-virtiofs.sock \
    --shared-dir "$SSH_SOCK_DIR" \
    --sandbox namespace 2>/dev/null &
  SSH_VIRTIO_PID=$!
fi

cleanup() {
  kill $GPU_PID $VIRTIO_PID $SSH_VIRTIO_PID 2>/dev/null || true
  sudo rm -f /tmp/vm-nix-virtiofs.sock /tmp/vm-nix-virtiofs.sock.pid
  sudo rm -f /tmp/vm-ssh-virtiofs.sock /tmp/vm-ssh-virtiofs.sock.pid
  sudo rm -f /tmp/vgpu.sock
}
trap cleanup EXIT

echo "Waiting for sockets to initialize..."
sleep 2

# 4. Network setup (both strategies need this for SSH access)
TAP_IF="vm-tap"
TAP_NET="192.168.200"
GATEWAY="${TAP_NET}.1"
VM_IP="${TAP_NET}.2"

sudo ip tuntap add dev "$TAP_IF" mode tap user "$(whoami)" 2>/dev/null || true
sudo ip link set "$TAP_IF" up
sudo ip addr add "${GATEWAY}/24" dev "$TAP_IF" 2>/dev/null || true
NET_ARGS="--net tap=${TAP_IF},ip=${GATEWAY},mask=255.255.255.0"
CMDLINE="console=hvc0 root=/dev/vda rw systemd.unit=multi-user.target net.ifnames=0"

FS_ARGS="--fs tag=host_nix,socket=/tmp/vm-nix-virtiofs.sock"
# DISK override: point at an image built elsewhere (e.g. unfer_nixvm's
# `nix build .#vm-perf-with-unfer`) instead of this directory's ./result.
DISK_IMG="${DISK:-$SCRIPT_DIR/result/nixos.img}"
if [[ "$STRATEGY" == "sec" ]]; then
  FS_ARGS="$FS_ARGS --fs tag=host_ssh,socket=/tmp/vm-ssh-virtiofs.sock"
fi

# 5. Launch cloud-hypervisor in background
echo "Launching NixOS MicroVM via Cloud Hypervisor (strategy=$STRATEGY)..."
sudo "$CH_BIN" \
  --kernel "$KERNEL_IMG" \
  --initramfs "$INITRD_IMG" \
  --cmdline "$CMDLINE" \
  --cpus boot=4 \
  --memory size=4G,shared=on,hugepages=on \
  --disk path="$DISK_IMG" \
  $FS_ARGS \
  $NET_ARGS \
  --gpu socket=/tmp/vgpu.sock &

CH_PID=$!

# 6. Wait for VM, then SSH
echo "Waiting for VM to boot at ${VM_IP}..."
for i in $(seq 1 30); do
  if ping -c 1 -W 1 "$VM_IP" &>/dev/null; then
    echo "VM is ready at ${VM_IP}."
    if [[ "$STRATEGY" == "perf" ]]; then
      echo "Connecting via SSH with agent forwarding (-A)..."
      ssh -A -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        "agent@${VM_IP}" || true
    else
      echo "Connect with: ssh agent@${VM_IP}"
      echo "(Agent socket is forwarded via virtio-fs, SSH -A not needed)"
    fi
    break
  fi
  sleep 2
done

if ! ping -c 1 -W 1 "$VM_IP" &>/dev/null; then
  echo "Timed out waiting for VM to boot. Check console."
fi

wait $CH_PID