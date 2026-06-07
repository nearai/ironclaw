#!/usr/bin/env bash
# Emit a deterministic checksum manifest of the t3n-mcp tool SURFACE — the
# names (index.ts), descriptions/annotations (metadata.ts), and input/output
# schemas (schema.ts) the CodeAct agent sees. handler.ts and shared helpers
# (logic, not surface) are intentionally excluded.
#
# The committed snapshot lives at .github/trinity-tool-surface.snapshot and is
# checked in CI by .github/workflows/trinity-tool-surface.yml. When the pinned
# trinity ref (.github/trinity-sdk-ref) is bumped, this manifest changes, CI
# fails, and you must regenerate + review it — forcing a conscious look at the
# tool-surface change before it ships. See CLAUDE.md → "Pinning the trinity
# tool surface".
#
# Usage: gen-trinity-tool-surface-snapshot.sh <path to client/mcp/t3n-mcp/src/server/tools>
set -euo pipefail

tools_dir="${1:?usage: gen-trinity-tool-surface-snapshot.sh <t3n-mcp tools dir>}"
[ -d "$tools_dir" ] || { echo "not a directory: $tools_dir" >&2; exit 1; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

cd "$tools_dir"
# Sorted, locale-stable, NUL-safe walk over the surface-defining files only.
find . -type f \( -name 'index.ts' -o -name 'metadata.ts' -o -name 'schema.ts' \) -print0 \
  | LC_ALL=C sort -z \
  | while IFS= read -r -d '' f; do
      printf '%s  %s\n' "$(sha256 "$f")" "${f#./}"
    done
