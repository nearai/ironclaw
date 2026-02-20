#!/usr/bin/env bash
# Idempotent label bootstrap for IronClaw PR automation.
# Uses `gh label create --force` so it can be re-run safely.
#
# Usage: bash .github/scripts/create-labels.sh
# Requires: gh CLI authenticated with repo scope

set -euo pipefail

if ! command -v gh &>/dev/null; then
  echo "Error: gh CLI is required. Install from https://cli.github.com" >&2
  exit 1
fi

create() {
  local name="$1" color="$2" description="$3"
  gh label create "$name" --color "$color" --description "$description" --force
}

echo "==> Creating size labels..."
create "size: XS"    "F9D0C4" "< 10 changed lines (excluding docs)"
create "size: S"     "F5A3A3" "10-49 changed lines"
create "size: M"     "E57373" "50-199 changed lines"
create "size: L"     "D32F2F" "200-499 changed lines"
create "size: XL"    "B71C1C" "500+ changed lines"

echo "==> Creating risk labels..."
create "risk: low"    "4CAF50" "Changes to docs, tests, or low-risk modules"
create "risk: medium" "FFC107" "Business logic, config, or moderate-risk modules"
create "risk: high"   "F44336" "Safety, secrets, auth, or critical infrastructure"
create "risk: manual" "9E9E9E" "Risk level set manually (sticky, not overwritten)"

echo "==> Creating scope labels..."
create "scope: agent"         "006B75" "Agent core (agent loop, router, scheduler)"
create "scope: channel"       "00838F" "Channel infrastructure"
create "scope: channel/cli"   "00897B" "TUI / CLI channel"
create "scope: channel/web"   "00796B" "Web gateway channel"
create "scope: channel/wasm"  "00695C" "WASM channel runtime"
create "scope: tool"          "1565C0" "Tool infrastructure"
create "scope: tool/builtin"  "1976D2" "Built-in tools"
create "scope: tool/wasm"     "1E88E5" "WASM tool sandbox"
create "scope: tool/mcp"      "2196F3" "MCP client"
create "scope: tool/builder"  "42A5F5" "Dynamic tool builder"
create "scope: db"            "4A148C" "Database trait / abstraction"
create "scope: db/postgres"   "6A1B9A" "PostgreSQL backend"
create "scope: db/libsql"     "7B1FA2" "libSQL / Turso backend"
create "scope: safety"        "880E4F" "Prompt injection defense"
create "scope: llm"           "4527A0" "LLM integration"
create "scope: workspace"     "283593" "Persistent memory / workspace"
create "scope: orchestrator"  "0D47A1" "Container orchestrator"
create "scope: worker"        "01579B" "Container worker"
create "scope: secrets"       "BF360C" "Secrets management"
create "scope: config"        "E65100" "Configuration"
create "scope: extensions"    "33691E" "Extension management"
create "scope: setup"         "827717" "Onboarding / setup"
create "scope: evaluation"    "558B2F" "Success evaluation"
create "scope: estimation"    "9E9D24" "Cost/time estimation"
create "scope: sandbox"       "00BFA5" "Docker sandbox"
create "scope: hooks"         "6D4C41" "Git/event hooks"
create "scope: pairing"       "4E342E" "Pairing mode"
create "scope: ci"            "546E7A" "CI/CD workflows"
create "scope: docs"          "78909C" "Documentation"
create "scope: dependencies"  "90A4AE" "Dependency updates"

echo "==> Creating contributor labels..."
create "contributor: new"         "FFF9C4" "First-time contributor"
create "contributor: regular"     "FFE082" "2-5 merged PRs"
create "contributor: experienced" "FFB74D" "6-19 merged PRs"
create "contributor: core"        "FF8A65" "20+ merged PRs"

echo "Done. All labels created/updated."
