# Gateway-ops trace fixtures

These fixtures drive `TraceRunner` (see `src/testing/trace_runner.rs`) and
replay an ordered list of tool invocations against a libSQL test database.
They're distinct from the agentic LLM-stream fixtures under
`tests/fixtures/llm_traces/`.

## Why two systems

- **`llm_traces/`**: replay an LLM stream, assert the agent re-produces the
  same tool calls from the same user input — *"does the agent behave
  deterministically?"*
- **`gateway_traces/`**: replay an ordered caller-dispatched tool sequence,
  assert the `Tool → ActionRecord → Database::save_action` pipeline works
  and matches declared expectations — *"does the gateway-ops pipeline
  preserve the actions we asked for?"*

Both layers ultimately cover trace-replay coverage (#643 / Phase 2 of
#2828), but they approach it from opposite directions.

## Wire format

```json
{
  "name": "human-readable-id",
  "operations": [
    {
      "tool_name": "echo",
      "params": { "message": "hi" },
      "expected": {
        "kind": "success",
        "assertions": { "contains_text": "hi" }
      }
    },
    {
      "tool_name": "missing",
      "params": {},
      "expected": {
        "kind": "failure",
        "error_contains": "not registered"
      }
    }
  ]
}
```

`TraceExpectation::Success.assertions` supports three keys:

| Key              | Meaning                                                                 |
|------------------|-------------------------------------------------------------------------|
| `eq`             | Deep equality against the entire tool output JSON                       |
| `contains_text`  | The output (stringified) must contain this substring                    |
| `fields`         | Object of dot-path → expected-value; each path must match in the output |

Omit `assertions` (or set it to `null`) to skip output checks — useful when
you only care that the tool *ran* successfully, not what it returned.

`TraceExpectation::Failure.error_contains` is substring-matched against
`ToolError::to_string()`.

## Current fixtures

| File | Scenario |
|------|----------|
| `echo_roundtrip.json` | Minimal success path (echo tool, sanity check) |
| `idempotency.json` | Same input twice, both succeed with identical outputs |
| `unknown_tool_fails.json` | Unregistered tool produces a `failure` outcome |
| `assertion_mix.json` | Covers `eq`, `contains_text`, `fields`, and a deliberate failure path in one replay |

## Deferred fixtures

- **Settings CRUD** (`settings_set/get/delete`): blocked on #640 landing the
  mutating `Submission` variants. Once those tools exist, add
  `settings_crud.json` here.
- **Extension lifecycle** (`tool_install/tool_remove/tool_list`): tool
  install requires network fetch or pre-seeded local WASM. Deferred to a
  follow-up that either stubs `ExtensionManager` or uses `sandbox::proxy`
  to serve a local manifest. `tool_list` alone is not mutating, so it
  doesn't exercise the interesting pipeline.

## Determinism invariants

When the same trace is replayed twice (same inputs, same DB migrations,
no external I/O), the sequence of `ActionRecord` fields must match
**except** for:

- `id` (new `Uuid::new_v4()` per replay)
- `executed_at` (wall-clock `Utc::now()`)
- `duration` (measured via `Instant::now()`)

`tests/e2e_gateway_trace_harness.rs` has an explicit determinism test
that strips those fields before comparing.
