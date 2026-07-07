#!/usr/bin/env bash
# Per-feature status wrapper. Copy to lfd/<feature>/status.sh and replace
# cleanup-old-architecture.
set -euo pipefail
FEATURE="cleanup-old-architecture"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/status_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
