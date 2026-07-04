#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="${REPO:-nearai/ironclaw}"
base_branch="${BASE_BRANCH:-main}"
candidate_branch="${CANDIDATE_BRANCH:-}"
candidate_args=()
if [ -n "$candidate_branch" ]; then
  candidate_args=(--candidate-branch "$candidate_branch")
fi

echo "# CI build-time probe"
echo
echo "Primary workflow set"
WORKFLOWS="${DEV_WORKFLOWS:-reborn-tests.yml,reborn-e2e.yml,reborn-coverage.yml,test.yml}" \
  "$script_dir/score.sh" --repo "$repo" --base-branch "$base_branch" "${candidate_args[@]}"

echo
echo "Probe workflow set"
if [ -n "${PROBE_BASE_RUN_IDS:-}" ] || [ -n "${PROBE_CANDIDATE_RUN_IDS:-}" ]; then
  BASE_RUN_IDS="${PROBE_BASE_RUN_IDS:-}" \
  CANDIDATE_RUN_IDS="${PROBE_CANDIDATE_RUN_IDS:-}" \
    "$script_dir/score.sh" --repo "$repo" --base-branch "$base_branch" "${candidate_args[@]}"
else
  echo "No probe run IDs supplied."
  echo "Set PROBE_BASE_RUN_IDS and PROBE_CANDIDATE_RUN_IDS for live-canary/coverage/reborn-playwright holdout checks."
fi
