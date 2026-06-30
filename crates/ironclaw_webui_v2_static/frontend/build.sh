#!/usr/bin/env bash
# One-shot refresh of WebUI v2 frontend artifacts for local inspection or
# vendored dependency updates. Cargo builds the SPA bundle into OUT_DIR when
# `webui-v2-beta` is enabled, so static/dist/ is ignored and must not be
# committed. Only commit static/vendor/ changes when refreshing pinned vendor
# assets.
#
#   ./build.sh           # vendor + npm ci + bundle
#   ./build.sh --no-vendor   # skip re-downloading vendored CDN assets
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ "${1:-}" != "--no-vendor" ]]; then
  bash vendor.sh
fi

echo "Installing build dependencies (npm ci)…"
if [[ ! -f package-lock.json ]]; then
  echo "Error: package-lock.json is missing — refusing to fall back to 'npm install'." >&2
  echo "Artifacts must build from the committed lockfile. Restore it and re-run." >&2
  exit 1
fi
npm ci

echo "Bundling SPA…"
node build.mjs

echo "All WebUI v2 frontend artifacts rebuilt."
