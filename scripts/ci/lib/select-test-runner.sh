#!/usr/bin/env bash
#
# Shared by scripts/ci/quality_gate.sh (local dev) and every CI-facing
# runner script (scripts/ci/run-reborn-root-partition.sh,
# scripts/ci/reborn-coverage-lane-run.sh, and
# scripts/ci/run-hermetic-deterministic-suite.sh's run_crate_tests /
# run_integration_tier). ONE selection seam — a prior draft of this plan
# proposed a second, parallel implementation for CI; that was a fork, not
# an extension, and is not what this file is.
#
# select_test_runner <policy>
#   policy = optional      -- local contract: nextest when installed,
#                              cargo test otherwise, regardless of CI.
#   policy = require-in-ci -- CI runner scripts' contract: nextest when
#                              installed; when absent AND CI=true, a hard,
#                              loud failure (the job installed nextest, so
#                              absence means the job is broken -- never a
#                              silent 3x-slower fallback); when absent and
#                              CI is unset (local reproduction), the same
#                              warn + fallback as "optional".
#   IRONCLAW_GATE_TEST_RUNNER (both policies): cargo | nextest | auto
#     (default). An explicit "nextest" always hard-fails if the binary is
#     missing, in EITHER policy. An explicit "cargo" always forces the
#     sequential runner -- the bisect knob: force cargo even inside CI to
#     isolate a nextest-only failure.
select_test_runner() {
  local policy="${1:?select_test_runner requires a policy: optional|require-in-ci}"
  case "${policy}" in
    optional|require-in-ci) ;;
    *)
      echo "select_test_runner: unknown policy '${policy}' (expected optional|require-in-ci)" >&2
      return 1
      ;;
  esac

  case "${IRONCLAW_GATE_TEST_RUNNER:-auto}" in
    cargo) echo "cargo" ;;
    nextest)
      if command -v cargo-nextest >/dev/null 2>&1; then
        echo "nextest"
      else
        echo "IRONCLAW_GATE_TEST_RUNNER=nextest requires cargo-nextest" >&2
        return 1
      fi
      ;;
    auto)
      if command -v cargo-nextest >/dev/null 2>&1; then
        echo "nextest"
      elif [[ "${policy}" == "require-in-ci" && "${CI:-}" == "true" ]]; then
        echo "cargo-nextest is required in CI but was not found on PATH" >&2
        return 1
      else
        echo "cargo"
      fi
      ;;
    *)
      echo "unknown IRONCLAW_GATE_TEST_RUNNER: ${IRONCLAW_GATE_TEST_RUNNER}" >&2
      return 1
      ;;
  esac
}
