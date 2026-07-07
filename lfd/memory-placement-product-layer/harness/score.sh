#!/usr/bin/env bash
# Per-feature scorer wrapper (lane: memory-placement-product-layer).
# Extra args are passed through (e.g. --outcomes <dir>, --holdout,
# --probe <map.json>).
set -euo pipefail
FEATURE="memory-placement-product-layer"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/score_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
