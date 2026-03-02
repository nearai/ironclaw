# Requirements — Agent Reasoning Summary

## Finalized decisions from shaping

1. **Rationale source**: per-tool, mandatory
   - Thread deterministic per-tool rationale from `ToolSelection.reasoning` through to each recorded tool call.
   - Keep `RespondResult::ToolCalls.content` as optional turn-level narrative only (not a substitute for per-tool rationale).
   - No extra LLM call.

2. **Narrative generation**: deterministic only
   - Do not add extra LLM call for summary generation.

3. **CLI behavior**: always on
   - Reasoning summaries are visible by default in CLI flow.

4. **HTTP shape**: inline on turn response
   - Add reasoning payload inline to turn responses, keyed by session/thread/turn.

5. **Tool parameters**: redacted JSON
   - Expose parameter JSON with sensitive values redacted in tool decision entries.

6. **Safety boundary**: full SafetyLayer
   - Apply full sanitization/leak/policy pipeline to reasoning text.

7. **Scope**: include worker path
   - Do not exclude worker/job loop from design scope.

8. **Retention**: full session lifetime (in-memory)
   - Keep reasoning data for the full in-memory session duration.

9. **Identity key**: session + thread + turn
   - Use `(session_id, thread_id, turn_number)` as canonical addressing.

10. **CLI command**: first-class
    - Add dedicated `/reasoning` command in addition to always-on output.

11. **Reasoning data threading**: per-tool rationale path
    - Thread `ToolSelection.reasoning` through `RespondResult::ToolCalls` and into each recorded tool decision.
    - Require one rationale per tool decision (with deterministic fallback text if upstream reasoning is missing after sanitization).
    - Keep `RespondResult::ToolCalls.content` as optional turn-level context only.

12. **Alternatives field**: omit for v1
    - `ToolSelection.alternatives` currently low-signal/empty; defer until reliable.

13. **Parallel tool presentation**: best-practice grouped output
    - Preserve requested order and annotate grouped parallel batches.

14. **Naming**: `reasoning`
    - CLI command `/reasoning`; HTTP payload naming uses `reasoning`.

15. **SSE**: include now
    - Emit reasoning events over SSE as part of v1.

16. **Visual assets**: none provided
    - Spec should include proposed wireframe-style examples for CLI/HTTP views.

## Standards alignment constraints

- Follow `droidz/standards/*.md` as canonical.
- Architecture-first, minimal churn, existing patterns/libraries only.
- Keep security pipeline intact and avoid sensitive leakage.
- No DB persistence for this feature in v1.
- Maintain feature parity discipline if behavior status changes.

## Non-goals

- No raw chain-of-thought exposure.
- No token-level reasoning stream from model internals.
- No dependency on external UI mock assets.
