#!/usr/bin/env bash
# Per-feature lint wrapper. Copy to lfd/<feature>/lint.sh and replace
# user-voice-model. Prints "OK" or "VOID: constraint violation" (details go to
# the lint-reports directory, never stdout).
set -euo pipefail
FEATURE="user-voice-model"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/lint_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
