#!/usr/bin/env bash
set -euo pipefail
FEATURE="smoke-pilot"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
if [[ $# -eq 0 ]]; then
  set -- --out "$ROOT/lfd/$FEATURE/eval/probe/cases"
fi
exec python3 "$ROOT/lfd/_shared/scorer/probe_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
