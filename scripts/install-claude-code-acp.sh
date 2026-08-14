#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd "${script_dir}/.." && pwd)"
pin_file="${repository_root}/Dockerfile.claude-code-acp"

read_pin() {
  local name="$1"
  sed -n "s/^ARG ${name}=//p" "${pin_file}"
}

claude_code_version="$(read_pin CLAUDE_CODE_VERSION)"
adapter_version="$(read_pin CLAUDE_AGENT_ACP_VERSION)"
if [[ -z "${claude_code_version}" || -z "${adapter_version}" ]]; then
  echo "could not read Claude Code ACP pins from ${pin_file}" >&2
  exit 1
fi

npm install --global \
  "@anthropic-ai/claude-code@${claude_code_version}" \
  "@agentclientprotocol/claude-agent-acp@${adapter_version}"

claude --version
command -v claude-agent-acp > /dev/null
echo "installed claude-agent-acp ${adapter_version}"
