#!/usr/bin/env bash
#
# Build an Alpine Linux VM image with Docker pre-installed for benchmarking.
#
# Output: alpine-agent.qcow2
#
# This script uses virt-customize (from libguestfs-tools) to modify an Alpine
# cloud image. If virt-customize is not available, it prints manual instructions.
#
# Prerequisites:
#   - libguestfs-tools (apt install libguestfs-tools)
#   - wget or curl
#   - ~2GB disk space
#
# Usage:
#   ./build-image.sh [output-path]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH_DIR="$(dirname "$SCRIPT_DIR")"
OUTPUT="${1:-${SCRIPT_DIR}/alpine-agent.qcow2}"
ALPINE_VERSION="3.19"
ALPINE_ARCH="x86_64"
ALPINE_URL="https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_VERSION}/releases/cloud/nocloud_alpine-${ALPINE_VERSION}.0-${ALPINE_ARCH}-bios-cloudinit-r0.qcow2"

echo "=== Building Alpine VM image with Docker ==="
echo "Output: ${OUTPUT}"
echo ""

# Download Alpine cloud image if not cached
CACHED_IMAGE="${SCRIPT_DIR}/.alpine-base.qcow2"
if [ ! -f "$CACHED_IMAGE" ]; then
    echo "Downloading Alpine cloud image..."
    curl -fSL -o "$CACHED_IMAGE" "$ALPINE_URL"
    echo "Downloaded: $CACHED_IMAGE"
else
    echo "Using cached base image: $CACHED_IMAGE"
fi

# Copy base image to output
cp "$CACHED_IMAGE" "$OUTPUT"

# Resize to accommodate Docker + images
qemu-img resize "$OUTPUT" 4G

# Check for virt-customize
if ! command -v virt-customize &>/dev/null; then
    echo ""
    echo "ERROR: virt-customize not found."
    echo ""
    echo "Install it with: sudo apt install libguestfs-tools"
    echo ""
    echo "Or build the image manually:"
    echo "  1. Boot the image: qemu-system-x86_64 -enable-kvm -m 2048 -drive file=${OUTPUT},format=qcow2 -nographic"
    echo "  2. Log in as root (no password)"
    echo "  3. Run: apk add docker python3 py3-pip bash coreutils && pip install docker && rc-update add docker default"
    echo "  4. Copy agent.py and worker.py into /usr/local/bin/"
    echo "  5. docker pull and save the worker image"
    echo "  6. Shut down and use the image"
    exit 1
fi

echo "Customizing image with virt-customize..."

# Build the worker image tarball to bake into the VM
WORKER_TAR="${SCRIPT_DIR}/.worker-image.tar"
if [ ! -f "$WORKER_TAR" ]; then
    echo "Saving worker Docker image to tarball..."
    docker save bench-worker:latest -o "$WORKER_TAR" 2>/dev/null || {
        echo "WARNING: bench-worker:latest not found. Run 'make images' first."
        echo "The VM image will be built without the pre-loaded worker image."
        WORKER_TAR=""
    }
fi

# Customize the image
CUSTOMIZE_ARGS=(
    --format qcow2
    -a "$OUTPUT"
    # Install packages (including Docker Python SDK for agent.py)
    --run-command "apk add --no-cache docker python3 py3-pip bash coreutils"
    --run-command "pip3 install --break-system-packages docker"
    # Enable Docker daemon
    --run-command "rc-update add docker default"
    # Configure Docker
    --run-command "mkdir -p /etc/docker"
    --write '/etc/docker/daemon.json:{"storage-driver":"overlay2","log-driver":"json-file","log-opts":{"max-size":"10m"}}'
    # Disable swap inside guest
    --run-command "swapoff -a 2>/dev/null; sed -i '/swap/d' /etc/fstab 2>/dev/null || true"
    # Copy scripts
    --copy-in "${BENCH_DIR}/workload/agent.py:/usr/local/bin/"
    --copy-in "${BENCH_DIR}/workload/worker.py:/usr/local/bin/"
    # Enable serial console
    --run-command "sed -i 's/^#ttyS0/ttyS0/' /etc/inittab || true"
)

# Pre-load worker image if available
if [ -n "${WORKER_TAR:-}" ] && [ -f "$WORKER_TAR" ]; then
    CUSTOMIZE_ARGS+=(
        --copy-in "${WORKER_TAR}:/opt/"
        --firstboot-command "docker load < /opt/.worker-image.tar && rm -f /opt/.worker-image.tar"
    )
fi

virt-customize "${CUSTOMIZE_ARGS[@]}"

echo ""
echo "VM image built successfully: ${OUTPUT}"
echo "Size: $(du -h "$OUTPUT" | cut -f1)"
