#!/usr/bin/env bash
# Per-feature lint wrapper (lane: memory-placement-product-layer).
# Prints "OK" or "VOID: constraint violation"; details go to the
# lint-reports directory outside the optimizer's read surface.
set -euo pipefail
FEATURE="memory-placement-product-layer"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/lint_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
