#!/usr/bin/env bash
# Setup script for IronClaw Edge (Raspberry Pi)
# Run as root (sudo ./setup-edge.sh)

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
  echo "Please run as root"
  exit 1
fi

echo "Installing dependencies..."
apt-get update
apt-get install -y socat libgpiod-dev

# Build the MCP GPIO server if cargo is available and source code is present
if command -v cargo &> /dev/null && [ -d "../tools-src/mcp-gpio" ]; then
    echo "Building ironclaw-mcp-gpio..."
    # We must be in deploy/ directory
    cd "$(dirname "$0")/.."
    cargo build --release --bin ironclaw-mcp-gpio
    install -m 755 target/release/ironclaw-mcp-gpio /usr/local/bin/
    echo "Installed ironclaw-mcp-gpio to /usr/local/bin/"
else
    echo "Cargo not found or source missing. Skipping build."
    echo "Please manually copy 'ironclaw-mcp-gpio' to /usr/local/bin/ if not already present."
fi

# Install systemd service
if [ -f "deploy/systemd/ironclaw-gpio.service" ]; then
    echo "Installing systemd service..."
    cp deploy/systemd/ironclaw-gpio.service /etc/systemd/system/
    systemctl daemon-reload
    systemctl enable ironclaw-gpio
    systemctl start ironclaw-gpio
    echo "Service ironclaw-gpio started."
else
    echo "Service file not found. Skipping systemd setup."
fi

echo "Setup complete!"
echo "MCP GPIO Server listening on unix:/var/run/ironclaw-gpio.sock with group 'gpio' access."
echo "IronClaw agents can connect via this socket."
