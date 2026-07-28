#!/usr/bin/env bash
# The automated deletion test (docs/reborn/extension-runtime/overview.md §8,
# checklist DEL-9): every generic Reborn crate's dependency graph must be free
# of concrete extension crates, and its tests must pass without them.
#
# Generic crates are derived from `cargo metadata` — every `ironclaw_*`
# workspace crate declaring a Reborn layer — minus the concrete extension
# crates themselves, the package inventory crate, and the sanctioned
# assemblers (the binary and the architecture test crate).
# This mirrors `crates/ironclaw_architecture/tests/reborn_extension_specificity.rs`;
# keep the two lists in sync.
#
# TEMPORARY_EXCEPTIONS below mirrors CONCRETE_DEPENDENCY_EXCEPTIONS in that
# test: crates listed there are skipped with a warning until the phase that
# deletes the edge lands. The list must be empty by P7 (checklist DEL-7/DEL-9).
#
# Usage: scripts/ci/check-generic-without-concrete.sh [--trees-only]
#   --trees-only  verify dependency graphs but skip the per-crate test runs
#                 (fast local mode; CI runs the full form)
set -euo pipefail

cd "$(dirname "$0")/../.."

TREES_ONLY=false
if [ "${1:-}" = "--trees-only" ]; then
  TREES_ONLY=true
fi

CONCRETE_CRATES=(
  ironclaw_slack_extension
  ironclaw_telegram_extension
)

# dependent-crate:removal-phase — mirrors CONCRETE_DEPENDENCY_EXCEPTIONS.
# Empty since P6 deleted the composition→slack edge (DEL-7/DEL-9).
TEMPORARY_EXCEPTIONS=()

GENERIC_CRATES=()
while IFS= read -r crate_name; do
  GENERIC_CRATES+=("$crate_name")
done < <(cargo metadata --format-version 1 | python3 -c '
import json, sys

REBORN_LAYERS = {"contracts", "substrates", "runtimes", "kernel", "loops", "products", "app"}
EXCLUDED = {
    # Concrete extension crates (the deletion test subjects, not its scope).
    "ironclaw_slack_extension",
    "ironclaw_telegram_extension",
    # The package inventory crate owns the concrete packages.
    "ironclaw_first_party_extensions",
    # Sanctioned assemblers.
    "ironclaw_reborn_cli",
    "ironclaw_architecture",
    "ironclaw_stress",
}

metadata = json.load(sys.stdin)
for package in metadata["packages"]:
    name = package["name"]
    if not (name == "ironclaw" or name.startswith("ironclaw_")):
        continue
    if name in EXCLUDED:
        continue
    layer = (package.get("metadata") or {}).get("ironclaw", {}).get("layer")
    if layer in REBORN_LAYERS:
        print(name)
' | sort)

if [ "${#GENERIC_CRATES[@]}" -eq 0 ]; then
  echo "error: derived no generic crates from cargo metadata" >&2
  exit 1
fi

echo "checking ${#GENERIC_CRATES[@]} generic crates for concrete extension dependencies"

is_excepted() {
  local crate="$1"
  # `${arr[@]+...}` keeps `set -u` happy on bash 3.2 when the list is empty.
  for entry in ${TEMPORARY_EXCEPTIONS[@]+"${TEMPORARY_EXCEPTIONS[@]}"}; do
    if [ "${entry%%:*}" = "$crate" ]; then
      echo "${entry##*:}"
      return 0
    fi
  done
  return 1
}

failures=()
for crate in "${GENERIC_CRATES[@]}"; do
  if phase="$(is_excepted "$crate")"; then
    echo "SKIP  $crate (temporary exception, removed in $phase)"
    continue
  fi
  tree="$(cargo tree -p "$crate" --all-features -e normal,build --prefix none 2>/dev/null || true)"
  if [ -z "$tree" ]; then
    failures+=("$crate: cargo tree produced no output")
    continue
  fi
  for concrete in "${CONCRETE_CRATES[@]}"; do
    if printf '%s\n' "$tree" | grep -q "^${concrete} "; then
      failures+=("$crate: dependency graph contains concrete extension crate ${concrete}")
    fi
  done
done

if [ "${#failures[@]}" -gt 0 ]; then
  printf 'concrete extension crates leaked into generic dependency graphs:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi
echo "dependency graphs clean"

if [ "$TREES_ONLY" = true ]; then
  echo "--trees-only: skipping per-crate test runs"
  exit 0
fi

for crate in "${GENERIC_CRATES[@]}"; do
  if phase="$(is_excepted "$crate")"; then
    continue
  fi
  # Run each crate with the SAME feature recipe the CI crate-tests job uses
  # (feature-gated test targets like product_adapters' contract tests do not
  # compile bare).
  flags="$(scripts/ci/package-feature-flags.sh "$crate")"
  echo "==> cargo test -p $crate ${flags}"
  # shellcheck disable=SC2086
  cargo test -p "$crate" ${flags} --quiet
done
echo "generic crates build and pass tests without concrete extension crates"
