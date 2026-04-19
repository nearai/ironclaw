"""Mock OpenAI-compatible LLM server for E2E tests.

Serves OpenAI-compatible endpoints for chat completions and model listing.
Supports both streaming and non-streaming responses, plus function calling
via TOOL_CALL_PATTERNS.
"""

import argparse
import asyncio
import json
import re
import time
import uuid
from aiohttp import web

CANNED_RESPONSES = [
    (re.compile(r"empty routine response", re.IGNORECASE), ""),
    (re.compile(r"hello|hi|hey", re.IGNORECASE), "Hello! How can I help you today?"),
    (re.compile(r"2\s*\+\s*2|two plus two", re.IGNORECASE), "The answer is 4."),
    (
        re.compile(r"Tool `gmail` returned:.*Quarterly update", re.IGNORECASE | re.DOTALL),
        "You have one unread Gmail message from ceo@example.com: Quarterly update.",
    ),
    (
        re.compile(r"Tool `http` returned:.*Budget Q1\.xlsx", re.IGNORECASE | re.DOTALL),
        "I found these Google Drive files: Budget Q1.xlsx and Roadmap.md.",
    ),
    (
        re.compile(r"Tool `mock_mcp_mock_search` returned:", re.IGNORECASE | re.DOTALL),
        "Mock MCP search completed successfully.",
    ),
    (re.compile(r"skill|install", re.IGNORECASE), "I can help you with skills management."),
    (re.compile(r"html.?test|injection.?test", re.IGNORECASE),
     'Here is some content: <script>alert("xss")</script> and <img src=x onerror="alert(1)">'
     ' and <iframe src="javascript:alert(2)"></iframe> end of content.'),
    (re.compile(r"long response", re.IGNORECASE),
     ("First paragraph. " * 200 + "\n\n" + "Second paragraph. " * 200)),
    # For tool intent nudge test: first response expresses intent without tool call
    (re.compile(r"search intent", re.IGNORECASE),
     "Let me search for that information now."),
    # After the orchestrator sends its nudge, recover with a final completion.
    # The exact nudge prefix is "You said you would perform an action..." —
    # see `signals_tool_intent` + the nudge append in
    # `crates/ironclaw_engine/orchestrator/default.py`. Match either the new
    # phrasing or the legacy "You expressed intent" so older deployments
    # still work.
    (re.compile(r"You said you would perform an action|You expressed intent", re.IGNORECASE),
     "I found the information you requested."),
]
DEFAULT_RESPONSE = "I understand your request."

TOOL_FAILURE_TRIGGER = re.compile(r"issue 1780 tool failure", re.IGNORECASE)
TRUNCATED_TOOL_CALL_TRIGGER = re.compile(
    r"issue 1780 truncated tool call",
    re.IGNORECASE,
)
EMPTY_REPLY_TRIGGER = re.compile(r"issue 1780 empty reply", re.IGNORECASE)
LOOP_FOREVER_TRIGGER = re.compile(r"issue 1780 loop forever", re.IGNORECASE)

TOOL_CALL_PATTERNS = [
    (re.compile(r"echo (.+)", re.IGNORECASE), "echo", lambda m: {"message": m.group(1)}),
    (
        re.compile(r"loop until cap", re.IGNORECASE),
        "echo",
        lambda _: {"message": "loop-until-cap"},
    ),
    (
        re.compile(r"make approval post (?P<label>[a-z0-9_-]+)", re.IGNORECASE),
        "http",
        lambda m: {
            "method": "POST",
            "url": f"https://example.com/{m.group('label')}",
            "body": {"label": m.group("label")},
        },
    ),
    (
        re.compile(r"check gmail unread|gmail unread", re.IGNORECASE),
        "gmail",
        lambda _: {
            "action": "list_messages",
            "query": "is:unread",
            "max_results": 1,
        },
    ),
    (
        re.compile(r"fetch latest news|latest news", re.IGNORECASE),
        "web-search",
        lambda _: {
            "query": "latest news",
            "count": 5,
        },
    ),
    (
        re.compile(r"check mock mcp|mock mcp search", re.IGNORECASE),
        "mock_mcp_mock_search",
        lambda _: {"query": "refresh-check"},
    ),
    (re.compile(r"what time|current time", re.IGNORECASE), "time", lambda _: {"operation": "now"}),
    (
        re.compile(
            r"create lightweight owner routine (?P<name>[a-z0-9][a-z0-9_-]*)",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Owner-scope routine {m.group('name')}",
            "trigger_type": "manual",
            "prompt": f"Confirm that {m.group('name')} executed.",
            "action_type": "lightweight",
            "use_tools": False,
        },
    ),
    (
        re.compile(
            r"create failing lightweight owner routine (?P<name>[a-z0-9][a-z0-9_-]*)",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Failing lightweight routine {m.group('name')}",
            "trigger_type": "manual",
            "prompt": f"Empty routine response for {m.group('name')}.",
            "action_type": "lightweight",
            "use_tools": False,
        },
    ),
    (
        re.compile(
            r"create full[- ]job owner routine (?P<name>[a-z0-9][a-z0-9_-]*)",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Owner-scope full-job routine {m.group('name')}",
            "trigger_type": "manual",
            "prompt": f"Complete the routine job for {m.group('name')}.",
            "action_type": "full_job",
        },
    ),
    (
        re.compile(
            r"create looping full[- ]job owner routine (?P<name>[a-z0-9][a-z0-9_-]*)",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Looping full-job routine {m.group('name')}",
            "trigger_type": "manual",
            "prompt": f"Loop until cap for {m.group('name')}.",
            "action_type": "full_job",
            "max_iterations": 1,
        },
    ),
    (
        re.compile(
            r"create cron owner routine (?P<name>[a-z0-9][a-z0-9_-]*)",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Cron routine {m.group('name')}",
            "trigger_type": "cron",
            "schedule": "0 */5 * * * *",
            "timezone": "UTC",
            "prompt": f"Confirm that cron routine {m.group('name')} executed.",
            "action_type": "lightweight",
            "use_tools": False,
        },
    ),
    (
        re.compile(
            r"create event routine (?P<name>[a-z0-9][a-z0-9_-]*) "
            r"channel (?P<channel>[a-z0-9_-]+) pattern (?P<pattern>[a-z0-9_|-]+)"
            r"(?: cooldown (?P<cooldown>\d+))?",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Event routine {m.group('name')}",
            "trigger_type": "event",
            "event_channel": None if m.group("channel").lower() == "any" else m.group("channel"),
            "event_pattern": m.group("pattern"),
            "prompt": f"Acknowledge that {m.group('name')} fired.",
            "action_type": "lightweight",
            "use_tools": False,
            "cooldown_secs": int(m.group("cooldown") or 0),
        },
    ),
    (
        re.compile(r"list owner routines", re.IGNORECASE),
        "routine_list",
        lambda _: {},
    ),
    (
        re.compile(r"list.*issues.*(?:nearai|ironclaw)|github.*issues", re.IGNORECASE),
        "http",
        lambda _: {
            "method": "GET",
            "url": f"{_github_api_url}/repos/nearai/ironclaw/issues?per_page=5",
        },
    ),
    # For max iterations test: always returns a tool call, never FINAL
    (
        re.compile(r"loop forever", re.IGNORECASE),
        "echo",
        lambda _: {"message": "iteration continues"},
    ),
    # For google drive API test
    (
        re.compile(r"list.*(?:google|drive).*files|show.*drive", re.IGNORECASE),
        "http",
        lambda _: {
            "method": "GET",
            "url": f"{_github_api_url}/drive/v3/files",
        },
    ),
    # Plan mode: create a plan → calls plan_update tool with draft checklist
    (
        re.compile(r"\[PLAN MODE\].*create.*plan", re.IGNORECASE),
        "plan_update",
        lambda _: {
            "plan_id": "test-plan-001",
            "title": "Test Execution Plan",
            "status": "draft",
            "steps": [
                {"title": "Analyze requirements", "status": "pending"},
                {"title": "Implement changes", "status": "pending"},
                {"title": "Run verification", "status": "pending"},
            ],
        },
    ),
    # Plan mode: approve → calls plan_update with executing status
    (
        re.compile(r"\[PLAN MODE\].*approve", re.IGNORECASE),
        "plan_update",
        lambda _: {
            "plan_id": "test-plan-001",
            "title": "Test Execution Plan",
            "status": "executing",
            "steps": [
                {"title": "Analyze requirements", "status": "in_progress"},
                {"title": "Implement changes", "status": "pending"},
                {"title": "Run verification", "status": "pending"},
            ],
            "mission_id": "00000000-0000-0000-0000-000000000001",
        },
    ),
    # Plan mode: status → calls plan_update to refresh UI
    (
        re.compile(r"\[PLAN MODE\].*(?:status|show status)", re.IGNORECASE),
        "plan_update",
        lambda _: {
            "plan_id": "test-plan-001",
            "title": "Test Execution Plan",
            "status": "executing",
            "steps": [
                {"title": "Analyze requirements", "status": "completed", "result": "No issues found"},
                {"title": "Implement changes", "status": "in_progress"},
                {"title": "Run verification", "status": "pending"},
            ],
            "mission_id": "00000000-0000-0000-0000-000000000001",
        },
    ),
    # ---- Frontend customization via chat (PR #1725 widget system) ----
    #
    # These triggers let an E2E Playwright scenario drive the agent into
    # writing layout / widget files into the workspace via the
    # ``memory_write`` tool. The customization patterns intentionally
    # exercise *multiple* tool calls per assistant turn so the v2 engine /
    # CodeAct dispatch paths get covered: real models often emit several
    # tool calls in one response, and pinning the tests to a single call
    # per turn would silently bypass that codepath.
    #
    # The args function returns a ``list[dict]`` to opt into multi-call
    # mode (see ``_normalize_tool_calls``). Each item supplies its own
    # ``tool_name`` so a single trigger can fan out across different
    # tools if needed; here every call is a ``memory_write`` writing one
    # file under ``.system/gateway/``.
    #
    # 1. Move the top tab bar into a left side panel by writing the
    #    ``.system/gateway/custom.css`` overlay file. The CSS targets the
    #    real DOM nodes (`#app`, `.tab-bar`) defined in
    #    ``crates/ironclaw_gateway/static/index.html``.
    (
        re.compile(r"customize:\s*move tab bar to left", re.IGNORECASE),
        "memory_write",
        lambda _: [{
            "tool_name": "memory_write",
            "arguments": {
                "target": ".system/gateway/custom.css",
                "append": False,
                "force": True,
                "content": (
                    "/* e2e: tab bar to left side panel */\n"
                    "#app { display: flex; flex-direction: row; align-items: stretch; }\n"
                    ".tab-bar {\n"
                    "  flex-direction: column !important;\n"
                    "  width: 220px;\n"
                    "  min-height: 100vh;\n"
                    "  border-right: 1px solid var(--color-border, #333);\n"
                    "  align-items: stretch;\n"
                    "}\n"
                    ".tab-bar button { width: 100%; text-align: left; }\n"
                    ".tab-bar .tab-indicator { display: none; }\n"
                    ".tab-content { flex: 1; }\n"
                ),
            },
        }],
    ),
    # 2. Install a "Skills Viewer" widget in one shot: manifest +
    #    implementation are written by two parallel ``memory_write``
    #    calls in the same assistant turn. This deliberately exercises
    #    multi-tool-call dispatch — the gateway's widget loader requires
    #    both files to be present before the new tab can mount, so the
    #    test would not even reach the assertion phase if the engine
    #    silently dropped the second call.
    (
        re.compile(r"customize:\s*install skills viewer widget", re.IGNORECASE),
        "memory_write",
        lambda _: [
            {
                "tool_name": "memory_write",
                "arguments": {
                    "target": ".system/gateway/widgets/skills-viewer/manifest.json",
                    "append": False,
                    "force": True,
                    "content": json.dumps(
                        {
                            "id": "skills-viewer",
                            "name": "Skills",
                            "slot": "tab",
                            "icon": "📚",
                        },
                        indent=2,
                    ),
                },
            },
            {
                "tool_name": "memory_write",
                "arguments": {
                    "target": ".system/gateway/widgets/skills-viewer/index.js",
                    "append": False,
                    "force": True,
                    "content": (
                "IronClaw.registerWidget({\n"
                "  id: 'skills-viewer',\n"
                "  name: 'Skills',\n"
                "  slot: 'tab',\n"
                "  icon: '📚',\n"
                "  init: async function(container, api) {\n"
                "    container.setAttribute('data-testid', 'skills-viewer-root');\n"
                "    container.innerHTML = '<div class=\"sv-header\">' +\n"
                "      '<h2 data-testid=\"skills-viewer-title\">Workspace Skills</h2>' +\n"
                "      '</div>' +\n"
                "      '<div class=\"sv-list\" data-testid=\"skills-viewer-list\">' +\n"
                "      '<div data-testid=\"skills-viewer-status\">Loading…</div>' +\n"
                "      '</div>';\n"
                "    var listEl = container.querySelector('[data-testid=\"skills-viewer-list\"]');\n"
                "    try {\n"
                "      var resp = await api.fetch('/api/skills');\n"
                "      var data = await resp.json();\n"
                "      var skills = (data && data.skills) || [];\n"
                "      if (!skills.length) {\n"
                "        listEl.innerHTML = '<div data-testid=\"skills-viewer-empty\">' +\n"
                "          'No skills installed yet.' +\n"
                "          '</div>';\n"
                "        return;\n"
                "      }\n"
                "      listEl.innerHTML = '';\n"
                "      skills.forEach(function(s) {\n"
                "        var card = document.createElement('div');\n"
                "        card.className = 'sv-card';\n"
                "        card.dataset.testid = 'skills-viewer-card';\n"
                "        card.dataset.skillName = s.name || '';\n"
                "        var title = document.createElement('div');\n"
                "        title.className = 'sv-card-title';\n"
                "        title.textContent = s.name || '(unnamed)';\n"
                "        var body = document.createElement('pre');\n"
                "        body.className = 'sv-card-body';\n"
                "        body.textContent = (s.description || '').slice(0, 200);\n"
                "        var edit = document.createElement('button');\n"
                "        edit.type = 'button';\n"
                "        edit.className = 'sv-card-edit';\n"
                "        edit.dataset.testid = 'skills-viewer-edit';\n"
                "        edit.textContent = 'Edit';\n"
                "        card.appendChild(title);\n"
                "        card.appendChild(body);\n"
                "        card.appendChild(edit);\n"
                "        listEl.appendChild(card);\n"
                "      });\n"
                "    } catch (e) {\n"
                "      listEl.innerHTML = '<div data-testid=\"skills-viewer-error\">' +\n"
                "        'Failed to load skills: ' + (e && e.message ? e.message : e) +\n"
                "        '</div>';\n"
                        "    }\n"
                        "  }\n"
                        "});\n"
                    ),
                },
            },
        ],
    ),
]


# Runtime-configurable mock API URL for github tool call tests.
# Set via POST /__mock/set_github_api_url with {"url": "http://..."}
_github_api_url: str = "https://api.github.com"
_last_chat_request: dict | None = None


def _new_oauth_state() -> dict:
    return {
        "exchange_count": 0,
        "refresh_count": 0,
        "last_exchange": None,
        "last_refresh": None,
    }


def _message_text(msg: dict) -> str:
    content = msg.get("content") or ""
    if isinstance(content, list):
        content = " ".join(
            p.get("text") or "" for p in content if p.get("type") == "text"
        )
    return content


def _last_user_content(messages: list[dict]) -> str:
    for msg in reversed(messages):
        if msg.get("role") == "user":
            return _message_text(msg)
    return ""


def _extract_resumed_action_result(last_user: str) -> tuple[str, str] | None:
    prefix = "The pending action '"
    marker = "Continue from this result:\n"
    if not last_user.startswith(prefix) or marker not in last_user:
        return None
    rest = last_user[len(prefix):]
    action_name, _, tail = rest.partition("' has already been executed.")
    if not action_name or not tail:
        return None
    _, _, rendered = last_user.partition(marker)
    rendered = rendered.strip()
    if not rendered:
        return None
    return action_name, rendered


def _resumed_action_summary(messages: list[dict]) -> str | None:
    resumed = _extract_resumed_action_result(_last_user_content(messages))
    if not resumed:
        return None
    action_name, rendered = resumed
    return f"The {action_name} tool returned: {rendered}"

def _conversation_has_user_trigger(messages: list[dict], pattern: re.Pattern[str]) -> bool:
    for msg in messages:
        if msg.get("role") == "user" and pattern.search(_message_text(msg)):
            return True
    return False


def _job_contains_marker(messages: list[dict], marker: str) -> bool:
    marker_lower = marker.lower()
    for msg in messages:
        if msg.get("role") != "user":
            continue
        if marker_lower in _message_text(msg).lower():
            return True
    return False


def _is_job_mode(messages: list[dict]) -> bool:
    """Detect if this conversation is a background job (not chat)."""
    for msg in messages:
        if msg.get("role") == "system":
            content = msg.get("content", "")
            if "autonomous agent working on a job" in content:
                return True
    return False


def _count_tool_results(messages: list[dict]) -> int:
    """Count how many tool result messages are in the conversation."""
    return sum(1 for m in messages if m.get("role") == "tool")


def match_job_response(messages: list[dict], has_tools: bool) -> dict | None:
    """Handle background job conversations.

    Returns a dict with either {"text": ...} or {"tool_call": ...},
    or None if this isn't a job conversation.
    """
    if not _is_job_mode(messages):
        return None

    last_user = _last_user_content(messages)
    tool_result_count = _count_tool_results(messages)
    loop_until_cap = _job_contains_marker(messages, "loop until cap")

    if loop_until_cap and "create a plan" in last_user.lower():
        return {"text": json.dumps({
            "goal": "Keep iterating until the worker hits the iteration cap",
            "actions": [],
            "estimated_cost": 0.001,
            "estimated_time_secs": 5,
            "confidence": 0.8,
        })}

    if loop_until_cap and "all planned actions have been executed" in last_user.lower():
        return {"text": "loop until cap still requires more work"}

    if loop_until_cap and has_tools:
        return {"tool_call": {
            "tool_name": "echo",
            "arguments": {"message": "loop-until-cap"},
        }}

    # Planning call (no tools available = complete() not complete_with_tools())
    if "create a plan" in last_user.lower():
        return {"text": json.dumps({
            "goal": "Complete the requested routine job",
            "actions": [
                {
                    "tool_name": "echo",
                    "parameters": {"message": "job-step-1"},
                    "reasoning": "First step: echo a test message",
                    "expected_outcome": "Echo returns the message",
                },
                {
                    "tool_name": "time",
                    "parameters": {"operation": "now"},
                    "reasoning": "Second step: get the current time",
                    "expected_outcome": "Returns current timestamp",
                },
            ],
            "estimated_cost": 0.001,
            "estimated_time_secs": 5,
            "confidence": 0.95,
        })}

    # Post-plan completion check: after tool results, say complete
    if "planned actions" in last_user.lower() and tool_result_count >= 2:
        return {"text": "The job is complete. All tasks are done."}

    # Continuation prompt (from our fix): the plan didn't fully complete,
    # now the agentic loop should call tools
    if "continue executing now" in last_user.lower() and has_tools:
        return {"tool_call": {
            "tool_name": "echo",
            "arguments": {"message": "continuation-step"},
        }}

    # After a tool result in the agentic loop, signal completion
    if tool_result_count > 0 and has_tools:
        return {"text": "The job is complete. All requested work has been finished."}

    return None


def match_response(messages: list[dict]) -> str:
    content = _last_user_content(messages)
    resumed = _resumed_action_summary(messages)
    if resumed:
        return resumed
    for pattern, response in CANNED_RESPONSES:
        if pattern.search(content):
            return response
    return DEFAULT_RESPONSE


def _normalize_tool_calls(tool_name: str, value: object) -> list[dict]:
    """Normalize the result of a TOOL_CALL_PATTERNS args function.

    Patterns historically returned a single ``dict`` of arguments and the
    tuple's ``tool_name`` field named the tool. To let one chat turn emit
    multiple tool calls (the v2 engine and CodeAct both dispatch lists of
    tool calls per assistant message), the args function may now return:

    * ``dict`` — a single tool call with the tuple's ``tool_name``.
    * ``list[dict]`` — multiple tool calls. Each item is itself a
      ``{"tool_name": ..., "arguments": ...}`` dict so callers can mix
      different tools in one response. ``tool_name`` falls back to the
      tuple's value when omitted.

    The return value is always a list of ``{"tool_name", "arguments"}``
    dicts so the dispatcher can stay shape-agnostic.
    """
    if isinstance(value, list):
        calls = []
        for index, item in enumerate(value):
            # Defensive: a malformed pattern that returns a list of
            # tuples / strings / None would otherwise crash with an
            # opaque ``AttributeError: 'tuple' object has no attribute
            # 'get'`` deep inside aiohttp's request handler, taking the
            # whole mock server down mid-test. Raise a clear TypeError
            # at the offending call site so the failing pattern is
            # obvious from the traceback.
            if not isinstance(item, dict):
                raise TypeError(
                    f"_normalize_tool_calls: TOOL_CALL_PATTERNS entry for "
                    f"{tool_name!r} returned a list whose element [{index}] "
                    f"is {type(item).__name__}, expected dict. "
                    f"Each multi-call entry must be "
                    f'{{"tool_name": ..., "arguments": ...}}.'
                )
            arguments = item.get("arguments", {})
            if not isinstance(arguments, dict):
                raise TypeError(
                    f"_normalize_tool_calls: TOOL_CALL_PATTERNS entry for "
                    f"{tool_name!r} element [{index}] has "
                    f"arguments={type(arguments).__name__}, expected dict."
                )
            calls.append({
                "tool_name": item.get("tool_name", tool_name),
                "arguments": arguments,
            })
        return calls
    # Single-call form: the legacy contract is that the args function
    # returns a dict directly. Anything else (a tuple, a string, None)
    # is a pattern bug — fail loudly here too rather than letting the
    # bad shape ride through to the dispatcher.
    if not isinstance(value, dict):
        raise TypeError(
            f"_normalize_tool_calls: TOOL_CALL_PATTERNS entry for "
            f"{tool_name!r} returned {type(value).__name__}, expected "
            "dict (single tool call) or list[dict] (multi-call)."
        )
    return [{"tool_name": tool_name, "arguments": value}]


def match_tool_call(messages: list[dict], has_tools: bool) -> list[dict] | None:
    """Return the list of tool calls to emit for the latest user message.

    Returns ``None`` when no pattern matches or when the request did not
    advertise any tools (so the LLM should fall back to a text response).
    Otherwise returns a non-empty list of ``{"tool_name", "arguments"}``
    dicts — single-call patterns produce a one-element list, multi-call
    patterns can produce many.
    """
    if not has_tools:
        return None
    content = _last_user_content(messages)
    for pattern, tool_name, args_fn in TOOL_CALL_PATTERNS:
        m = pattern.search(content)
        if m:
            return _normalize_tool_calls(tool_name, args_fn(m))
    return None


def _extract_tool_name(msg: dict) -> str:
    """Extract tool name from a message, checking both 'name' field and XML content."""
    name = msg.get("name")
    if name:
        return name
    # ironclaw wraps tool output as <tool_output name="...">
    content = msg.get("content", "")
    m = re.search(r'<tool_output\s+name="([^"]+)"', content)
    if m:
        return m.group(1)
    return "unknown"


def _find_tool_results(messages: list[dict]) -> list[dict]:
    """Collect every fresh tool result that follows the most recent user turn.

    A single assistant turn can dispatch *several* tool calls (the v2 engine
    fans them out in parallel and CodeAct can call multiple Python helpers
    per script), so the LLM may be invoked again with a tail of multiple
    ``role: tool`` messages. Returning all of them lets the summary path
    acknowledge each result instead of dropping all but the first.
    """
    last_user_idx = -1
    for i in range(len(messages) - 1, -1, -1):
        if messages[i].get("role") == "user":
            last_user_idx = i
            break

    results: list[dict] = []
    for i in range(last_user_idx + 1, len(messages)):
        if messages[i].get("role") == "tool":
            results.append({
                "name": _extract_tool_name(messages[i]),
                "content": messages[i].get("content", ""),
            })
    return results


def _find_tool_result(messages: list[dict]) -> dict | None:
    """Backward-compat single-result helper used by the special-response path."""
    results = _find_tool_results(messages)
    return results[0] if results else None


def _make_base(completion_id: str) -> dict:
    return {"id": completion_id, "object": "chat.completion.chunk",
            "created": int(time.time()), "model": "mock-model"}


async def _send_sse(resp: web.StreamResponse, data: dict):
    await resp.write(f"data: {json.dumps(data)}\n\n".encode())


def match_special_response(messages: list[dict], has_tools: bool) -> dict | None:
    """Deterministic issue-specific responses for agent-loop recovery tests."""
    last_user = _last_user_content(messages)

    if _conversation_has_user_trigger(messages, LOOP_FOREVER_TRIGGER):
        if has_tools:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "echo",
                    "arguments": {"message": "loop-iteration"},
                },
            }
        return {
            "type": "text",
            "text": "Recovered after hitting the tool iteration limit.",
        }

    if _conversation_has_user_trigger(messages, TRUNCATED_TOOL_CALL_TRIGGER):
        if TRUNCATED_TOOL_CALL_TRIGGER.search(last_user) and has_tools:
            return {
                "type": "truncated_tool_call",
                "tool_call": {
                    "tool_name": "time",
                    "arguments": {},
                },
                "content": "Attempting a tool call but the response was truncated.",
            }
        return {
            "type": "text",
            "text": "Recovered after discarding a truncated tool call.",
        }

    if TOOL_FAILURE_TRIGGER.search(last_user) and has_tools:
        return {
            "type": "tool_call",
            "tool_call": {
                "tool_name": "time",
                "arguments": {"operation": "broken-operation"},
            },
        }

    if EMPTY_REPLY_TRIGGER.search(last_user):
        return {"type": "empty_text"}

    return None


async def _dispatch_special_response(
    request: web.Request,
    cid: str,
    stream: bool,
    special: dict,
) -> web.StreamResponse | web.Response:
    if special["type"] == "tool_call":
        tc = special["tool_call"]
        if not stream:
            return _tool_call_response(cid, tc)
        return await _stream_tool_call(request, cid, tc)
    if special["type"] == "truncated_tool_call":
        tc = special["tool_call"]
        content = special["content"]
        if not stream:
            return _truncated_tool_call_response(cid, tc, content)
        return await _stream_truncated_tool_call(request, cid, tc, content)
    if special["type"] == "empty_text":
        if not stream:
            return _text_response(cid, "")
        return await _stream_text(request, cid, "")

    text = special["text"]
    if not stream:
        return _text_response(cid, text)
    return await _stream_text(request, cid, text)


async def chat_completions(request: web.Request) -> web.StreamResponse:
    """Handle POST /v1/chat/completions and /chat/completions."""
    global _last_chat_request
    body = await request.json()
    _last_chat_request = body
    messages = body.get("messages", [])
    stream = body.get("stream", False)
    has_tools = bool(body.get("tools"))
    cid = f"mock-{uuid.uuid4().hex[:8]}"

    # Job-mode conversations (background routine/job execution)
    job_resp = match_job_response(messages, has_tools)
    if job_resp:
        if "tool_call" in job_resp:
            tc = job_resp["tool_call"]
            if not stream:
                return _tool_call_response(cid, tc)
            return await _stream_tool_call(request, cid, tc)
        text = job_resp["text"]
        if not stream:
            return _text_response(cid, text)
        return await _stream_text(request, cid, text)

    # Special chat-loop recovery cases that intentionally override the normal
    # tool-result summary path (for example, the looping case).
    special = match_special_response(messages, has_tools)
    if special and _conversation_has_user_trigger(messages, LOOP_FOREVER_TRIGGER):
        return await _dispatch_special_response(request, cid, stream, special)

    # Tool result(s) in messages -> text summary covering every fresh result
    tool_results = _find_tool_results(messages)
    if tool_results:
        if len(tool_results) == 1:
            tr = tool_results[0]
            text = f"The {tr['name']} tool returned: {tr['content']}"
        else:
            # Multi-call summary: list every result so tests asserting on a
            # specific tool name still find it, while v2-engine paths that
            # dispatch several tools per turn get an acknowledging response.
            lines = [f"Dispatched {len(tool_results)} tools:"]
            for tr in tool_results:
                lines.append(f"- {tr['name']}: {tr['content']}")
            text = "\n".join(lines)
        if not stream:
            return _text_response(cid, text)
        return await _stream_text(request, cid, text)

    resumed_text = _resumed_action_summary(messages)
    if resumed_text:
        if not stream:
            return _text_response(cid, resumed_text)
        return await _stream_text(request, cid, resumed_text)

    if special:
        return await _dispatch_special_response(request, cid, stream, special)

    # Tool-call pattern match
    tc = match_tool_call(messages, has_tools)
    if tc:
        if not stream:
            return _tool_call_response(cid, tc)
        return await _stream_tool_call(request, cid, tc)

    # Default text response
    text = match_response(messages)
    if not stream:
        return _text_response(cid, text)
    return await _stream_text(request, cid, text)


def _text_response(cid: str, text: str) -> web.Response:
    return web.json_response({
        "id": cid, "object": "chat.completion", "created": int(time.time()),
        "model": "mock-model",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": text},
                      "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": len(text.split()), "total_tokens": 15},
    })


def _tool_call_response(cid: str, calls: list[dict] | dict) -> web.Response:
    """Build a non-streaming chat completion containing one or more tool calls.

    Accepts either a single ``{"tool_name", "arguments"}`` dict (legacy
    callers) or a list of them (the new multi-call shape used by the v2
    engine / CodeAct test paths).
    """
    if isinstance(calls, dict):
        calls = [calls]
    tool_calls = [
        {
            "id": f"call_{uuid.uuid4().hex[:8]}",
            "type": "function",
            "function": {
                "name": tc["tool_name"],
                "arguments": json.dumps(tc["arguments"]),
            },
        }
        for tc in calls
    ]
    return web.json_response({
        "id": cid, "object": "chat.completion", "created": int(time.time()),
        "model": "mock-model",
        "choices": [{"index": 0, "message": {
            "role": "assistant", "content": None,
            "tool_calls": tool_calls,
        }, "finish_reason": "tool_calls"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
    })


def _truncated_tool_call_response(cid: str, tc: dict, content: str) -> web.Response:
    tool_tag = json.dumps({
        "name": tc["tool_name"],
        "arguments": tc["arguments"],
    })
    return web.json_response({
        "id": cid,
        "object": "chat.completion",
        "created": int(time.time()),
        "model": "mock-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": f"{content}\n<tool_call>{tool_tag}</tool_call>",
            },
            "finish_reason": "length",
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
    })


async def _stream_text(request: web.Request, cid: str, text: str) -> web.StreamResponse:
    resp = web.StreamResponse(status=200, headers={
        "Content-Type": "text/event-stream", "Cache-Control": "no-cache"})
    await resp.prepare(request)
    base = _make_base(cid)
    chunk = {**base, "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""},
                                   "finish_reason": None}]}
    await _send_sse(resp, chunk)
    for i, word in enumerate(text.split(" ")):
        chunk["choices"][0]["delta"] = {"content": word if i == 0 else f" {word}"}
        await _send_sse(resp, chunk)
    chunk["choices"][0]["delta"] = {}
    chunk["choices"][0]["finish_reason"] = "stop"
    await _send_sse(resp, chunk)
    await resp.write(b"data: [DONE]\n\n")
    return resp


async def _stream_tool_call(
    request: web.Request,
    cid: str,
    calls: list[dict] | dict,
) -> web.StreamResponse:
    """Stream a chat completion containing one or more tool calls.

    Each tool call gets its own ``index`` in the OpenAI delta protocol,
    so accumulating clients build up a list of distinct calls. We emit
    a header chunk and an arguments chunk per tool call so streaming
    parsers exercise their per-index merging logic the same way real
    providers force them to.
    """
    if isinstance(calls, dict):
        calls = [calls]
    resp = web.StreamResponse(status=200, headers={
        "Content-Type": "text/event-stream", "Cache-Control": "no-cache"})
    await resp.prepare(request)
    base = _make_base(cid)
    for idx, tc in enumerate(calls):
        call_id = f"call_{uuid.uuid4().hex[:8]}"
        # Header chunk: declare a new tool call slot at this index. Only
        # the very first chunk in the stream needs the assistant role.
        delta: dict = {
            "tool_calls": [{
                "index": idx,
                "id": call_id,
                "type": "function",
                "function": {"name": tc["tool_name"], "arguments": ""},
            }],
        }
        if idx == 0:
            delta["role"] = "assistant"
        chunk = {**base, "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": None,
        }]}
        await _send_sse(resp, chunk)
        # Arguments chunk: bind the JSON payload to the same index.
        chunk["choices"][0]["delta"] = {
            "tool_calls": [{
                "index": idx,
                "function": {"arguments": json.dumps(tc["arguments"])},
            }],
        }
        await _send_sse(resp, chunk)
    # Final chunk: finish reason terminates the assistant turn.
    final = {**base, "choices": [{
        "index": 0,
        "delta": {},
        "finish_reason": "tool_calls",
    }]}
    await _send_sse(resp, final)
    await resp.write(b"data: [DONE]\n\n")
    return resp


async def _stream_truncated_tool_call(
    request: web.Request,
    cid: str,
    tc: dict,
    content: str,
) -> web.StreamResponse:
    resp = web.StreamResponse(status=200, headers={
        "Content-Type": "text/event-stream", "Cache-Control": "no-cache"})
    await resp.prepare(request)
    base = _make_base(cid)
    tool_tag = json.dumps({
        "name": tc["tool_name"],
        "arguments": tc["arguments"],
    })

    chunk = {**base, "choices": [{"index": 0, "delta": {
        "role": "assistant",
        "content": content,
    }, "finish_reason": None}]}
    await _send_sse(resp, chunk)

    chunk["choices"][0]["delta"] = {
        "content": f"\n<tool_call>{tool_tag}</tool_call>",
    }
    await _send_sse(resp, chunk)

    chunk["choices"][0]["delta"] = {}
    chunk["choices"][0]["finish_reason"] = "length"
    await _send_sse(resp, chunk)
    await resp.write(b"data: [DONE]\n\n")
    return resp


async def oauth_exchange(request: web.Request) -> web.Response:
    """Mock OAuth token exchange proxy for E2E tests.

    Accepts the generic hosted OAuth proxy contract used by IronClaw and
    returns a fake token response. MCP callback tests assert that provider-
    specific token params such as RFC 8707 `resource` are forwarded here.
    """
    data = await request.post()
    oauth_state = request.app["oauth_state"]
    oauth_state["exchange_count"] += 1
    oauth_state["last_exchange"] = {
        "authorization": request.headers.get("Authorization"),
        "form": dict(data),
    }
    code = data.get("code", "")
    access_token_field = data.get("access_token_field", "access_token")

    if code == "mock_mcp_code":
        if not data.get("token_url", "").endswith("/oauth/token"):
            return web.json_response({"error": "missing_token_url"}, status=400)
        if not data.get("client_id"):
            return web.json_response({"error": "missing_client_id"}, status=400)
        if not data.get("resource"):
            return web.json_response({"error": "missing_resource"}, status=400)

    return web.json_response({
        access_token_field: f"mock-token-{code}",
        "refresh_token": "mock-refresh-token",
        "expires_in": 3600,
    })


async def oauth_refresh(request: web.Request) -> web.Response:
    """Mock OAuth token refresh proxy for hosted refresh E2E tests."""
    data = await request.post()
    oauth_state = request.app["oauth_state"]
    oauth_state["refresh_count"] += 1
    oauth_state["last_refresh"] = {
        "authorization": request.headers.get("Authorization"),
        "form": dict(data),
    }

    if request.headers.get("Authorization") != "Bearer e2e-test-token":
        return web.json_response({"error": "invalid_gateway_auth"}, status=401)

    provider = data.get("provider", "")
    if provider.startswith("mcp:"):
        if data.get("client_id") != "mock-mcp-client-id":
            return web.json_response({"error": "invalid_mcp_client_id"}, status=400)
        if data.get("client_secret") != "mock-mcp-client-secret":
            return web.json_response({"error": "missing_mcp_client_secret"}, status=400)
        if not data.get("token_url", "").endswith("/oauth/token"):
            return web.json_response({"error": "invalid_mcp_token_url"}, status=400)
        if data.get("resource") != f"http://127.0.0.1:{request.app['port']}/mcp":
            return web.json_response({"error": "missing_mcp_resource"}, status=400)
    else:
        if data.get("client_id") != "hosted-google-client-id":
            return web.json_response({"error": "invalid_client_id"}, status=400)
        if "client_secret" in data:
            return web.json_response({"error": "unexpected_client_secret"}, status=400)

    return web.json_response({
        "access_token": "mock-refreshed-access-token",
        "token_type": "Bearer",
        "refresh_token": "mock-rotated-refresh-token",
        "expires_in": 3600,
        "scope": "mock-scope",
    })


async def oauth_state_handler(request: web.Request) -> web.Response:
    return web.json_response(request.app["oauth_state"])


async def oauth_reset(request: web.Request) -> web.Response:
    request.app["oauth_state"] = _new_oauth_state()
    return web.json_response({"ok": True})


async def models(_request: web.Request) -> web.Response:
    return web.json_response({
        "object": "list",
        "data": [{"id": "mock-model", "object": "model", "owned_by": "test"}],
    })


# ── Mock MCP Server ──────────────────────────────────────────────────────────
#
# Simulates an MCP server that requires OAuth.  Unauthenticated requests get
# 401 + WWW-Authenticate (standard MCP flow) or 400 "Authorization header is
# badly formatted" (GitHub-style).  Authenticated requests return valid
# JSON-RPC responses for initialize and tools/list.


async def mcp_endpoint(request: web.Request) -> web.Response:
    """Handle POST /mcp — JSON-RPC MCP endpoint requiring Bearer auth."""
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer ") or len(auth.split(" ", 1)[1].strip()) == 0:
        # Return 401 with WWW-Authenticate header for OAuth discovery
        resource_meta_url = f"http://127.0.0.1:{request.app['port']}/.well-known/oauth-protected-resource"
        return web.Response(
            status=401,
            headers={"WWW-Authenticate": f'Bearer resource_metadata="{resource_meta_url}"'},
            text="Unauthorized",
        )
    return await _mcp_handle_authed(request)


async def mcp_endpoint_400(request: web.Request) -> web.Response:
    """Handle POST /mcp-400 — MCP endpoint that returns 400 (GitHub-style).

    Simulates GitHub's MCP server which returns 400 "Authorization header
    is badly formatted" instead of 401 when auth is missing or invalid.
    """
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer ") or len(auth.split(" ", 1)[1].strip()) == 0:
        return web.Response(
            status=400,
            text="bad request: Authorization header is badly formatted",
        )
    return await _mcp_handle_authed(request)


async def _mcp_handle_authed(request: web.Request) -> web.Response:
    """Handle an authenticated MCP JSON-RPC request."""
    body = await request.json()
    method = body.get("method", "")
    req_id = body.get("id")

    if method == "initialize":
        return web.json_response({
            "jsonrpc": "2.0", "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mock-mcp", "version": "1.0.0"},
            },
        })
    if method == "notifications/initialized":
        return web.json_response({"jsonrpc": "2.0", "id": req_id, "result": {}})
    if method == "tools/list":
        return web.json_response({
            "jsonrpc": "2.0", "id": req_id,
            "result": {"tools": [{
                "name": "mock_search",
                "description": "A mock search tool for testing",
                "inputSchema": {"type": "object", "properties": {
                    "query": {"type": "string"},
                }},
            }]},
        })
    return web.json_response({"jsonrpc": "2.0", "id": req_id, "error": {
        "code": -32601, "message": f"Method not found: {method}",
    }})


async def mcp_protected_resource(request: web.Request) -> web.Response:
    """GET /.well-known/oauth-protected-resource[/{path}] — RFC 9728 discovery.

    Production code appends the MCP server path after the well-known suffix
    (e.g. /.well-known/oauth-protected-resource/mcp-400), so this handler
    accepts an optional tail and returns a resource matching the request.
    """
    port = request.app["port"]
    tail = request.match_info.get("tail", "mcp")
    return web.json_response({
        "resource": f"http://127.0.0.1:{port}/{tail}",
        "authorization_servers": [f"http://127.0.0.1:{port}"],
    })


async def mcp_auth_server_metadata(request: web.Request) -> web.Response:
    """GET /.well-known/oauth-authorization-server[/{path}] — OAuth metadata."""
    port = request.app["port"]
    base = f"http://127.0.0.1:{port}"
    return web.json_response({
        "issuer": base,
        "authorization_endpoint": f"{base}/oauth/authorize",
        "token_endpoint": f"{base}/oauth/token",
        "registration_endpoint": f"{base}/oauth/register",
        "scopes_supported": ["read", "write"],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
    })


async def mcp_oauth_register(request: web.Request) -> web.Response:
    """POST /oauth/register — Dynamic Client Registration."""
    body = await request.json()
    return web.json_response({
        "client_id": "mock-mcp-client-id",
        "client_secret": "mock-mcp-client-secret",
        "client_name": body.get("client_name", "IronClaw"),
        "redirect_uris": body.get("redirect_uris", []),
    })


async def mcp_oauth_token(request: web.Request) -> web.Response:
    """POST /oauth/token — Token endpoint for MCP OAuth."""
    data = await request.post()
    code = data.get("code", "")
    return web.json_response({
        "access_token": f"mcp-token-{code}",
        "token_type": "Bearer",
        "expires_in": 3600,
    })


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()
    app = web.Application()
    app["oauth_state"] = _new_oauth_state()
    # Register both /v1/ and non-/v1/ paths (rig-core omits the /v1/ prefix)
    app.router.add_post("/v1/chat/completions", chat_completions)
    app.router.add_post("/chat/completions", chat_completions)
    app.router.add_get("/v1/models", models)
    app.router.add_get("/models", models)
    app.router.add_post("/oauth/exchange", oauth_exchange)
    app.router.add_post("/oauth/refresh", oauth_refresh)
    app.router.add_get("/__mock/oauth/state", oauth_state_handler)
    app.router.add_post("/__mock/oauth/reset", oauth_reset)

    async def set_github_api_url(request: web.Request) -> web.Response:
        global _github_api_url
        body = await request.json()
        _github_api_url = body["url"]
        return web.json_response({"ok": True, "url": _github_api_url})

    async def get_github_api_url(request: web.Request) -> web.Response:
        return web.json_response({"url": _github_api_url})

    async def get_last_chat_request(request: web.Request) -> web.Response:
        return web.json_response(_last_chat_request or {})

    app.router.add_post("/__mock/set_github_api_url", set_github_api_url)
    app.router.add_get("/__mock/github_api_url", get_github_api_url)
    app.router.add_get("/__mock/last_chat_request", get_last_chat_request)
    # Mock MCP server endpoints
    app.router.add_post("/mcp", mcp_endpoint)
    app.router.add_post("/mcp-400", mcp_endpoint_400)
    app.router.add_get("/.well-known/oauth-protected-resource", mcp_protected_resource)
    app.router.add_get("/.well-known/oauth-protected-resource/{tail:.*}", mcp_protected_resource)
    app.router.add_get("/.well-known/oauth-authorization-server", mcp_auth_server_metadata)
    app.router.add_get("/.well-known/oauth-authorization-server/{tail:.*}", mcp_auth_server_metadata)
    app.router.add_post("/oauth/register", mcp_oauth_register)
    app.router.add_post("/oauth/token", mcp_oauth_token)

    async def start():
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        app["port"] = port  # used by MCP handlers
        print(f"MOCK_LLM_PORT={port}", flush=True)
        await asyncio.Event().wait()

    asyncio.run(start())


if __name__ == "__main__":
    main()
