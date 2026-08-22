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

# The directory basename is `group_<x>`; the `[[test]]` `name` field is
# `reborn_group_<x>` (see Cargo.toml) — the two differ by the `reborn_` prefix,
# so rewrite it explicitly rather than assuming dir basename == test name (e.g.
# tests/integration/group_memory -> reborn_group_memory). The `sh -c` predicate
# skips a half-scaffolded group dir (no main.rs yet) — returning false from
# `-exec` just filters that dir, it does NOT make `find` exit non-zero, so no
# `|| true` is needed; genuine `find` failures stay visible under `set -e`.
# `sh -c` (not a bare `{}/main.rs`) avoids POSIX implementation-defined `{}`
# substring substitution so discovery is portable across GNU/BSD find.
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

# Sequential loop retained (Decision 3, T2 plan): every reborn_group_*
# binary shares one libsql-backed group store; PR #5751 traced a real
# SIGABRT to concurrent access against that shape, and the fix required a
# 90-run soak (50 libsql + 40 in-memory, 0 flakes) to trust
# (tests/integration/group_approvals/main.rs:9-12). Each binary already
# runs its own scenarios sequentially by construction; this job's whole
# purpose is keeping DIFFERENT group binaries from running concurrently
# against each other, which is exactly what a nextest pool would do.
#
# Full-signal (R1): run every group binary before exiting non-zero.
failed=0
for test_name in "${test_names[@]}"; do
  echo "::group::cargo test --test ${test_name}"
  if ! timeout --signal=INT --kill-after=30s "${test_timeout}" \
    cargo test -p ironclaw_integration_tests --test "${test_name}" --no-fail-fast -- --nocapture; then
    echo "::error::group suite failed: ${test_name}"
    failed=1
  fi
  echo "::endgroup::"
done
exit "${failed}"
