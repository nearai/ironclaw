---
paths:
  - "src/agent/**"
  - "src/tools/**"
  - "src/channels/web/**"
  - "crates/ironclaw_engine/**"
---
# Tool Evidence and Side-Effect Verification

The most dangerous user-visible bug class is **claim/evidence drift**: the agent narrates "message sent" / "file attached" / "tool installed" with no corresponding side effect. The agent-facing half of this rule lives in `crates/ironclaw_engine/prompts/codeact_postamble.md` ("Claims in FINAL() need tool evidence"). This file documents the *target* code invariants that make the rule enforceable at the tool layer. Several of these are aspirational — where that's the case, it's called out inline so new contributors don't assume an enforcement mechanism exists.

## Engine v2 Side-Effect Gate (target invariant)

Engine v2 should classify user turns for side-effect intent (send / save / install / schedule / post / write / delete) and a model-final turn that lacks at least one successful tool call matching the intent should be rejected before it reaches the user — surfacing "action not performed" instead of the agent's narration.

**Current state:** only a soft tool-intent *nudge* exists in `crates/ironclaw_engine/src/executor/loop_engine.rs`; there is no hard rejection gate. Adding one belongs on the engine roadmap. Until it lands, the prompt-side guidance in `codeact_postamble.md` is the primary defence. Reference: #2544, #2580, #2582, #2541, #2447.

## Empty-Fast Outputs Are Errors (tool-author convention)

A tool that completes in `< 1 ms` **and** returns empty content is almost always a silent failure. Tool authors must treat this shape as an error at the tool implementation: return a descriptive `ToolError::ExecutionFailed("empty result from <service>: …")` (or the closest matching variant in `src/tools/tool.rs`) rather than a successful empty `ToolOutput`.

**Current state:** the dispatcher does not today enforce a generic "fast + empty = error" rule, and `ToolOutput` / `ActionRecord` do not carry a dedicated byte-count field — timing is captured on `ToolOutput.duration` and content size is implicit in the serialized `result`. A future enforcement path (dedicated `ToolError::EmptyResult` variant, explicit byte-count on `ActionRecord`, UI suppression of the success checkmark on zero-byte output) is desirable; when adding those, update this rule to cite the concrete APIs. Reference: #2545.

## External-Effect Tools Must Read Back

A tool whose side effect is visible only to an external system (Telegram send, Slack post, file write, extension install, OAuth completion) MUST read back the effect before returning success:

- `telegram_send` → capture and return `message_id` from the API response; error if the response lacks one.
- `file_write` → re-stat and return the actual byte count; error on mismatch.
- `extension_install` → call `extensions_list` and assert the new extension is present and active.
- OAuth completion → perform a minimal authenticated read against the provider before declaring success.

A tool without a read-back path is claim-only. There is no canonical `unverified` field on `ToolOutput` today — when you write a claim-only tool, include an `unverified: true` key in the JSON `result` body and a clear hedge in the text output ("submitted; delivery not confirmed") so downstream layers and the user can see it. If/when a first-class field lands on `ToolOutput`, migrate to it. References: #2411 Telegram token Save, #2543 Linear MCP OAuth, #2586 Slack Install.

## Setup UI Round-Trip

Save / Install / Connect buttons in the setup UI must issue a read-back verification immediately after the write succeeds and render the read-back value (or explicit error) to the user — not a local optimistic checkmark. Install actions dispatch through `ToolDispatcher::dispatch` and surface the resulting `ActionRecord`. A UI success state with no corresponding backend read-back is the same bug class as agent claim drift. References: #2411, #2534, #2543, #2586.
