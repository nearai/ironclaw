# IronClaw — LangGraph Edition

A Python reimplementation of [IronClaw](../README.md) using [LangGraph](https://langchain-ai.github.io/langgraph/).

## Architecture

The Rust agentic loop is rewritten as a LangGraph `StateGraph`:

```
START → route_input → check_signals → call_llm → [text → END]
                ↑                                  [tool_calls → execute_tools ──┘]
                └─────────────────────────────────────────────────────────────────┘
                                         (loop)
```

### Mapping: Rust → Python

| Rust component | Python equivalent |
|---|---|
| `run_agentic_loop()` | `build_agent_graph()` in `graph.py` |
| `LoopDelegate` trait | `AgentDeps` + node functions |
| `ReasoningContext` | `AgentState` (LangGraph state) |
| `LoopSignal` | `state.signal` field |
| `LoopOutcome::Response` | Graph terminates, last `AIMessage` is result |
| `LoopOutcome::NeedApproval` | `state.pending_approval` set, graph exits to wait |
| `LoopOutcome::Stopped` | `signal="stop"` → conditional edge to `END` |
| `LoopOutcome::MaxIterations` | `iteration >= max_iterations` → `END` |
| `Scheduler` | `JobScheduler` (asyncio tasks) |
| `SessionManager` | LangGraph `MemorySaver` checkpointer |
| `SafetyLayer` | `ironclaw.safety.SafetyLayer` |
| `ToolRegistry` | `ironclaw.tools.ToolRegistry` |
| `Channel` trait | `ironclaw.channels.Channel` ABC |

### Graph Topology

```
route_input
    ├── signal=stop → END
    └── → check_signals
            ├── signal=stop → END
            └── → call_llm
                    ├── response=text → END
                    ├── response=tool_calls → execute_tools
                    │       ├── need_approval → END (wait)
                    │       └── → check_signals (loop)
                    ├── response=tool_intent_nudge → check_signals (inject nudge)
                    └── iteration >= max → END
```

### Session / Thread / Checkpointing

Each conversation thread is identified by a `thread_id` string.  LangGraph's
`MemorySaver` checkpointer stores the `AgentState` per thread, providing the
same isolation as the Rust `SessionManager`.  For production, replace
`MemorySaver` with `AsyncPostgresSaver` from `langgraph-checkpoint-postgres`.

### Background Jobs

`JobScheduler` dispatches background jobs as asyncio Tasks.  Each job runs
an independent graph invocation with its own `thread_id` so checkpointing
keeps job state separate from interactive chat — identical to the Rust
`Scheduler`'s parallel execution model.

## Quick Start

```bash
cd langgraph/
pip install -e ".[dev]"

# Set your LLM credentials
export LLM_BACKEND=anthropic
export ANTHROPIC_API_KEY=sk-ant-...

# Run the REPL
ironclaw run

# Or with debug logging
ironclaw run --debug
```

## Configuration

All settings come from environment variables (or a `.env` file):

```env
# LLM
LLM_BACKEND=anthropic          # openai | anthropic | openai_compatible | ollama
LLM_MODEL=claude-sonnet-4-6
LLM_API_KEY=...
LLM_MAX_TOKENS=4096

# Agent
AGENT_MAX_PARALLEL_JOBS=5
AGENT_MAX_ITERATIONS=50
AGENT_AUTO_APPROVE_TOOLS=false
AGENT_ALLOW_LOCAL_TOOLS=true

# Safety
SAFETY_INJECTION_CHECK_ENABLED=true
SAFETY_MAX_OUTPUT_LENGTH=100000

# Channels
HTTP_ENABLED=false
HTTP_PORT=8080
HTTP_SECRET=...
```

## Builtin Tools

| Tool | Description | Requires approval |
|---|---|---|
| `echo` | Echo text verbatim | No |
| `current_time` | Return current date/time | No |
| `memory_search` | Full-text search in workspace | No |
| `memory_write` | Write to workspace memory | No |
| `memory_read` | Read from workspace memory | No |
| `read_file` | Read a workspace file | No |
| `write_file` | Write a workspace file | Yes |
| `list_dir` | List a workspace directory | No |
| `http_get` | HTTP GET request | No |
| `http_post` | HTTP POST request | Yes |
| `shell` | Execute shell command | Yes |

## Testing

```bash
pytest tests/ -v
```

## Production Checklist

- [ ] Replace `MemorySaver` with `AsyncPostgresSaver` for persistent checkpointing
- [ ] Replace in-memory `Workspace` with PostgreSQL + pgvector for semantic search
- [ ] Add MCP client support (`langchain-mcp-adapters`)
- [ ] Add WASM tool sandbox (or Docker sandbox via the Rust orchestrator API)
- [ ] Add web gateway (FastAPI + SSE) for the browser UI
- [ ] Add Telegram/Slack channel adapters
- [ ] Add cost guard middleware
- [ ] Add routine engine (cron + event triggers)
