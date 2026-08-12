#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <packages-json-array>" >&2
  exit 2
fi

packages_json="$1"

if ! jq -e 'type == "array" and all(.[]?; type == "string")' >/dev/null 2>&1 <<< "${packages_json}"; then
  echo "error: input must be a JSON array of package-name strings" >&2
  exit 1
fi

jq -c -n --argjson packages "${packages_json}" '
  def bucket_order: [
    "host-runtime",
    "agent-runtime",
    "reborn-core",
    "composition-core",
    "product-workflow",
    "webui-ingress",
    "wasm-sandbox",
    "llm-mcp",
    "events-conversations",
    "auth-security",
    "memory-skills",
    "channel-adapters",
    "extension-operator",
    "architecture-misc"
  ];

  def bucket_map:
    {
      ironclaw_host_runtime: "host-runtime",

      ironclaw_agent_loop: "agent-runtime",
      ironclaw_approvals: "agent-runtime",
      ironclaw_capabilities: "agent-runtime",
      ironclaw_host_api: "agent-runtime",
      ironclaw_loop_contracts: "agent-runtime",
      ironclaw_loop_host: "agent-runtime",

      ironclaw_turn_runner: "reborn-core",
      ironclaw: "reborn-core",
      ironclaw_config: "reborn-core",
      ironclaw_event_store: "reborn-core",
      ironclaw_identity: "reborn-core",
      ironclaw_openai_compat: "reborn-core",

      ironclaw_composition: "composition-core",

      ironclaw_assistant: "product-workflow",
      # The product-tier contract crate rides with the product workflow it
      # describes: a change to the membrane or its DTOs breaks product first.
      # Buckets group by what a change can break, not by layer.
      ironclaw_product_contracts: "product-workflow",

      ironclaw_attachments: "webui-ingress",
      ironclaw_webui: "webui-ingress",
      ironclaw_resources: "webui-ingress",

      ironclaw_extension_support: "wasm-sandbox",
      ironclaw_wasm: "wasm-sandbox",
      ironclaw_wasm_limiter: "wasm-sandbox",
      ironclaw_wasm_sandbox_core: "wasm-sandbox",

      ironclaw_filesystem: "llm-mcp",
      ironclaw_llm: "llm-mcp",
      ironclaw_mcp: "llm-mcp",
      ironclaw_network: "llm-mcp",
      ironclaw_outbound: "llm-mcp",
      ironclaw_sandbox: "llm-mcp",
      ironclaw_processes: "llm-mcp",

      ironclaw_conversations: "events-conversations",
      ironclaw_event_projections: "events-conversations",
      ironclaw_event_streams: "events-conversations",
      ironclaw_event_log: "events-conversations",
      ironclaw_prompt_envelope: "events-conversations",
      ironclaw_threads: "events-conversations",
      ironclaw_turns: "events-conversations",

      ironclaw_auth: "auth-security",
      ironclaw_authorization: "auth-security",
      ironclaw_hooks: "auth-security",
      ironclaw_runtime_policy: "auth-security",
      ironclaw_safety: "auth-security",
      ironclaw_secrets: "auth-security",
      ironclaw_trust: "auth-security",

      ironclaw_extractors: "memory-skills",
      ironclaw_memory: "memory-skills",
      ironclaw_memory_native: "memory-skills",
      ironclaw_memory_mem0: "memory-skills",
      ironclaw_observability: "memory-skills",
      ironclaw_skill_learning: "memory-skills",
      ironclaw_skills: "memory-skills",

      ironclaw_host_ingress: "channel-adapters",
      ironclaw_slack_extension: "channel-adapters",
      ironclaw_telegram_extension: "channel-adapters",

      ironclaw_extension_contracts: "extension-operator",
      ironclaw_extension_host: "extension-operator",
      ironclaw_extension_manager: "extension-operator",
      ironclaw_extension_registry: "extension-operator",
      ironclaw_operator: "extension-operator",

      ironclaw_architecture_tests: "architecture-misc",
      ironclaw_common: "architecture-misc",
      ironclaw_libsql_runtime: "architecture-misc",
      ironclaw_trace_commons: "architecture-misc",
      ironclaw_triggers: "architecture-misc"
    };

  bucket_map as $bucket_map
  | [
    bucket_order[]? as $bucket
    | {
        name: $bucket,
        packages: [
          $packages[]?
          | select(($bucket_map[.] // "architecture-misc") == $bucket)
        ]
      }
    | select(.packages | length > 0)
  ]
'
