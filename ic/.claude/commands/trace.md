---
description: Trace a data flow or bug through the IronClaw codebase end-to-end
allowed-tools: Read, Glob, Grep, Bash(cargo test:*)
argument-hint: <symptom or feature name>
model: sonnet
---

Trace the flow of `$ARGUMENTS` through the IronClaw codebase. Your job is to map every file and function involved, identify where data transforms or could break, and report the full chain.

## Architecture Reference

IronClaw has three main data flow paths. Identify which one(s) are relevant and trace through them:

### Message Flow (user input to LLM response)
```
Channel (cli/web/wasm) → IncomingMessage
  → Agent::run() message loop (agent_loop.rs)
    → handle_message() dispatches by Submission type
      → SubmissionParser::parse() (submission.rs) classifies input
      → process_user_input() for new turns
      → process_approval() for tool approval responses
      → handle_command() for /commands
    → run_agentic_loop() iterates LLM calls
      → Reasoning::respond_with_tools() (reasoning.rs)
        → LlmProvider::complete_with_tools() (nearai_chat.rs or nearai.rs)
      → Tool execution with approval gating
      → Context message accumulation
    → Response flows back through Channel::send_response()
```

### SSE Event Flow (backend status to web UI)
```
StatusUpdate variant (channel.rs)
  → Channel::send_status() trait method
    → WebChannel::send_status() (web/mod.rs) maps to SseEvent
      → broadcast via tokio::broadcast channel
    → SSE endpoint streams events (web/server.rs)
      → Browser EventSource listener (app.js)
        → DOM update function
        → CSS styling (style.css)
```

### Tool Flow (tool definition to execution)
```
Tool trait impl (tools/builtin/*.rs or tools/mcp/client.rs or tools/wasm/wrapper.rs)
  → ToolRegistry::register() (tools/registry.rs)
  → tool_definitions() builds Vec<ToolDefinition> for LLM
    → ToolDefinition { name, description, parameters } (llm/provider.rs)
    → Serialized to ChatCompletionTool (nearai_chat.rs)
  → LLM returns ToolCall { id, name, arguments }
  → agent_loop.rs executes via execute_chat_tool()
    → Safety layer sanitizes output
    → Result added as ChatMessage::tool_result()
```

## Tracing Instructions

1. **Read** each file in the relevant flow path, focusing on the functions that handle the data.
2. **Identify transforms**: Where does the data change shape? (e.g., `McpTool.input_schema` → `ToolDefinition.parameters` → `ChatCompletionTool.function.parameters`)
3. **Identify failure points**: Where could the data be lost, malformed, or misrouted?
4. **Report the chain**: List every file:line involved, what happens at each step, and where the issue (if any) is.

## Key Files Quick Reference

| Area | File | Key Functions |
|------|------|---------------|
| Message dispatch | `src/agent/agent_loop.rs` | `handle_message`, `process_user_input`, `process_approval`, `run_agentic_loop` |
| Input parsing | `src/agent/submission.rs` | `SubmissionParser::parse` |
| LLM reasoning | `src/llm/reasoning.rs` | `respond_with_tools`, `select_tools`, `plan` |
| Chat completions | `src/llm/nearai_chat.rs` | `complete_with_tools`, `From<ChatMessage>` |
| Responses API | `src/llm/nearai.rs` | `complete_with_tools`, `split_messages` |
| Channel trait | `src/channels/channel.rs` | `Channel`, `StatusUpdate`, `IncomingMessage` |
| Web gateway | `src/channels/web/mod.rs` | `send_status`, `send_response` |
| Web server | `src/channels/web/server.rs` | Route handlers, SSE endpoints |
| Web frontend | `src/channels/web/static/app.js` | SSE listeners, DOM builders |
| Tool registry | `src/tools/registry.rs` | `tool_definitions`, `get`, `register` |
| MCP tools | `src/tools/mcp/client.rs` | `McpToolWrapper`, `list_tools`, `call_tool` |
| MCP protocol | `src/tools/mcp/protocol.rs` | `McpTool`, `inputSchema` |
| Safety | `src/safety/sanitizer.rs` | `sanitize_tool_output`, `wrap_for_llm` |
| Session state | `src/agent/session.rs` | `ThreadState`, `Turn`, `PendingApproval` |

## Output Format

Report your findings as:

1. **Flow path**: The specific chain of files and functions involved
2. **Data transforms**: How the data changes at each step
3. **Findings**: Any bugs, missing data, or suspicious patterns
4. **Recommendation**: What to fix or investigate further
