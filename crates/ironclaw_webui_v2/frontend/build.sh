#!/usr/bin/env bash
# One-shot refresh of WebUI v2 frontend artifacts for local inspection or
# vendored dependency updates. Cargo builds the SPA bundle into OUT_DIR when
# `webui-v2-beta` is enabled, so static/dist/ is ignored and must not be
# committed. Only commit static/vendor/ changes when refreshing pinned vendor
# assets.
#
#   ./build.sh           # vendor + pnpm install + bundle
#   ./build.sh --no-vendor   # skip re-downloading vendored CDN assets
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ "${1:-}" != "--no-vendor" ]]; then
  bash vendor.sh
fi

echo "Installing build dependencies (pnpm install --frozen-lockfile)…"
if [[ ! -f pnpm-lock.yaml ]]; then
  echo "Error: pnpm-lock.yaml is missing — refusing to install without a lockfile." >&2
  echo "Artifacts must build from the committed lockfile. Restore it and re-run." >&2
  exit 1
fi
corepack pnpm install --frozen-lockfile

echo "Bundling SPA…"
node build.mjs

echo "All WebUI v2 frontend artifacts rebuilt."
