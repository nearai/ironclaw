# Python Orchestrator: Move the Engine Loop to CodeAct

**Date:** 2026-03-25
**Status:** Design
**Context:** The engine's Rust loop has frequent bugs in the glue layer (tool dispatch, output formatting, state management, truncation). The LLM can't fix Rust at runtime. Moving the loop to Python via CodeAct makes the orchestration layer self-modifiable by the self-improvement Mission.

---

## Architecture

### Before (current)

```
ExecutionLoop::run() [Rust, 900 lines]
  ├── Build system prompt
  ├── for iteration in 0..max:
  │     ├── Check signals
  │     ├── Check budgets
  │     ├── Build context (messages + actions)
  │     ├── Call LLM
  │     ├── Match response:
  │     │     ├── Text → extract FINAL(), check nudge
  │     │     ├── ActionCalls → execute_action_calls()
  │     │     └── Code → execute_code() via Monty
  │     ├── Format output metadata
  │     ├── Update persisted state
  │     └── Persist checkpoint
  └── Return ThreadOutcome
```

### After (proposed)

```
ExecutionLoop::run() [Rust, ~50 lines — bootstrap only]
  ├── Load orchestrator code from Store (versioned MemoryDoc)
  ├── If missing, use compiled-in default
  ├── Set up Monty VM with host functions
  ├── Execute orchestrator Python code
  └── Return ThreadOutcome from Python's return value

Host functions [Rust, exposed to Python via Monty suspension]:
  ├── llm_complete(messages, actions, config) → response
  ├── execute_action(name, params) → result (lease + policy + safety)
  ├── check_signals() → signal or None
  ├── save_checkpoint(state) → persist thread/step/events
  ├── emit_event(kind) → broadcast + record
  ├── transition_to(state, reason) → validated state change
  ├── retrieve_docs(goal, max) → memory docs
  ├── get_actions() → available ActionDefs
  └── check_budget() → remaining tokens/time/usd

Orchestrator [Python, versioned, self-modifiable]:
  └── run_loop(context, goal, actions, state, config) → outcome
        ├── Tool dispatch + name resolution
        ├── Output formatting + truncation
        ├── State management (persisted_state dict)
        ├── FINAL() extraction
        ├── Tool intent nudge detection
        ├── Context compaction decisions
        └── The step loop itself
```

---

## Versioned Orchestrator Code

The orchestrator Python source is stored as a MemoryDoc:

```
DocType: Note
Title: "orchestrator:main"
Tag: "orchestrator_code"
Content: <Python source code>
Metadata: {
  "version": 3,
  "parent_version": 2,
  "source_thread_id": "...",  // which self-improvement thread created this
  "created_at": "2026-03-25T10:00:00Z"
}
```

### Version lifecycle

```
v0 (compiled-in) → v1 (self-improvement fix) → v2 (another fix) → ...
                                                    ↑
                                              auto-rollback if v2 causes
                                              3 consecutive thread failures
```

### Operations

- **Load**: Query Store for `orchestrator:main` docs, pick highest version
- **Update**: Self-improvement Mission saves a new version with `parent_version` pointing to current
- **Rollback**: On consecutive failures, load the `parent_version` doc instead
- **Reset**: Delete all runtime versions, fall back to compiled-in v0

### Auto-rollback logic

Tracked per-version in mission metadata or thread config:

```python
# Pseudo-logic in the bootstrap (Rust side)
consecutive_failures = count_recent_failures(orchestrator_version)
if consecutive_failures >= 3:
    orchestrator = load_version(parent_version)
    emit_event(SelfImprovementRollback { from: current, to: parent })
```

---

## Host Functions

These replace direct Rust calls with Python-callable suspension points, using the same mechanism Monty already uses for tool calls.

### `llm_complete(messages, actions=None, config=None)`

```python
# Python side
response = llm_complete(
    messages=[{"role": "user", "content": "search for AI news"}],
    actions=get_actions(),
    config={"force_text": False}
)
# response = {"type": "text", "content": "..."}
#           | {"type": "actions", "calls": [...]}
#           | {"type": "code", "code": "..."}
# Also: response["usage"] = {"input_tokens": N, "output_tokens": M}
```

Rust side: calls `LlmBackend::complete()`, converts `LlmOutput` to JSON dict.

### `execute_action(name, params)`

```python
result = execute_action("web_search", {"query": "AI news", "count": 5})
# result = {"output": {...}, "is_error": false, "duration_ms": 123}
# Includes: lease check, policy evaluation, safety sanitization, hooks
```

Rust side: full `EffectExecutor::execute_action()` pipeline with all v1 security controls.

### `check_signals()`

```python
signal = check_signals()
# signal = None | "stop" | {"inject": "new message"} | "suspend"
```

Rust side: `signal_rx.try_recv()` on the tokio channel.

### `save_checkpoint(state, step=None)`

```python
save_checkpoint(state={"last_return": result, "web_search": data})
```

Rust side: serializes to thread metadata, optionally saves Step + events to Store.

### `emit_event(kind, **kwargs)`

```python
emit_event("action_executed", action_name="web_search", duration_ms=123)
emit_event("step_completed", tokens={"input": 500, "output": 200})
```

Rust side: constructs `EventKind` variant, broadcasts + records.

### `transition_to(state, reason=None)`

```python
transition_to("completed", reason="FINAL() called")
# Raises error if transition is invalid (state machine enforcement stays in Rust)
```

### `retrieve_docs(goal, max_docs=5)`

```python
docs = retrieve_docs("search for AI news", max_docs=5)
# docs = [{"type": "LESSON", "title": "...", "content": "..."}, ...]
```

### `check_budget()`

```python
budget = check_budget()
# budget = {"tokens_remaining": 50000, "time_remaining_ms": 25000, "usd_remaining": 0.45}
```

### `get_actions()`

```python
actions = get_actions()
# actions = [{"name": "web_search", "description": "...", "params": {...}}, ...]
```

---

## Default Orchestrator (v0)

The compiled-in Python code that ships with the binary. This is what `include_str!` loads as the seed version. It replicates the current Rust loop logic:

```python
def run_loop(context, goal, actions, state, config):
    """Engine v2 orchestrator — the self-modifiable execution loop."""
    max_iterations = config.get("max_iterations", 30)
    max_nudges = config.get("max_tool_intent_nudges", 2)
    nudge_count = 0
    consecutive_errors = 0

    for step in range(max_iterations):
        # 1. Check signals
        signal = check_signals()
        if signal == "stop":
            transition_to("completed", "stopped by signal")
            return {"type": "stopped"}
        if signal and "inject" in signal:
            context.append({"role": "user", "content": signal["inject"]})

        # 2. Check budget
        budget = check_budget()
        if budget["tokens_remaining"] <= 0:
            transition_to("completed", "token budget exhausted")
            return {"type": "completed", "response": "Token budget exhausted."}

        # 3. Build messages for LLM
        messages = list(context)  # copy

        # 4. Inject prior knowledge on first step
        if step == 0:
            docs = retrieve_docs(goal)
            if docs:
                knowledge = format_docs(docs)
                if messages and messages[0]["role"] == "system":
                    messages[0]["content"] += "\n\n" + knowledge

        # 5. Call LLM
        emit_event("step_started")
        response = llm_complete(messages, actions)
        emit_event("step_completed", tokens=response["usage"])

        # 6. Handle response
        if response["type"] == "text":
            text = response["content"]

            # Check for FINAL()
            final = extract_final(text)
            if final is not None:
                context.append({"role": "assistant", "content": text})
                transition_to("completed", "FINAL() called")
                return {"type": "completed", "response": final}

            # Check for tool intent nudge
            if nudge_count < max_nudges and signals_tool_intent(text):
                nudge_count += 1
                context.append({"role": "assistant", "content": text})
                context.append({"role": "user", "content":
                    "You described what you'd do but didn't write code. "
                    "Please write a ```repl code block to execute your plan."})
                continue

            # Plain text response — done
            context.append({"role": "assistant", "content": text})
            transition_to("completed", "text response")
            return {"type": "completed", "response": text}

        elif response["type"] == "code":
            code = response["code"]
            nudge_count = 0
            context.append({"role": "assistant", "content": f"```repl\n{code}\n```"})

            # Code is executed by the Monty VM outside this function.
            # We receive results via state dict after execution.
            # The host handles code execution and resumes us with results.
            result = execute_code_step(code, state)

            # Update state with results
            state[f"step_{step}_return"] = result.get("return_value")
            state["last_return"] = result.get("return_value")
            for r in result.get("action_results", []):
                state[r["action_name"]] = r["output"]

            # Format output for next iteration
            output = format_output(result)
            context.append({"role": "user", "content": output})

            # Check for FINAL() in code output
            if result.get("final_answer") is not None:
                transition_to("completed", "FINAL() in code")
                return {"type": "completed", "response": result["final_answer"]}

            # Track errors
            if result.get("had_error"):
                consecutive_errors += 1
                if consecutive_errors >= 5:
                    transition_to("failed", "too many consecutive errors")
                    return {"type": "failed", "error": "5 consecutive code errors"}
            else:
                consecutive_errors = 0

            save_checkpoint(state)

        elif response["type"] == "actions":
            # Tier 0: structured tool calls
            nudge_count = 0
            results = []
            for call in response["calls"]:
                r = execute_action(call["name"], call.get("params", {}))
                results.append(r)
                if r.get("need_approval"):
                    save_checkpoint(state)
                    return {"type": "need_approval",
                            "action_name": call["name"],
                            "call_id": call.get("call_id", ""),
                            "parameters": call.get("params", {})}

            # Add results to context
            for r in results:
                context.append({"role": "tool", "content": format_action_result(r)})
            save_checkpoint(state)

    # Max iterations reached
    transition_to("completed", "max iterations")
    return {"type": "max_iterations"}


# ── Helper functions (the self-modifiable glue) ──────────────

def extract_final(text):
    """Extract FINAL() content from text. Returns None if not found."""
    idx = text.find("FINAL(")
    if idx < 0:
        return None
    after = text[idx + 6:]
    # Handle triple-quoted strings
    if after.startswith('"""'):
        end = after.find('"""', 3)
        if end >= 0:
            return after[3:end]
    # Handle quoted strings
    if after.startswith('"') or after.startswith("'"):
        quote = after[0]
        end = after.find(quote, 1)
        if end >= 0:
            return after[1:end]
    # Handle balanced parens
    depth = 1
    for i, ch in enumerate(after):
        if ch == '(':
            depth += 1
        elif ch == ')':
            depth -= 1
            if depth == 0:
                return after[:i]
    return None


def signals_tool_intent(text):
    """Check if text describes tool usage without actually using tools."""
    lower = text.lower()
    intent_phrases = ["i will", "i'll", "let me", "i would", "i should",
                      "i can", "i need to", "we should", "we can"]
    tool_phrases = ["search", "fetch", "call", "run", "execute", "use the"]
    has_intent = any(p in lower for p in intent_phrases)
    has_tool = any(p in lower for p in tool_phrases)
    return has_intent and has_tool


def format_output(result, max_chars=8000):
    """Format code execution result for the next LLM context message."""
    parts = []

    stdout = result.get("stdout", "")
    if stdout:
        parts.append(f"[stdout]\n{stdout}")

    for r in result.get("action_results", []):
        name = r.get("action_name", "?")
        output = str(r.get("output", ""))
        if r.get("is_error"):
            parts.append(f"[{name} ERROR] {output}")
        else:
            preview = output[:500] + "..." if len(output) > 500 else output
            parts.append(f"[{name}] {preview}")

    ret = result.get("return_value")
    if ret is not None:
        parts.append(f"[return] {ret}")

    text = "\n\n".join(parts)

    # Truncate from the front (keep the tail, which has the most recent results)
    if len(text) > max_chars:
        text = "... (truncated) ...\n" + text[-max_chars:]

    return text


def format_docs(docs):
    """Format memory docs for context injection."""
    parts = ["## Prior Knowledge (from completed threads)\n"]
    for doc in docs:
        label = doc["type"].upper()
        content = doc["content"][:500]
        truncated = "..." if len(doc["content"]) > 500 else ""
        parts.append(f"### [{label}] {doc['title']}\n{content}{truncated}\n")
    return "\n".join(parts)


def format_action_result(result):
    """Format a single action result for the LLM context."""
    name = result.get("action_name", "unknown")
    output = result.get("output", {})
    if result.get("is_error"):
        return f"Tool '{name}' failed: {output}"
    return str(output)
```

---

## Implementation Steps

### Step 1: Expose host functions in scripting.rs

Add new `FunctionCall` handlers alongside the existing tool dispatch:

- `__llm_complete__` → calls `LlmBackend::complete()`
- `__check_signals__` → calls `signal_rx.try_recv()`
- `__save_checkpoint__` → persists thread state
- `__emit_event__` → broadcasts event
- `__transition_to__` → validates + transitions thread state
- `__retrieve_docs__` → queries RetrievalEngine
- `__check_budget__` → reads remaining tokens/time/usd
- `__get_actions__` → enumerates available ActionDefs from leases

These use `__dunder__` names to avoid collision with user tools.

### Step 2: Create the bootstrap in loop_engine.rs

Replace `ExecutionLoop::run()` body with:

1. Load orchestrator code from Store (`orchestrator:main` MemoryDoc, highest version)
2. If no runtime version, use `include_str!("../../orchestrator/default.py")`
3. Inject context variables: `context`, `goal`, `actions`, `state`, `config`
4. Execute via Monty with the orchestrator code
5. Parse the return value as `ThreadOutcome`
6. Handle auto-rollback if execution fails

### Step 3: Write the default orchestrator

Create `crates/ironclaw_engine/orchestrator/default.py` with the v0 code shown above.

### Step 4: Wire versioning into the self-improvement Mission

Update the Mission goal prompt to include:
- How to read the current orchestrator: `memory_search("orchestrator:main")`
- How to update it: `memory_write` with title="orchestrator:main", tag="orchestrator_code", metadata with version++
- The constraint: changes must be minimal, one fix at a time

### Step 5: Add auto-rollback

In the bootstrap (Step 2), after orchestrator execution fails:
- Increment a failure counter in thread metadata
- If counter >= 3, load `parent_version` instead
- Emit `SelfImprovementRollback` event
- Reset failure counter

### Step 6: Add `execute_code_step` host function

This is the interesting one — the orchestrator needs to run user Python code (the CodeAct step). Two options:

**Option A: Nested Monty execution** — The orchestrator Python calls `execute_code_step(code, state)` which suspends to Rust, Rust creates a nested Monty VM for the user code, runs it with tool dispatch, returns results. Clean but complex.

**Option B: Host-managed code execution** — The orchestrator returns a `{"type": "execute_code", "code": "...", "state": {...}}` action, Rust runs the code in the existing Monty pipeline, then re-enters the orchestrator with results. Simpler but requires the orchestrator to yield/resume.

Recommend **Option A** for clean separation. The orchestrator is a management layer; user code runs in a sandboxed sub-VM.

---

## What This Enables

1. **Self-improvement Mission fixes glue bugs at runtime** — no Rust rebuild
2. **Format_output bug?** Mission patches `format_output()` in the orchestrator
3. **Tool name mismatch?** Mission adds an alias in the orchestrator's dispatch
4. **State persistence bug?** Mission fixes `save_checkpoint()` call
5. **New feature?** Mission adds a new helper function
6. **Bad fix?** Auto-rollback to previous version after 3 failures

The Rust layer becomes an OS kernel — stable, provides capabilities. The Python orchestrator is userspace — where iteration happens fast.

---

## Safety

| Concern | Mitigation |
|---------|-----------|
| Orchestrator loops forever | Rust-enforced timeout (existing 30s per code step, plus thread-level budget) |
| Orchestrator skips safety checks | `execute_action()` enforces lease + policy in Rust regardless |
| Orchestrator calls `transition_to("failed")` inappropriately | State machine validation stays in Rust |
| Bad version breaks all threads | Auto-rollback after 3 consecutive failures |
| Orchestrator tries to escape sandbox | Monty blocks OS calls, network, filesystem |
| Self-improvement Mission writes bad code | Versioning allows instant rollback; compiled v0 always available |

---

## Migration Path

1. **Phase 1**: Add host functions, keep Rust loop as-is. Test that Python can call `llm_complete()` etc.
2. **Phase 2**: Write default orchestrator in Python. Run it alongside Rust loop, compare outcomes.
3. **Phase 3**: Switch to Python orchestrator as primary. Remove Rust loop code.
4. **Phase 4**: Wire versioning + self-improvement Mission + auto-rollback.
