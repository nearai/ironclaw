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
    "adapters-misc"
  ];

  def bucket_map:
    {
      ironclaw_host_runtime: "host-runtime",

      ironclaw_agent_loop: "agent-runtime",
      ironclaw_approvals: "agent-runtime",
      ironclaw_capabilities: "agent-runtime",
      ironclaw_dispatcher: "agent-runtime",
      ironclaw_host_api: "agent-runtime",
      ironclaw_loop_host: "agent-runtime",

      ironclaw_runner: "reborn-core",
      ironclaw_reborn_cli: "reborn-core",
      ironclaw_reborn_config: "reborn-core",
      ironclaw_reborn_event_store: "reborn-core",
      ironclaw_reborn_identity: "reborn-core",
      ironclaw_reborn_openai_compat: "reborn-core",

      ironclaw_reborn_composition: "composition-core",

      ironclaw_product_adapter_registry: "product-workflow",
      ironclaw_product_adapters: "product-workflow",
      ironclaw_product_context: "product-workflow",
      ironclaw_product_workflow: "product-workflow",

      ironclaw_attachments: "webui-ingress",
      ironclaw_projects: "webui-ingress",
      ironclaw_webui: "webui-ingress",
      ironclaw_resources: "webui-ingress",

      ironclaw_first_party_extension_ports: "wasm-sandbox",
      ironclaw_first_party_extensions: "wasm-sandbox",
      ironclaw_wasm: "wasm-sandbox",
      ironclaw_wasm_limiter: "wasm-sandbox",
      ironclaw_wasm_product_adapters: "wasm-sandbox",
      ironclaw_wasm_sandbox_core: "wasm-sandbox",

      ironclaw_filesystem: "llm-mcp",
      ironclaw_llm: "llm-mcp",
      ironclaw_mcp: "llm-mcp",
      ironclaw_network: "llm-mcp",
      ironclaw_outbound: "llm-mcp",
      ironclaw_process_sandbox: "llm-mcp",
      ironclaw_processes: "llm-mcp",

      ironclaw_conversations: "events-conversations",
      ironclaw_event_projections: "events-conversations",
      ironclaw_event_streams: "events-conversations",
      ironclaw_events: "events-conversations",
      ironclaw_prompt_envelope: "events-conversations",
      ironclaw_run_state: "events-conversations",
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
      ironclaw_observability: "memory-skills",
      ironclaw_scripts: "memory-skills",
      ironclaw_skill_learning: "memory-skills",
      ironclaw_skills: "memory-skills",

      ironclaw_architecture: "adapters-misc",
      ironclaw_common: "adapters-misc",
      ironclaw_extensions: "adapters-misc",
      ironclaw_reborn_traces: "adapters-misc",
      ironclaw_slack_v2_adapter: "adapters-misc",
      ironclaw_telegram_v2_adapter: "adapters-misc"
    };

  bucket_map as $bucket_map
  | [
    bucket_order[]? as $bucket
    | {
        name: $bucket,
        packages: [
          $packages[]?
          | select(($bucket_map[.] // "adapters-misc") == $bucket)
        ]
      }
    | select(.packages | length > 0)
  ]
'
