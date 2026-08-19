"""Pure compilation of recorded traces into executable provider journeys."""

import json
from copy import deepcopy
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from journey_types import ProviderJourneyReplayFacts, SlackChannelFixture

MISSING_SLACK_CHANNEL_ID = "C_REBORN_QA_10E_MISSING"


@dataclass(frozen=True)
class CompiledProviderJourneyTrace:
    """An execution copy plus immutable facts extracted from its declaration."""

    source: str
    trace: dict[str, Any]


def compile_provider_journey_trace(
    recorded_trace: dict[str, Any],
    *,
    source: str,
    facts: ProviderJourneyReplayFacts,
    provider_tools: frozenset[str],
    slack_state: dict[str, str] | None,
) -> CompiledProviderJourneyTrace:
    """Compile without mutating the committed recording loaded by the caller."""
    trace = _provider_leg(deepcopy(recorded_trace), provider_tools)
    _normalize_google_arguments(trace, facts.google_spreadsheet_id)
    if slack_state is not None:
        _normalize_slack_arguments(trace, slack_state, facts.slack_channel)
    if facts.expected_capability_failure is not None:
        trace["steps"][-1]["request_hint"] = {
            "expected_failed_tool_result_contains": (facts.expected_capability_failure)
        }
    return CompiledProviderJourneyTrace(source=source, trace=trace)


def recorded_trace_uses_tool_prefix(
    recorded_trace: dict[str, Any], prefix: str
) -> bool:
    return any(
        call["name"].startswith(prefix)
        for step in recorded_trace["steps"]
        for call in step["response"].get("tool_calls", [])
    )


def recorded_provider_calls(
    trace: dict[str, Any], provider_tools: frozenset[str]
) -> list[dict]:
    return [
        call
        for step in trace["steps"]
        for call in step["response"].get("tool_calls", [])
        if call["name"] in provider_tools
    ]


def _provider_leg(
    trace: dict[str, Any], provider_tools: frozenset[str]
) -> dict[str, Any]:
    """Keep the recorded provider decisions and final response in order."""
    provider_steps = []
    final_text = None
    for step in trace["steps"][1:]:
        response = step["response"]
        if response["type"] == "tool_calls":
            calls = [
                call
                for call in response["tool_calls"]
                if call["name"] in provider_tools
            ]
            if calls:
                provider_steps.append(
                    {"response": {"type": "tool_calls", "tool_calls": calls}}
                )
        elif response["type"] == "text":
            final_text = response

    assert provider_steps, "provider journey must retain at least one tool call"
    assert final_text is not None, "provider journey must retain a final response"
    return {
        **trace,
        "steps": [trace["steps"][0], *provider_steps, {"response": final_text}],
    }


def _result_binding(tool_call_id: str, pointer: str) -> dict:
    return {
        "$trace_result": {
            "tool_call_id": tool_call_id,
            "pointer": pointer,
        }
    }


def _normalize_google_arguments(trace: dict[str, Any], seeded_spreadsheet: str) -> None:
    _pin_google_provider_arguments(trace)
    _rewrite_google_docs_calls(trace)
    _bind_google_sheets_arguments(trace, seeded_spreadsheet)
    _prune_empty_tool_call_steps(trace)


def _pin_google_provider_arguments(trace: dict[str, Any]) -> None:
    for step in trace["steps"]:
        for call in step["response"].get("tool_calls", []):
            name = call["name"]
            arguments = call["arguments"]
            _replace_value(arguments, "EMAIL_REDACTED", "e2e.google@example.com")
            if name == "gmail__get_message":
                arguments["message_id"] = "msg_emulate_near_inbound"
            elif name == "google-drive__download_file":
                arguments["file_id"] = "drv_pepsico_account_brief"


def _rewrite_google_docs_calls(trace: dict[str, Any]) -> None:
    document_upload_contents = []
    for step in trace["steps"]:
        for call in step["response"].get("tool_calls", []):
            if call["name"] == "google-docs__create_document":
                document_upload_contents.append("")
            elif call["name"] == "google-docs__insert_text":
                for index, content in enumerate(document_upload_contents):
                    if content == "":
                        document_upload_contents[index] = call["arguments"].get(
                            "text", ""
                        )
                        break
    document_upload_index = 0
    created_document_call_id = None

    for step in trace["steps"]:
        if "tool_calls" not in step["response"]:
            continue
        normalized_calls = []
        for call in step["response"].get("tool_calls", []):
            name = call["name"]
            arguments = call["arguments"]

            if name == "google-docs__create_document":
                created_document_call_id = call.get("id")
                call["name"] = "google-drive__upload_file"
                content = (
                    document_upload_contents[document_upload_index]
                    if document_upload_index < len(document_upload_contents)
                    else ""
                )
                document_upload_index += 1
                call["arguments"] = {
                    "name": arguments["title"],
                    "content": content,
                    "mime_type": "text/plain",
                }
            elif name == "google-docs__insert_text":
                continue
            elif name.startswith("google-docs__") and "document_id" in arguments:
                call["name"] = "google-drive__download_file"
                call["arguments"] = {
                    "file_id": (
                        _result_binding(created_document_call_id, "/file/id")
                        if created_document_call_id
                        else "drv_near_ai_strategy"
                    )
                }
            normalized_calls.append(call)
        step["response"]["tool_calls"] = normalized_calls


def _bind_google_sheets_arguments(
    trace: dict[str, Any], seeded_spreadsheet: str
) -> None:
    created_spreadsheet_call_id = None
    seeded_sheet_id = 0
    for step in trace["steps"]:
        for call in step["response"].get("tool_calls", []):
            name = call["name"]
            arguments = call["arguments"]
            if name == "google-sheets__create_spreadsheet":
                created_spreadsheet_call_id = call.get("id")
            elif name.startswith("google-sheets__"):
                if "spreadsheet_id" in arguments:
                    arguments["spreadsheet_id"] = (
                        _result_binding(created_spreadsheet_call_id, "/spreadsheet_id")
                        if created_spreadsheet_call_id
                        else seeded_spreadsheet
                    )
                if "sheet_id" in arguments:
                    arguments["sheet_id"] = (
                        _result_binding(
                            created_spreadsheet_call_id,
                            "/sheets/0/sheet_id",
                        )
                        if created_spreadsheet_call_id
                        else seeded_sheet_id
                    )


def _prune_empty_tool_call_steps(trace: dict[str, Any]) -> None:
    trace["steps"] = [
        step
        for step in trace["steps"]
        if step["response"].get("type") != "tool_calls"
        or step["response"].get("tool_calls")
    ]


def _normalize_slack_arguments(
    trace: dict[str, Any],
    slack_state: dict[str, str],
    channel_fixture: SlackChannelFixture,
) -> None:
    channel_id = (
        MISSING_SLACK_CHANNEL_ID
        if channel_fixture is SlackChannelFixture.MISSING
        else slack_state["channel_id"]
    )
    for step in trace["steps"]:
        for call in step["response"].get("tool_calls", []):
            if not call["name"].startswith("slack__"):
                continue
            arguments = call["arguments"]
            if "channel" in arguments:
                arguments["channel"] = channel_id
            if "conversation" in arguments:
                arguments["conversation"] = channel_id
            if "user_id" in arguments:
                arguments["user_id"] = slack_state["reviewer_id"]
            if "thread_ts" in arguments:
                arguments["thread_ts"] = slack_state["thread_ts"]
            if "text" in arguments:
                arguments["text"] = arguments["text"].replace(
                    "SLACK_ID_REDACTED", slack_state["reviewer_id"]
                )
            if "query" in arguments:
                arguments["query"] = arguments["query"].replace(
                    "SLACK_ID_REDACTED", slack_state["channel_name"]
                )


def _replace_value(value: object, old: str, new: str) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if child == old:
                value[key] = new
            else:
                _replace_value(child, old, new)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            if child == old:
                value[index] = new
            else:
                _replace_value(child, old, new)


def load_recorded_trace(trace_path: Path) -> dict[str, Any]:
    """Load one recording without applying execution-time normalization."""
    return json.loads(trace_path.read_text(encoding="utf-8"))
