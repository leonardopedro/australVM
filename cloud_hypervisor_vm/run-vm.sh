#!/bin/bash
# run-vm.sh
# Usage:
#   ./run-vm.sh               (defaults to perf — boots VM, then ssh -A into it)
#   ./run-vm.sh --strategy sec

set -euo pipefail

STRATEGY="${1:-perf}"
if [[ "$1" == "--strategy" ]]; then
  STRATEGY="$2"
fi

if [[ "$STRATEGY" != "perf" && "$STRATEGY" != "sec" ]]; then
  echo "Usage: $0 [--strategy perf|sec]"
  exit 1
fi

echo "=== Launching with strategy: $STRATEGY ==="

# Network — always set up (needed for SSH access to the VM)
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
if [[ "$STRATEGY" == "sec" ]]; then
  FS_ARGS="$FS_ARGS --fs tag=host_ssh,socket=/tmp/vm-ssh-virtiofs.sock"
fi

sudo cloud-hypervisor \
  --kernel /boot/vmlinuz-$(uname -r) \
  --initramfs /boot/initrd.img-$(uname -r) \
  --cmdline "$CMDLINE" \
  --cpus boot=4 \
  --memory size=4G,shared=on,hugepages=on \
  --disk path=./result/nixos.img \
  $FS_ARGS \
  $NET_ARGS \
  --gpu socket=/tmp/vgpu.sock &

CH_PID=$!

# Wait for VM to boot, then SSH with agent forwarding for perf
echo "Waiting for VM to boot at ${VM_IP}..."
for i in $(seq 1 30); do
  if ping -c 1 -W 1 "$VM_IP" &>/dev/null; then
    echo "VM is ready at ${VM_IP}."
    if [[ "$STRATEGY" == "perf" ]]; then
      echo "Connecting via SSH with agent forwarding (-A)..."
      ssh -A -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        "agent@${VM_IP}"
    else
      echo "Connect with: ssh agent@${VM_IP}"
      echo "(Agent socket is forwarded via virtio-fs, no -A needed)"
    fi
    break
  fi
  sleep 2
done

if ! ping -c 1 -W 1 "$VM_IP" &>/dev/null; then
  echo "Timed out waiting for VM to boot. Check console."
fi

wait $CH_PID