#!/usr/bin/env bash
# Per-feature scorer wrapper. Copy to lfd/<feature>/harness/score.sh and replace
# user-voice-model with the feature name. Extra args are passed through
# (e.g. --outcomes <dir>, --holdout, --probe <map.json>).
set -euo pipefail
FEATURE="user-voice-model"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/score_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
