#!/usr/bin/env bash
# Per-feature scorer wrapper (lane: cleanup-old-architecture, lives in
# lfd/<feature>/harness/ per INSTRUMENTS). Extra args are passed through
# (e.g. --outcomes <dir>, --holdout, --probe <map.json>).
set -euo pipefail
FEATURE="cleanup-old-architecture"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/score_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
