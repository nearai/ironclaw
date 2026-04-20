---
paths:
  - "src/agent/**"
  - "src/tools/**"
  - "crates/ironclaw_engine/**"
---
# Agent Claims and Evidence

The most dangerous user-visible bug class in IronClaw is **claim/evidence drift**: the agent's final text says "message sent" / "file attached" / "tool installed" with no corresponding side effect. LLM output is narration, not proof.

## Side-Effect Claims Must Cite Tool Evidence

When the user request implies a side effect (send, save, install, schedule, post, write, delete), the agent's final turn must either:

1. Reference a completed tool call whose `ToolOutput` contains a provider-issued identifier (`message_id`, `bytes_written`, `external_id`, `job_id`, `created_at`), **or**
2. Explicitly state that the action was not performed and why.

Engine v2 detects side-effect intent on the user turn and must reject a model-final turn that has no matching successful tool call. Plain-text "I've sent your message" without a tool call is a failure mode to flag, not behavior to trust. References: #2544, #2580, #2582, #2541, #2447.

## Empty-Fast Tool Outputs Are Errors

A tool that completes in `< 1ms` *and* returns empty content is almost always a silent failure. The dispatcher treats `duration < 1ms && content.is_empty()` as `ToolError::EmptyResult` unless the tool explicitly opts in via a sentinel (idempotent ack, documented no-op confirmation).

`ActionRecord` must always capture byte count and timing so this check is auditable at review time. The UI must suppress the success checkmark when output bytes are zero. Reference: #2545.

## External-Effect Tools Must Read Back

Tools whose side effect is visible only to an external system (Telegram send, Slack post, file write, extension install, OAuth completion) MUST read back the effect before returning success:

- `telegram_send` → capture and return `message_id` from the API response; error if the response lacks one.
- `file_write` → re-stat and return the actual byte count; error on mismatch.
- `extension_install` → call `extensions_list` and assert the new extension is present and active.
- OAuth completion → perform a minimal authenticated read against the provider before declaring success.

A tool without a read-back path is claim-only and must mark its output `unverified: true` so downstream layers can warn. References: #2411 Telegram token Save, #2543 Linear MCP OAuth, #2586 Slack Install.

## Setup UI Actions Round-Trip

Save / Install / Connect buttons in the setup UI must issue a read-back verification immediately after the write succeeds and render the read-back value (or explicit error) to the user — not a local optimistic checkmark. Install actions must dispatch through `ToolDispatcher::dispatch` and surface the resulting `ActionRecord`. A UI success state with no corresponding backend read-back is the same bug class as agent claim drift. References: #2411, #2534, #2543, #2586.
