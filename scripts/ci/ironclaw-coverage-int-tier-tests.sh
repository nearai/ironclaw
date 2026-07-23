#!/usr/bin/env bash
#
# Print the cargo `--test <name>` arguments for the IronClaw in-process
# integration-tier test binaries, one per line.
#
# Integration-tier (task T0-COV) is the set of in-process suites under
# tests/integration/ (post-restructure home of the roadmap integration suite;
# see docs/superpowers/specs/2026-06-26-ironclaw-integration-test-framework-design.md):
#   - tests/integration/<name>.rs   (flat [[test]] binaries; Cargo `name` is
#                                    ironclaw_integration_<name>)
#   - tests/integration/group_<x>/  ([[test]] binaries; Cargo `name` is
#                                    ironclaw_group_<x>)
#
# Discovery is dynamic so coverage automatically picks up new int-tier suites
# as they land (mirrors scripts/ci/run-ironclaw-group-tests.sh's dir->name
# rewrite and scripts/ci/run-ironclaw-root-partition.sh's overall shape).
#
# Candidate names are filtered against Cargo.toml's `[[test]] name = "..."`
# entries: not every flat tests/integration/<name>.rs file is its own binary
# — a file can be a #[path]-mounted shared-fixture sibling included by two or
# more real suites instead (see slack_pairing_fixtures.rs, mounted by
# slack_pairing_redeem.rs / slack_pairing_actor_resolution.rs), and such
# siblings have no `[[test]]` entry of their own. Without this filter, a bare
# directory scan derives a nonexistent `--test ironclaw_integration_<sibling>`
# arg and `cargo llvm-cov ... test` fails outright with "no test target
# named" before running anything in the lane.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

mapfile -t names < <(
  {
    find tests/integration -maxdepth 1 -type f -name '*.rs' \
      | sed -E 's#^tests/integration/#ironclaw_integration_#; s#\.rs$##'
    find tests/integration -mindepth 1 -maxdepth 1 -type d -name 'group_*' \
      -exec sh -c 'test -f "$1/main.rs"' _ {} ';' -print \
      | sed -E 's#^tests/integration/group_#ironclaw_group_#'
  } | LC_ALL=C sort -u | while IFS= read -r candidate; do
    if awk -v name="${candidate}" '
      /^\[\[test\]\]/ { in_test=1; next }
      /^\[/ { in_test=0 }
      in_test && $0 == "name = \"" name "\"" { found=1; exit }
      END { exit !found }
    ' Cargo.toml; then
      printf '%s\n' "${candidate}"
    fi
  done
)

if [ "${#names[@]}" -eq 0 ]; then
  echo "No IronClaw integration-tier test binaries discovered" >&2
  exit 1
fi

for name in "${names[@]}"; do
  printf -- '--test\n%s\n' "${name}"
done
