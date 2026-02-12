#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="$ROOT_DIR/artifacts"
mkdir -p "$ARTIFACT_DIR"

if ! command -v openclaw >/dev/null 2>&1; then
  echo "openclaw CLI not found in PATH. Install OpenClaw first."
  exit 1
fi

APPLY_FIXES=0
if [[ "${1:-}" == "--fix" ]]; then
  APPLY_FIXES=1
fi

STAMP="$(date +%Y%m%d_%H%M%S)"
LOG_PATH="$ARTIFACT_DIR/preflight_${STAMP}.log"

echo "Writing log to $LOG_PATH"

{
  echo "== OpenClaw Preflight Hardening =="
  echo "timestamp: $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  echo

  echo "[1/6] status"
  openclaw status || true
  echo

  echo "[2/6] deep status"
  openclaw status --deep || true
  echo

  echo "[3/6] doctor"
  if [[ "$APPLY_FIXES" -eq 1 ]]; then
    openclaw doctor --repair || true
  else
    openclaw doctor || true
  fi
  echo

  echo "[4/6] channels probe"
  openclaw channels status --probe || true
  echo

  echo "[5/6] models status"
  openclaw models status || true
  echo

  echo "[6/6] security audit"
  if [[ "$APPLY_FIXES" -eq 1 ]]; then
    openclaw security audit --deep --fix || true
  else
    openclaw security audit --deep || true
  fi
} | tee "$LOG_PATH"

echo "Preflight complete."
