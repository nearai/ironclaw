---
description: Trace a data flow or bug through the IronClaw codebase end-to-end
allowed-tools: Read, Glob, Grep, Bash(cargo test:*), Bash(bash scripts/codebase-graph.sh:*), mcp__codebase-memory-mcp__search_graph, mcp__codebase-memory-mcp__get_code_snippet, mcp__codebase-memory-mcp__trace_path, mcp__codebase-memory-mcp__get_architecture, mcp__codebase-memory-mcp__query_graph, mcp__codebase-memory-mcp__index_repository, mcp__codebase-memory-mcp__detect_changes
argument-hint: <symptom or feature name>
model: sonnet
---

Trace the flow of `$ARGUMENTS` through the IronClaw codebase. Map every file and function involved, identify where data transforms or could break, and report the full chain.

## Step 0 — probe the graph and pick the stack

Discovery order: `bash scripts/codebase-graph.sh status` once — if the graph is FRESH and the codebase-memory MCP is connected, use `trace_path(mode="cross_service"|"data_flow")`; otherwise fall back to the anchors + recipes below without stalling.

Everything is **Reborn** (`crates/`) — the v1 `src/` monolith and its crates (`ironclaw_engine`, `ironclaw_tui`, `ironclaw_gateway`, `ironclaw_oauth`) have been deleted, so there is no second stack to disambiguate against. If the graph is missing/stale/unavailable: `grep -rn --include='*.rs' "<symptom>" crates/ | head`.

## Reborn flow anchors (verify with the recipe beside each — do not trust this table blindly)

The canonical prose flow-map lives in `.claude/skills/ironclaw-reborn-orientation/SKILL.md` — read
that first for the one-paragraph version. The table below is this command's own value-add: a
per-hop, independently re-derivable grep recipe for tracing a specific symptom through each stage,
not a second copy of the map. If the two ever disagree, the orientation skill is canonical; fix this
table to match it rather than the other way around.

| Hop | Anchor | Re-derive with |
|---|---|---|
| Browser JS | `crates/product/ironclaw_webui/frontend/src` API client and page modules | `rg -n "apiFetch\(" crates/product/ironclaw_webui/frontend/src` |
| Route + policy | `crates/product/ironclaw_webui/src/webui_v2/descriptors.rs`, `router.rs`, `handlers.rs` | `grep -n "WEBUI_V2_PATTERN_\|_descriptor" crates/product/ironclaw_webui/src/webui_v2/descriptors.rs` |
| Product surface | `ProductSurface` in `ironclaw_product_contracts` (`crates/contracts/ironclaw_product_contracts/src/surface.rs`), descriptors in `crates/product/ironclaw_assistant/src/reborn_services.rs` | `rg -n "ProductSurface|ProductView|ProductSurfaceCommandDescriptor" crates/contracts/ironclaw_product_contracts crates/product/ironclaw_assistant` |
| Composition | `crates/app/ironclaw_composition/src` | `rg -n "build_.*service|impl .*Service" crates/app/ironclaw_composition/src` |
| Turn accept | `SessionThreadService::accept_inbound_message` (`crates/domains/ironclaw_threads`) → `TurnCoordinator::submit_turn` (`crates/kernel/ironclaw_turns/src/coordinator.rs`) | `grep -rn --include='*.rs' "submit_turn(" crates/` |
| Claim + execute | `TurnRunScheduler` → `RebornTurnRunExecutor` (`crates/loop/ironclaw_turn_runner/src/`) | `grep -n "claim_next_run\|invoke_driver" crates/loop/ironclaw_turn_runner/src/turn_scheduler.rs crates/loop/ironclaw_turn_runner/src/turn_run_executor.rs` |
| Loop | `PlannedDriver` (`crates/loop/ironclaw_turn_runner/src/planned_driver.rs`) → `CanonicalAgentLoopExecutor` (`crates/loop/ironclaw_agent_loop/src/executor.rs`) → host ports (`crates/loop/ironclaw_loop_host`) | `grep -rn "invoke_capability\|stream_model" crates/loop/ironclaw_agent_loop/src/executor` |
| Model call | `crates/loop/ironclaw_loop_host/src/model_gateway.rs` → `ironclaw_llm` provider chain | `grep -n "complete_model_request\|CompletionRequest" crates/loop/ironclaw_loop_host/src/model_gateway.rs` |
| Effects | `CapabilityHost::invoke_json` (`crates/kernel/ironclaw_capabilities/src/host/`) → dispatcher → wasm / script-sandbox / mcp / first-party lanes | `grep -n "invoke_json" crates/kernel/ironclaw_capabilities/src/host/invoke.rs` |
| Reply to browser | SSE projection drain: `stream_events` (`crates/product/ironclaw_webui/src/webui_v2/handlers.rs`) over `ProjectionStream` | `grep -n "stream_events" crates/product/ironclaw_webui/src/webui_v2/handlers.rs` |

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
