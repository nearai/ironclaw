# LLM Trace Fixtures

Trace fixtures are JSON files that script LLM behavior for deterministic E2E testing. The `TraceLlm` provider (`tests/support/trace_llm.rs`) replays these canned responses in order, allowing tests to exercise the full agent loop -- tool dispatch, safety layer, context accumulation -- without calling a real LLM.

## Trace Format

```json
{
  "model_name": "descriptive-name",
  "steps": [
    {
      "request_hint": {
        "last_user_message_contains": "optional substring",
        "min_message_count": 1
      },
      "response": { "..." }
    }
  ]
}
```

### Top-level fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model_name` | string | yes | Identifier returned by `LlmProvider::model_name()`. Convention: `{category}-{scenario}` (e.g. `spot-smoke-greeting`, `advanced-tool-error-recovery`). |
| `steps` | array | yes | Ordered list of steps. Each `complete()` or `complete_with_tools()` call consumes the next step. If calls exceed the number of steps, `TraceLlm` returns an error. |

### Step fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `request_hint` | object | no | Soft validation against the incoming request. Mismatches log a warning and increment a counter but do **not** fail the call -- the canned response is still returned. |
| `response` | object | yes | The canned response for this step. |

### Request hints

| Field | Type | Description |
|-------|------|-------------|
| `last_user_message_contains` | string | Asserts the last `Role::User` message contains this substring. |
| `min_message_count` | integer | Asserts the message list has at least this many entries. Useful for verifying context accumulation across turns. |

Hints are intentionally soft -- they help catch wiring mistakes during test development without making traces brittle.

### Response types

Responses are tagged via the `type` field.

#### `text` -- plain text completion

```json
{
  "type": "text",
  "content": "The capital of France is Paris.",
  "input_tokens": 40,
  "output_tokens": 10
}
```

Returns a `CompletionResponse` / `ToolCompletionResponse` with no tool calls and `FinishReason::Stop`. If `complete()` is called (not `complete_with_tools()`), this is the only valid response type.

#### `tool_calls` -- one or more tool invocations

```json
{
  "type": "tool_calls",
  "tool_calls": [
    {
      "id": "call_write_1",
      "name": "write_file",
      "arguments": { "path": "/tmp/test.txt", "content": "hello" }
    }
  ],
  "input_tokens": 80,
  "output_tokens": 25
}
```

Returns a `ToolCompletionResponse` with `FinishReason::ToolUse`. The agent loop executes the tool calls against real tool implementations, feeds the results back as tool-result messages, then calls the LLM again (consuming the next step).

**Important:** `tool_calls` steps cause real tool execution. The tools run against the actual tool registry, so side effects (file writes, memory operations) happen for real. This is what makes these E2E tests -- the only mock is the LLM itself.

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Unique call ID. Convention: `call_{tool}_{n}`. |
| `name` | string | Must match a registered tool name (e.g. `echo`, `write_file`, `read_file`, `memory_write`, `shell`). |
| `arguments` | object | Tool parameters as JSON. Must conform to the tool's `parameters_schema()`. |

### Token counts

Every response includes `input_tokens` and `output_tokens`. These are synthetic values for cost tracking -- set them to reasonable estimates for your scenario.

## What gets mocked vs. what runs for real

| Component | Mocked? | Notes |
|-----------|---------|-------|
| LLM responses | Yes | `TraceLlm` replays canned responses from the trace |
| Tool execution | **No** | Real tools run: file I/O, memory ops, shell commands all execute |
| Safety layer | **No** | Sanitizer, validator, policy, leak detector all run |
| Context/message accumulation | **No** | Messages accumulate naturally across turns |
| Token counting | Partial | Uses synthetic counts from the trace |

## Directory structure

```
llm_traces/
  simple_text.json          # Minimal single-turn text response
  file_write_read.json      # Write then read a file
  memory_write_read.json    # Memory write then text confirmation
  error_path.json           # Tool call with missing params, then recovery
  spot/                     # Quick smoke tests (1-3 steps each)
    smoke_greeting.json     # Simple greeting, no tools
    smoke_math.json         # Math question, no tools
    robust_no_tool.json     # Factual question, no tools
    tool_echo.json          # Single echo tool call + confirmation
    tool_time.json          # Single time tool call + confirmation
    chain_write_read.json   # Write file -> read file -> confirm
    memory_save_recall.json # Memory write -> memory search -> confirm
    robust_correct_tool.json
  coverage/                 # Broader tool and feature coverage
    shell_echo.json         # Shell command execution
    list_dir.json           # Directory listing
    apply_patch_chain.json  # File patching workflow
    json_operations.json    # JSON tool usage
    injection_in_echo.json  # Prompt injection in tool output
    memory_full_cycle.json  # Full memory write/search/read cycle
    status_events_tool_chain.json
  advanced/                 # Multi-step and edge-case scenarios
    long_tool_chain.json    # Many sequential tool calls
    tool_error_recovery.json # Failed tool call -> retry with valid path
    multi_turn_memory.json  # Memory across multiple conversation turns
    workspace_search.json   # Workspace search workflows
    prompt_injection_resilience.json
    iteration_limit.json    # Tests agent loop iteration bounds
```

## Writing a new trace

1. **Pick a category**: `spot/` for quick smoke tests, `coverage/` for tool/feature coverage, `advanced/` for complex multi-step scenarios.

2. **Name the model**: Use `{category}-{scenario}` (e.g. `spot-tool-echo`, `coverage-shell-echo`).

3. **Script the conversation**: Think through the turn sequence. Each LLM call is one step. After a `tool_calls` step, the agent executes the tools and calls the LLM again with the results -- that's the next step.

4. **Add request hints** on the first step (at minimum) to catch wiring issues. Later steps often omit hints since the message content depends on tool output.

5. **End with a `text` step** so the agent has a final response to return.

Example -- a two-turn echo test:

```json
{
  "model_name": "spot-tool-echo",
  "steps": [
    {
      "request_hint": { "last_user_message_contains": "echo" },
      "response": {
        "type": "tool_calls",
        "tool_calls": [
          { "id": "call_echo_1", "name": "echo", "arguments": { "message": "hello" } }
        ],
        "input_tokens": 60,
        "output_tokens": 20
      }
    },
    {
      "response": {
        "type": "text",
        "content": "The echo tool returned: hello",
        "input_tokens": 80,
        "output_tokens": 15
      }
    }
  ]
}
```

## TraceLlm API

The provider exposes inspection methods for test assertions:

```rust
let llm = TraceLlm::from_file("tests/fixtures/llm_traces/spot/tool_echo.json")?;

// ... run agent loop ...

assert_eq!(llm.calls(), 2);              // Total LLM calls made
assert_eq!(llm.hint_mismatches(), 0);     // Request hint failures
let reqs = llm.captured_requests();       // Vec<Vec<ChatMessage>> of all requests
```
