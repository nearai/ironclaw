#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${PORT:-8099}"

bash "$ROOT_DIR/scripts/refresh_nuro_interface_data.sh"

echo ""
echo "nuro interface"
echo "URL: http://localhost:${PORT}"
echo "Ctrl+C to stop"

auto_open() {
  if command -v open >/dev/null 2>&1; then
    open "http://localhost:${PORT}" >/dev/null 2>&1 || true
  fi
}

auto_open
cd "$ROOT_DIR/interface"
python3 -m http.server "$PORT"
