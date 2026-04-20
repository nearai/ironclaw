---
paths:
  - "src/agent/**"
  - "src/tools/**"
  - "src/channels/web/**"
  - "crates/ironclaw_engine/**"
---
# Tool Evidence and Side-Effect Verification

The most dangerous user-visible bug class is **claim/evidence drift**: the agent narrates "message sent" / "file attached" / "tool installed" with no corresponding side effect. The agent-facing half of this rule lives in `crates/ironclaw_engine/prompts/codeact_postamble.md` ("Evidence before claiming side effects"). The code invariants that make the rule enforceable live here.

## Engine v2 Side-Effect Gate

Engine v2 classifies user turns for side-effect intent (send / save / install / schedule / post / write / delete). A model-final turn that lacks at least one successful tool call matching the intent must be rejected before it reaches the user — surface "action not performed" instead of the agent's narration. Reference: #2544, #2580, #2582, #2541, #2447.

## Empty-Fast Outputs Are Errors

A tool that completes in `< 1 ms` **and** returns empty content is almost always a silent failure. The dispatcher treats `duration < 1ms && content.is_empty()` as `ToolError::EmptyResult` unless the tool opts in via a documented no-op sentinel (idempotent ack).

`ActionRecord` must always capture byte count and timing so this check is auditable at review time. The UI must suppress the success checkmark when output bytes are zero. Reference: #2545.

## External-Effect Tools Must Read Back

A tool whose side effect is visible only to an external system (Telegram send, Slack post, file write, extension install, OAuth completion) MUST read back the effect before returning success:

- `telegram_send` → capture and return `message_id` from the API response; error if the response lacks one.
- `file_write` → re-stat and return the actual byte count; error on mismatch.
- `extension_install` → call `extensions_list` and assert the new extension is present and active.
- OAuth completion → perform a minimal authenticated read against the provider before declaring success.

A tool without a read-back path is claim-only and must mark its output `unverified: true` so downstream layers can warn. References: #2411 Telegram token Save, #2543 Linear MCP OAuth, #2586 Slack Install.

## Setup UI Round-Trip

Save / Install / Connect buttons in the setup UI must issue a read-back verification immediately after the write succeeds and render the read-back value (or explicit error) to the user — not a local optimistic checkmark. Install actions dispatch through `ToolDispatcher::dispatch` and surface the resulting `ActionRecord`. A UI success state with no corresponding backend read-back is the same bug class as agent claim drift. References: #2411, #2534, #2543, #2586.
