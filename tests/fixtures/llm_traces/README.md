# LLM Trace Fixtures

Trace fixtures are JSON files that script LLM behavior for deterministic E2E testing. The `TraceLlm` provider (`tests/support/trace_llm.rs`) replays these canned responses in order, allowing tests to exercise the full agent loop -- tool dispatch, safety layer, context accumulation -- without calling a real LLM.

## Trace Format

A trace is a model name and a list of **turns**. Each turn pairs a user message with the LLM response steps that follow it.

```json
{
  "model_name": "descriptive-name",
  "turns": [
    {
      "user_input": "Write hello to /tmp/test.txt",
      "steps": [
        {
          "response": {
            "type": "tool_calls",
            "tool_calls": [{ "id": "c1", "name": "write_file", "arguments": {"path": "/tmp/test.txt", "content": "hello"} }],
            "input_tokens": 60, "output_tokens": 20
          }
        },
        {
          "response": {
            "type": "text",
            "content": "Done, wrote hello to the file.",
            "input_tokens": 80, "output_tokens": 15
          }
        }
      ]
    },
    {
      "user_input": "Actually, change it to goodbye instead",
      "steps": [
        {
          "response": {
            "type": "tool_calls",
            "tool_calls": [{ "id": "c2", "name": "write_file", "arguments": {"path": "/tmp/test.txt", "content": "goodbye"} }],
            "input_tokens": 100, "output_tokens": 20
          }
        },
        {
          "response": {
            "type": "text",
            "content": "Updated the file to say goodbye.",
            "input_tokens": 120, "output_tokens": 15
          }
        }
      ]
    }
  ]
}
```

`TestRig::run_trace()` drives the entire conversation automatically -- no test code needed to send user messages.

### Legacy flat format

For backward compatibility, traces with a top-level `"steps"` array (no `"turns"`) are accepted. They are deserialized as a single turn with a placeholder user message. Existing fixtures work unchanged; test code provides the user message via `rig.send_message()`.

```json
{
  "model_name": "descriptive-name",
  "steps": [
    { "response": { "type": "text", "content": "Hello", "input_tokens": 10, "output_tokens": 5 } }
  ]
}
```

### Top-level fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model_name` | string | yes | Identifier returned by `LlmProvider::model_name()`. Convention: `{category}-{scenario}`. |
| `turns` | array | yes* | List of turns. Each turn has `user_input` (string) and `steps` (array of response steps). |

*Or `steps` for the legacy flat format (deserialized as a single turn).

### Turn fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_input` | string | yes | The user message that starts this turn. |
| `steps` | array | yes | Ordered list of LLM response steps for this turn. |

### Step fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `request_hint` | object | no | Soft validation against the incoming request. Mismatches log a warning but do **not** fail the call. |
| `response` | object | yes | The canned response for this step. |

### Request hints

| Field | Type | Description |
|-------|------|-------------|
| `last_user_message_contains` | string | Asserts the last `Role::User` message contains this substring. |
| `min_message_count` | integer | Asserts the message list has at least this many entries. |

Hints are intentionally soft -- they help catch wiring mistakes during test development without making traces brittle.

### Determinism requirement

Trace fixtures must produce deterministic results across runs. **Do not use tools whose output varies by time or environment state.** Specifically:

**Avoid:**
- `time` -- output changes every run
- `list_dir` on directories not created by the trace itself
- `shell` with commands that depend on system state (e.g. `date`, `ps`, `ls /var`)
- `http` -- external endpoints may change or be unavailable
- `memory_search` unless the trace writes the memory entry first

**Prefer:**
- `echo` -- always returns its input
- `json` -- deterministic parsing/formatting
- `write_file` + `read_file` -- self-contained if the trace writes first
- `memory_write` + `memory_read` -- deterministic if the trace writes first
- `shell` with deterministic commands (e.g. `echo "hello"`, `printf`)

When a trace needs to exercise a stateful tool (like `list_dir`), have an earlier step create the expected state (e.g. `write_file` to create the directory contents first).

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
    tool_json.json          # JSON parse tool call + confirmation
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
    multi_turn_memory.json  # Memory across multiple turns
    steering.json           # User steering: correct agent mid-conversation
    workspace_search.json   # Workspace search workflows
    prompt_injection_resilience.json
    iteration_limit.json    # Tests agent loop iteration bounds
```

## Writing a new trace

1. **Pick a category**: `spot/` for quick smoke tests, `coverage/` for tool/feature coverage, `advanced/` for complex multi-step scenarios.

2. **Name the model**: Use `{category}-{scenario}` (e.g. `spot-tool-echo`, `coverage-shell-echo`).

3. **Script the conversation**: Think through the turn sequence. Each LLM call is one step. After a `tool_calls` step, the agent executes the tools and calls the LLM again with the results -- that's the next step.

4. **Add request hints** on the first step of each turn (at minimum) to catch wiring issues. Later steps often omit hints since the message content depends on tool output.

5. **End each turn with a `text` step** so the agent has a final response to return.

Example -- single-turn trace:

```json
{
  "model_name": "spot-tool-echo",
  "turns": [
    {
      "user_input": "Please echo hello for me",
      "steps": [
        {
          "request_hint": { "last_user_message_contains": "echo" },
          "response": {
            "type": "tool_calls",
            "tool_calls": [{ "id": "call_echo_1", "name": "echo", "arguments": { "message": "hello" } }],
            "input_tokens": 60, "output_tokens": 20
          }
        },
        {
          "response": {
            "type": "text",
            "content": "The echo tool returned: hello",
            "input_tokens": 80, "output_tokens": 15
          }
        }
      ]
    }
  ]
}
```

Example -- multi-turn steering:

```json
{
  "model_name": "advanced-steering",
  "turns": [
    {
      "user_input": "Write hello to /tmp/test.txt",
      "steps": [
        {
          "response": {
            "type": "tool_calls",
            "tool_calls": [{ "id": "c1", "name": "write_file", "arguments": {"path": "/tmp/test.txt", "content": "hello"} }],
            "input_tokens": 60, "output_tokens": 20
          }
        },
        { "response": { "type": "text", "content": "Done.", "input_tokens": 80, "output_tokens": 5 } }
      ]
    },
    {
      "user_input": "Actually, change it to goodbye",
      "steps": [
        {
          "response": {
            "type": "tool_calls",
            "tool_calls": [{ "id": "c2", "name": "write_file", "arguments": {"path": "/tmp/test.txt", "content": "goodbye"} }],
            "input_tokens": 100, "output_tokens": 20
          }
        },
        { "response": { "type": "text", "content": "Updated.", "input_tokens": 120, "output_tokens": 5 } }
      ]
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

## TestRig::run_trace()

For traces with multiple turns, `run_trace()` drives the entire conversation automatically:

```rust
let trace = LlmTrace::from_file("tests/fixtures/llm_traces/advanced/steering.json")?;
let rig = TestRigBuilder::new()
    .with_trace(trace.clone())
    .with_tools(tools_with_file_support())
    .build()
    .await;

// Sends each turn's user_input, waits for response, accumulates results.
let all_responses = rig.run_trace(&trace, Duration::from_secs(15)).await;

assert!(!all_responses[0].is_empty(), "Turn 1: no response");
assert!(!all_responses[1].is_empty(), "Turn 2: no response");
```

For legacy flat traces or when you need fine-grained control, use `send_message()` + `wait_for_responses()` directly.
