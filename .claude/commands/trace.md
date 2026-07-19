---
description: Trace a data flow or bug through the IronClaw codebase end-to-end
allowed-tools: Read, Glob, Grep, Bash(cargo test:*), Bash(bash scripts/codebase-graph.sh:*), mcp__codebase-memory__search_graph, mcp__codebase-memory__get_code_snippet, mcp__codebase-memory__trace_path, mcp__codebase-memory__get_architecture, mcp__codebase-memory__query_graph, mcp__codebase-memory__index_repository, mcp__codebase-memory__detect_changes
argument-hint: <symptom or feature name>
model: sonnet
---

Trace the flow of `$ARGUMENTS` through the IronClaw codebase. Map every file and function involved, identify where data transforms or could break, and report the full chain.

## Step 0 — probe the graph and pick the stack

Discovery order: `bash scripts/codebase-graph.sh status` once — if the graph is FRESH and the codebase-memory MCP is connected, use `trace_path(mode="cross_service"|"data_flow")`; otherwise fall back to the anchors + recipes below without stalling.

The supported runtime is Reborn under `crates/`. The root v1 monolith and its
legacy crates have been retired, so a trace that only resolves to one of their
old paths is stale documentation or history, not a live implementation path.

## Reborn flow anchors (verify with the recipe beside each — do not trust this table blindly)

| Hop | Anchor | Re-derive with |
|---|---|---|
| Browser UI | `crates/ironclaw_webui/frontend/src/` | `rg -n "apiFetch|EventSource|WebSocket" crates/ironclaw_webui/frontend/src` |
| Route + policy | `crates/ironclaw_webui/src/webui_v2/descriptors.rs`, `router.rs`, `handlers.rs` | `grep -n "WEBUI_V2_PATTERN_\|_descriptor" crates/ironclaw_webui/src/webui_v2/descriptors.rs` |
| Facade | `RebornServicesApi` in `crates/ironclaw_product_workflow/src/reborn_services.rs` | `grep -n "async fn <name>" crates/ironclaw_product_workflow/src/reborn_services.rs` |
| Port impl | `crates/ironclaw_reborn_composition/src/<feature>*.rs` | `grep -rn "impl <PortTrait>" crates/ironclaw_reborn_composition/src` |
| Turn accept | `SessionThreadService::accept_inbound_message` (`crates/ironclaw_threads`) → `TurnCoordinator::submit_turn` (`crates/ironclaw_turns/src/coordinator.rs`) | `grep -rn --include='*.rs' "submit_turn(" crates/` |
| Claim + execute | `TurnRunScheduler` → `RebornTurnRunExecutor` (`crates/ironclaw_runner/src/`) | `grep -n "claim_next_run\|invoke_driver" crates/ironclaw_runner/src/turn_scheduler.rs crates/ironclaw_runner/src/turn_run_executor.rs` |
| Loop | `PlannedDriver` (`crates/ironclaw_runner/src/planned_driver.rs`) → `CanonicalAgentLoopExecutor` (`crates/ironclaw_agent_loop/src/executor.rs`) → host ports (`crates/ironclaw_loop_host`) | `grep -rn "invoke_capability\|stream_model" crates/ironclaw_agent_loop/src/executor` |
| Model call | `crates/ironclaw_runner/src/model_gateway.rs` → `ironclaw_llm` provider chain | `grep -n "complete_model_request\|CompletionRequest" crates/ironclaw_runner/src/model_gateway.rs` |
| Effects | `CapabilityHost::invoke_json` (`crates/ironclaw_capabilities/src/host.rs`) → dispatcher → wasm/scripts/mcp/first-party lanes | `grep -n "invoke_json" crates/ironclaw_capabilities/src/host.rs` |
| Reply to browser | SSE projection drain: `stream_events` (`crates/ironclaw_webui/src/webui_v2/handlers.rs`) over `ProjectionStream` | `grep -n "stream_events" crates/ironclaw_webui/src/webui_v2/handlers.rs` |

## Tracing instructions

1. **Read** each file on the relevant path, focusing on the functions that handle the data.
2. **Identify transforms**: where does the data change shape? Name each conversion type.
3. **Identify failure points**: where could data be lost, malformed, misrouted, or blocked (gates, idempotency, policy, redaction)?
4. **Report the chain**: every file:line involved, what happens at each step, and where the issue (if any) is.

## Output format

1. **Flow path** — the specific chain of files and functions
2. **Data transforms** — how the data changes at each step
3. **Findings** — bugs, missing data, suspicious patterns
4. **Recommendation** — what to fix or investigate further
