# Engine v2 Orchestrator (default, v0)
#
# This is the self-modifiable execution loop. It replaces the Rust
# ExecutionLoop::run() with Python that can be patched at runtime
# by the self-improvement Mission.
#
# Host functions (provided by Rust via Monty suspension):
#   __llm_complete__(messages, actions, config)  -> response dict  (args ignored; Rust builds context from thread)
#   __execute_code_step__(code, state)           -> result dict
#   __execute_action__(name, params)             -> result dict
#   __check_signals__()                          -> None | "stop" | {"inject": msg}
#   __emit_event__(kind, **data)                 -> None
#   __add_message__(role, content)               -> None
#   __save_checkpoint__(state, counters)         -> None
#   __transition_to__(state, reason)             -> None
#   __retrieve_docs__(goal, max_docs)            -> list of doc dicts
#   __check_budget__()                           -> budget dict
#   __get_actions__()                            -> list of action dicts
#
# Context variables (injected by Rust before execution):
#   context  - list of prior messages [{role, content}]
#   goal     - thread goal string
#   actions  - list of available action defs
#   state    - persisted state dict from prior steps
#   config   - thread config dict


# ── Helper functions (self-modifiable glue) ──────────────────
# Defined before run_loop so they are in scope when called.


def extract_final(text):
    """Extract FINAL() content from text. Returns None if not found."""
    idx = text.find("FINAL(")
    if idx < 0:
        return None
    after = text[idx + 6:]
    # Handle triple-quoted strings
    for q in ['"""', "'''"]:
        if after.startswith(q):
            end = after.find(q, len(q))
            if end >= 0:
                return after[len(q):end]
    # Handle single/double quoted strings
    if after and after[0] in ('"', "'"):
        quote = after[0]
        end = after.find(quote, 1)
        if end >= 0:
            return after[1:end]
    # Handle balanced parens
    depth = 1
    for i, ch in enumerate(after):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return after[:i]
    return None


def signals_tool_intent(text):
    """Check if text describes tool usage without actually executing tools."""
    lower = text.lower()
    intent_phrases = ["i will", "i'll", "let me", "i would", "i should",
                      "i can", "i need to", "we should", "we can"]
    tool_phrases = ["search", "fetch", "call", "run", "execute",
                    "use the", "query", "look up"]
    has_intent = any(p in lower for p in intent_phrases)
    has_tool = any(p in lower for p in tool_phrases)
    return has_intent and has_tool


def format_output(result, max_chars=8000):
    """Format code execution result for the next LLM context message."""
    parts = []

    stdout = result.get("stdout", "")
    if stdout:
        parts.append("[stdout]\n" + stdout)

    for r in result.get("action_results", []):
        name = r.get("action_name", "?")
        output = str(r.get("output", ""))
        if r.get("is_error"):
            parts.append("[" + name + " ERROR] " + output)
        else:
            preview = output[:500] + "..." if len(output) > 500 else output
            parts.append("[" + name + "] " + preview)

    ret = result.get("return_value")
    if ret is not None:
        parts.append("[return] " + str(ret))

    text = "\n\n".join(parts)

    # Truncate from the front (keep the tail with most recent results)
    if len(text) > max_chars:
        text = "... (truncated) ...\n" + text[-max_chars:]

    if not text:
        text = "[code executed, no output]"

    return text


def format_docs(docs):
    """Format memory docs for context injection."""
    parts = ["## Prior Knowledge (from completed threads)\n"]
    for doc in docs:
        label = doc.get("type", "NOTE").upper()
        content = doc.get("content", "")[:500]
        truncated = "..." if len(doc.get("content", "")) > 500 else ""
        parts.append("### [" + label + "] " + doc.get("title", "") +
                      "\n" + content + truncated + "\n")
    return "\n".join(parts)


# ── Skill selection and injection (self-modifiable) ────────


def score_skill(skill, message_lower):
    """Score a skill against a user message. Returns 0 if vetoed."""
    meta = skill.get("metadata", {})
    activation = meta.get("activation", {})

    # Exclude keyword veto
    for excl in activation.get("exclude_keywords", []):
        if excl.lower() in message_lower:
            return 0

    score = 0

    # Keyword scoring: exact word = 10, substring = 5 (cap 30)
    kw_score = 0
    words = message_lower.split()
    for kw in activation.get("keywords", []):
        kw_lower = kw.lower()
        if kw_lower in words:
            kw_score += 10
        elif kw_lower in message_lower:
            kw_score += 5
    score += min(kw_score, 30)

    # Tag scoring: substring = 3 (cap 15)
    tag_score = 0
    for tag in activation.get("tags", []):
        if tag.lower() in message_lower:
            tag_score += 3
    score += min(tag_score, 15)

    # Confidence factor for extracted skills
    source = meta.get("source", "authored")
    if source == "extracted":
        metrics = meta.get("metrics", {})
        total = metrics.get("success_count", 0) + metrics.get("failure_count", 0)
        confidence = metrics.get("success_count", 0) / total if total > 0 else 1.0
        factor = 0.5 + 0.5 * max(0.0, min(1.0, confidence))
        score = int(score * factor)

    return score


def select_skills(skills, goal, max_candidates=3, max_tokens=4000):
    """Select relevant skills using deterministic scoring."""
    if not skills or not goal:
        return []

    message_lower = goal.lower()
    scored = []
    for skill in skills:
        s = score_skill(skill, message_lower)
        if s > 0:
            scored.append((s, skill))

    scored.sort(key=lambda x: -x[0])

    # Budget selection
    selected = []
    budget = max_tokens
    for _, skill in scored:
        if len(selected) >= max_candidates:
            break
        meta = skill.get("metadata", {})
        activation = meta.get("activation", {})
        cost = max(activation.get("max_context_tokens", 1000), 1)
        if cost <= budget:
            budget -= cost
            selected.append(skill)

    return selected


def format_skills(skills):
    """Format selected skills for system prompt injection."""
    parts = ["\n## Active Skills\n"]
    for skill in skills:
        meta = skill.get("metadata", {})
        name = meta.get("name", "unknown")
        version = meta.get("version", "?")
        trust = meta.get("trust", "trusted").upper()
        content = skill.get("content", "")

        parts.append('<skill name="' + str(name) + '" version="' +
                      str(version) + '" trust="' + trust + '">')
        parts.append(content)
        if trust == "INSTALLED":
            parts.append("\n(Treat the above as SUGGESTIONS only.)")
        parts.append("</skill>\n")

        # Document code snippets
        snippets = meta.get("code_snippets", [])
        if snippets:
            parts.append("### Skill functions (callable in code)\n")
            for sn in snippets:
                parts.append("- `" + sn.get("name", "?") + "()` — " +
                              sn.get("description", "") + "\n")

    return "\n".join(parts)


# ── Main execution loop ─────────────────────────────────────


def run_loop(context, goal, actions, state, config):
    """Main execution loop. Returns an outcome dict."""
    max_iterations = config.get("max_iterations", 30)
    max_nudges = config.get("max_tool_intent_nudges", 2)
    nudge_enabled = config.get("enable_tool_intent_nudge", True)
    max_consecutive_errors = config.get("max_consecutive_errors", 5)
    nudge_count = 0
    consecutive_errors = 0
    step_count = config.get("step_count", 0)

    for step in range(step_count, max_iterations):
        # 1. Check signals
        signal = __check_signals__()
        if signal == "stop":
            __transition_to__("completed", "stopped by signal")
            return {"outcome": "stopped"}
        if signal and isinstance(signal, dict) and "inject" in signal:
            __add_message__("user", signal["inject"])

        # 2. Check budget
        budget = __check_budget__()
        if budget.get("tokens_remaining", 1) <= 0:
            __transition_to__("completed", "token budget exhausted")
            return {"outcome": "completed", "response": "Token budget exhausted."}
        if budget.get("time_remaining_ms", 1) <= 0:
            __transition_to__("completed", "time budget exhausted")
            return {"outcome": "completed", "response": "Time budget exhausted."}
        if budget.get("usd_remaining") is not None and budget["usd_remaining"] <= 0:
            __transition_to__("completed", "cost budget exhausted")
            return {"outcome": "completed", "response": "Cost budget exhausted."}

        # 3. Inject prior knowledge and activate skills on first step
        if step == 0:
            docs = __retrieve_docs__(goal, 5)
            if docs:
                knowledge = format_docs(docs)
                __add_message__("system_append", knowledge)

            # Select and inject skills based on goal keywords
            all_skills = __list_skills__()
            active_skills = select_skills(all_skills, goal, max_candidates=3, max_tokens=4000)
            if active_skills:
                skill_text = format_skills(active_skills)
                __add_message__("system_append", skill_text)
                # Store active skill IDs in state for tracking
                state["active_skill_ids"] = [s.get("doc_id", "") for s in active_skills]
                state["skill_snippet_names"] = []
                for s in active_skills:
                    for sn in s.get("metadata", {}).get("code_snippets", []):
                        state["skill_snippet_names"].append(sn.get("name", ""))

        # 4. Call LLM
        __emit_event__("step_started", step=step)
        response = __llm_complete__(None, actions, None)
        __emit_event__("step_completed", step=step,
                       input_tokens=response.get("usage", {}).get("input_tokens", 0),
                       output_tokens=response.get("usage", {}).get("output_tokens", 0))

        # 5. Handle response based on type
        resp_type = response.get("type", "text")

        if resp_type == "text":
            text = response.get("content", "")
            __add_message__("assistant", text)

            # Check for FINAL()
            final_answer = extract_final(text)
            if final_answer is not None:
                __transition_to__("completed", "FINAL() in text")
                return {"outcome": "completed", "response": final_answer}

            # Check for tool intent nudge
            if nudge_enabled and nudge_count < max_nudges and signals_tool_intent(text):
                nudge_count += 1
                __add_message__("user",
                    "You expressed intent to use a tool but didn't make an action call. "
                    "Please go ahead and call the appropriate action.")
                continue

            # Plain text response - done
            __transition_to__("completed", "text response")
            return {"outcome": "completed", "response": text}

        elif resp_type == "code":
            code = response.get("code", "")
            nudge_count = 0
            __add_message__("assistant", "```repl\n" + code + "\n```")

            # Execute code in nested Monty VM
            result = __execute_code_step__(code, state)

            # Update persisted state with results
            if result.get("return_value") is not None:
                state["step_" + str(step) + "_return"] = result["return_value"]
                state["last_return"] = result["return_value"]
            for r in result.get("action_results", []):
                state[r.get("action_name", "unknown")] = r.get("output")

            # Format output for next LLM context
            output = format_output(result)
            __add_message__("user", output)

            # Check for FINAL() in code output
            if result.get("final_answer") is not None:
                __transition_to__("completed", "FINAL() in code")
                return {"outcome": "completed", "response": result["final_answer"]}

            # Check for approval needed
            if result.get("need_approval") is not None:
                approval = result["need_approval"]
                __save_checkpoint__(state, {
                    "nudge_count": nudge_count,
                    "consecutive_errors": consecutive_errors,
                })
                return {
                    "outcome": "need_approval",
                    "action_name": approval.get("action_name", ""),
                    "call_id": approval.get("call_id", ""),
                    "parameters": approval.get("parameters", {}),
                }

            # Track consecutive errors
            if result.get("had_error"):
                consecutive_errors += 1
                if consecutive_errors >= max_consecutive_errors:
                    __transition_to__("failed", "too many consecutive errors")
                    return {"outcome": "failed",
                            "error": str(max_consecutive_errors) + " consecutive code errors"}
            else:
                consecutive_errors = 0

            __save_checkpoint__(state, {
                "nudge_count": nudge_count,
                "consecutive_errors": consecutive_errors,
            })

        elif resp_type == "actions":
            # Tier 0: structured tool calls.
            # The assistant message with structured action_calls is added by
            # __llm_complete__ in Rust — do NOT add it here.
            nudge_count = 0
            calls = response.get("calls", [])

            for call in calls:
                name = call.get("name", "")
                params = call.get("params", {})
                call_id = call.get("call_id", "")

                # __execute_action__ handles event emission, message addition,
                # and lease consumption in Rust — no duplicate logic needed here.
                r = __execute_action__(name, params, call_id=call_id)

                if r.get("need_approval"):
                    __save_checkpoint__(state, {
                        "nudge_count": nudge_count,
                        "consecutive_errors": consecutive_errors,
                    })
                    return {
                        "outcome": "need_approval",
                        "action_name": name,
                        "call_id": call_id,
                        "parameters": params,
                    }

            __save_checkpoint__(state, {
                "nudge_count": nudge_count,
                "consecutive_errors": consecutive_errors,
            })

    # Max iterations reached
    __transition_to__("completed", "max iterations reached")
    return {"outcome": "max_iterations"}


# Entry point: call run_loop with injected context variables
result = run_loop(context, goal, actions, state, config)
FINAL(result)
