#!/usr/bin/env bash
#
# codebase-graph.sh — freshness status for the codebase-memory knowledge graph.
#
# The graph (.codebase-memory/graph.db.zst) is a git-ignored build artifact indexed
# by the codebase-memory MCP server. This script only INSPECTS freshness; the actual
# (re)indexing is done through the MCP tools invoked by an agent:
#   - build:        index_repository(repo_path=".")
#   - delta/impact: detect_changes(since="<indexed-commit>")
#
# Usage: bash scripts/codebase-graph.sh [status]
#
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
artifact="$repo_root/.codebase-memory/artifact.json"

read_field() { python3 -c "import json,sys;print(json.load(open('$artifact')).get('$1','?'))"; }

status() {
  if [ ! -f "$artifact" ]; then
    echo "graph:   MISSING (no .codebase-memory/artifact.json)"
    echo "action:  build it once — call index_repository(repo_path=\".\") via the codebase-memory MCP"
    return 2
  fi

  local indexed head nodes edges when
  indexed="$(read_field commit)"
  nodes="$(read_field nodes)"
  edges="$(read_field edges)"
  when="$(read_field indexed_at)"
  head="$(git rev-parse HEAD)"

  echo "graph:   indexed @ ${indexed:0:9}  (${nodes} nodes / ${edges} edges, ${when})"
  echo "HEAD:    ${head:0:9}"

  if [ "$indexed" = "$head" ]; then
    echo "status:  FRESH (matches HEAD)"
    return 0
  fi

  if git merge-base --is-ancestor "$indexed" HEAD 2>/dev/null; then
    local n
    n="$(git rev-list --count "$indexed"..HEAD 2>/dev/null || echo '?')"
    echo "status:  STALE — ${n} commit(s) behind HEAD"
    echo "action:  delta — detect_changes(since=\"$indexed\")   (changed symbols + blast radius)"
    echo "         or full refresh — index_repository(repo_path=\".\")"
    return 1
  fi

  echo "status:  DIVERGED — indexed commit is not in current history (rebase/force-push?)"
  echo "action:  full re-index — index_repository(repo_path=\".\")"
  return 1
}

case "${1:-status}" in
  status) status ;;
  *) echo "usage: $0 [status]" >&2; exit 64 ;;
esac
