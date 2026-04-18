# Bug-bash regression fixtures

Each file in this directory is a replay fixture that pins a **specific open
bug** to a deterministic snapshot. When the bug is fixed, the snapshot diff
in the PR is the reviewable proof that the fix changed the observed
behaviour. When someone reintroduces the bug, the snapshot drifts and CI
blocks the merge.

## Recording a new fixture

Fixtures should be **recorded from the real agent**, not hand-written.
Hand-written JSON is fine for unit-style coverage, but it can't replicate
the prompt/context shape that triggered a production bug.

```bash
# 1. Reproduce the bug live against staging.
IRONCLAW_RECORD_TRACE=1 \
IRONCLAW_TRACE_OUTPUT=tests/fixtures/llm_traces/bug_bash/<name>.json \
IRONCLAW_TRACE_MODEL_NAME=bug-bash-<issue>-<slug> \
cargo run

# 2. Interact with the agent until you observe the bug.

# 3. Exit — the fixture is written on shutdown.

# 4. Add a snapshot test that replays the fixture and asserts the
#    ReplayOutcome shape. Example in tests/e2e_engine_v2.rs:
#        snapshot_summarization_uses_tools.
```

## Coverage map

| Issue | Fixture | Regression assertion encoded in the snapshot |
|-------|---------|-----------------------------------------------|
| [#2540](https://github.com/near/ironclaw/issues/2540) | `routine_timeout_regression.json` (TODO — record) | `final_state == Done`, total wall time under 300 s |
| [#2541](https://github.com/near/ironclaw/issues/2541) | `summarization_uses_tools.json` | at least one `echo` tool call on a "do X" prompt |
| [#2542](https://github.com/near/ironclaw/issues/2542) | `routine_setup_has_terminal.json` (TODO — record) | conversation ends with `Done` or `Failed`, non-empty surface |
| [#2543](https://github.com/near/ironclaw/issues/2543) | `linear_oauth_recognized.json` (TODO — record) | no retrospective `tool_error` with `authorization` category |
| [#2544](https://github.com/near/ironclaw/issues/2544) | `plan_followed_by_execution.json` (TODO — record) | at least one `ActionExecuted` event after the plan step |
| [#2545](https://github.com/near/ironclaw/issues/2545) | `tool_result_non_empty.json` (TODO — record) | `ToolResult` preview length bucket > 0 |
| [#2546](https://github.com/near/ironclaw/issues/2546) | `orchestrator_error_wrapped.json` (TODO — record) | no raw "HTTP 502" in conversation surface |

The TODO entries are placeholders until we have a staging environment that
can reproduce the bug. Each line names the fixture file the snapshot test
expects and the specific property the snapshot will pin.

## Why hand-written is not enough

Hand-written fixtures encode **our mental model** of the bug. Real
recordings capture the exact LLM reasoning and tool-sequence that produced
it. When the fix lands, the recorded fixture drifts in a way the
hand-written stub wouldn't — which is exactly what we want the snapshot
gate to catch.
