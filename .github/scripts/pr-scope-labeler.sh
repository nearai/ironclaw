#!/usr/bin/env bash
# Apply additive scope labels to a PR based on changed files.
# Called by the pr-label-scope workflow.
#
# Inputs (env vars):
#   PR_NUMBER  — pull request number
#   REPO       — owner/repo (e.g. "user/ironclaw")
#
# Requires: gh CLI

set -euo pipefail

PR_NUMBER="${PR_NUMBER:?PR_NUMBER is required}"
REPO="${REPO:?REPO is required}"

declare -A wanted_labels=()
declare -A current_labels=()

mark_label() {
  wanted_labels["$1"]=1
}

while IFS= read -r label; do
  [[ -n "$label" ]] || continue
  current_labels["$label"]=1
done < <(gh pr view "$PR_NUMBER" --repo "$REPO" --json labels --jq '.labels[].name')

while IFS= read -r file; do
  [[ -n "$file" ]] || continue

  case "$file" in
    src/agent/*)
      mark_label "scope: agent"
      ;;
    src/channels/channel.rs|src/channels/manager.rs|src/channels/mod.rs)
      mark_label "scope: channel"
      ;;
    src/channels/cli/*|src/cli/*)
      mark_label "scope: channel/cli"
      ;;
    src/channels/web/*)
      mark_label "scope: channel/web"
      ;;
    src/channels/wasm/*)
      mark_label "scope: channel/wasm"
      ;;
    src/tools/tool.rs|src/tools/registry.rs|src/tools/mod.rs|src/tools/sandbox.rs)
      mark_label "scope: tool"
      ;;
    src/tools/builtin/*)
      mark_label "scope: tool/builtin"
      ;;
    src/tools/wasm/*)
      mark_label "scope: tool/wasm"
      ;;
    src/tools/mcp/*)
      mark_label "scope: tool/mcp"
      ;;
    src/tools/builder/*)
      mark_label "scope: tool/builder"
      ;;
    src/db/mod.rs)
      mark_label "scope: db"
      ;;
    src/db/postgres.rs|migrations/*)
      mark_label "scope: db/postgres"
      ;;
    src/db/libsql_backend.rs|src/db/libsql_migrations.rs)
      mark_label "scope: db/libsql"
      ;;
    src/safety/*)
      mark_label "scope: safety"
      ;;
    src/llm/*)
      mark_label "scope: llm"
      ;;
    src/workspace/*)
      mark_label "scope: workspace"
      ;;
    src/orchestrator/*)
      mark_label "scope: orchestrator"
      ;;
    src/worker/*)
      mark_label "scope: worker"
      ;;
    src/secrets/*)
      mark_label "scope: secrets"
      ;;
    src/config.rs|src/settings.rs)
      mark_label "scope: config"
      ;;
    src/extensions/*)
      mark_label "scope: extensions"
      ;;
    src/setup/*)
      mark_label "scope: setup"
      ;;
    src/evaluation/*)
      mark_label "scope: evaluation"
      ;;
    src/estimation/*)
      mark_label "scope: estimation"
      ;;
    src/sandbox/*|Dockerfile*)
      mark_label "scope: sandbox"
      ;;
    src/hooks/*)
      mark_label "scope: hooks"
      ;;
    src/pairing/*)
      mark_label "scope: pairing"
      ;;
    .github/workflows/*|.github/scripts/*)
      mark_label "scope: ci"
      ;;
    docs/*|*.md|LICENSE*)
      mark_label "scope: docs"
      ;;
    Cargo.toml|Cargo.lock)
      mark_label "scope: dependencies"
      ;;
  esac
done < <(gh api "repos/${REPO}/pulls/${PR_NUMBER}/files" --paginate --jq '.[].filename')

if [[ "${#wanted_labels[@]}" -eq 0 ]]; then
  echo "No scope labels matched."
  exit 0
fi

for label in "${!wanted_labels[@]}"; do
  if [[ -n "${current_labels[$label]:-}" ]]; then
    echo "Scope label already present: ${label}"
    continue
  fi
  echo "Adding scope label: ${label}"
  gh api "repos/${REPO}/issues/${PR_NUMBER}/labels" -X POST -f "labels[]=${label}" >/dev/null
done

echo "Done."
