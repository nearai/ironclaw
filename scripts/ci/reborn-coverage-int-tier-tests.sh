#!/usr/bin/env bash
#
# Print the cargo `--test <name>` arguments for the Reborn in-process
# integration-tier test binaries, one per line.
#
# Integration-tier (task T0-COV) is the set of in-process suites under
# tests/integration/ (post-restructure home of the roadmap integration suite;
# see docs/superpowers/specs/2026-06-26-reborn-integration-test-framework-design.md):
#   - tests/integration/<name>.rs   (flat [[test]] binaries; Cargo `name` is
#                                    reborn_integration_<name>)
#   - tests/integration/group_<x>/  ([[test]] binaries; Cargo `name` is
#                                    reborn_group_<x>)
#
# Discovery is dynamic so coverage automatically picks up new int-tier suites
# as they land (mirrors scripts/ci/run-reborn-group-tests.sh's dir->name
# rewrite and scripts/ci/run-reborn-root-partition.sh's overall shape).

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

mapfile -t names < <(
  {
    find tests/integration -maxdepth 1 -type f -name '*.rs' \
      | sed -E 's#^tests/integration/#reborn_integration_#; s#\.rs$##'
    find tests/integration -mindepth 1 -maxdepth 1 -type d -name 'group_*' \
      -exec sh -c 'test -f "$1/main.rs"' _ {} ';' -print \
      | sed -E 's#^tests/integration/group_#reborn_group_#'
  } | LC_ALL=C sort -u
)

if [ "${#names[@]}" -eq 0 ]; then
  echo "No Reborn integration-tier test binaries discovered" >&2
  exit 1
fi

for name in "${names[@]}"; do
  printf -- '--test\n%s\n' "${name}"
done
