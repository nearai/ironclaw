#!/usr/bin/env bash
# Per-feature lint wrapper. Prints "OK" or "VOID: constraint violation";
# details go to the lint-reports directory, never stdout.
set -euo pipefail
FEATURE="custom-build-tools"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec python3 "$ROOT/lfd/_shared/scorer/lint_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
