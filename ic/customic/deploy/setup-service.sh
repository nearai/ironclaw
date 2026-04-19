#!/bin/bash
# Ironclaw Service Management Script
# Usage: sudo ./deploy/setup-service.sh

set -e

echo "🚀 Setting up Ironclaw systemd service..."

# Copy service file to systemd directory
sudo cp /home/starforce/ironclaw/deploy/ironclaw.service /etc/systemd/system/

# Reload systemd to pick up new service
sudo systemctl daemon-reload

# Enable service to start on boot
sudo systemctl enable ironclaw.service

echo "✅ Service installed and enabled"
echo ""
echo "📋 Available commands:"
echo "  sudo systemctl start ironclaw    # Start the service"
echo "  sudo systemctl stop ironclaw     # Stop the service"
echo "  sudo systemctl restart ironclaw  # Restart the service"
echo "  sudo systemctl status ironclaw   # Check service status"
echo "  sudo journalctl -u ironclaw -f   # View live logs"
echo ""
echo "🎯 To start the service now:"
echo "  sudo systemctl start ironclaw"
echo ""