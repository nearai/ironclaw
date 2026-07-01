#!/usr/bin/env bash
#
# Print the cargo `--test <name>` arguments for the Reborn in-process
# integration-tier test binaries, one per line.
#
# Integration-tier (task T0-COV) is the set of in-process suites:
#   - tests/reborn_integration_*.rs        (single-file root test binaries)
#   - tests/reborn_group_*/                ([[test]] binaries; name == dir name)
#
# Discovery is dynamic so coverage automatically picks up new int-tier suites
# as they land (mirrors scripts/ci/run-reborn-root-partition.sh).

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

mapfile -t names < <(
  {
    find tests -maxdepth 1 -type f -name 'reborn_integration_*.rs' \
      | sed -E 's#^tests/##; s#\.rs$##'
    find tests -maxdepth 1 -type d -name 'reborn_group_*' \
      | sed -E 's#^tests/##'
  } | LC_ALL=C sort -u
)

if [ "${#names[@]}" -eq 0 ]; then
  echo "No Reborn integration-tier test binaries discovered" >&2
  exit 1
fi

for name in "${names[@]}"; do
  printf -- '--test\n%s\n' "${name}"
done
