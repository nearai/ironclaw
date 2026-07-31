"""Recorded-model trace validation, replay, and HTTP control surface."""

import json
import re
from copy import deepcopy

from aiohttp import web


def _message_text(message: dict) -> str:
    content = message.get("content") or ""
    if isinstance(content, list):
        parts = []
        for part in content:
            if part.get("type") == "text":
                parts.append(part.get("text") or "")
            else:
                try:
                    parts.append(json.dumps(part, sort_keys=True))
                except TypeError:
                    parts.append(str(part))
        content = " ".join(parts)
    return content


def _last_user_content(messages: list[dict]) -> str:
    for message in reversed(messages):
        if message.get("role") == "user":
            return _message_text(message)
    return ""


def _extract_tool_name(message: dict) -> str:
    """Extract a tool name from either the explicit field or wrapped output."""
    name = message.get("name")
    if name:
        return name
    content = message.get("content", "")
    match = re.search(r'<tool_output\s+name="([^"]+)"', content)
    if match:
        return match.group(1)
    return "unknown"


def _find_tool_results(
    messages: list[dict],
    *,
    after_latest_user: bool = True,
) -> list[dict]:
    """Collect tool results, optionally limited to the most recent user turn."""
    last_user_index = -1
    if after_latest_user:
        for index in range(len(messages) - 1, -1, -1):
            if messages[index].get("role") == "user":
                last_user_index = index
                break

    tool_call_names: dict[str, str] = {}
    results: list[dict] = []
    for index in range(last_user_index + 1, len(messages)):
        message = messages[index]
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
            results.append(
                {
                    "name": name,
                    "tool_call_id": message.get("tool_call_id"),
                    "content": message.get("content", ""),
                }
            )
    return results


def _find_named_tool_results(messages: list[dict], name: str) -> list[dict]:
    return [
        result for result in _find_tool_results(messages) if result.get("name") == name
    ]


def _new_llm_trace_state() -> dict:
    return {
        "source": None,
        "responses": [],
        "next_response": 0,
        "expected_user_inputs": {},
        "request_hints": [],
        "pending_tool_calls": [],
        "tool_call_id_aliases": {},
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
    seen_tool_call_ids: set[str] = set()
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
                    or not isinstance(tool_call.get("id"), str)
                    or not tool_call["id"]
                    or not isinstance(tool_call.get("name"), str)
                    or not isinstance(tool_call.get("arguments"), dict)
                ):
                    raise ValueError(
                        f"trace.steps[{index}].tool_calls[{tool_index}] is invalid"
                    )
                tool_call_id = tool_call["id"]
                if tool_call_id in seen_tool_call_ids:
                    raise ValueError(
                        f"trace.steps[{index}].tool_calls[{tool_index}] "
                        f"reuses tool call id {tool_call_id!r}"
                    )
                seen_tool_call_ids.add(tool_call_id)
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
        "pending_tool_calls": [],
        "tool_call_id_aliases": {},
        "error": None,
    }


def next_llm_trace_response(
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

    try:
        capture_trace_tool_call_id_aliases(state, messages)
    except ValueError as error:
        state["error"] = str(error)
        raise web.HTTPConflict(text=state["error"]) from error

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
            response["tool_calls"] = resolve_trace_result_bindings(
                response["tool_calls"],
                trace_tool_results_with_recorded_ids(state, messages),
            )
        except ValueError as error:
            state["error"] = str(error)
            raise web.HTTPConflict(text=state["error"]) from error
        state["pending_tool_calls"] = deepcopy(response["tool_calls"])

    state["next_response"] += 1
    return response


def resolve_trace_result_bindings(
    value: object,
    tool_results: list[dict],
) -> object:
    """Resolve exact tool-call-ID/JSON-Pointer markers recursively."""
    if isinstance(value, list):
        return [resolve_trace_result_bindings(item, tool_results) for item in value]
    if not isinstance(value, dict):
        return value
    if "$trace_result" not in value:
        return {
            key: resolve_trace_result_bindings(item, tool_results)
            for key, item in value.items()
        }
    if set(value) != {"$trace_result"}:
        raise ValueError("$trace_result marker object must contain only $trace_result")

    binding = value["$trace_result"]
    if not isinstance(binding, dict) or set(binding) != {
        "tool_call_id",
        "pointer",
    }:
        raise ValueError("$trace_result must contain exactly tool_call_id and pointer")
    tool_call_id = binding["tool_call_id"]
    pointer = binding["pointer"]
    if not isinstance(tool_call_id, str) or not tool_call_id:
        raise ValueError("$trace_result.tool_call_id must be a non-empty string")
    if not isinstance(pointer, str) or (pointer and not pointer.startswith("/")):
        raise ValueError("$trace_result.pointer must be empty or start with '/'")

    matching_results = [
        candidate
        for candidate in tool_results
        if candidate.get("tool_call_id") == tool_call_id
    ]
    if not matching_results:
        observed_ids = [candidate.get("tool_call_id") for candidate in tool_results]
        raise ValueError(
            f"trace result has no tool call with id {tool_call_id!r}; "
            f"observed tool call IDs: {observed_ids}"
        )
    if len(matching_results) > 1:
        raise ValueError(
            f"trace result has multiple tool results with id {tool_call_id!r}"
        )
    result = matching_results[0]
    parsed_content = _parse_trace_result_content(result.get("content"))
    content = _canonical_trace_result_payload(parsed_content)
    if content is parsed_content and _is_trace_result_evidence(parsed_content):
        raise ValueError(
            f"trace result for tool call {tool_call_id!r} "
            f"has no JSON Pointer {pointer!r}"
        )
    try:
        return _resolve_trace_json_pointer(content, pointer)
    except (KeyError, IndexError, TypeError, ValueError) as error:
        raise ValueError(
            f"trace result for tool call {tool_call_id!r} "
            f"has no JSON Pointer {pointer!r}"
        ) from error


def _resolve_trace_json_pointer(document: object, pointer: str) -> object:
    current = document
    if not pointer:
        return current
    for raw_token in pointer[1:].split("/"):
        token = _decode_trace_pointer_token(raw_token)
        if isinstance(current, dict):
            current = current[token]
        elif isinstance(current, list):
            if not token.isascii() or not token.isdecimal():
                raise ValueError("array pointer token must be a decimal index")
            current = current[int(token)]
        else:
            raise TypeError("pointer traversed a scalar")
    return current


def _decode_trace_pointer_token(token: str) -> str:
    decoded = []
    index = 0
    while index < len(token):
        if token[index] != "~":
            decoded.append(token[index])
            index += 1
            continue
        if index + 1 >= len(token) or token[index + 1] not in {"0", "1"}:
            raise ValueError("invalid JSON Pointer escape")
        decoded.append("~" if token[index + 1] == "0" else "/")
        index += 2
    return "".join(decoded)


def _canonical_trace_result_payload(content: object) -> object:
    if not isinstance(content, dict):
        return content
    if not _is_trace_result_evidence(content):
        return content
    detail = content.get("detail")
    if not isinstance(detail, dict) or not isinstance(detail.get("preview"), str):
        return content
    byte_len = detail.get("byte_len")
    total_bytes = detail.get("total_bytes")
    if (
        isinstance(byte_len, bool)
        or not isinstance(byte_len, int)
        or isinstance(total_bytes, bool)
        or not isinstance(total_bytes, int)
        or byte_len != total_bytes
        or detail.get("next_offset") is not None
    ):
        return content
    try:
        return json.loads(detail["preview"])
    except json.JSONDecodeError:
        return content


def _is_trace_result_evidence(content: object) -> bool:
    return isinstance(content, dict) and {
        "schema_version",
        "status",
        "trust",
    } <= set(content)


def capture_trace_tool_call_id_aliases(state: dict, messages: list[dict]) -> None:
    """Map stable fixture call IDs to IDs normalized by the provider stack."""
    pending = state.get("pending_tool_calls") or []
    if not pending:
        return
    actual_calls = next(
        (
            message.get("tool_calls") or []
            for message in reversed(messages)
            if message.get("role") == "assistant" and message.get("tool_calls")
        ),
        [],
    )
    if len(actual_calls) != len(pending):
        state["pending_tool_calls"] = []
        raise ValueError(
            "trace tool-call alias count mismatch: "
            f"recorded {len(pending)}, observed {len(actual_calls)}"
        )

    recorded_calls_by_identity: dict[tuple[str, str], list[dict]] = {}
    for recorded in pending:
        identity = _trace_tool_call_identity(recorded)
        if identity is not None:
            recorded_calls_by_identity.setdefault(identity, []).append(recorded)

    actual_calls_by_identity: dict[tuple[str, str], list[dict]] = {}
    for actual in actual_calls:
        identity = _trace_tool_call_identity(actual)
        if identity is not None:
            actual_calls_by_identity.setdefault(identity, []).append(actual)

    aliases = state["tool_call_id_aliases"]
    for identity, recorded_calls in recorded_calls_by_identity.items():
        actual_candidates = actual_calls_by_identity.get(identity, [])
        if len(recorded_calls) > 1 or len(actual_candidates) > 1:
            raise ValueError(
                "ambiguous trace tool-call aliases for normalized tool name "
                f"{identity[0]!r}; repeated calls must have distinct arguments"
            )
        if len(recorded_calls) != 1 or len(actual_candidates) != 1:
            continue
        recorded = recorded_calls[0]
        actual = actual_candidates[0]
        recorded_id = recorded.get("id")
        actual_id = actual.get("id")
        if (
            isinstance(recorded_id, str)
            and recorded_id
            and isinstance(actual_id, str)
            and actual_id
        ):
            aliases[recorded_id] = actual_id
    state["pending_tool_calls"] = []


def _trace_tool_call_identity(call: dict) -> tuple[str, str] | None:
    function = call.get("function")
    if isinstance(function, dict):
        name = function.get("name") or call.get("name")
        arguments = function.get("arguments", call.get("arguments"))
    else:
        name = call.get("name")
        arguments = call.get("arguments")
    if isinstance(arguments, str):
        try:
            arguments = json.loads(arguments)
        except json.JSONDecodeError:
            return None
    if not isinstance(name, str) or not isinstance(arguments, dict):
        return None
    return (
        name.replace("-", "_"),
        json.dumps(arguments, sort_keys=True, separators=(",", ":")),
    )


def trace_tool_results_with_recorded_ids(
    state: dict,
    messages: list[dict],
) -> list[dict]:
    """Present runtime results under their stable fixture IDs."""
    runtime_to_recorded = {
        runtime_id: recorded_id
        for recorded_id, runtime_id in state["tool_call_id_aliases"].items()
    }
    results = []
    for observed in _find_tool_results(messages, after_latest_user=False):
        rewritten = dict(observed)
        runtime_id = rewritten.get("tool_call_id")
        rewritten["tool_call_id"] = runtime_to_recorded.get(runtime_id, runtime_id)
        results.append(rewritten)
    return results


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


async def _set_llm_trace(request: web.Request) -> web.Response:
    body = await request.json()
    try:
        request.app["llm_trace_state"] = _parse_llm_trace(
            body.get("trace"), body.get("source")
        )
    except ValueError as error:
        return web.json_response({"ok": False, "error": str(error)}, status=400)
    return web.json_response({"ok": True})


async def _get_llm_trace(request: web.Request) -> web.Response:
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


async def _reset_llm_trace(request: web.Request) -> web.Response:
    request.app["llm_trace_state"] = _new_llm_trace_state()
    return web.json_response({"ok": True})


def register_llm_trace_routes(app: web.Application) -> None:
    """Install recorded-trace state and its control endpoints."""
    app["llm_trace_state"] = _new_llm_trace_state()
    app.router.add_post("/__mock/llm_trace", _set_llm_trace)
    app.router.add_get("/__mock/llm_trace", _get_llm_trace)
    app.router.add_post("/__mock/llm_trace/reset", _reset_llm_trace)
