#!/usr/bin/env bash
# Per-feature probe wrapper. With no args, writes perturbed cases + map.json to
# lfd/<feature>/eval/probe/cases; pass --out <dir> to override.
set -euo pipefail
FEATURE="custom-build-tools"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
if [[ $# -eq 0 ]]; then
  set -- --out "$ROOT/lfd/$FEATURE/eval/probe/cases"
fi
exec python3 "$ROOT/lfd/_shared/scorer/probe_core.py" \
  --feature "$FEATURE" --lfd-root "$ROOT/lfd" "$@"
