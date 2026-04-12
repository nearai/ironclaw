#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VIDEO_DIR="$PROJECT_ROOT/docs/architecture-video"
OUTPUT="${1:-$PROJECT_ROOT/ironclaw-architecture.mp4}"
case "$OUTPUT" in
  /*) ;;
  *) OUTPUT="$PROJECT_ROOT/$OUTPUT" ;;
esac

if ! command -v node &>/dev/null; then
  echo "Error: node is required. Install Node.js >= 18." >&2
  exit 1
fi

if ! command -v npx &>/dev/null; then
  echo "Error: npx is required (comes with npm)." >&2
  exit 1
fi

if [ ! -d "$VIDEO_DIR/node_modules" ]; then
  echo "Installing dependencies..."
  if [ -f "$VIDEO_DIR/package-lock.json" ]; then
    (cd "$VIDEO_DIR" && npm ci --no-fund --no-audit)
  else
    (cd "$VIDEO_DIR" && npm install --no-fund --no-audit)
  fi
fi

echo "Rendering IronClaw architecture video..."
(cd "$VIDEO_DIR" && npx remotion render IronClawArchitecture "$OUTPUT")

echo ""
echo "Done: $OUTPUT"
