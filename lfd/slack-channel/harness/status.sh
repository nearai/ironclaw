#!/usr/bin/env bash
# Per-feature status wrapper for the Slack channel LFD lane.
set -euo pipefail
FEATURE="slack-channel"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/status_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
