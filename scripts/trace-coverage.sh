#!/usr/bin/env bash
# Report which EventKind variants have snapshot coverage.
#
# A replay snapshot proves the engine reached a given EventKind variant in at
# least one scenario. This script is a diagnostic: it enumerates EventKind
# variants and checks whether each appears in any `tests/snapshots/*.snap`.
#
# Runs as a **soft gate** by default (always exit 0). Pass `--strict` to
# promote uncovered variants to a hard failure — CI uses strict mode for
# EventKind because that enum directly describes engine-observable behaviour.
#
# Intentionally skipped: `ThreadState` and `EffectType` are also engine
# enums, but their variants flow through EventKind (`StateChanged`,
# `LeaseGranted`) rather than appearing as top-level keys, so snapshotting
# them independently would just duplicate coverage.

set -euo pipefail

STRICT=false
for arg in "$@"; do
  case "$arg" in
    --strict) STRICT=true ;;
    *) echo "unknown flag: $arg" >&2; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SNAP_DIR="$ROOT/tests/snapshots"
EVENT_RS="$ROOT/crates/ironclaw_engine/src/types/event.rs"

if [ ! -d "$SNAP_DIR" ]; then
  echo "No tests/snapshots directory yet — skipping coverage gate."
  exit 0
fi

# Extract the EventKind enum body and pull top-level identifiers.
# Matches `EventKind {` opening and stops at the matching closing brace. Only
# picks up variant names — lines starting with a capital letter followed by
# `{`, `,`, or a newline.
variants=$(awk '
  /^pub enum EventKind/ { start=1; depth=0; next }
  start {
    depth += gsub(/\{/, "{")
    depth -= gsub(/\}/, "}")
    if (depth<0) { exit }
    # Variant lines: `    VariantName {` or `    VariantName,`
    if (match($0, /^[[:space:]]+([A-Z][A-Za-z0-9_]+)[[:space:]]*(\{|,|$)/, m)) {
      print m[1]
    }
  }
' "$EVENT_RS" | sort -u)

covered=()
missing=()
while IFS= read -r v; do
  [ -z "$v" ] && continue
  # Match the YAML-serialized form in our ThreadSummary output, e.g.
  # `- StateChanged` inside `event_kinds:`.
  if grep -rqF -- "- $v" "$SNAP_DIR"; then
    covered+=("$v")
  else
    missing+=("$v")
  fi
done <<< "$variants"

total=$(( ${#covered[@]} + ${#missing[@]} ))
echo "EventKind coverage (${#covered[@]}/${total} — see snapshot event_kinds lists):"
for v in ${covered[@]+"${covered[@]}"}; do
  echo "  [x] $v"
done
for v in ${missing[@]+"${missing[@]}"}; do
  echo "  [ ] $v"
done

if [ ${#missing[@]} -gt 0 ] && [ "$STRICT" = true ]; then
  echo
  echo "Uncovered EventKind variants fail --strict mode. Add a replay fixture"
  echo "that exercises each, or mark the variant #[allow(dead_code)] if it is"
  echo "genuinely unreachable."
  exit 1
fi
