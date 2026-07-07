#!/usr/bin/env bash
set -euo pipefail
FEATURE="smoke-pilot"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/score_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
