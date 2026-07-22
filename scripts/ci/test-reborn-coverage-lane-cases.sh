#!/usr/bin/env bash
#
# E-section cases for reborn-coverage-lane-run.sh, sourced by
# test-reborn-coverage.sh so these caller-path checks can reuse its capture and
# assertion helpers. Running this file directly does nothing useful.
#
# shellcheck disable=SC2154 # lane_sh/tmp_root/PATH and the assertion helpers
# are assigned by the sourcing parent.

# E1: every discovered non-group target reaches cargo, even when its manifest
# name does not follow the historical reborn_integration_* convention. The
# fake timeout records the production caller's complete command instead of
# launching cargo llvm-cov.
e1="${tmp_root}/e1"
e1_bin="${e1}/bin"
e1_scripts="${e1}/scripts/ci"
e1_timeout_log="${e1}/timeout.log"
mkdir -p "${e1_bin}" "${e1_scripts}"
cp "${lane_sh}" "${e1_scripts}/reborn-coverage-lane-run.sh"
chmod +x "${e1_scripts}/reborn-coverage-lane-run.sh"

cat > "${e1_scripts}/reborn-coverage-int-tier-tests.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' \
  --test reborn_group_bar \
  --test manifest_named_target \
  --test reborn_integration_alpha
EOF
chmod +x "${e1_scripts}/reborn-coverage-int-tier-tests.sh"

cat > "${e1_bin}/timeout" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
: "${FAKE_TIMEOUT_LOG:?FAKE_TIMEOUT_LOG must be set}"
printf '%s\n' "$@" > "${FAKE_TIMEOUT_LOG}"
EOF
chmod +x "${e1_bin}/timeout"

capture env \
  PATH="${e1_bin}:${PATH}" \
  FAKE_TIMEOUT_LOG="${e1_timeout_log}" \
  REBORN_COV_LANE_MODE=flat-partition \
  REBORN_COV_LANE_PARTITIONS=1 \
  REBORN_COV_LANE_INDEX=0 \
  "${e1_scripts}/reborn-coverage-lane-run.sh" "${e1}/part-0.lcov"
assert_exit_code "E1: non-group lane execution exits 0" 0 "${CAP_RC}"

e1_timeout_args="$(cat "${e1_timeout_log}")"
assert_contains "E1: arbitrary manifest target reaches cargo llvm-cov" \
  "${e1_timeout_args}" "$(printf -- '--test\nmanifest_named_target')"
assert_contains "E1: conventional integration target still reaches cargo llvm-cov" \
  "${e1_timeout_args}" "$(printf -- '--test\nreborn_integration_alpha')"
assert_not_contains "E1: group target stays out of the non-group lane" \
  "${e1_timeout_args}" "reborn_group_bar"
