#!/usr/bin/env bash
set -euo pipefail

# Reborn shared-persistence "group" suites are subdirectory `[[test]]` binaries
# (tests/reborn_group_*/main.rs). Unlike the single-file reborn_*.rs root tests,
# each spins up one runtime and drives several tenants' shared libsql-backed
# stores across threads, so they run in a dedicated low-contention job instead
# of the modulo-partitioned root runner. `--features libsql` is explicit so the
# group binaries exercise the libsql-backed shared store independently of any
# future change to the default feature set.

test_timeout="${REBORN_GROUP_TEST_TIMEOUT:-28m}"

# The `[[test]]` `name` field equals the directory basename, so emit the
# directory path and let the `s#^tests/##` rewrite turn it into the Cargo test
# name (e.g. tests/reborn_group_memory -> reborn_group_memory). The `sh -c`
# predicate skips a half-scaffolded group dir (no main.rs yet) — returning false
# from `-exec` just filters that dir, it does NOT make `find` exit non-zero, so
# no `|| true` is needed; genuine `find` failures stay visible under `set -e`.
# `sh -c` (not a bare `{}/main.rs`) avoids POSIX implementation-defined `{}`
# substring substitution so discovery is portable across GNU/BSD find.
mapfile -t test_names < <(
  find tests -mindepth 1 -maxdepth 1 -type d -name 'reborn_group_*' \
    -exec sh -c 'test -f "$1/main.rs"' _ {} ';' -print \
    | sed -E 's#^tests/##' \
    | LC_ALL=C sort
)

if [ "${#test_names[@]}" -eq 0 ]; then
  echo "No Reborn group tests discovered" >&2
  exit 1
fi

for test_name in "${test_names[@]}"; do
  echo "::group::cargo test --test ${test_name} --features libsql"
  timeout --signal=INT --kill-after=30s "${test_timeout}" \
    cargo test --test "${test_name}" --features libsql -- --nocapture
  echo "::endgroup::"
done
