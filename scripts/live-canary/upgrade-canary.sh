#!/usr/bin/env bash
set -euo pipefail

# Verify that a database created by the previous release can be opened and used
# by the current checkout. This is a pre-release/manual lane, not a PR gate.

PREVIOUS_REF="${PREVIOUS_REF:-}"
CURRENT_REF="${CURRENT_REF:-HEAD}"
WORK_ROOT="${WORK_ROOT:-${TMPDIR:-/tmp}/ironclaw-upgrade-canary}"
PREVIOUS_DIR="${WORK_ROOT}/previous"
CURRENT_DIR="${WORK_ROOT}/current"
DB_PATH="${DB_PATH:-${WORK_ROOT}/upgrade-canary.db}"

if [[ -z "${PREVIOUS_REF}" ]]; then
  PREVIOUS_REF="$(git describe --tags --abbrev=0 2>/dev/null || true)"
fi

if [[ -z "${PREVIOUS_REF}" ]]; then
  echo "PREVIOUS_REF is required when no tag can be auto-detected." >&2
  exit 2
fi

cleanup() {
  git worktree remove --force "${PREVIOUS_DIR}" >/dev/null 2>&1 || true
  git worktree remove --force "${CURRENT_DIR}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

mkdir -p "${WORK_ROOT}"
rm -f "${DB_PATH}"

echo "[upgrade-canary] previous=${PREVIOUS_REF} current=${CURRENT_REF} db=${DB_PATH}"

git worktree add --detach "${PREVIOUS_DIR}" "${PREVIOUS_REF}"
git worktree add --detach "${CURRENT_DIR}" "${CURRENT_REF}"

common_env=(
  "DATABASE_BACKEND=libsql"
  "LIBSQL_PATH=${DB_PATH}"
  "ONBOARD_COMPLETED=true"
  "LLM_BACKEND=openai_compatible"
  "LLM_BASE_URL=http://127.0.0.1:9/v1"
  "LLM_MODEL=upgrade-canary-placeholder"
  "LLM_API_KEY=upgrade-canary-placeholder"
  "RUST_LOG=ironclaw=info"
)

echo "[upgrade-canary] building previous release"
(
  cd "${PREVIOUS_DIR}"
  cargo build --no-default-features --features libsql
  env "${common_env[@]}" cargo test --features libsql --test config_round_trip -- --nocapture
)

echo "[upgrade-canary] building current checkout"
(
  cd "${CURRENT_DIR}"
  cargo build --no-default-features --features libsql
  env "${common_env[@]}" cargo test --features libsql --test config_round_trip -- --nocapture
  env "${common_env[@]}" cargo test --features libsql --test workspace_integration -- --nocapture
)

echo "[upgrade-canary] completed"
