#!/usr/bin/env bash
# Per-feature scorer wrapper (lane: custom-build-tools, lives in
# lfd/<feature>/harness/ per portfolio convention). Extra args are passed through
# (e.g. --outcomes <dir>, --holdout, --probe <map.json>).
set -euo pipefail
FEATURE="custom-build-tools"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/score_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
