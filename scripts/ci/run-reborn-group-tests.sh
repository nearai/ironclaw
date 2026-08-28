#!/usr/bin/env bash
set -euo pipefail

# Reborn shared-persistence "group" suites are subdirectory `[[test]]` binaries
# (tests/integration/group_*/main.rs, `[[test]]` name reborn_group_<x>). Unlike
# the single-file reborn_integration_*.rs suites, each spins up one runtime and
# drives several tenants' shared libsql-backed stores across threads, so they
# run in a dedicated low-contention job instead of the modulo-partitioned
# integration runner. Database backends always compile, so no backend feature
# flag is required to exercise the libsql-backed shared store.

test_timeout="${REBORN_GROUP_TEST_TIMEOUT:-28m}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ponytail: S2B makes this runner consume inventory-selected names and deletes
# the independent find/sed projection retained by this behavior-only slice.
python3 "${script_dir}/lib/integration_test_inventory.py" \
  --validate-group-topology "${PWD}"

# The directory basename is `group_<x>`; the `[[test]]` `name` field is
# `reborn_group_<x>` (see Cargo.toml) — the two differ by the `reborn_` prefix,
# so rewrite it explicitly rather than assuming dir basename == test name (e.g.
# tests/integration/group_memory -> reborn_group_memory). Topology validation
# rejects incomplete and unregistered groups before this portable GNU/BSD find
# produces the existing sorted execution order.
mapfile -t test_names < <(
  find tests/integration -mindepth 1 -maxdepth 1 -type d -name 'group_*' \
    -exec sh -c 'test -f "$1/main.rs"' _ {} ';' -print \
    | sed -E 's#^tests/integration/group_#reborn_group_#' \
    | LC_ALL=C sort
)

if [ "${#test_names[@]}" -eq 0 ]; then
  echo "No Reborn group tests discovered" >&2
  exit 1
fi

for test_name in "${test_names[@]}"; do
  echo "::group::cargo test --test ${test_name}"
  timeout --signal=INT --kill-after=30s "${test_timeout}" \
    cargo test -p ironclaw_integration_tests --test "${test_name}" --ignore-rust-version -- --nocapture
  echo "::endgroup::"
done
