#!/usr/bin/env bash
# Per-feature status wrapper. Copy to lfd/<feature>/status.sh and replace
# missions.
set -euo pipefail
FEATURE="missions"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/status_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
