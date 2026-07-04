#!/usr/bin/env bash
set -euo pipefail

# Reborn shared-persistence "group" suites are subdirectory `[[test]]` binaries
# (tests/integration/group_*/main.rs, `[[test]]` name reborn_group_<x>). Unlike
# the single-file reborn_integration_*.rs suites, each spins up one runtime and
# drives several tenants' shared libsql-backed stores across threads, so they
# run in a dedicated low-contention job instead of the modulo-partitioned
# integration runner. `--features libsql` is explicit so the group binaries
# exercise the libsql-backed shared store independently of any future change to
# the default feature set.

test_timeout="${REBORN_GROUP_TEST_TIMEOUT:-28m}"

run_focused_test() {
  local description="$1"
  shift

  echo "::group::${description}"
  timeout --signal=INT --kill-after=30s "${test_timeout}" "$@"
  echo "::endgroup::"
}

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

for test_name in "${test_names[@]}"; do
  run_focused_test "cargo test --test ${test_name} --features libsql" \
    cargo test --test "${test_name}" --features libsql -- --nocapture
done

# Keep libSQL persistence coverage after the broad crate buckets drop their
# libSQL feature flags. These are the feature-gated tests that otherwise stop
# running when `ironclaw_host_runtime` and `ironclaw_reborn` avoid the libSQL
# compile graph in their all-targets buckets.
run_focused_test \
  "cargo test -p ironclaw_host_runtime --features test-support,libsql --test first_party_builtin_tools builtin_coding_blocks_sensitive_resolved_libsql_paths" \
  cargo test -p ironclaw_host_runtime --features test-support,libsql \
    --test first_party_builtin_tools \
    builtin_coding_blocks_sensitive_resolved_libsql_paths \
    -- --nocapture

for test_name in \
  production_root_filesystem_selection_accepts_libsql_root_filesystem \
  production_turn_state_selection_accepts_filesystem_turn_state_store \
  production_turn_coordinator_uses_configured_store_and_notifier \
  production_turn_coordinator_requires_explicit_run_profile_resolver \
  host_runtime_services_preserves_combined_store_after_root_filesystem_selection
do
  run_focused_test \
    "cargo test -p ironclaw_host_runtime --features test-support,libsql --test host_runtime_services_contract ${test_name}" \
    cargo test -p ironclaw_host_runtime --features test-support,libsql \
      --test host_runtime_services_contract \
      "${test_name}" \
      -- --nocapture
done

run_focused_test \
  "cargo test -p ironclaw_host_runtime --features test-support,libsql --test reborn_durable_restart_integration approval_resume_survives_durable_libsql_reopen_and_consumes_lease_once" \
  cargo test -p ironclaw_host_runtime --features test-support,libsql \
    --test reborn_durable_restart_integration \
    approval_resume_survives_durable_libsql_reopen_and_consumes_lease_once \
    -- --nocapture

run_focused_test \
  "cargo test -p ironclaw_reborn --features libsql-secrets --test secrets" \
  cargo test -p ironclaw_reborn --features libsql-secrets \
    --test secrets \
    -- --nocapture

run_focused_test \
  "cargo test -p ironclaw_reborn --features libsql-restart-tests --test loop_driver_host turn_runner_worker_completes_after_libsql_turn_and_thread_services_reopen" \
  cargo test -p ironclaw_reborn --features libsql-restart-tests \
    --test loop_driver_host \
    turn_runner_worker_completes_after_libsql_turn_and_thread_services_reopen \
    -- --nocapture
