"""Mock OpenAI-compatible LLM server for E2E tests.

Serves OpenAI-compatible endpoints for chat completions and model listing.
Supports both streaming and non-streaming responses, plus function calling
via TOOL_CALL_PATTERNS.
"""

import argparse
import asyncio
from copy import deepcopy
import json
import os
import re
import time
import uuid
from aiohttp import web

DENIAL_PATTERN = re.compile(
    r"user denied action|user denied tool|denied:\s*",
    re.IGNORECASE,
)

CANNED_RESPONSES = [
    (re.compile(r"empty routine response", re.IGNORECASE), ""),
    # Reborn attachment e2e: the inbound pipeline extracts a document's text
    # and folds it into the model-visible <attachments> block. A unique marker
    # in the uploaded file proves the extracted text reached the prompt.
    (
        re.compile(r"IRONCLAW_ATTACHMENT_MARKER_4644", re.IGNORECASE),
        "I can read the attached document text.",
    ),
    # Markdown link reply for the WebChat v2 "links open in a new tab" smoke.
    # The renderer must add target=_blank to the rendered anchor.
    (re.compile(r"link test", re.IGNORECASE),
     "See [the pull request](https://example.com/pr/1) for details."),
    # Reborn v2 download chips: after the agent writes a CSV and a PDF (the
    # builtin__write_file dispatch lives in TOOL_CALL_PATTERNS), it replies
    # referencing their /workspace paths so the WebUI renders downloadable file
    # chips. Fires after the tool calls run (match_tool_call dedups the
    # already-run writes).
    (
        re.compile(r"produce a downloadable csv and pdf", re.IGNORECASE),
        "Done — I saved /workspace/report.csv and /workspace/report.pdf. "
        "Both are ready to download.",
    ),
    (
        re.compile(r"reborn write approval file (?P<label>[a-z0-9_-]+)", re.IGNORECASE),
        "Done - saved the approval test file.",
    ),
    (
        re.compile(
            r"reborn create automation rename target (?P<label>[a-z0-9_-]+)",
            re.IGNORECASE,
        ),
        "Created the automation for rename testing.",
    ),
    (re.compile(r"\bhello\b|\bhi\b|\bhey\b", re.IGNORECASE), "Hello! How can I help you today?"),
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
        re.compile(
            r"Tool `mock_mcp_mock_search` returned:|The mock_mcp_mock_search tool returned:",
            re.IGNORECASE | re.DOTALL,
        ),
        "Mock MCP search completed successfully.",
    ),
    (
        re.compile(r"Tool `gmail` returned:|The gmail tool returned:", re.IGNORECASE | re.DOTALL),
        "Gmail check completed successfully.",
    ),
    (
        re.compile(r"Tool `google_calendar` returned:", re.IGNORECASE | re.DOTALL),
        "Calendar check completed successfully.",
    ),
    (
        re.compile(r"Tool `github` returned:", re.IGNORECASE | re.DOTALL),
        "GitHub issue lookup completed successfully.",
    ),
    (
        re.compile(r"Tool `notion_notion_search` returned:", re.IGNORECASE | re.DOTALL),
        "Notion search completed successfully.",
    ),
    (re.compile(r"portfolio|defi|rebalance|yield.*positions", re.IGNORECASE),
     "I'll analyze your DeFi portfolio. The portfolio skill is active and I can scan "
     "your wallet addresses across chains to discover positions, check yields, and "
     "suggest rebalancing opportunities."),
    (re.compile(r"0x[a-fA-F0-9]{40}", re.IGNORECASE),
     "I found your wallet address. Let me scan your portfolio across all supported "
     "chains to discover DeFi positions and classify them against known protocols."),
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
EMULATE_GITHUB_BEARER = "ghp_emulate_github_token"
EMULATE_SLACK_BEARER = "emulate-slack-token"


def _new_llm_trace_state() -> dict:
    return {
        "source": None,
        "responses": [],
        "next_response": 0,
        "expected_user_inputs": {},
        "request_hints": [],
        "error": None,
    }


def _parse_llm_trace(trace: object, source: str | None = None) -> dict:
    """Validate a recorded Reborn trace and make it executable by this mock."""
    if not isinstance(trace, dict):
        raise ValueError("trace must be an object")
    steps = trace.get("steps")
    if not isinstance(steps, list) or not steps:
        raise ValueError("trace.steps must be a non-empty list")

    first = steps[0]
    if not isinstance(first, dict) or not isinstance(first.get("response"), dict):
        raise ValueError("trace.steps[0].response must be an object")
    first_response = first["response"]
    if first_response.get("type") != "user_input" or not isinstance(
        first_response.get("content"), str
    ):
        raise ValueError("trace must start with a user_input response")

    responses = []
    expected_user_inputs = {0: first_response["content"]}
    request_hints = []
    pending_user_input = True
    for index, step in enumerate(steps[1:], start=1):
        if not isinstance(step, dict) or not isinstance(step.get("response"), dict):
            raise ValueError(f"trace.steps[{index}].response must be an object")
        response = step["response"]
        response_type = response.get("type")
        if response_type == "user_input":
            if not isinstance(response.get("content"), str):
                raise ValueError(
                    f"trace.steps[{index}] user_input content must be a string"
                )
            if pending_user_input:
                raise ValueError(
                    f"trace.steps[{index}] has consecutive user_input responses"
                )
            expected_user_inputs[len(responses)] = response["content"]
            pending_user_input = True
            continue
        if response_type == "text":
            if not isinstance(response.get("content"), str):
                raise ValueError(f"trace.steps[{index}] text content must be a string")
        elif response_type == "tool_calls":
            tool_calls = response.get("tool_calls")
            if not isinstance(tool_calls, list) or not tool_calls:
                raise ValueError(
                    f"trace.steps[{index}] tool_calls must be a non-empty list"
                )
            for tool_index, tool_call in enumerate(tool_calls):
                if (
                    not isinstance(tool_call, dict)
                    or not isinstance(tool_call.get("name"), str)
                    or not isinstance(tool_call.get("arguments"), dict)
                ):
                    raise ValueError(
                        f"trace.steps[{index}].tool_calls[{tool_index}] is invalid"
                    )
        else:
            raise ValueError(
                f"trace.steps[{index}] has unsupported response type {response_type!r}"
            )
        request_hint = step.get("request_hint", {})
        if not isinstance(request_hint, dict):
            raise ValueError(f"trace.steps[{index}].request_hint must be an object")
        last_user_message_contains = request_hint.get("last_user_message_contains")
        if last_user_message_contains is not None and not isinstance(
            last_user_message_contains, str
        ):
            raise ValueError(
                f"trace.steps[{index}].request_hint.last_user_message_contains "
                "must be a string"
            )
        min_message_count = request_hint.get("min_message_count")
        if min_message_count is not None and (
            isinstance(min_message_count, bool)
            or not isinstance(min_message_count, int)
            or min_message_count < 0
        ):
            raise ValueError(
                f"trace.steps[{index}].request_hint.min_message_count "
                "must be a non-negative integer"
            )
        expected_failed_result = request_hint.get(
            "expected_failed_tool_result_contains"
        )
        if expected_failed_result is not None and (
            not isinstance(expected_failed_result, str) or not expected_failed_result
        ):
            raise ValueError(
                f"trace.steps[{index}].request_hint."
                "expected_failed_tool_result_contains must be a non-empty string"
            )
        responses.append(response)
        request_hints.append(request_hint)
        pending_user_input = False

    if not responses:
        raise ValueError("trace must contain at least one model response")
    if pending_user_input:
        raise ValueError("trace must not end with a user_input response")
    return {
        "source": source,
        "responses": responses,
        "next_response": 0,
        "expected_user_inputs": expected_user_inputs,
        "request_hints": request_hints,
        "error": None,
    }


def _next_llm_trace_response(
    state: dict,
    messages: list[dict],
    available_tool_names: set[str],
) -> dict | None:
    """Return the next recorded response, failing loudly on replay drift."""
    responses = state.get("responses") or []
    if not responses:
        return None
    next_index = state["next_response"]
    if next_index >= len(responses):
        state["error"] = (
            "recorded LLM trace is exhausted but the agent requested another response"
        )
        raise web.HTTPConflict(text=state["error"])

    request_hint = state["request_hints"][next_index]
    min_message_count = request_hint.get("min_message_count")
    if min_message_count is not None and len(messages) < min_message_count:
        state["error"] = (
            "recorded LLM trace request has too few messages before response "
            f"{next_index}: expected at least {min_message_count}, got {len(messages)}"
        )
        raise web.HTTPConflict(text=state["error"])

    hinted_user_input = request_hint.get("last_user_message_contains")
    if hinted_user_input is not None and hinted_user_input not in _last_user_content(
        messages
    ):
        state["error"] = (
            "recorded LLM trace request hint does not match the last user message "
            f"before response {next_index}"
        )
        raise web.HTTPConflict(text=state["error"])

    expected_input = state["expected_user_inputs"].get(next_index)
    if expected_input is not None:
        actual_input = _last_user_content(messages)
        if expected_input not in actual_input:
            state["error"] = (
                "recorded LLM trace user input does not match the conversation "
                f"before response {next_index}"
            )
            raise web.HTTPConflict(text=state["error"])

    failed_result = _failed_tool_result(messages)
    expected_failed_result = request_hint.get("expected_failed_tool_result_contains")
    if failed_result is None and expected_failed_result is not None:
        state["error"] = (
            "recorded LLM trace expected a failed capability result containing "
            f"{expected_failed_result!r} before response {next_index}"
        )
        raise web.HTTPConflict(text=state["error"])
    if failed_result is not None and (
        expected_failed_result is None
        or expected_failed_result not in failed_result["content"]
    ):
        state["error"] = (
            "recorded LLM trace observed a failed capability result before response "
            f"{next_index}: {failed_result['summary']}"
        )
        raise web.HTTPConflict(text=state["error"])

    response = deepcopy(responses[next_index])
    if response["type"] == "tool_calls":
        available_tool_names = set(available_tool_names)
        for result in _find_named_tool_results(messages, "capability_info"):
            parsed = _parse_trace_result_content(result.get("content"))
            disclosed_name = _find_trace_result_field(parsed, ["name"])
            if isinstance(disclosed_name, str):
                available_tool_names.add(disclosed_name)
                available_tool_names.add(disclosed_name.replace(".", "__"))
        missing = {
            tool_call["name"]
            for tool_call in response["tool_calls"]
            if tool_call["name"] not in available_tool_names
        }
        if missing:
            available_provider_tools = sorted(
                name
                for name in available_tool_names
                if "__" in name and not name.startswith("builtin__")
            )
            state["error"] = (
                "recorded LLM trace requested unavailable tools: "
                + ", ".join(sorted(missing))
                + "; available provider tools: "
                + ", ".join(available_provider_tools)
                + "; all available tools: "
                + ", ".join(sorted(available_tool_names))
            )
            raise web.HTTPConflict(text=state["error"])
        try:
            response["tool_calls"] = _resolve_trace_result_bindings(
                response["tool_calls"], messages
            )
        except ValueError as error:
            state["error"] = str(error)
            raise web.HTTPConflict(text=state["error"]) from error

    state["next_response"] += 1
    return response


def _resolve_trace_result_bindings(value: object, messages: list[dict]) -> object:
    """Resolve test-only arguments from earlier real capability results.

    Harvested traces necessarily contain the provider IDs returned during the
    live run. Full-path replay creates fresh Docs and Sheets resources, so a
    later recorded call must consume the ID returned by the local provider,
    not the stale live ID. Tests opt into that behavior with an argument value
    shaped like::

        {"$trace_result": {"tool": "google-docs__create_document",
                            "fields": ["documentId", "document_id", "id"]}}

    The marker is accepted only inside the mock server; committed trace files
    remain unchanged and production code never sees it.
    """
    if isinstance(value, list):
        return [_resolve_trace_result_bindings(item, messages) for item in value]
    if not isinstance(value, dict):
        return value

    if set(value) == {"$trace_result"}:
        binding = value["$trace_result"]
        if not isinstance(binding, dict):
            raise ValueError("$trace_result binding must be an object")
        tool = binding.get("tool")
        fields = binding.get("fields")
        if not isinstance(tool, str) or not tool:
            raise ValueError("$trace_result.tool must be a non-empty string")
        if (
            not isinstance(fields, list)
            or not fields
            or not all(isinstance(field, str) and field for field in fields)
        ):
            raise ValueError("$trace_result.fields must be non-empty strings")

        named_results = _find_named_tool_results(messages, tool)
        for result in reversed(named_results):
            payload = _parse_trace_result_content(result.get("content"))
            found = _find_trace_result_field(payload, fields)
            if found is not None:
                return found
        observed = [
            {
                "name": result.get("name"),
                "content": str(result.get("content", ""))[:500],
            }
            for result in _find_tool_results(messages)
        ]
        raise ValueError(
            f"recorded LLM trace could not bind a result from {tool} "
            f"using fields {fields}; observed tool results: {observed}"
        )

    return {
        key: _resolve_trace_result_bindings(item, messages)
        for key, item in value.items()
    }


def _parse_trace_result_content(content: object) -> object:
    if not isinstance(content, str):
        return content
    try:
        return json.loads(content)
    except json.JSONDecodeError:
        return content


def _find_trace_result_field(value: object, fields: list[str]) -> object | None:
    if isinstance(value, dict):
        for field in fields:
            candidate = value.get(field)
            if isinstance(candidate, (str, int)) and not isinstance(candidate, bool):
                return candidate
        for child in value.values():
            candidate = _find_trace_result_field(child, fields)
            if candidate is not None:
                return candidate
    elif isinstance(value, list):
        for child in value:
            candidate = _find_trace_result_field(child, fields)
            if candidate is not None:
                return candidate
    elif isinstance(value, str) and value[:1] in {"{", "["}:
        try:
            nested = json.loads(value)
        except json.JSONDecodeError:
            return None
        return _find_trace_result_field(nested, fields)
    return None


def _failed_tool_result(messages: list[dict]) -> dict | None:
    for message in messages:
        if message.get("role") != "tool":
            continue
        parsed = _parse_trace_result_content(message.get("content"))
        status = _find_trace_result_field(parsed, ["status"])
        if status in {"failed", "error"}:
            return {
                "content": json.dumps(parsed, sort_keys=True),
                "summary": f"{message.get('name', 'unknown tool')} status={status}",
            }
    return None

TOOL_FAILURE_TRIGGER = re.compile(r"issue 1780 tool failure", re.IGNORECASE)
TRUNCATED_TOOL_CALL_TRIGGER = re.compile(
    r"issue 1780 truncated tool call",
    re.IGNORECASE,
)
EMPTY_REPLY_TRIGGER = re.compile(r"issue 1780 empty reply", re.IGNORECASE)
LOOP_FOREVER_TRIGGER = re.compile(r"issue 1780 loop forever", re.IGNORECASE)
MULTI_STEP_TRIGGER = re.compile(r"multi step echo then time", re.IGNORECASE)
REBORN_EXTERNAL_TOOL_LOOP_TRIGGER = re.compile(
    r"reborn external tool loop",
    re.IGNORECASE,
)
REBORN_EXTERNAL_TOOL_FAILURE_TRIGGER = re.compile(
    r"reborn external tool failure",
    re.IGNORECASE,
)
REBORN_MIXED_INTERNAL_EXTERNAL_TRIGGER = re.compile(
    r"reborn mixed internal external tools",
    re.IGNORECASE,
)

# Lifecycle canary triggers for write+cleanup flows against real provider APIs.
GITHUB_ISSUE_LIFECYCLE_TRIGGER = re.compile(
    r"create a github issue in (?P<owner>[A-Za-z0-9_.-]+)/(?P<repo>[A-Za-z0-9_.-]+) titled",
    re.IGNORECASE,
)
GMAIL_ROUNDTRIP_TRIGGER = re.compile(
    r"send an email to (?P<email>\S+@\S+) with subject",
    re.IGNORECASE,
)
GCAL_LIFECYCLE_TRIGGER = re.compile(
    r"create a google calendar event titled",
    re.IGNORECASE,
)
GDRIVE_UPLOAD_LIFECYCLE_TRIGGER = re.compile(
    r"upload a google drive file titled",
    re.IGNORECASE,
)
SLACK_DELIVERY_LIFECYCLE_TRIGGER = re.compile(
    r"send slack canary (?P<marker>\S+) to (?P<channel>[A-Z0-9]+)",
    re.IGNORECASE,
)
GITHUB_RELEASE_SLACK_TRIGGER = re.compile(
    r"notify slack channel (?P<channel>[A-Z0-9]+) about the latest release in "
    r"(?P<owner>[A-Za-z0-9_.-]+)/(?P<repo>[A-Za-z0-9_.-]+) with marker "
    r"(?P<marker>\S+)",
    re.IGNORECASE,
)
CALENDAR_DRIVE_SLACK_TRIGGER = re.compile(
    r"prepare meeting and notify slack channel (?P<channel>[A-Z0-9]+) with marker "
    r"(?P<marker>\S+)",
    re.IGNORECASE,
)
GMAIL_SLACK_TRIGGER = re.compile(
    r"check unread gmail and notify slack channel (?P<channel>[A-Z0-9]+) with "
    r"marker (?P<marker>\S+)",
    re.IGNORECASE,
)
SLACK_DRIVE_SLACK_TRIGGER = re.compile(
    r"read slack channel (?P<source>[A-Z0-9]+), look up drive, and notify "
    r"(?P<target>[A-Z0-9]+) with marker (?P<marker>\S+)",
    re.IGNORECASE,
)
NOTION_SEARCH_LIFECYCLE_TRIGGER = re.compile(
    r"search notion for .*, then search again",
    re.IGNORECASE,
)

TOOL_CALL_PATTERNS = [
    # Reborn parallel tool-call port: the Reborn provider-visible builtin tool
    # names are namespaced/sanitized, while the legacy engine keeps using the
    # unqualified trigger below.
    (
        re.compile(r"reborn parallel echo and time", re.IGNORECASE),
        "builtin__echo",
        lambda _: [
            {"tool_name": "builtin__echo", "arguments": {"message": "parallel-test"}},
            {"tool_name": "builtin__time", "arguments": {"operation": "now"}},
        ],
    ),
    # Parallel tool calls: return both echo and time in one response
    (
        re.compile(r"parallel echo and time", re.IGNORECASE),
        "echo",
        lambda _: [
            {"tool_name": "echo", "arguments": {"message": "parallel-test"}},
            {"tool_name": "time", "arguments": {"operation": "now"}},
        ],
    ),
    (
        re.compile(r"reborn builtin echo (.+)", re.IGNORECASE),
        "builtin__echo",
        lambda m: {"message": m.group(1)},
    ),
    (
        re.compile(r"reborn builtin time", re.IGNORECASE),
        "builtin__time",
        lambda _: {"operation": "now"},
    ),
    (re.compile(r"echo (.+)", re.IGNORECASE), "echo", lambda m: {"message": m.group(1)}),
    # Private tool installs (#5459 P1) — the three test-tools/ fixture bundles
    # (test-tools/README.md). The provider-visible tool name sanitizes the
    # dotted capability id's "." to "__" (`encode_provider_tool_name` in
    # ironclaw_reborn::tool_disclosure); the model gateway's provider_tool_name
    # validator rejects a raw "." outright ("only ASCII letters, digits, '_',
    # and '-' are allowed"), so the mock LLM must emit the encoded form, not
    # the dotted capability id.
    # The combined pattern is checked first so it doesn't get shadowed by the
    # single-tool "ascii renderer to draw a" pattern below.
    (
        re.compile(r"ascii renderer and market data", re.IGNORECASE),
        "ascii-renderer__draw",
        lambda _: [
            {"tool_name": "ascii-renderer__draw", "arguments": {"subject": "robot"}},
            {"tool_name": "market-data__snp500", "arguments": {}},
        ],
    ),
    (
        re.compile(r"ascii renderer to draw a (?P<subject>cat|dog|robot)", re.IGNORECASE),
        "ascii-renderer__draw",
        lambda m: {"subject": m.group("subject").lower()},
    ),
    (
        re.compile(r"hacker news tool", re.IGNORECASE),
        "hacker-news__top_stories",
        lambda _: {},
    ),
    # Reborn v2 download chips: one assistant turn writes a CSV and a PDF into
    # the project workspace. Reborn exposes this first-party tool by capability
    # id; the provider-facing tool name sanitizes dots as "__". After both
    # results land, match_tool_call dedups builtin__write_file and the
    # conversation falls through to the CANNED_RESPONSES reply that
    # references the two paths.
    (
        re.compile(r"produce a downloadable csv and pdf", re.IGNORECASE),
        "builtin__write_file",
        lambda _: [
            {
                "tool_name": "builtin__write_file",
                "arguments": {
                    "path": "/workspace/report.csv",
                    "content": "name,score\nalice,90\nbob,85\n",
                },
            },
            {
                "tool_name": "builtin__write_file",
                "arguments": {
                    "path": "/workspace/report.pdf",
                    "content": (
                        "%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\n"
                        "trailer<</Root 1 0 R>>\n%%EOF\n"
                    ),
                },
            },
        ],
    ),
    (
        re.compile(r"reborn write approval file (?P<label>[a-z0-9_-]+)", re.IGNORECASE),
        "builtin__write_file",
        lambda m: {
            "path": f"/workspace/reborn-approval-{m.group('label')}.txt",
            "content": f"approved {m.group('label')}\n",
        },
    ),
    (
        re.compile(
            r"reborn create automation rename target (?P<label>[a-z0-9_-]+)",
            re.IGNORECASE,
        ),
        "builtin__trigger_create",
        lambda m: {
            "name": f"E2E rename original {m.group('label')}",
            "prompt": f"E2E automation rename prompt {m.group('label')}",
            "schedule": {
                "kind": "once",
                "at": "2999-06-02T00:00:00",
                "timezone": "UTC",
            },
        },
    ),
    (
        re.compile(
            r"install https://github\.com/Pika-Labs/Pika-Skills/?(?=$|\s)",
            re.IGNORECASE,
        ),
        "skill_install",
        lambda _: {
            "name": "pikastream-video-meeting",
            "url": "https://github.com/Pika-Labs/Pika-Skills",
        },
    ),
    (
        re.compile(r"install (?P<url>https?://\S+)", re.IGNORECASE),
        "skill_install",
        lambda m: {
            "name": _derive_skill_name_from_url(m.group("url")),
            "url": m.group("url"),
        },
    ),
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
    # Workflow-canary NL-driven routine creation: when a chat message
    # carries the [CANARY-WORKFLOW-NL-CREATE] sentinel, emit a
    # routine_create tool call so the canary can verify the agent's
    # NL → tool dispatch → routine row pipeline.
    (
        re.compile(r"\[CANARY-WORKFLOW-NL-CREATE\]", re.IGNORECASE),
        "routine_create",
        lambda _: {
            "name": "canary-nl-created",
            "prompt": (
                "send a Telegram acknowledgement\n\n"
                "[CANARY-WORKFLOW-nl_create] inner-prompt"
            ),
            "trigger_type": "cron",
            "schedule": "0 */1 * * *",
            "description": "canary: NL-driven routine creation",
        },
    ),
    # Workflow-canary NL-driven schedule update: when a chat message
    # carries [CANARY-WORKFLOW-NL-UPDATE], emit a routine_update tool
    # call retargeting the canary's pre-seeded routine.
    (
        re.compile(r"\[CANARY-WORKFLOW-NL-UPDATE\]", re.IGNORECASE),
        "routine_update",
        lambda _: {
            "name": "canary-nl-update-target",
            "schedule": "0 */6 * * *",
        },
    ),
    # Workflow-canary Sheet-write side-effect probe. When a routine's
    # Lightweight prompt carries [CANARY-WORKFLOW-SHEET-APPEND], emit
    # an http POST to the mock Google Sheets API's values:append
    # endpoint. The IRONCLAW_TEST_HTTP_REMAP set in
    # run_workflow_canary.py routes sheets.googleapis.com to the local
    # sheets_mock subprocess, which records the captured row for the
    # scenario's assertion.
    #
    # Spreadsheet ID is hardcoded; the scenario pre-seeds the sheet
    # with that ID + headers via /__mock/seed_spreadsheet.
    (
        re.compile(r"\[CANARY-WORKFLOW-SHEET-APPEND\]", re.IGNORECASE),
        "http",
        lambda _: {
            "method": "POST",
            "url": (
                "https://sheets.googleapis.com/v4/spreadsheets/"
                "canary-bug-logger/values/Sheet1:append"
                "?valueInputOption=USER_ENTERED"
            ),
            "body": {
                "range": "Sheet1",
                "majorDimension": "ROWS",
                "values": [
                    [
                        "2026-04-28T00:00:00Z",
                        "login button is unresponsive on mobile",
                        "telegram",
                    ]
                ],
            },
        },
    ),
    # Workflow-canary Calendar events.list + prep summary. When a
    # routine's Lightweight prompt carries [CANARY-WORKFLOW-CAL-LIST],
    # emit a PARALLEL pair of http tool calls in one response:
    #   1. GET events.list against the mock Calendar API
    #   2. POST sendMessage with a prep briefing referencing the seeded
    #      event title.
    # IRONCLAW_TEST_HTTP_REMAP routes both: www.googleapis.com →
    # calendar_mock, api.telegram.org → telegram_mock. Parallel emit is
    # required because the engine's lightweight loop dedup
    # (match_tool_call:1178-1179) skips re-dispatching the same tool —
    # so a multi-step flow has to fan out in one response.
    (
        re.compile(r"\[CANARY-WORKFLOW-CAL-LIST\]", re.IGNORECASE),
        "http",
        lambda _: [
            {
                "tool_name": "http",
                "arguments": {
                    "method": "GET",
                    "url": (
                        "https://www.googleapis.com/calendar/v3/calendars/"
                        "primary/events?maxResults=10"
                    ),
                },
            },
            # Web-search lookup for the meeting attendee's company.
            # Routes to web_search_mock via
            # IRONCLAW_TEST_HTTP_REMAP=api.search.brave.com=<mock>.
            {
                "tool_name": "http",
                "arguments": {
                    "method": "GET",
                    "url": (
                        "https://api.search.brave.com/res/v1/web/search"
                        "?q=Acme%20Corp%20company%20background"
                    ),
                },
            },
            {
                "tool_name": "http",
                "arguments": {
                    "method": "POST",
                    "url": (
                        "https://api.telegram.org/bot111222333:CANARY/sendMessage"
                    ),
                    "body": {
                        "chat_id": 8800800800,
                        "text": (
                            "[canary-workflow:calendar_prep] prep for "
                            "'Canary kickoff with Acme' — Acme Corp is "
                            "a fintech in Series B."
                        ),
                    },
                },
            },
        ],
    ),
    # Workflow-canary Hacker News fetch + summary. [CANARY-WORKFLOW-HN-FETCH]
    # emits a parallel pair: GET /newest (hn_mock returns deterministic
    # HTML) + POST sendMessage with the canary post summary.
    (
        re.compile(r"\[CANARY-WORKFLOW-HN-FETCH\]", re.IGNORECASE),
        "http",
        lambda _: [
            {
                "tool_name": "http",
                "arguments": {
                    "method": "GET",
                    "url": "https://news.ycombinator.com/newest",
                },
            },
            {
                "tool_name": "http",
                "arguments": {
                    "method": "POST",
                    "url": (
                        "https://api.telegram.org/bot111222333:CANARY/sendMessage"
                    ),
                    "body": {
                        "chat_id": 8800800800,
                        "text": (
                            "[canary-workflow:hn_monitor] new Show HN posts: "
                            "Show HN: Canary Post Alpha "
                            "(https://example.com/alpha by canary_alpha); "
                            "Show HN: Canary Post Beta "
                            "(https://example.com/beta by canary_beta)"
                        ),
                    },
                },
            },
        ],
    ),
    # Workflow-canary CRM tracker: gmail unread → classify → append
    # only sales leads to Sheets. [CANARY-WORKFLOW-CRM-CLASSIFY] emits
    # a parallel triplet: mock Gmail GET messages, mock Sheets POST
    # values:append for the lead, telegram sendMessage acking the run.
    (
        re.compile(r"\[CANARY-WORKFLOW-CRM-CLASSIFY\]", re.IGNORECASE),
        "http",
        lambda _: [
            {
                "tool_name": "http",
                "arguments": {
                    "method": "GET",
                    "url": (
                        "https://gmail.googleapis.com/gmail/v1/users/me/"
                        "messages?q=is:unread"
                    ),
                },
            },
            {
                "tool_name": "http",
                "arguments": {
                    "method": "POST",
                    "url": (
                        "https://sheets.googleapis.com/v4/spreadsheets/"
                        "canary-crm-tracker/values/Sheet1:append"
                        "?valueInputOption=USER_ENTERED"
                    ),
                    "body": {
                        "range": "Sheet1",
                        "majorDimension": "ROWS",
                        "values": [
                            [
                                "Acme Corp",
                                "Jane Lead",
                                "jane.lead@acme.example",
                                "new",
                                "Inbound interest in enterprise tier",
                                "schedule discovery call",
                            ]
                        ],
                    },
                },
            },
            {
                "tool_name": "http",
                "arguments": {
                    "method": "POST",
                    "url": (
                        "https://api.telegram.org/bot111222333:CANARY/sendMessage"
                    ),
                    "body": {
                        "chat_id": 8800800800,
                        "text": (
                            "[canary-workflow:crm_tracker] logged 1 new "
                            "lead from Acme Corp"
                        ),
                    },
                },
            },
        ],
    ),
    # Default workflow-canary scenarios tag their routine prompt with
    # [CANARY-WORKFLOW-<key>] so this matcher emits a deterministic
    # `http` tool call that reaches mock_telegram via
    # IRONCLAW_TEST_HTTP_REMAP=api.telegram.org=<mock>. The chat_id is
    # the canary's simulated test user (mock_telegram DEFAULT_USER_ID).
    # The text echoes the scenario key so the scenario can disambiguate
    # which message belongs to it when the mock_telegram is shared
    # across the workflow-canary lane's probes.
    #
    # Important: this generic matcher must come AFTER the specific
    # NL-CREATE / NL-UPDATE matchers above. mock_llm.py iterates
    # TOOL_CALL_PATTERNS in order and stops at the first match —
    # without ordering, [CANARY-WORKFLOW-nl_create] inside a routine
    # PROMPT would emit an http tool call instead of routine_create.
    (
        re.compile(r"\[CANARY-WORKFLOW-(?P<key>[a-z_0-9]+)\]", re.IGNORECASE),
        "http",
        lambda m: {
            "method": "POST",
            "url": (
                "https://api.telegram.org/bot111222333:CANARY/sendMessage"
            ),
            "body": {
                "chat_id": 8800800800,
                "text": f"[canary-workflow:{m.group('key').lower()}] ack",
            },
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
    (
        re.compile(r"list next calendar event|check calendar next event", re.IGNORECASE),
        "google_calendar",
        lambda _: {
            "action": "list_events",
            "calendar_id": "primary",
            "max_results": 1,
        },
    ),
    (
        re.compile(
            r"read github issue (?P<owner>[A-Za-z0-9_.-]+)/(?P<repo>[A-Za-z0-9_.-]+)#(?P<num>\d+)",
            re.IGNORECASE,
        ),
        "github",
        lambda m: {
            "action": "get_issue",
            "owner": m.group("owner"),
            "repo": m.group("repo"),
            "issue_number": int(m.group("num")),
        },
    ),
    (
        re.compile(r"search notion for (?P<query>.+)", re.IGNORECASE),
        "notion_notion_search",
        lambda m: {"query": m.group("query").strip()},
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
        re.compile(
            r"create (?:an )?issue.*(?:nearai|ironclaw)|issue in nearai/ironclaw",
            re.IGNORECASE,
        ),
        "http",
        lambda _: {
            "method": "POST",
            "url": f"{_github_api_url}/repos/nearai/ironclaw/issues",
            "body": {
                "title": "E2E auth flow test issue",
                "body": "Created by the E2E mock LLM auth-flow scenario.",
            },
        },
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
_chat_requests: list[dict] = []
_llm_fault_scripts: list[dict] = []


def _reset_llm_fault_scripts() -> None:
    _llm_fault_scripts.clear()


def _latest_user_matches_fault(messages: list[dict], match_text: str) -> bool:
    latest_user = _last_user_content(messages).lower()
    return match_text.lower() in latest_user


def _next_llm_fault_action(messages: list[dict]) -> dict | None:
    for script in list(_llm_fault_scripts):
        match_text = str(script.get("match", ""))
        actions = script.get("actions")
        if not match_text or not isinstance(actions, list):
            continue
        if not _latest_user_matches_fault(messages, match_text):
            continue
        if not actions:
            continue
        action = actions.pop(0)
        script["applied"] = int(script.get("applied", 0)) + 1
        return action if isinstance(action, dict) else None
    return None


def _new_oauth_state() -> dict:
    return {
        "exchange_count": 0,
        "refresh_count": 0,
        "last_exchange": None,
        "last_refresh": None,
    }


def _new_mcp_state() -> dict:
    return {
        "requests": [],
    }


def _message_text(msg: dict) -> str:
    content = msg.get("content") or ""
    if isinstance(content, list):
        parts = []
        for p in content:
            if p.get("type") == "text":
                parts.append(p.get("text") or "")
            else:
                try:
                    parts.append(json.dumps(p, sort_keys=True))
                except TypeError:
                    parts.append(str(p))
        content = " ".join(parts)
    return content


def _last_user_content(messages: list[dict]) -> str:
    for msg in reversed(messages):
        if msg.get("role") == "user":
            return _message_text(msg)
    return ""


def _last_user_message(messages: list[dict]) -> dict:
    for msg in reversed(messages):
        if msg.get("role") == "user":
            return msg
    return {}


def _message_payload_text(msg: dict) -> str:
    try:
        return json.dumps(msg, sort_keys=True).lower()
    except TypeError:
        return str(msg).lower()


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


def _conversation_has_active_skill(messages: list[dict], skill_name: str) -> bool:
    needle = f'<skill name="{skill_name}"'
    for msg in messages:
        if msg.get("role") == "system" and needle in _message_text(msg):
            return True
        if msg.get("role") == "user" and f"/{skill_name}" in _message_text(msg):
            return True
    return False


def _conversation_uses_codeact(messages: list[dict]) -> bool:
    for msg in messages:
        if msg.get("role") != "system":
            continue
        text = _message_text(msg)
        if "Python REPL environment" in text and "```repl" in text:
            return True
    return False


def _conversation_includes_denial(messages: list[dict]) -> bool:
    for msg in messages:
        text = f"{_message_text(msg)}\n{_message_payload_text(msg)}"
        if DENIAL_PATTERN.search(text):
            return True
    return False


def _active_skill_names(messages: list[dict]) -> set[str]:
    names = set()
    for msg in messages:
        if msg.get("role") != "system":
            continue
        for name in re.findall(r'<skill name="([^"]+)"', _message_text(msg)):
            if name:
                names.add(name.lower())
    return names


def _typed_user_content_for_skill_detection(messages: list[dict]) -> str:
    """Return user-authored text without generated attachment context.

    Reborn appends a model-visible ``<attachments>`` block to user messages so
    tools can reason about uploaded files and their /workspace storage paths.
    Those paths are not user-typed slash skills, so the mock's missing-skill
    heuristic must ignore that generated block.
    """
    return re.sub(
        r"\n+<attachments>.*?</attachments>\s*$",
        "",
        _last_user_content(messages),
        flags=re.DOTALL,
    )


def _missing_explicit_skills(messages: list[dict]) -> list[str]:
    active = _active_skill_names(messages)
    missing = []
    seen = set()
    content = _typed_user_content_for_skill_detection(messages)
    for match in re.finditer(r'(^|[\s"\(])/(?P<name>[A-Za-z0-9._-]+)', content):
        name = match.group("name").lower()
        if name in active or name in seen:
            continue
        seen.add(name)
        missing.append(name)
    return missing


def _active_skill_bundle_path(messages: list[dict], skill_name: str) -> str | None:
    needle = f'<skill name="{skill_name}"'
    for msg in messages:
        if msg.get("role") != "system":
            continue
        content = _message_text(msg)
        if needle not in content:
            continue
        match = re.search(r"Installed bundle path on disk:\s*`([^`]+)`", content)
        if match:
            return match.group(1)
    return None


def _derive_skill_name_from_url(url: str) -> str:
    cleaned = re.sub(r"[?#].*$", "", url).rstrip("/")
    if not cleaned:
        return "remote-skill"
    last = cleaned.rsplit("/", 1)[-1]
    last = re.sub(r"\.git$", "", last, flags=re.IGNORECASE)
    last = re.sub(r"\.md$", "", last, flags=re.IGNORECASE)
    slug = re.sub(r"[^a-z0-9._-]+", "-", last.lower()).strip("-")
    return slug or "remote-skill"


def _conversation_slow_response_delay(messages: list[dict]) -> float:
    if _conversation_has_user_trigger(
        messages,
        re.compile(r"editable composer slow response", re.IGNORECASE),
    ):
        return 5.0
    if _conversation_has_user_trigger(
        messages,
        re.compile(r"refresh-mid-response|slow response|slowly", re.IGNORECASE),
    ):
        return 2.0
    return 0.0


def _assistant_has_phrase(messages: list[dict], phrase: str) -> bool:
    target = phrase.lower()
    for msg in messages:
        if msg.get("role") != "assistant":
            continue
        if target in _message_text(msg).lower():
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
    payload_text = _message_payload_text(_last_user_message(messages))
    resumed = _resumed_action_summary(messages)
    if resumed:
        return resumed
    denial_text = f"{content}\n{payload_text}"
    if DENIAL_PATTERN.search(denial_text):
        action_match = re.search(
            r"User denied action '([^']+)'", denial_text, re.IGNORECASE
        )
        tool_match = re.search(
            r"user denied tool '([^']+)'", denial_text, re.IGNORECASE
        )
        if action_match:
            action_name = action_match.group(1)
        elif tool_match:
            action_name = tool_match.group(1)
        else:
            action_name = "that action"
        return (
            f"The request for {action_name} was denied. "
            "No installation or setup was performed."
        )
    missing_slash_skills = _missing_explicit_skills(messages)
    if missing_slash_skills:
        if len(missing_slash_skills) == 1:
            return (
                f"Skill '/{missing_slash_skills[0]}' is not installed or was not found. "
                "Type `/` to see the available commands and installed skills."
            )
        rendered = ", ".join(f"`/{name}`" for name in missing_slash_skills)
        return (
            f"These slash skills are not installed or were not found: {rendered}. "
            "Type `/` to see the available commands and installed skills."
        )
    if _conversation_has_active_skill(messages, "pikastream-video-meeting"):
        lower = content.lower()
        payload_lower = payload_text.lower()
        if "meet.google.com" in lower or "hangouts.google.com" in lower:
            return (
                "I need an avatar image for the video meeting. "
                "Send me an image, or say \"generate\" and I'll create one for you."
            )
        if lower.strip() == "generate":
            return "Avatar generated. Want to keep this avatar or regenerate?"
        if (
            "avatar.png" in lower or "portrait.png" in lower or "headshot" in lower
            or "avatar.png" in payload_lower or "portrait.png" in payload_lower
        ):
            return (
                "Avatar received. Now send a short audio sample, or say \"skip\" to use the default voice."
            )
        if (
            ("hello.pdf" in lower or ".pdf" in lower or "application/pdf" in lower
             or "hello.pdf" in payload_lower or "application/pdf" in payload_lower
             or "hello world" in lower)
            and not _assistant_has_phrase(messages, "audio sample")
        ):
            return (
                "I still need an avatar image for the video meeting. "
                "Please upload an image file."
            )
        if (
            "voice.ogg" in lower or "voice.wav" in lower or "voice.mp3" in lower
            or "audio sample" in lower or "voice.ogg" in payload_lower
        ):
            return "Voice sample received. The session is ready for Google Meet / Hangouts setup."
        if (
            _assistant_has_phrase(messages, "audio sample")
            and ("avatar.png" in lower or "portrait.png" in lower or ".png" in lower or ".jpg" in lower)
        ):
            return "I still need a short audio sample before I can finish the Hangouts setup."
        if _conversation_has_tool_name(messages, "shell"):
            return (
                "I need an avatar image for the video meeting. "
                "Send me an image, or say \"generate\" and I'll create one for you."
            )
    # Nudge recovery: when the engine sends a "you expressed intent but
    # didn't call a tool" nudge, check whether the conversation has
    # portfolio/wallet context from an earlier user message and return a
    # portfolio-relevant response so the nudge pattern (which matches
    # before the portfolio patterns in CANNED_RESPONSES) doesn't swallow
    # the domain context.
    _nudge_re = re.compile(
        r"You said you would perform an action|You expressed intent",
        re.IGNORECASE,
    )
    if _nudge_re.search(content):
        for msg in messages:
            if msg.get("role") == "user":
                msg_text = _message_text(msg)
                if re.search(r"portfolio|defi|rebalance|yield.*positions", msg_text, re.IGNORECASE):
                    return (
                        "I'll analyze your DeFi portfolio. The portfolio skill is active and I can scan "
                        "your wallet addresses across chains to discover positions, check yields, and "
                        "suggest rebalancing opportunities."
                    )
                if re.search(r"0x[a-fA-F0-9]{40}", msg_text, re.IGNORECASE):
                    return (
                        "I found your wallet address. Let me scan your portfolio across all supported "
                        "chains to discover DeFi positions and classify them against known protocols."
                    )

    explicit = _explicit_canned_response(content)
    if explicit is not None:
        return explicit
    return DEFAULT_RESPONSE


def _explicit_canned_response(content: str) -> str | None:
    """Return the first ``CANNED_RESPONSES`` match for ``content``, or ``None``
    when only the default would apply.

    Lets a caller prefer a specific canned reply over a generic fallback
    (e.g. the post-tool-call summary) without collapsing every unmatched
    conversation into the default response.
    """
    for pattern, response in CANNED_RESPONSES:
        if pattern.search(content):
            return response
    return None


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


def _advertised_tool_names(tools: object) -> set[str]:
    names: set[str] = set()
    if not isinstance(tools, list):
        return names
    for tool in tools:
        if not isinstance(tool, dict):
            continue
        function = tool.get("function")
        if isinstance(function, dict) and isinstance(function.get("name"), str):
            names.add(function["name"])
        elif isinstance(tool.get("name"), str):
            names.add(tool["name"])
    return names


def _available_tool_names(tools: object) -> set[str]:
    """Include deferred tools named by the runtime's tool-search catalog."""
    names = _advertised_tool_names(tools)
    if not isinstance(tools, list):
        return names
    for tool in tools:
        if not isinstance(tool, dict):
            continue
        function = tool.get("function") if isinstance(tool.get("function"), dict) else tool
        if not isinstance(function, dict):
            continue
        name = function.get("name")
        description = function.get("description")
        if not isinstance(name, str) or not name.endswith("tool_search"):
            continue
        if not isinstance(description, str):
            continue
        for line in description.splitlines():
            candidate = line.removeprefix("- ") if line.startswith("- ") else ""
            if candidate and all(
                character.isalnum() or character in "_.-" for character in candidate
            ):
                names.add(candidate)
                names.add(candidate.replace(".", "__"))
    return names


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
    if _missing_explicit_skills(messages):
        return None
    lower = content.lower()
    recent_tool_results = _find_tool_results(messages)
    # #3533: gmail-install-then-retry sequence.
    #
    # Turn 1: user says "check gmail unread" → match_tool_call below dispatches
    #         a direct `gmail` call. Engine rejects with either "Extension not
    #         installed:" (pre-#3533 wording, from the bridge-side
    #         not-installed reject — the chat-driven install path was wired up
    #         here in mock_llm but non-functional because `tool_install` was
    #         hidden from the agent surface) or "is not callable in this
    #         execution context" (post-#3533, engine-side preflight rejection,
    #         tool_install restored on the agent surface).
    # Turn 2: this branch fires — call `tool_install("gmail")`.
    # Turn 3: install succeeded → call `gmail` again, this time the engine's
    #         auth preflight raises an Authentication gate.
    # Turn 4 (after OAuth completes): mock LLM falls through to the
    #         tool-result-summary path returning the "Quarterly update" text.
    if "check gmail unread" in lower or "gmail unread" in lower:
        # Three engine paths can surface gmail-unavailable depending on
        # whether gmail is in the registry, installed-but-blocked, or
        # entirely unknown:
        #   * "Extension not installed: gmail"           — registry has it, not installed
        #   * "is not callable in this execution context" — installed but engine-v2 blocked
        #   * "Tool gmail not found"                      — not even in the dispatcher (workflow-canary stack)
        # A real LLM would treat all three the same way and reach for
        # `tool_install`. Mirror that — restricting to the first two
        # made the workflow-canary `tool_install_chat` probe fall
        # through to text and never recover.
        gmail_error = next(
            (
                tr
                for tr in recent_tool_results
                if tr["name"] == "gmail"
                and (
                    "Extension not installed:" in tr["content"]
                    or "is not callable in this execution context" in tr["content"]
                    or "Tool gmail not found" in tr["content"]
                )
            ),
            None,
        )
        install_done = any(
            tr["name"] == "tool_install"
            and "error" not in tr["content"].lower()
            for tr in recent_tool_results
        )
        if gmail_error and not install_done:
            return [{
                "tool_name": "tool_install",
                "arguments": {"name": "gmail"},
            }]
        if install_done and not any(
            tr["name"] == "gmail" and "is not callable" not in tr["content"]
            and "Extension not installed" not in tr["content"]
            and "Tool gmail not found" not in tr["content"]
            for tr in recent_tool_results
        ):
            # Retry gmail after install — the engine's auth preflight will
            # raise an Authentication gate, which surfaces the auth card.
            # After OAuth completes, this re-fires and reaches the actual
            # gmail tool with the correct `list_messages` action.
            return [{
                "tool_name": "gmail",
                "arguments": {"action": "list_messages"},
            }]
    if _conversation_has_active_skill(messages, "pikastream-video-meeting"):
        bundle_path = _active_skill_bundle_path(messages, "pikastream-video-meeting")
        if (
            bundle_path
            and ("meet.google.com" in lower or "hangouts.google.com" in lower)
        ):
            return [{
                "tool_name": "shell",
                "arguments": {
                    "command": (
                        'python3 -m venv .venv && '
                        f'./.venv/bin/pip install -q --disable-pip-version-check '
                        f'-r "{bundle_path}/requirements.txt"'
                    ),
                    "workdir": bundle_path,
                    "timeout": 60,
                },
            }]
    for pattern, tool_name, args_fn in TOOL_CALL_PATTERNS:
        m = pattern.search(content)
        if m:
            # Don't re-dispatch a tool that already ran this turn: the
            # agentic loop calls the mock LLM again after each tool
            # result, and the "last user content" (which is what we
            # pattern-match against) hasn't changed. A real LLM would
            # see the tool result and respond with text — we mirror that
            # by falling through to the match_response text path when a
            # matching call is already sitting in the tool_results
            # buffer.
            if any(tr["name"] == tool_name for tr in recent_tool_results):
                return None
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

    tool_call_names: dict[str, str] = {}
    results: list[dict] = []
    for i in range(last_user_idx + 1, len(messages)):
        message = messages[i]
        if message.get("role") == "assistant":
            for tool_call in message.get("tool_calls") or []:
                tool_call_id = tool_call.get("id")
                tool_name = (
                    tool_call.get("function", {}).get("name")
                    or tool_call.get("name")
                    or "unknown"
                )
                if tool_call_id:
                    tool_call_names[tool_call_id] = tool_name
            continue
        if message.get("role") == "tool":
            name = _extract_tool_name(message)
            if name == "unknown":
                name = tool_call_names.get(message.get("tool_call_id", ""), name)
            results.append({
                "name": name,
                "content": message.get("content", ""),
            })
    return results


def _find_tool_result(messages: list[dict]) -> dict | None:
    """Backward-compat single-result helper used by the special-response path."""
    results = _find_tool_results(messages)
    return results[0] if results else None


def _find_named_tool_results(messages: list[dict], name: str) -> list[dict]:
    """Collect fresh tool results for one tool name."""
    return [result for result in _find_tool_results(messages) if result.get("name") == name]


REBORN_SCRIPTED_TOOL_SCENARIOS = (
    {
        "trigger": REBORN_EXTERNAL_TOOL_LOOP_TRIGGER,
        "batches": (
            (("lookup_weather", {"city": "Boston"}),),
            (("lookup_time", {"city": "Boston"}),),
            (("lookup_fact", {"topic": "Boston"}),),
        ),
        "missing_text": "Reborn external tool loop missing tool definitions.",
        "complete_prefix": "Reborn external tool loop complete: ",
        "summary_order": ("lookup_weather", "lookup_time", "lookup_fact"),
    },
    {
        "trigger": REBORN_EXTERNAL_TOOL_FAILURE_TRIGGER,
        "batches": ((("lookup_weather", {"city": "Boston"}),),),
        "missing_text": "Reborn external tool failure missing tool definitions.",
        "complete_prefix": "Reborn external tool failure observed: ",
        "summary_order": ("lookup_weather",),
    },
    {
        "trigger": REBORN_MIXED_INTERNAL_EXTERNAL_TRIGGER,
        "batches": (
            (
                ("builtin__echo", {"message": "mixed-internal-echo"}),
                ("lookup_weather", {"city": "Boston"}),
            ),
        ),
        "missing_text": "Reborn mixed tool run missing tool definitions.",
        "complete_prefix": "Reborn mixed tool run complete: ",
        "summary_order": ("builtin__echo", "lookup_weather"),
    },
)


def match_reborn_scripted_tool_response(
    messages: list[dict],
    has_tools: bool,
) -> dict | None:
    """Run the matching table-driven Responses API tool scenario."""
    scenario = next(
        (
            candidate
            for candidate in REBORN_SCRIPTED_TOOL_SCENARIOS
            if _conversation_has_user_trigger(messages, candidate["trigger"])
        ),
        None,
    )
    if scenario is None:
        return None

    result_by_name = {
        result["name"]: result["content"] for result in _find_tool_results(messages)
    }
    for batch in scenario["batches"]:
        missing_calls = [
            {"tool_name": name, "arguments": arguments}
            for name, arguments in batch
            if name not in result_by_name
        ]
        if not missing_calls:
            continue
        if not result_by_name and not has_tools:
            return {"type": "text", "text": scenario["missing_text"]}
        return {
            "type": "tool_call",
            "tool_call": missing_calls[0] if len(missing_calls) == 1 else missing_calls,
        }

    summary = "; ".join(
        f"{name}={result_by_name[name]}" for name in scenario["summary_order"]
    )
    return {
        "type": "text",
        "text": f"{scenario['complete_prefix']}{summary}",
    }


def _tool_results_include_denial(tool_results: list[dict]) -> bool:
    return any(DENIAL_PATTERN.search(tr.get("content", "")) for tr in tool_results)


def _recent_tool_names(messages: list[dict]) -> set[str]:
    """Collect tool names referenced after the most recent user turn."""
    last_user_idx = -1
    for i in range(len(messages) - 1, -1, -1):
        if messages[i].get("role") == "user":
            last_user_idx = i
            break

    tool_names: set[str] = set()
    tool_call_names: dict[str, str] = {}
    for i in range(last_user_idx + 1, len(messages)):
        message = messages[i]
        if message.get("role") == "assistant":
            for tool_call in message.get("tool_calls") or []:
                tool_name = (
                    tool_call.get("function", {}).get("name")
                    or tool_call.get("name")
                    or "unknown"
                )
                if tool_name != "unknown":
                    tool_names.add(tool_name)
                tool_call_id = tool_call.get("id")
                if tool_call_id:
                    tool_call_names[tool_call_id] = tool_name
            continue
        if message.get("role") == "tool":
            tool_name = _extract_tool_name(message)
            if tool_name == "unknown":
                tool_name = tool_call_names.get(message.get("tool_call_id", ""), tool_name)
            if tool_name != "unknown":
                tool_names.add(tool_name)
    return tool_names


def _conversation_has_tool_name(messages: list[dict], expected_name: str) -> bool:
    """Return True when the conversation references a given tool name anywhere."""
    tool_call_names: dict[str, str] = {}
    for message in messages:
        if message.get("role") == "assistant":
            for tool_call in message.get("tool_calls") or []:
                tool_name = (
                    tool_call.get("function", {}).get("name")
                    or tool_call.get("name")
                    or "unknown"
                )
                if tool_name == expected_name:
                    return True
                tool_call_id = tool_call.get("id")
                if tool_call_id:
                    tool_call_names[tool_call_id] = tool_name
            continue
        if message.get("role") == "tool":
            tool_name = _extract_tool_name(message)
            if tool_name == "unknown":
                tool_name = tool_call_names.get(message.get("tool_call_id", ""), tool_name)
            if tool_name == expected_name:
                return True
    return False


# ── Lifecycle canary helpers ────────────────────────────────────────────────
#
# These extract structured data from real provider tool-result JSON so the
# multi-step lifecycle flows can pass IDs between steps (e.g. the issue
# number from create_issue feeds into create_issue_comment, the event_id
# from create_event feeds into delete_event, etc.).


def _extract_canary_title(text: str) -> str:
    """Extract a quoted title like '[canary] 1713...' from a user prompt."""
    m = re.search(r"titled\s+'([^']+)'", text)
    if m:
        return m.group(1)
    m = re.search(r"titled\s+\"([^\"]+)\"", text)
    if m:
        return m.group(1)
    return "[canary] lifecycle-test"


def _extract_canary_subject(text: str) -> str:
    """Extract a subject like '[canary] 1713...' from a user prompt."""
    m = re.search(r"subject\s+'([^']+)'", text)
    if m:
        return m.group(1)
    m = re.search(r"subject\s+\"([^\"]+)\"", text)
    if m:
        return m.group(1)
    return "[canary] lifecycle-test"


def _extract_issue_number(content: str) -> int | None:
    """Extract the issue number from a GitHub create_issue tool result."""
    try:
        data = json.loads(content)
        if isinstance(data, dict) and "number" in data:
            return int(data["number"])
    except (json.JSONDecodeError, ValueError, TypeError):
        pass
    m = re.search(r'"number"\s*:\s*(\d+)', content)
    if m:
        return int(m.group(1))
    return None


def _extract_gmail_message_id(content: str) -> str | None:
    """Extract the message id from a Gmail send_message tool result."""
    try:
        data = json.loads(content)
        if isinstance(data, dict):
            return data.get("id") or data.get("message_id")
    except (json.JSONDecodeError, ValueError, TypeError):
        pass
    m = re.search(r'"id"\s*:\s*"([^"]+)"', content)
    if m:
        return m.group(1)
    return None


def _extract_calendar_event_id(content: str) -> str | None:
    """Extract the event id from a Google Calendar create_event tool result."""
    try:
        data = json.loads(content)
        if isinstance(data, dict):
            event = data.get("event", data)
            return event.get("id") or event.get("event_id")
    except (json.JSONDecodeError, ValueError, TypeError):
        pass
    m = re.search(r'"id"\s*:\s*"([^"]+)"', content)
    if m:
        return m.group(1)
    return None


def _extract_drive_file_id(content: str) -> str | None:
    """Extract the file id from a Google Drive upload/get tool result."""
    def from_value(value: object) -> str | None:
        if isinstance(value, dict):
            for key in ("id", "file_id"):
                candidate = value.get(key)
                if isinstance(candidate, str):
                    return candidate
            for key in ("file", "output", "result"):
                candidate = from_value(value.get(key))
                if candidate:
                    return candidate
            for candidate_value in value.values():
                candidate = from_value(candidate_value)
                if candidate:
                    return candidate
        if isinstance(value, list):
            for item in value:
                candidate = from_value(item)
                if candidate:
                    return candidate
        if isinstance(value, str):
            try:
                return from_value(json.loads(value))
            except (json.JSONDecodeError, ValueError, TypeError):
                return None
        return None

    try:
        parsed = json.loads(content)
        extracted = from_value(parsed)
        if extracted:
            return extracted
    except (json.JSONDecodeError, ValueError, TypeError):
        pass
    m = re.search(r'"(?:id|file_id)"\s*:\s*"([^"]+)"', content)
    if m:
        return m.group(1)
    return None


def _extract_drive_file_id_from_results(tool_results: list[dict]) -> str | None:
    """Extract a Drive file id from any prior lifecycle tool result."""
    for result in tool_results:
        file_id = _extract_drive_file_id(result.get("content", ""))
        if file_id:
            return file_id
    return None


def _tomorrow_10am_utc() -> str:
    """Return an RFC3339 timestamp for tomorrow at 10:00 UTC."""
    from datetime import datetime, timedelta, timezone
    tomorrow = datetime.now(timezone.utc).replace(
        hour=10, minute=0, second=0, microsecond=0,
    ) + timedelta(days=1)
    return tomorrow.strftime("%Y-%m-%dT%H:%M:%SZ")


def _tomorrow_1030am_utc() -> str:
    """Return an RFC3339 timestamp for tomorrow at 10:30 UTC."""
    from datetime import datetime, timedelta, timezone
    tomorrow = datetime.now(timezone.utc).replace(
        hour=10, minute=30, second=0, microsecond=0,
    ) + timedelta(days=1)
    return tomorrow.strftime("%Y-%m-%dT%H:%M:%SZ")


def _make_base(completion_id: str) -> dict:
    return {"id": completion_id, "object": "chat.completion.chunk",
            "created": int(time.time()), "model": "mock-model"}


async def _send_sse(resp: web.StreamResponse, data: dict):
    await resp.write(f"data: {json.dumps(data)}\n\n".encode())


def _preferred_tool_name(available_tool_names: set[str], legacy: str) -> str:
    reborn_name = {
        "echo": "builtin__echo",
        "time": "builtin__time",
    }.get(legacy)
    if reborn_name and reborn_name in available_tool_names:
        return reborn_name
    return legacy


def match_special_response(
    messages: list[dict],
    has_tools: bool,
    available_tool_names: set[str] | None = None,
) -> dict | None:
    """Deterministic issue-specific responses for agent-loop recovery tests."""
    last_user = _last_user_content(messages)
    available_tool_names = available_tool_names or set()
    echo_tool = _preferred_tool_name(available_tool_names, "echo")
    time_tool = _preferred_tool_name(available_tool_names, "time")

    if _conversation_has_user_trigger(messages, LOOP_FOREVER_TRIGGER):
        if has_tools:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": echo_tool,
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
                    "tool_name": time_tool,
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
                "tool_name": time_tool,
                "arguments": {"operation": "broken-operation"},
            },
        }

    if EMPTY_REPLY_TRIGGER.search(last_user):
        return {"type": "empty_text"}

    # Multi-step tool chain: echo first, then time, then text completion.
    # Uses result count (not names) because v2 engine tool results don't
    # always include the tool name in a parseable format.
    if _conversation_has_user_trigger(messages, MULTI_STEP_TRIGGER):
        tool_results = _find_tool_results(messages)
        n = len(tool_results)
        if n == 0 and has_tools:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": echo_tool,
                    "arguments": {"message": "step-one"},
                },
            }
        if n == 1 and has_tools:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": time_tool,
                    "arguments": {"operation": "now"},
                },
            }
        return {
            "type": "text",
            "text": "Multi-step complete: executed echo then time.",
        }

    # ── Lifecycle canary: GitHub issue create → comment → verify ─────────
    m = GITHUB_ISSUE_LIFECYCLE_TRIGGER.search(last_user)
    if m and has_tools:
        owner = m.group("owner")
        repo = m.group("repo")
        tool_results = _find_named_tool_results(messages, "github")
        n = len(tool_results)
        if n == 0:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "github",
                    "arguments": {
                        "action": "create_issue",
                        "owner": owner,
                        "repo": repo,
                        "title": _extract_canary_title(last_user),
                        "body": "Automated canary lifecycle test.",
                        "labels": ["canary"],
                    },
                },
            }
        if n == 1:
            issue_number = _extract_issue_number(tool_results[0].get("content", ""))
            if issue_number:
                return {
                    "type": "tool_call",
                    "tool_call": {
                        "tool_name": "github",
                        "arguments": {
                            "action": "create_issue_comment",
                            "owner": owner,
                            "repo": repo,
                            "issue_number": issue_number,
                            "body": "Canary verification",
                        },
                    },
                }
        if n == 2:
            issue_number = _extract_issue_number(tool_results[0].get("content", ""))
            if issue_number:
                return {
                    "type": "tool_call",
                    "tool_call": {
                        "tool_name": "github",
                        "arguments": {
                            "action": "get_issue",
                            "owner": owner,
                            "repo": repo,
                            "issue_number": issue_number,
                        },
                    },
                }
        return {
            "type": "text",
            "text": "github issue lifecycle complete. Issue created, commented, and verified.",
        }

    # ── Lifecycle canary: Gmail send → list → trash ──────────────────────
    m = GMAIL_ROUNDTRIP_TRIGGER.search(last_user)
    if m and has_tools:
        email = m.group("email")
        tool_results = _find_named_tool_results(messages, "gmail")
        n = len(tool_results)
        if n == 0:
            subject = _extract_canary_subject(last_user)
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "gmail",
                    "arguments": {
                        "action": "send_message",
                        "to": email,
                        "subject": subject,
                        "body": "Canary test",
                    },
                },
            }
        if n == 1:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "gmail",
                    "arguments": {
                        "action": "list_messages",
                        "query": "subject:[canary] newer_than:1h",
                        "max_results": 5,
                    },
                },
            }
        if n == 2:
            message_id = _extract_gmail_message_id(tool_results[0].get("content", ""))
            if message_id:
                return {
                    "type": "tool_call",
                    "tool_call": {
                        "tool_name": "gmail",
                        "arguments": {
                            "action": "trash_message",
                            "message_id": message_id,
                        },
                    },
                }
        return {
            "type": "text",
            "text": "gmail roundtrip complete. Message sent, verified, and trashed.",
        }

    # ── Lifecycle canary: Google Calendar create → list → delete ─────────
    if GCAL_LIFECYCLE_TRIGGER.search(last_user) and has_tools:
        tool_results = _find_named_tool_results(messages, "google_calendar")
        n = len(tool_results)
        if n == 0:
            title = _extract_canary_title(last_user)
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "google_calendar",
                    "arguments": {
                        "action": "create_event",
                        "calendar_id": "primary",
                        "summary": title,
                        "start_datetime": _tomorrow_10am_utc(),
                        "end_datetime": _tomorrow_1030am_utc(),
                        "timezone": "UTC",
                    },
                },
            }
        if n == 1:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "google_calendar",
                    "arguments": {
                        "action": "list_events",
                        "calendar_id": "primary",
                        "max_results": 5,
                    },
                },
            }
        if n == 2:
            event_id = _extract_calendar_event_id(tool_results[0].get("content", ""))
            if event_id:
                return {
                    "type": "tool_call",
                    "tool_call": {
                        "tool_name": "google_calendar",
                        "arguments": {
                            "action": "delete_event",
                            "calendar_id": "primary",
                            "event_id": event_id,
                        },
                    },
                }
        return {
            "type": "text",
            "text": "google_calendar lifecycle complete. Event created, verified, and deleted.",
        }

    # ── Lifecycle canary: Google Drive upload → download ────────────────
    if GDRIVE_UPLOAD_LIFECYCLE_TRIGGER.search(last_user) and has_tools:
        tool_results = _find_named_tool_results(messages, "google_drive")
        n = len(tool_results)
        title = _extract_canary_title(last_user)
        if n == 0:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "google_drive",
                    "arguments": {
                        "action": "upload_file",
                        "name": title,
                        "content": f"Canary Google Drive content for {title}",
                        "mime_type": "text/plain",
                    },
                },
            }
        if n == 1:
            file_id = _extract_drive_file_id_from_results(tool_results)
            if file_id:
                return {
                    "type": "tool_call",
                    "tool_call": {
                        "tool_name": "google_drive",
                        "arguments": {"action": "download_file", "file_id": file_id},
                    },
                }
        return {
            "type": "text",
            "text": "google_drive lifecycle complete. File uploaded and downloaded.",
        }

    # ── Lifecycle canary: Slack send through the real extension ─────────
    m = SLACK_DELIVERY_LIFECYCLE_TRIGGER.search(last_user)
    if m and has_tools:
        tool_results = _find_named_tool_results(messages, "slack_tool")
        if not tool_results:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "slack_tool",
                    "arguments": {
                        "action": "send_message",
                        "channel": m.group("channel"),
                        "text": m.group("marker"),
                    },
                },
            }
        return {
            "type": "text",
            "text": "slack delivery lifecycle complete. Message sent exactly once.",
        }

    # ── Cross-provider canary: GitHub release → Slack ───────────────────
    m = GITHUB_RELEASE_SLACK_TRIGGER.search(last_user)
    if m and has_tools:
        github_results = _find_named_tool_results(messages, "github")
        slack_results = _find_named_tool_results(messages, "slack_tool")
        if not github_results:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "github",
                    "arguments": {
                        "action": "list_releases",
                        "owner": m.group("owner"),
                        "repo": m.group("repo"),
                        "limit": 1,
                    },
                },
            }
        if not slack_results:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "slack_tool",
                    "arguments": {
                        "action": "send_message",
                        "channel": m.group("channel"),
                        "text": m.group("marker"),
                    },
                },
            }
        return {
            "type": "text",
            "text": "github release to slack lifecycle complete.",
        }

    # ── Cross-provider canary: Calendar + Drive → Slack ─────────────────
    m = CALENDAR_DRIVE_SLACK_TRIGGER.search(last_user)
    if m and has_tools:
        if not _find_named_tool_results(messages, "google_calendar"):
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "google_calendar",
                    "arguments": {
                        "action": "list_events",
                        "calendar_id": "primary",
                        "time_min": "2026-01-01T00:00:00Z",
                        "max_results": 5,
                        "query": "PepsiCo",
                    },
                },
            }
        if not _find_named_tool_results(messages, "google_drive"):
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "google_drive",
                    "arguments": {
                        "action": "list_files",
                        "query": "name contains 'PepsiCo'",
                        "page_size": 5,
                    },
                },
            }
        if not _find_named_tool_results(messages, "slack_tool"):
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "slack_tool",
                    "arguments": {
                        "action": "send_message",
                        "channel": m.group("channel"),
                        "text": m.group("marker"),
                    },
                },
            }
        return {"type": "text", "text": "calendar drive to slack complete."}

    # ── Cross-provider canary: Gmail → Slack ────────────────────────────
    m = GMAIL_SLACK_TRIGGER.search(last_user)
    if m and has_tools:
        if not _find_named_tool_results(messages, "gmail"):
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "gmail",
                    "arguments": {
                        "action": "list_messages",
                        "query": "is:unread",
                        "max_results": 5,
                    },
                },
            }
        if not _find_named_tool_results(messages, "slack_tool"):
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "slack_tool",
                    "arguments": {
                        "action": "send_message",
                        "channel": m.group("channel"),
                        "text": m.group("marker"),
                    },
                },
            }
        return {"type": "text", "text": "gmail to slack complete."}

    # ── Cross-provider canary: Slack → Drive → Slack ────────────────────
    m = SLACK_DRIVE_SLACK_TRIGGER.search(last_user)
    if m and has_tools:
        slack_results = _find_named_tool_results(messages, "slack_tool")
        if not slack_results:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "slack_tool",
                    "arguments": {
                        "action": "get_channel_history",
                        "channel": m.group("source"),
                        "limit": 10,
                    },
                },
            }
        if not _find_named_tool_results(messages, "google_drive"):
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "google_drive",
                    "arguments": {
                        "action": "list_files",
                        "query": "name contains 'Reborn QA Brief'",
                        "page_size": 5,
                    },
                },
            }
        if len(slack_results) == 1:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "slack_tool",
                    "arguments": {
                        "action": "send_message",
                        "channel": m.group("target"),
                        "text": m.group("marker"),
                    },
                },
            }
        return {"type": "text", "text": "slack drive to slack complete."}

    # ── Lifecycle canary: Notion search → search again ────────────────────
    if NOTION_SEARCH_LIFECYCLE_TRIGGER.search(last_user) and has_tools:
        tool_results = _find_tool_results(messages)
        n = len(tool_results)
        if n == 0:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "notion_notion_search",
                    "arguments": {
                        "query": "canary",
                    },
                },
            }
        if n == 1:
            return {
                "type": "tool_call",
                "tool_call": {
                    "tool_name": "notion_notion_search",
                    "arguments": {
                        "query": "test",
                    },
                },
            }
        return {
            "type": "text",
            "text": "notion search lifecycle complete. Both searches executed successfully.",
        }

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


async def _broken_stream_before_text(request: web.Request, cid: str) -> web.StreamResponse:
    resp = web.StreamResponse(
        status=200,
        headers={"Content-Type": "text/event-stream", "Cache-Control": "no-cache"},
    )
    await resp.prepare(request)
    await resp.write(f"data: {{\"id\":\"{cid}\",\"choices\":[{{\"delta\":".encode())
    return resp


async def _apply_llm_fault_action(
    request: web.Request,
    cid: str,
    stream: bool,
    action: dict,
) -> web.StreamResponse | web.Response | None:
    action_type = action.get("type")
    if action_type == "delay":
        await asyncio.sleep(float(action.get("seconds", 1.0)))
        return None
    if action_type == "http_error":
        status = int(action.get("status", 502))
        return web.json_response(
            {
                "error": {
                    "message": action.get("message", "scripted mock LLM failure"),
                    "type": "server_error",
                }
            },
            status=status,
        )
    if action_type == "broken_stream_before_text":
        if stream:
            return await _broken_stream_before_text(request, cid)
        return web.json_response(
            {
                "error": {
                    "message": "scripted stream fault requested for non-streaming call",
                    "type": "server_error",
                }
            },
            status=502,
        )
    return None


async def chat_completions(request: web.Request) -> web.StreamResponse:
    """Handle POST /v1/chat/completions and /chat/completions."""
    global _last_chat_request
    body = await request.json()
    _last_chat_request = body
    _chat_requests.append(body)
    messages = body.get("messages", [])
    stream = body.get("stream", False)
    tools = body.get("tools")
    has_tools = bool(tools)
    available_tool_names = _available_tool_names(tools)
    cid = f"mock-{uuid.uuid4().hex[:8]}"

    trace_response = _next_llm_trace_response(
        request.app["llm_trace_state"], messages, available_tool_names
    )
    if trace_response is not None:
        if trace_response["type"] == "tool_calls":
            calls = [
                {
                    "tool_name": tool_call["name"],
                    "arguments": tool_call["arguments"],
                }
                for tool_call in trace_response["tool_calls"]
            ]
            if not stream:
                return _tool_call_response(cid, calls)
            return await _stream_tool_call(request, cid, calls)
        text = trace_response["content"]
        if not stream:
            return _text_response(cid, text)
        return await _stream_text(request, cid, text)

    fault_action = _next_llm_fault_action(messages)
    if fault_action:
        fault_response = await _apply_llm_fault_action(
            request,
            cid,
            stream,
            fault_action,
        )
        if fault_response is not None:
            return fault_response

    slow_response_delay = _conversation_slow_response_delay(messages)
    if slow_response_delay > 0:
        await asyncio.sleep(slow_response_delay)

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
    special = match_special_response(messages, has_tools, available_tool_names)
    if special and _conversation_has_user_trigger(messages, LOOP_FOREVER_TRIGGER):
        return await _dispatch_special_response(request, cid, stream, special)
    # Multi-step chain: must bypass tool-result-summary to issue second tool call
    if special and _conversation_has_user_trigger(messages, MULTI_STEP_TRIGGER):
        return await _dispatch_special_response(request, cid, stream, special)
    # Lifecycle canary multi-step chains: create → verify → cleanup → summarize
    for lifecycle_trigger in (
        GITHUB_ISSUE_LIFECYCLE_TRIGGER,
        GMAIL_ROUNDTRIP_TRIGGER,
        GCAL_LIFECYCLE_TRIGGER,
        GDRIVE_UPLOAD_LIFECYCLE_TRIGGER,
        SLACK_DELIVERY_LIFECYCLE_TRIGGER,
        GITHUB_RELEASE_SLACK_TRIGGER,
        CALENDAR_DRIVE_SLACK_TRIGGER,
        GMAIL_SLACK_TRIGGER,
        SLACK_DRIVE_SLACK_TRIGGER,
        NOTION_SEARCH_LIFECYCLE_TRIGGER,
    ):
        if special and _conversation_has_user_trigger(messages, lifecycle_trigger):
            return await _dispatch_special_response(request, cid, stream, special)

    tool_results = _find_tool_results(messages)
    # #3533: when a multi-step recovery is in progress (e.g. gmail not
    # installed → `tool_install` → retry gmail), the next move is another
    # tool call, not a text summary of the failure. Let `match_tool_call`
    # take precedence over the tool-result-summary fallback whenever it
    # has a follow-up call to emit.
    if tool_results:
        followup = match_tool_call(messages, has_tools)
        if followup:
            if not stream:
                return _tool_call_response(cid, followup)
            return await _stream_tool_call(request, cid, followup)

    reborn_scripted_tool = match_reborn_scripted_tool_response(messages, has_tools)
    if reborn_scripted_tool:
        return await _dispatch_special_response(
            request, cid, stream, reborn_scripted_tool
        )
    if (
        not tool_results
        and _conversation_uses_codeact(messages)
        and re.search(
            r"list.*(?:google|drive).*files|show.*drive",
            _last_user_content(messages),
            re.IGNORECASE,
        )
    ):
        text = (
            "```repl\n"
            f"result = await http(method=\"GET\", url=\"{_github_api_url}/drive/v3/files\")\n"
            "FINAL(str(result))\n"
            "```"
        )
        if not stream:
            return _text_response(cid, text)
        return await _stream_text(request, cid, text)

    # Tool result(s) in messages -> text summary covering every fresh result
    if _conversation_has_active_skill(messages, "pikastream-video-meeting"):
        if _conversation_includes_denial(messages) or _tool_results_include_denial(tool_results):
            text = (
                "The request for shell was denied. "
                "No installation or setup was performed."
            )
            if not stream:
                return _text_response(cid, text)
            return await _stream_text(request, cid, text)
        recent_tool_names = _recent_tool_names(messages)
        if "shell" in recent_tool_names:
            text = (
                "Python dependencies are prepared for the Pika video-meeting skill. "
                "I need an avatar image for the video meeting. "
                "Send me an image, or say \"generate\" and I'll create one for you."
            )
            if not stream:
                return _text_response(cid, text)
            return await _stream_text(request, cid, text)
    if tool_results:
        if _conversation_has_active_skill(messages, "pikastream-video-meeting"):
            if _tool_results_include_denial(tool_results):
                text = (
                    "The request for shell was denied. "
                    "No installation or setup was performed."
                )
                if not stream:
                    return _text_response(cid, text)
                return await _stream_text(request, cid, text)
            if any(tr["name"] == "shell" for tr in tool_results):
                text = (
                    "Python dependencies are prepared for the Pika video-meeting skill. "
                    "I need an avatar image for the video meeting. "
                    "Send me an image, or say \"generate\" and I'll create one for you."
                )
                if not stream:
                    return _text_response(cid, text)
                return await _stream_text(request, cid, text)
        # When the latest user message has an explicit canned reply (e.g. the v2
        # download-chips flow: "produce a downloadable csv and pdf"), return it
        # once its tool calls have run and `match_tool_call` has dedup'd them,
        # instead of the generic multi-tool summary below. Only an explicit match
        # wins — absent one we fall through to the summary.
        #
        # Suppress it on a failed/denied run: `CANNED_RESPONSES` triggers are
        # broad, so a success-style "ready to download" reply must not mask a
        # tool that was denied or errored — those turns fall through to the
        # summary so the real outcome stays visible to assertions.
        tool_run_failed = _tool_results_include_denial(tool_results) or any(
            "error" in tr.get("content", "").lower() for tr in tool_results
        )
        explicit = (
            None
            if tool_run_failed
            else _explicit_canned_response(_last_user_content(messages))
        )
        if explicit is not None:
            if not stream:
                return _text_response(cid, explicit)
            return await _stream_text(request, cid, explicit)
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


def _is_google_token_url(url: str) -> bool:
    """Whether an OAuth `token_url` points at Google.

    Used to gate the `AUTH_LIVE_GOOGLE_*` live-token override so
    non-Google providers (GitHub, Notion, MCP) cannot accidentally
    receive Google tokens during auth-live-seeded canary runs. The
    earlier `not code.startswith("mock_mcp_code")` gate only ruled out
    the MCP code-prefix convention, not GitHub/Notion flows.
    """
    if not url:
        return False
    lowered = url.lower()
    return "googleapis.com" in lowered or "accounts.google.com" in lowered


def _is_github_token_url(url: str) -> bool:
    if not url:
        return False
    return "github.com/login/oauth/access_token" in url.lower()


def _is_slack_token_url(url: str) -> bool:
    if not url:
        return False
    lowered = url.lower()
    return "slack.com/api/oauth.v2.access" in lowered


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

    if code.startswith("mock_mcp_code"):
        if not data.get("token_url", "").endswith("/oauth/token"):
            return web.json_response({"error": "missing_token_url"}, status=400)
        if not data.get("client_id"):
            return web.json_response({"error": "missing_client_id"}, status=400)
        if not data.get("resource"):
            return web.json_response({"error": "missing_resource"}, status=400)

    # When real provider tokens are available (auth-live-seeded canary),
    # return them instead of mock tokens so the extension gets real
    # credentials. Gate strictly on the Google token_url host: the
    # previous `not mcp_code` gate also matched GitHub and Notion
    # exchanges, which would have shipped Google tokens to the wrong
    # extension and masked real provider-specific failures.
    live_access = os.environ.get("AUTH_LIVE_GOOGLE_ACCESS_TOKEN", "").strip()
    live_refresh = os.environ.get("AUTH_LIVE_GOOGLE_REFRESH_TOKEN", "").strip()
    if live_access and _is_google_token_url(data.get("token_url", "")):
        resp = {
            access_token_field: live_access,
            "expires_in": 3600,
        }
        if live_refresh:
            resp["refresh_token"] = live_refresh
        return web.json_response(resp)

    if _is_github_token_url(data.get("token_url", "")):
        return web.json_response({
            access_token_field: EMULATE_GITHUB_BEARER,
            "refresh_token": "mock-github-refresh-token",
            "expires_in": 3600,
        })

    if _is_slack_token_url(data.get("token_url", "")):
        return web.json_response({
            access_token_field: EMULATE_SLACK_BEARER,
            "token_type": "bot",
            "scope": "chat:write,channels:read,channels:history,users:read",
            "bot_user_id": "B_EMULATE_REBORN_BOT",
        })

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

    # When real provider tokens are available (auth-live-seeded canary),
    # return them for Google refreshes instead of validating mock
    # client_id. Gate strictly on the Google token_url host: the
    # previous `not mcp:` gate still matched GitHub and Notion
    # refreshes, which would have returned Google tokens for the wrong
    # provider and hidden refresh-path bugs.
    live_access = os.environ.get("AUTH_LIVE_GOOGLE_ACCESS_TOKEN", "").strip()
    if live_access and _is_google_token_url(data.get("token_url", "")):
        live_refresh = os.environ.get("AUTH_LIVE_GOOGLE_REFRESH_TOKEN", "").strip()
        resp = {
            "access_token": live_access,
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "mock-scope",
        }
        if live_refresh:
            resp["refresh_token"] = live_refresh
        return web.json_response(resp)

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


async def google_oauth_token(request: web.Request) -> web.Response:
    """Minimal Google token endpoint for standalone Reborn OAuth tests."""
    data = await request.post()
    if data.get("grant_type") != "authorization_code":
        return web.json_response({"error": "unsupported_grant_type"}, status=400)
    # Full-path QA uses one pre-consented reusable Google identity. Google may
    # report the account's cumulative grants during a narrower scope-upgrade
    # flow, so the extension-specific codes expose that deterministic union.
    all_reborn_google_scopes = " ".join(
        (
            "https://www.googleapis.com/auth/gmail.readonly",
            "https://www.googleapis.com/auth/gmail.send",
            "https://www.googleapis.com/auth/gmail.modify",
            "https://www.googleapis.com/auth/calendar.readonly",
            "https://www.googleapis.com/auth/calendar.events",
            "https://www.googleapis.com/auth/drive.readonly",
            "https://www.googleapis.com/auth/drive",
            "https://www.googleapis.com/auth/documents",
            "https://www.googleapis.com/auth/documents.readonly",
            "https://www.googleapis.com/auth/spreadsheets",
            "https://www.googleapis.com/auth/spreadsheets.readonly",
            "https://www.googleapis.com/auth/presentations",
            "https://www.googleapis.com/auth/presentations.readonly",
        )
    )
    scopes_by_code = {
        "mock_auth_code": (
            "https://www.googleapis.com/auth/drive.readonly "
            "https://www.googleapis.com/auth/drive"
        ),
        "mock_auth_code_gmail": all_reborn_google_scopes,
        "mock_auth_code_google_calendar": all_reborn_google_scopes,
        "mock_auth_code_google_drive": all_reborn_google_scopes,
        "mock_auth_code_google_docs": all_reborn_google_scopes,
        "mock_auth_code_google_sheets": all_reborn_google_scopes,
        "mock_auth_code_google_slides": all_reborn_google_scopes,
    }
    code = data.get("code")
    scope = scopes_by_code.get(code)
    if scope is None:
        return web.json_response({"error": "invalid_grant"}, status=400)
    live_access = os.environ.get("AUTH_LIVE_GOOGLE_ACCESS_TOKEN", "").strip()
    live_refresh = os.environ.get("AUTH_LIVE_GOOGLE_REFRESH_TOKEN", "").strip()
    if live_access:
        response = {
            "access_token": live_access,
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": scope,
        }
        if live_refresh:
            response["refresh_token"] = live_refresh
        return web.json_response(response)
    return web.json_response(
        {
            "access_token": "mock-token-mock_auth_code",
            "refresh_token": "mock-refreshed-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": scope,
        }
    )


async def oauth_reset(request: web.Request) -> web.Response:
    request.app["oauth_state"] = _new_oauth_state()
    return web.json_response({"ok": True})


async def mcp_state_handler(request: web.Request) -> web.Response:
    return web.json_response(request.app["mcp_state"])


async def mcp_reset(request: web.Request) -> web.Response:
    request.app["mcp_state"] = _new_mcp_state()
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
    request.app["mcp_state"]["requests"].append({
        "method": method,
        "authorization": request.headers.get("Authorization"),
    })

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
    if method == "tools/call":
        params = body.get("params") or {}
        tool_name = params.get("name")
        arguments = params.get("arguments") or {}
        if tool_name == "mock_search":
            query = arguments.get("query", "")
            return web.json_response({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": f"Mock MCP search result for {query or 'empty query'}",
                    }],
                    "is_error": False,
                },
            })
        return web.json_response({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": f"Unknown mock MCP tool: {tool_name}",
                }],
                "is_error": True,
            },
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


# ── Gmail API mocks (#3133 / #3166) ──────────────────────────────────────
#
# Per-app counters that the e2e test for the mission auto-resume path
# inspects to confirm the gmail WASM tool actually fired against this
# mock (rather than hitting the real Gmail API or silently no-oping).
# Stored in `app["gmail_state"]` so each mock_llm instance owns its own
# counters.


def _new_gmail_state() -> dict:
    return {
        "drafts_created": 0,
        "messages_sent": 0,
        "messages_listed": 0,
        "last_draft": None,
        "last_send": None,
    }


async def gmail_create_draft(request: web.Request) -> web.Response:
    """POST /gmail/v1/users/me/drafts — minimal create-draft mock.

    Maps from the gmail WASM tool's `create_draft` action. The request
    body shape mirrors the real Gmail API: `{"message": {"raw": "..."}}`.
    Returns a deterministic draft id so the agent can quote it back.
    """
    body = await request.json()
    state = request.app["gmail_state"]
    state["drafts_created"] += 1
    state["last_draft"] = body
    draft_id = f"mock-draft-{state['drafts_created']}"
    return web.json_response({
        "id": draft_id,
        "message": {
            "id": f"mock-msg-{state['drafts_created']}",
            "threadId": f"mock-thread-{state['drafts_created']}",
            "labelIds": ["DRAFT"],
        },
    })


async def gmail_send_message(request: web.Request) -> web.Response:
    """POST /gmail/v1/users/me/messages/send — minimal send mock.

    Maps from the gmail WASM tool's `send_message` and `reply_to_message`
    actions. Returns the same shape Google does: `{id, threadId, labelIds}`.
    """
    body = await request.json()
    state = request.app["gmail_state"]
    state["messages_sent"] += 1
    state["last_send"] = body
    msg_id = f"mock-msg-{state['messages_sent']}"
    return web.json_response({
        "id": msg_id,
        "threadId": body.get("threadId") or f"mock-thread-{state['messages_sent']}",
        "labelIds": ["SENT"],
    })


async def gmail_list_messages(request: web.Request) -> web.Response:
    """GET /gmail/v1/users/me/messages — minimal list mock.

    Returns one canned message id so the agent has something to quote
    back. The list endpoint only returns ids; the gmail WASM tool then
    fetches metadata for each, which we serve from `gmail_get_message`.
    """
    state = request.app["gmail_state"]
    state["messages_listed"] += 1
    return web.json_response({
        "messages": [{"id": "mock-canned-msg-1", "threadId": "mock-canned-thread-1"}],
        "resultSizeEstimate": 1,
    })


async def gmail_get_message(request: web.Request) -> web.Response:
    """GET /gmail/v1/users/me/messages/{id} — minimal get-message mock."""
    msg_id = request.match_info["id"]
    return web.json_response({
        "id": msg_id,
        "threadId": "mock-canned-thread-1",
        "labelIds": ["INBOX", "UNREAD"],
        "snippet": "Mock canned snippet for the e2e test",
        "payload": {
            "headers": [
                {"name": "From", "value": "test@example.com"},
                {"name": "To", "value": "owner@example.com"},
                {"name": "Subject", "value": "Mock canned subject"},
                {"name": "Date", "value": "Mon, 1 Jan 2026 00:00:00 +0000"},
            ],
            "body": {"data": ""},
        },
    })


async def gmail_state_handler(request: web.Request) -> web.Response:
    """GET /__mock/gmail/state — read the gmail counters in tests."""
    return web.json_response(request.app["gmail_state"])


async def gmail_state_reset(request: web.Request) -> web.Response:
    """POST /__mock/gmail/reset — clear counters between tests."""
    request.app["gmail_state"] = _new_gmail_state()
    return web.json_response({"ok": True})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()
    app = web.Application()
    app["oauth_state"] = _new_oauth_state()
    app["mcp_state"] = _new_mcp_state()
    app["gmail_state"] = _new_gmail_state()
    app["llm_trace_state"] = _new_llm_trace_state()
    # Register both /v1/ and non-/v1/ paths (rig-core omits the /v1/ prefix)
    app.router.add_post("/v1/chat/completions", chat_completions)
    app.router.add_post("/chat/completions", chat_completions)
    app.router.add_get("/v1/models", models)
    app.router.add_get("/models", models)
    app.router.add_post("/oauth/exchange", oauth_exchange)
    app.router.add_post("/oauth/refresh", oauth_refresh)
    app.router.add_post("/token", google_oauth_token)
    app.router.add_get("/__mock/oauth/state", oauth_state_handler)
    app.router.add_post("/__mock/oauth/reset", oauth_reset)
    app.router.add_get("/__mock/mcp/state", mcp_state_handler)
    app.router.add_post("/__mock/mcp/reset", mcp_reset)

    async def set_github_api_url(request: web.Request) -> web.Response:
        global _github_api_url
        body = await request.json()
        _github_api_url = body["url"]
        return web.json_response({"ok": True, "url": _github_api_url})

    async def get_github_api_url(request: web.Request) -> web.Response:
        return web.json_response({"url": _github_api_url})

    async def get_last_chat_request(request: web.Request) -> web.Response:
        return web.json_response(_last_chat_request or {})

    async def get_chat_requests(request: web.Request) -> web.Response:
        return web.json_response({"requests": _chat_requests})

    async def reset_chat_requests(request: web.Request) -> web.Response:
        global _last_chat_request
        _last_chat_request = None
        _chat_requests.clear()
        return web.json_response({"ok": True})

    async def set_llm_trace(request: web.Request) -> web.Response:
        body = await request.json()
        try:
            request.app["llm_trace_state"] = _parse_llm_trace(
                body.get("trace"), body.get("source")
            )
        except ValueError as error:
            return web.json_response({"ok": False, "error": str(error)}, status=400)
        return web.json_response({"ok": True})

    async def get_llm_trace(request: web.Request) -> web.Response:
        state = request.app["llm_trace_state"]
        return web.json_response(
            {
                "source": state["source"],
                "next_response": state["next_response"],
                "response_count": len(state["responses"]),
                "complete": bool(state["responses"])
                and state["next_response"] == len(state["responses"]),
                "error": state["error"],
            }
        )

    async def reset_llm_trace(request: web.Request) -> web.Response:
        request.app["llm_trace_state"] = _new_llm_trace_state()
        return web.json_response({"ok": True})

    async def set_llm_faults(request: web.Request) -> web.Response:
        body = await request.json()
        faults = body.get("faults", [])
        if not isinstance(faults, list):
            return web.json_response(
                {"ok": False, "error": "faults must be a list"},
                status=400,
            )
        parsed_scripts = []
        for fault in faults:
            if not isinstance(fault, dict):
                return web.json_response(
                    {"ok": False, "error": "each fault must be an object"},
                    status=400,
                )
            match_text = fault.get("match")
            actions = fault.get("actions")
            if not isinstance(match_text, str) or not match_text:
                return web.json_response(
                    {"ok": False, "error": "fault.match must be a non-empty string"},
                    status=400,
                )
            if not isinstance(actions, list):
                return web.json_response(
                    {"ok": False, "error": "fault.actions must be a list"},
                    status=400,
                )
            if not all(isinstance(action, dict) for action in actions):
                return web.json_response(
                    {"ok": False, "error": "fault.actions entries must be objects"},
                    status=400,
                )
            parsed_scripts.append(
                {
                    "match": match_text,
                    "actions": [dict(action) for action in actions],
                    "applied": 0,
                }
            )
        _reset_llm_fault_scripts()
        _llm_fault_scripts.extend(parsed_scripts)
        return web.json_response({"ok": True, "faults": _llm_fault_scripts})

    async def get_llm_faults(request: web.Request) -> web.Response:
        return web.json_response({"faults": _llm_fault_scripts})

    async def reset_llm_faults(request: web.Request) -> web.Response:
        _reset_llm_fault_scripts()
        return web.json_response({"ok": True})

    app.router.add_post("/__mock/set_github_api_url", set_github_api_url)
    app.router.add_get("/__mock/github_api_url", get_github_api_url)
    app.router.add_get("/__mock/last_chat_request", get_last_chat_request)
    app.router.add_get("/__mock/chat_requests", get_chat_requests)
    app.router.add_post("/__mock/chat_requests/reset", reset_chat_requests)
    app.router.add_post("/__mock/llm_trace", set_llm_trace)
    app.router.add_get("/__mock/llm_trace", get_llm_trace)
    app.router.add_post("/__mock/llm_trace/reset", reset_llm_trace)
    app.router.add_post("/__mock/llm_faults", set_llm_faults)
    app.router.add_get("/__mock/llm_faults", get_llm_faults)
    app.router.add_post("/__mock/llm_faults/reset", reset_llm_faults)
    # Mock MCP server endpoints
    app.router.add_post("/mcp", mcp_endpoint)
    app.router.add_post("/mcp-400", mcp_endpoint_400)
    app.router.add_get("/.well-known/oauth-protected-resource", mcp_protected_resource)
    app.router.add_get("/.well-known/oauth-protected-resource/{tail:.*}", mcp_protected_resource)
    app.router.add_get("/.well-known/oauth-authorization-server", mcp_auth_server_metadata)
    app.router.add_get("/.well-known/oauth-authorization-server/{tail:.*}", mcp_auth_server_metadata)
    app.router.add_post("/oauth/register", mcp_oauth_register)
    app.router.add_post("/oauth/token", mcp_oauth_token)
    # Gmail API mocks (consumed by the WASM gmail tool when
    # IRONCLAW_TEST_HTTP_REWRITE_MAP routes gmail.googleapis.com here).
    app.router.add_post("/gmail/v1/users/me/drafts", gmail_create_draft)
    app.router.add_post("/gmail/v1/users/me/messages/send", gmail_send_message)
    app.router.add_get("/gmail/v1/users/me/messages", gmail_list_messages)
    app.router.add_get("/gmail/v1/users/me/messages/{id}", gmail_get_message)
    app.router.add_get("/__mock/gmail/state", gmail_state_handler)
    app.router.add_post("/__mock/gmail/reset", gmail_state_reset)

    async def start():
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        print(f"MOCK_LLM_PORT={port}", flush=True)
        app["port"] = port  # used by MCP handlers
        await asyncio.Event().wait()

    asyncio.run(start())


if __name__ == "__main__":
    main()
