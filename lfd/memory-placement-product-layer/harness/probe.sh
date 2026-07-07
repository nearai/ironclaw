#!/usr/bin/env bash
# Per-feature probe wrapper (lane: memory-placement-product-layer).
# With no args, writes perturbed cases + map.json to
# lfd/<feature>/eval/probe/cases.
set -euo pipefail
FEATURE="memory-placement-product-layer"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
if [[ $# -eq 0 ]]; then
  set -- --out "$ROOT/lfd/$FEATURE/eval/probe/cases"
fi
exec python3 "$ROOT/lfd/_shared/scorer/probe_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
