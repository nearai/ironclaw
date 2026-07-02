---
description: Trace a data flow or bug through the IronClaw codebase end-to-end
allowed-tools: Read, Glob, Grep, Bash(cargo test:*), Bash(bash scripts/codebase-graph.sh:*)
argument-hint: <symptom or feature name>
model: sonnet
---

Trace the flow of `$ARGUMENTS` through the IronClaw codebase. Map every file and function involved, identify where data transforms or could break, and report the full chain.

## Step 0 — pick the stack

New features and almost all current work are **Reborn** (`crates/`). Trace v1 (`src/`) only when the symptom is explicitly in the legacy monolith (v1 gateway UI, TUI, engine-v2 bridge). If unsure: `grep -rn "<symptom>" crates/ --include='*.rs' -l | head` first, `src/` second. The legacy enclave (`ironclaw_engine`, `ironclaw_tui`, `ironclaw_gateway`, `ironclaw_oauth`, `ironclaw_embeddings`) is v1 despite living in `crates/`.

Discovery order: `bash scripts/codebase-graph.sh status` once — if the graph is FRESH and the codebase-memory MCP is connected, use `trace_path(mode="cross_service"|"data_flow")`; otherwise fall back to the anchors + recipes below without stalling.

## Reborn flow anchors (verify with the recipe beside each — do not trust this table blindly)

| Hop | Anchor | Re-derive with |
|---|---|---|
| Browser JS | `crates/ironclaw_webui_v2_static/static/js/lib/api.js` (`apiFetch`) + `static/js/pages/*/lib/*-api.js` | `grep -rn "apiFetch(" crates/ironclaw_webui_v2_static/static/js/pages` |
| Route + policy | `crates/ironclaw_webui_v2/src/descriptors.rs`, `router.rs`, `handlers.rs` | `grep -n "WEBUI_V2_PATTERN_\|_descriptor" crates/ironclaw_webui_v2/src/descriptors.rs` |
| Facade | `RebornServicesApi` in `crates/ironclaw_product_workflow/src/reborn_services.rs` | `grep -n "async fn <name>" crates/ironclaw_product_workflow/src/reborn_services.rs` |
| Port impl | `crates/ironclaw_reborn_composition/src/<feature>*.rs` | `grep -rln "impl <PortTrait>" crates/ironclaw_reborn_composition/src` |
| Turn accept | `SessionThreadService::accept_inbound_message` (`crates/ironclaw_threads`) → `TurnCoordinator::submit_turn` (`crates/ironclaw_turns/src/coordinator.rs`) | `grep -rn "submit_turn(" crates/ --include='*.rs' -l` |
| Claim + execute | `TurnRunScheduler` (`crates/ironclaw_host_runtime/src/turn_scheduler.rs`) → `RebornTurnRunExecutor` (`crates/ironclaw_reborn/src/turn_run_executor.rs`) | `grep -n "claim_next_run\|invoke_driver" crates/ironclaw_host_runtime/src/turn_scheduler.rs crates/ironclaw_reborn/src/turn_run_executor.rs` |
| Loop | `PlannedDriver` (`crates/ironclaw_reborn/src/planned_driver.rs`) → `CanonicalAgentLoopExecutor` (`crates/ironclaw_agent_loop/src/executor.rs`) → host ports (`crates/ironclaw_loop_support`) | `grep -rn "invoke_capability\|stream_model" crates/ironclaw_agent_loop/src/executor` |
| Model call | `crates/ironclaw_reborn/src/model_gateway.rs` → `ironclaw_llm` provider chain | `grep -n "complete_model_request\|CompletionRequest" crates/ironclaw_reborn/src/model_gateway.rs` |
| Effects | `CapabilityHost::invoke_json` (`crates/ironclaw_capabilities/src/host.rs`) → dispatcher → wasm/scripts/mcp/first-party lanes | `grep -n "invoke_json" crates/ironclaw_capabilities/src/host.rs` |
| Reply to browser | SSE projection drain: `stream_events` (`crates/ironclaw_webui_v2/src/handlers.rs`) over `ProjectionStream` | `grep -n "stream_events" crates/ironclaw_webui_v2/src/handlers.rs` |

(Old docs may say `TurnRunnerWorker` — that component was split into the scheduler + executor above in #5085.)

## v1 anchors (legacy maintenance only)

Message flow: `src/agent/agent_loop.rs` (`handle_message`, `run_agentic_loop`) → `crates/ironclaw_llm/src/reasoning.rs` (`respond_with_tools`) → `crates/ironclaw_llm/src/nearai_chat.rs`. Web/SSE: `src/channels/web/` (split into `handlers/`, `platform/`, `features/` — the old `server.rs` is gone) → `crates/ironclaw_gateway/static/js/core/sse.js`. Tools: `src/tools/registry.rs` → `src/agent/agent_loop.rs` `execute_chat_tool()` → `crates/ironclaw_safety/src/sanitizer.rs`. Engine v2 bridge: `src/bridge/` ↔ `crates/ironclaw_engine`.

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
