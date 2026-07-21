"""Execute every harvested live-QA trace and probe its Emulate-backed tools.

Every trace is replayed response-by-response through the same mock LLM adapter
used by standalone Reborn. Provider operations supported by the pinned Emulate
fork are also exercised against seeded provider state. The closed inventory
fails if a harvested operation lacks a provider-backed assertion.
"""

import json
from pathlib import Path

import httpx
import pytest

from emulate_provider import (
    github_json,
    google_headers,
    raw_mime,
    slack_headers,
    slack_post,
)

pytest_plugins = ["reborn_webui_harness"]

ROOT = Path(__file__).resolve().parents[3]
TRACE_DIR = ROOT / "tests/fixtures/llm_traces/reborn_qa/live_canary"
MANIFEST_PATH = TRACE_DIR / "case-manifest.json"

EMULATE_SUPPORTED_TOOLS = {
    "gmail__get_message",
    "gmail__list_messages",
    "gmail__send_message",
    "github__get_authenticated_user",
    "github__get_repo",
    "github__list_releases",
    "google-calendar__list_calendars",
    "google-calendar__list_events",
    "google-docs__create_document",
    "google-docs__insert_text",
    "google-docs__read_content",
    "google-drive__download_file",
    "google-drive__list_files",
    "google-sheets__append_values",
    "google-sheets__create_spreadsheet",
    "google-sheets__get_spreadsheet",
    "google-sheets__read_values",
    "google-sheets__rename_sheet",
    "google-sheets__write_values",
    "slack__get_conversation_history",
    "slack__get_conversation_info",
    "slack__get_thread_replies",
    "slack__get_user_info",
    "slack__list_conversations",
    "slack__search_messages",
    "slack__send_message",
    "slack__whoami",
}

EMULATE_UNSUPPORTED_TOOLS: set[str] = set()

PROVIDER_PREFIXES = (
    "gmail__",
    "github__",
    "google-calendar__",
    "google-docs__",
    "google-drive__",
    "google-sheets__",
    "slack__",
)


def _model_cases() -> list[str]:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    no_model = set(manifest["no_model_cases"])
    return [case for case in manifest["selected_cases"] if case not in no_model]


MODEL_CASES = _model_cases()


def _load_case(case: str) -> dict:
    return json.loads((TRACE_DIR / f"{case}.json").read_text(encoding="utf-8"))


def _tool_calls(trace: dict) -> list[dict]:
    return [
        call
        for step in trace["steps"]
        for call in step["response"].get("tool_calls", [])
    ]


def _tool_definitions(trace: dict) -> list[dict]:
    names = sorted({call["name"] for call in _tool_calls(trace)})
    return [
        {
            "type": "function",
            "function": {
                "name": name,
                "description": f"Recorded QA capability {name}",
                "parameters": {"type": "object", "additionalProperties": True},
            },
        }
        for name in names
    ]


async def _install_trace(mock_llm_server: str, case: str, trace: dict) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_trace",
            json={"source": f"{case}.json", "trace": trace},
            timeout=15,
        )
    response.raise_for_status()


async def _replay_every_response(
    mock_llm_server: str,
    case: str,
    trace: dict,
) -> None:
    """Replay and compare every recorded response over OpenAI-compatible HTTP."""
    await _install_trace(mock_llm_server, case, trace)
    messages = []
    tools = _tool_definitions(trace)
    response_count = 0

    async with httpx.AsyncClient() as client:
        for step_index, step in enumerate(trace["steps"]):
            expected = step["response"]
            if expected["type"] == "user_input":
                messages.append({"role": "user", "content": expected["content"]})
                continue

            response = await client.post(
                f"{mock_llm_server}/v1/chat/completions",
                json={
                    "model": "mock-model",
                    "messages": messages,
                    "tools": tools,
                    "stream": False,
                },
                timeout=15,
            )
            assert response.status_code == 200, (
                f"{case} step {step_index} failed: {response.text}"
            )
            message = response.json()["choices"][0]["message"]
            response_count += 1

            if expected["type"] == "text":
                assert message["content"] == expected["content"]
                messages.append({"role": "assistant", "content": message["content"]})
                continue

            actual_calls = message.get("tool_calls", [])
            expected_calls = expected["tool_calls"]
            assert len(actual_calls) == len(expected_calls), case
            for actual, recorded in zip(actual_calls, expected_calls, strict=True):
                assert actual["function"]["name"] == recorded["name"]
                assert json.loads(actual["function"]["arguments"]) == recorded["arguments"]

            messages.append(
                {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": actual_calls,
                }
            )
            messages.extend(
                {
                    "role": "tool",
                    "tool_call_id": actual["id"],
                    "content": json.dumps({"ok": True, "source": "local replay"}),
                }
                for actual in actual_calls
            )

        state = await client.get(
            f"{mock_llm_server}/__mock/llm_trace",
            timeout=15,
        )
    state.raise_for_status()
    assert state.json() == {
        "source": f"{case}.json",
        "next_response": response_count,
        "response_count": response_count,
        "complete": True,
        "error": None,
    }


async def _probe_google_tool(
    client: httpx.AsyncClient,
    base_url: str,
    tool: str,
    case: str,
) -> None:
    headers = google_headers()
    if tool == "gmail__list_messages":
        response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages",
            headers=headers,
            params={"maxResults": 5},
        )
        response.raise_for_status()
        assert response.json().get("messages"), case
    elif tool == "gmail__get_message":
        response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages/msg_emulate_near_inbound",
            headers=headers,
            params={"format": "full"},
        )
        response.raise_for_status()
        assert response.json()["id"] == "msg_emulate_near_inbound"
    elif tool == "gmail__send_message":
        response = await client.post(
            f"{base_url}/gmail/v1/users/me/messages/send",
            headers=headers,
            json={
                "raw": raw_mime(
                    to="qa-recipient@example.com",
                    subject=f"{case} local replay",
                    body="Sent by the harvested trace Emulate probe.",
                )
            },
        )
        response.raise_for_status()
        assert "SENT" in response.json().get("labelIds", [])
    elif tool == "google-calendar__list_calendars":
        response = await client.get(
            f"{base_url}/calendar/v3/users/me/calendarList",
            headers=headers,
        )
        response.raise_for_status()
        assert any(item["id"] == "primary" for item in response.json()["items"])
    elif tool == "google-calendar__list_events":
        response = await client.get(
            f"{base_url}/calendar/v3/calendars/primary/events",
            headers=headers,
        )
        response.raise_for_status()
        assert response.json().get("items"), case
    elif tool.startswith("google-docs__"):
        await _probe_google_docs_tool(client, base_url, headers, tool, case)
    elif tool == "google-drive__list_files":
        response = await client.get(
            f"{base_url}/drive/v3/files",
            headers=headers,
            params={"q": "trashed=false"},
        )
        response.raise_for_status()
        assert any(
            item["id"] == "drv_reborn_qa_brief"
            for item in response.json()["files"]
        )
    elif tool == "google-drive__download_file":
        response = await client.get(
            f"{base_url}/drive/v3/files/drv_pepsico_account_brief",
            headers=headers,
            params={"alt": "media"},
        )
        response.raise_for_status()
        assert "PepsiCo account brief" in response.text
    elif tool.startswith("google-sheets__"):
        await _probe_google_sheets_tool(client, base_url, headers, tool, case)
    else:
        raise AssertionError(f"missing Google Emulate probe for {tool}")


async def _probe_google_docs_tool(
    client: httpx.AsyncClient,
    base_url: str,
    headers: dict[str, str],
    tool: str,
    case: str,
) -> None:
    marker = f"{case}-{tool}-local-replay"
    created = await client.post(
        f"{base_url}/v1/documents",
        headers=headers,
        json={"title": marker},
    )
    created.raise_for_status()
    document_id = created.json()["documentId"]
    assert document_id
    if tool == "google-docs__create_document":
        assert created.json()["title"] == marker
        return

    updated = await client.post(
        f"{base_url}/v1/documents/{document_id}:batchUpdate",
        headers=headers,
        json={
            "requests": [
                {
                    "insertText": {
                        "endOfSegmentLocation": {},
                        "text": marker,
                    }
                }
            ]
        },
    )
    updated.raise_for_status()
    assert updated.json()["writeControl"]["requiredRevisionId"]

    document = await client.get(
        f"{base_url}/v1/documents/{document_id}",
        headers=headers,
    )
    document.raise_for_status()
    text = "".join(
        element.get("textRun", {}).get("content", "")
        for structural in document.json()["body"]["content"]
        for element in structural.get("paragraph", {}).get("elements", [])
    )
    assert marker in text


async def _probe_google_sheets_tool(
    client: httpx.AsyncClient,
    base_url: str,
    headers: dict[str, str],
    tool: str,
    case: str,
) -> None:
    marker = f"{case}-{tool}-local-replay"
    created = await client.post(
        f"{base_url}/v4/spreadsheets",
        headers=headers,
        json={"properties": {"title": marker}},
    )
    created.raise_for_status()
    spreadsheet = created.json()
    spreadsheet_id = spreadsheet["spreadsheetId"]
    sheet_id = spreadsheet["sheets"][0]["properties"]["sheetId"]
    if tool == "google-sheets__create_spreadsheet":
        assert spreadsheet["properties"]["title"] == marker
        return

    if tool == "google-sheets__get_spreadsheet":
        metadata = await client.get(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}",
            headers=headers,
        )
        metadata.raise_for_status()
        assert metadata.json()["properties"]["title"] == marker
        return

    if tool == "google-sheets__rename_sheet":
        renamed = await client.post(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}:batchUpdate",
            headers=headers,
            json={
                "requests": [
                    {
                        "updateSheetProperties": {
                            "properties": {
                                "sheetId": sheet_id,
                                "title": "Results",
                            },
                            "fields": "title",
                        }
                    }
                ]
            },
        )
        renamed.raise_for_status()
        metadata = await client.get(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}",
            headers=headers,
        )
        metadata.raise_for_status()
        assert metadata.json()["sheets"][0]["properties"]["title"] == "Results"
        return

    written = await client.put(
        f"{base_url}/v4/spreadsheets/{spreadsheet_id}/values/Sheet1!A1:B1",
        headers=headers,
        json={"values": [[marker, "seed"]]},
    )
    written.raise_for_status()
    assert written.json()["updatedCells"] == 2

    if tool == "google-sheets__append_values":
        appended = await client.post(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}/values/Sheet1:append",
            headers=headers,
            json={"values": [[marker, "appended"]]},
        )
        appended.raise_for_status()
        assert appended.json()["updates"]["updatedCells"] == 2

    values = await client.get(
        f"{base_url}/v4/spreadsheets/{spreadsheet_id}/values/Sheet1!A1:B2",
        headers=headers,
    )
    values.raise_for_status()
    rows = values.json()["values"]
    assert rows[0] == [marker, "seed"]
    if tool == "google-sheets__append_values":
        assert rows[1] == [marker, "appended"]


async def _probe_github_tool(
    client: httpx.AsyncClient,
    base_url: str,
    tool: str,
) -> None:
    if tool == "github__get_authenticated_user":
        user = await github_json(client, base_url, "GET", "/user")
        assert user["login"] == "reborn-dev"
    elif tool == "github__get_repo":
        repo = await github_json(client, base_url, "GET", "/repos/nearai/ironclaw")
        assert repo["full_name"] == "nearai/ironclaw"
    elif tool == "github__list_releases":
        releases = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/releases",
        )
        assert isinstance(releases, list)
    else:
        raise AssertionError(f"missing GitHub Emulate probe for {tool}")


async def _slack_entities(
    client: httpx.AsyncClient,
    base_url: str,
) -> tuple[str, str]:
    channels = await slack_post(
        client,
        base_url,
        "conversations.list",
        {"types": "public_channel"},
    )
    channel_id = next(
        channel["id"]
        for channel in channels["channels"]
        if channel["name"] == "reborn-alerts"
    )
    users = await slack_post(client, base_url, "users.list")
    reviewer_id = next(
        user["id"] for user in users["members"] if user["name"] == "qa-reviewer"
    )
    return channel_id, reviewer_id


async def _probe_slack_tool(
    client: httpx.AsyncClient,
    base_url: str,
    tool: str,
    case: str,
) -> None:
    channel_id, reviewer_id = await _slack_entities(client, base_url)
    marker = f"{case}-{tool}-local-replay"
    if tool == "slack__whoami":
        body = await slack_post(client, base_url, "auth.test")
        assert body["user"] == "reborn-user"
    elif tool == "slack__list_conversations":
        body = await slack_post(
            client,
            base_url,
            "conversations.list",
            {"types": "public_channel"},
        )
        assert any(channel["id"] == channel_id for channel in body["channels"])
    elif tool == "slack__get_conversation_info":
        body = await slack_post(
            client,
            base_url,
            "conversations.info",
            {"channel": channel_id},
        )
        assert body["channel"]["name"] == "reborn-alerts"
    elif tool == "slack__get_user_info":
        body = await slack_post(
            client,
            base_url,
            "users.info",
            {"user": reviewer_id},
        )
        assert body["user"]["name"] == "qa-reviewer"
    elif tool == "slack__send_message":
        body = await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel_id, "text": marker},
        )
        assert body["message"]["text"] == marker
    elif tool == "slack__get_conversation_history":
        await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel_id, "text": marker},
        )
        body = await slack_post(
            client,
            base_url,
            "conversations.history",
            {"channel": channel_id, "limit": 10},
        )
        assert any(message["text"] == marker for message in body["messages"])
    elif tool == "slack__get_thread_replies":
        root = await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel_id, "text": f"{marker}-root"},
        )
        root_ts = root["ts"]
        await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel_id, "thread_ts": root_ts, "text": marker},
        )
        body = await slack_post(
            client,
            base_url,
            "conversations.replies",
            {"channel": channel_id, "ts": root_ts},
        )
        assert any(message["text"] == marker for message in body["messages"])
    elif tool == "slack__search_messages":
        await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel_id, "text": marker},
        )
        response = await client.get(
            f"{base_url}/api/search.messages",
            headers=slack_headers(),
            params={"query": marker, "count": 20, "sort": "timestamp"},
        )
        response.raise_for_status()
        body = response.json()
        assert body["ok"] is True
        assert any(
            match["text"] == marker for match in body["messages"]["matches"]
        )
    else:
        raise AssertionError(f"missing Slack Emulate probe for {tool}")


async def _probe_supported_tools(
    trace: dict,
    case: str,
    google_url: str,
    github_url: str,
    slack_url: str,
) -> None:
    provider_tools = {
        call["name"]
        for call in _tool_calls(trace)
        if call["name"].startswith(PROVIDER_PREFIXES)
    }
    classified = EMULATE_SUPPORTED_TOOLS | EMULATE_UNSUPPORTED_TOOLS
    assert provider_tools <= classified, (
        f"{case} has provider operations without an Emulate classification: "
        f"{sorted(provider_tools - classified)}"
    )

    async with httpx.AsyncClient(timeout=15) as client:
        for tool in sorted(provider_tools & EMULATE_SUPPORTED_TOOLS):
            if tool.startswith(
                (
                    "gmail__",
                    "google-calendar__",
                    "google-docs__",
                    "google-drive__",
                    "google-sheets__",
                )
            ):
                await _probe_google_tool(client, google_url, tool, case)
            elif tool.startswith("github__"):
                await _probe_github_tool(client, github_url, tool)
            elif tool.startswith("slack__"):
                await _probe_slack_tool(client, slack_url, tool, case)
            else:
                raise AssertionError(f"missing Emulate provider routing for {tool}")


@pytest.mark.parametrize("case", MODEL_CASES, ids=MODEL_CASES)
async def test_every_harvested_trace_replays_with_emulate_coverage(
    case,
    mock_llm_server,
    emulate_google_server,
    emulate_github_server,
    emulate_slack_server,
):
    """Every harvested model case is executable and provider-classified."""
    trace = _load_case(case)
    await _replay_every_response(mock_llm_server, case, trace)
    await _probe_supported_tools(
        trace,
        case,
        emulate_google_server["url"],
        emulate_github_server["url"],
        emulate_slack_server["url"],
    )


def test_fixture_catalog_has_no_unowned_cases_or_provider_operations():
    """The manifest, files, and Emulate classifications remain closed sets."""
    fixture_cases = {path.stem for path in TRACE_DIR.glob("qa_*.json")}
    assert set(MODEL_CASES) == fixture_cases

    observed_provider_tools = {
        call["name"]
        for case in MODEL_CASES
        for call in _tool_calls(_load_case(case))
        if call["name"].startswith(PROVIDER_PREFIXES)
    }
    assert observed_provider_tools == (
        EMULATE_SUPPORTED_TOOLS | EMULATE_UNSUPPORTED_TOOLS
    )
