#!/usr/bin/env bash
#
# QEMU/KVM VM launcher for Approach A (vm-per-agent) benchmarking.
#
# Usage:
#   launch.sh start <agent-id> <memory-mb> <cpus> <vm-image> [extra-env...]
#   launch.sh stop <agent-id>
#   launch.sh stop-all
#
# Each VM gets:
#   - A COW overlay on the base image (minimal disk usage)
#   - A unique SSH port (2200 + agent_id_number) for debugging
#   - Config passed via cloud-init or kernel cmdline
#
# Prerequisites:
#   - qemu-system-x86_64 with KVM support
#   - /dev/kvm accessible

set -euo pipefail

VM_BASE_DIR="${VM_BASE_DIR:-/tmp/bench-vms}"

start_vm() {
    local AGENT_ID="${1:?agent-id required}"
    local MEMORY_MB="${2:-4096}"
    local CPUS="${3:-2}"
    local VM_IMAGE="${4:?vm-image path required}"
    shift 4

    local VM_DIR="${VM_BASE_DIR}/${AGENT_ID}"
    mkdir -p "$VM_DIR"

    # Create COW overlay
    local OVERLAY="${VM_DIR}/overlay.qcow2"
    qemu-img create -f qcow2 -b "$(realpath "$VM_IMAGE")" -F qcow2 "$OVERLAY" > /dev/null

    # Parse agent numeric ID for port assignment
    local ID_NUM
    ID_NUM=$(echo "$AGENT_ID" | grep -o '[0-9]*$' || echo "0")
    local SSH_PORT=$((2200 + ID_NUM))

    # Build cloud-init ISO if we have the config
    local CLOUD_INIT_ARGS=""
    local CIDATA="${VM_DIR}/cidata.iso"
    if command -v genisoimage &>/dev/null || command -v mkisofs &>/dev/null; then
        mkdir -p "${VM_DIR}/cidata"
        # Create user-data with environment variables
        cat > "${VM_DIR}/cidata/user-data" <<USERDATA
#!/bin/sh
# Benchmark agent startup
export AGENT_ID="${AGENT_ID}"
$(for var in "$@"; do echo "export $var"; done)
python3 /usr/local/bin/agent.py &
USERDATA
        cat > "${VM_DIR}/cidata/meta-data" <<META
instance-id: ${AGENT_ID}
local-hostname: ${AGENT_ID}
META
        local ISO_CMD="genisoimage"
        command -v genisoimage &>/dev/null || ISO_CMD="mkisofs"
        "$ISO_CMD" -quiet -output "$CIDATA" -volid cidata -joliet -rock \
            "${VM_DIR}/cidata/user-data" "${VM_DIR}/cidata/meta-data" 2>/dev/null
        CLOUD_INIT_ARGS="-drive file=${CIDATA},format=raw,if=virtio,readonly=on"
    fi

    # Launch QEMU
    qemu-system-x86_64 \
        -enable-kvm \
        -m "$MEMORY_MB" \
        -smp "$CPUS" \
        -drive "file=${OVERLAY},format=qcow2,if=virtio" \
        ${CLOUD_INIT_ARGS} \
        -netdev "user,id=net0,hostfwd=tcp::${SSH_PORT}-:22" \
        -device virtio-net-pci,netdev=net0 \
        -nographic \
        -serial "file:${VM_DIR}/console.log" \
        > "${VM_DIR}/qemu.log" 2>&1 &

    local PID=$!
    echo "$PID" > "${VM_DIR}/qemu.pid"
    echo "$SSH_PORT" > "${VM_DIR}/ssh_port"

    echo "[vm] Started ${AGENT_ID}: PID=${PID}, memory=${MEMORY_MB}MB, ssh=localhost:${SSH_PORT}"
}

stop_vm() {
    local AGENT_ID="${1:?agent-id required}"
    local VM_DIR="${VM_BASE_DIR}/${AGENT_ID}"

    if [ -f "${VM_DIR}/qemu.pid" ]; then
        local PID
        PID=$(cat "${VM_DIR}/qemu.pid")
        kill "$PID" 2>/dev/null || true
        # Wait for process to exit
        for _ in $(seq 1 10); do
            kill -0 "$PID" 2>/dev/null || break
            sleep 0.5
        done
        # Force kill if still running
        kill -9 "$PID" 2>/dev/null || true
        rm -rf "$VM_DIR"
        echo "[vm] Stopped ${AGENT_ID}"
    else
        echo "[vm] ${AGENT_ID} not found"
    fi
}

stop_all() {
    if [ -d "$VM_BASE_DIR" ]; then
        for vm_dir in "${VM_BASE_DIR}"/*/; do
            if [ -f "${vm_dir}/qemu.pid" ]; then
                local AGENT_ID
                AGENT_ID=$(basename "$vm_dir")
                stop_vm "$AGENT_ID"
            fi
        done
        echo "[vm] All VMs stopped."
    fi
}

case "${1:?Usage: $0 start|stop|stop-all ...}" in
    start)
        shift
        start_vm "$@"
        ;;
    stop)
        shift
        stop_vm "$@"
        ;;
    stop-all)
        stop_all
        ;;
    *)
        echo "Usage: $0 start|stop|stop-all ..."
        exit 1
        ;;
esac
