#!/usr/bin/env bash
#
# Print the cargo `--test <name>` arguments for the Reborn in-process
# integration-tier test binaries, one per line.
#
# Integration-tier (task T0-COV) is every test target Cargo resolves from the
# root manifest whose source path is under tests/integration/ (the
# post-restructure home of the roadmap integration suite; see
# docs/superpowers/specs/2026-06-26-reborn-integration-test-framework-design.md).
# This includes flat, group, and other nested targets such as auth/*.rs.
#
# Cargo metadata is authoritative for target names and paths. This avoids
# deriving names from directory conventions and naturally excludes shared
# fixtures that are mounted by real suites but are not test targets themselves.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

metadata="$(cargo metadata --no-deps --format-version 1 --manifest-path Cargo.toml)"
names_output="$(
  printf '%s\n' "${metadata}" | jq -r '
    .workspace_root as $root
    | .packages[]
    | select(.manifest_path == ($root + "/Cargo.toml"))
    | .targets[]
    | select(.kind | index("test"))
    | select(.src_path | startswith($root + "/tests/integration/"))
    | .name
  ' | LC_ALL=C sort -u
)"
names=()
while IFS= read -r name; do
  if [ -n "${name}" ]; then
    names+=("${name}")
  fi
done <<< "${names_output}"

if [ "${#names[@]}" -eq 0 ]; then
  echo "No Reborn integration-tier test binaries discovered" >&2
  exit 1
fi

for name in "${names[@]}"; do
  printf -- '--test\n%s\n' "${name}"
done
