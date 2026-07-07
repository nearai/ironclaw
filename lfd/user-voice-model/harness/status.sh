#!/usr/bin/env bash
# Per-feature status wrapper. Copy to lfd/<feature>/status.sh and replace
# user-voice-model.
set -euo pipefail
FEATURE="user-voice-model"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/status_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
