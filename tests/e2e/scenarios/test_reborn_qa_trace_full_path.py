"""Run harvested Reborn QA traces through the full Emulate-backed path.

The trace controls model decisions only. Capability execution still crosses the
served Reborn runtime, first-party extension, credential boundary, HTTP rewrite,
and Emulate.dev provider. Assertions intentionally target those boundaries, not
the recorded model's final wording.
"""

import asyncio
import json
import uuid
from collections import Counter
from datetime import UTC, datetime, timedelta
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import httpx
import pytest

from emulate_provider import google_headers, slack_post
from helpers import EMULATE_GITHUB_BEARER, EMULATE_SLACK_BEARER
from reborn_webui_harness import (
    YOLO_PROFILE,
    capability_preview_payload,
    close_reborn_server,
    create_thread,
    enable_reborn_global_auto_approve,
    reborn_bearer_headers,
    send_message,
    start_reborn_webui_v2_server,
    wait_for_assistant_message,
)

pytest_plugins = ["reborn_webui_harness"]

ROOT = Path(__file__).resolve().parents[3]
TRACE_DIR = ROOT / "tests/fixtures/llm_traces/reborn_qa/live_canary"
MANIFEST_PATH = TRACE_DIR / "case-manifest.json"
GOOGLE_EXTENSIONS = (
    "gmail",
    "google-calendar",
    "google-drive",
    "google-docs",
    "google-sheets",
)
GOOGLE_EXTENSION_SCOPES = {
    "gmail": (
        "https://www.googleapis.com/auth/gmail.readonly",
        "https://www.googleapis.com/auth/gmail.send",
        "https://www.googleapis.com/auth/gmail.modify",
    ),
    "google-calendar": (
        "https://www.googleapis.com/auth/calendar.readonly",
        "https://www.googleapis.com/auth/calendar.events",
    ),
    "google-drive": (
        "https://www.googleapis.com/auth/drive.readonly",
        "https://www.googleapis.com/auth/drive",
    ),
    "google-docs": (
        "https://www.googleapis.com/auth/documents",
        "https://www.googleapis.com/auth/documents.readonly",
    ),
    "google-sheets": (
        "https://www.googleapis.com/auth/spreadsheets",
        "https://www.googleapis.com/auth/spreadsheets.readonly",
    ),
}
GOOGLE_CUMULATIVE_SCOPES = tuple(
    dict.fromkeys(
        scope
        for extension_scopes in GOOGLE_EXTENSION_SCOPES.values()
        for scope in extension_scopes
    )
)
GOOGLE_TOOL_PREFIXES = (
    "gmail__",
    "google-calendar__",
    "google-docs__",
    "google-drive__",
    "google-sheets__",
)
PROVIDER_TOOL_PREFIXES = (
    *GOOGLE_TOOL_PREFIXES,
    "github__",
    "slack__",
)
ALL_EXTENSIONS = (*GOOGLE_EXTENSIONS, "github", "slack")
TRACE_BOOTSTRAP_TOOLS = {"builtin__extension_search"}


def _provider_journey_cases() -> tuple[str, ...]:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    no_model = set(manifest["no_model_cases"])
    cases = []
    for case in manifest["selected_cases"]:
        if case in no_model:
            continue
        trace = json.loads((TRACE_DIR / f"{case}.json").read_text(encoding="utf-8"))
        if any(
            call["name"].startswith(PROVIDER_TOOL_PREFIXES)
            for step in trace["steps"]
            for call in step["response"].get("tool_calls", [])
        ):
            cases.append(case)
    return tuple(cases)


PROVIDER_JOURNEY_CASES = _provider_journey_cases()


@pytest.fixture(scope="module")
async def reborn_qa_emulate_provider_server(
    ironclaw_reborn_binary,
    mock_llm_server,
    emulate_google_server,
    emulate_github_server,
    emulate_slack_server,
    tmp_path_factory,
):
    """Start standalone Reborn with every supported provider routed to Emulate."""
    home_dir = tmp_path_factory.mktemp("reborn-qa-emulate-provider-home")
    mock_llm_address = urlparse(mock_llm_server)
    emulate_google_address = urlparse(emulate_google_server["url"])
    emulate_github_address = urlparse(emulate_github_server["url"])
    emulate_slack_address = urlparse(emulate_slack_server["url"])
    rewrite_map = ",".join(
        (
            f"oauth2.googleapis.com={mock_llm_address.hostname}:{mock_llm_address.port}",
            f"www.googleapis.com={emulate_google_address.hostname}:"
            f"{emulate_google_address.port}",
            f"gmail.googleapis.com={emulate_google_address.hostname}:"
            f"{emulate_google_address.port}",
            f"docs.googleapis.com={emulate_google_address.hostname}:"
            f"{emulate_google_address.port}",
            f"sheets.googleapis.com={emulate_google_address.hostname}:"
            f"{emulate_google_address.port}",
            f"api.github.com={emulate_github_address.hostname}:"
            f"{emulate_github_address.port}",
            f"slack.com={emulate_slack_address.hostname}:"
            f"{emulate_slack_address.port}",
        )
    )
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-qa-emulate-provider",
        extra_env={
            "IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP": rewrite_map,
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "reborn-qa-emulate-client",
            "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI": (
                "http://127.0.0.1/api/reborn/product-auth/oauth/google/callback"
            ),
            "IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI": (
                "http://127.0.0.1/api/reborn/product-auth/oauth/slack/callback"
            ),
        },
    )
    await enable_reborn_global_auto_approve(base_url)
    slack_state = await _seed_slack_workspace(emulate_slack_server["url"])
    await _configure_slack(base_url, slack_state)
    await _install_extensions(base_url, ALL_EXTENSIONS)
    for extension_id, scopes in GOOGLE_EXTENSION_SCOPES.items():
        await _seed_google_account(base_url, extension_id, scopes)
        await _activate_extensions(base_url, (extension_id,))
    await _seed_github_account(base_url)
    await _activate_extensions(base_url, ("github",))
    await _seed_slack_account(base_url, emulate_slack_server["url"], slack_state)
    await _activate_extensions(base_url, ("slack",))
    try:
        yield {
            "base_url": base_url,
            "emulate_google_url": emulate_google_server["url"],
            "emulate_github_url": emulate_github_server["url"],
            "emulate_slack_url": emulate_slack_server["url"],
            "slack_state": slack_state,
        }
    finally:
        await close_reborn_server(proc)


async def _seed_google_account(
    base_url: str,
    extension_id: str,
    scopes: tuple[str, ...],
) -> None:
    expires_at = (datetime.now(UTC) + timedelta(minutes=5)).isoformat()
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        started = await client.post(
            f"{base_url}/api/webchat/v2/extensions/{extension_id}/setup/oauth/start",
            json={
                "provider": "google",
                "account_label": f"Emulate Google account for {extension_id}",
                "scopes": list(scopes),
                "expires_at": expires_at,
                "invocation_id": str(uuid.uuid4()),
            },
            timeout=15,
        )
        assert started.is_success, started.text
        started_body = started.json()
        authorization_url = started_body["authorization_url"]
        state = parse_qs(urlparse(authorization_url).query)["state"][0]

        callback = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/google/callback",
            params={
                "state": state,
                "code": f"mock_auth_code_{extension_id.replace('-', '_')}",
                "scope": " ".join(GOOGLE_CUMULATIVE_SCOPES),
            },
            headers={"Accept": "application/json"},
            timeout=30,
        )
        assert callback.is_success, callback.text
        flow_status = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/flow/"
            f"{started_body['flow_id']}/status",
            params={
                "invocation_id": started_body["callback_scope"]["invocation_id"]
            },
            timeout=30,
        )
        flow_status.raise_for_status()
        assert flow_status.json()["status"] == "completed", flow_status.text
        invocation_id = started_body["callback_scope"]["invocation_id"]
        listed = await client.post(
            f"{base_url}/api/reborn/product-auth/accounts/list",
            json={
                "provider": "google",
                "requester_extension": extension_id,
                "invocation_id": invocation_id,
            },
            timeout=30,
        )
        listed.raise_for_status()
        accounts = listed.json()["accounts"]
        # Re-authenticating the same Emulate identity upgrades the existing
        # user-reusable account in its original invocation scope. Only the
        # first flow therefore exposes a newly selectable account here.
        if not accounts:
            return
        assert len(accounts) == 1, listed.text
        selected = await client.post(
            f"{base_url}/api/reborn/product-auth/accounts/select",
            json={
                "provider": "google",
                "requester_extension": extension_id,
                "account_id": accounts[0]["id"],
                "invocation_id": invocation_id,
            },
            timeout=30,
        )
        selected.raise_for_status()


async def _seed_slack_workspace(emulate_url: str) -> dict[str, str]:
    """Create deterministic Slack data and return its provider-issued IDs."""
    async with httpx.AsyncClient(timeout=15) as client:
        identity = await slack_post(client, emulate_url, "auth.test")
        users = await slack_post(client, emulate_url, "users.list")
        by_name = {member["name"]: member for member in users["members"]}
        channels = await slack_post(
            client,
            emulate_url,
            "conversations.list",
            {"types": "public_channel"},
        )
        channel = next(
            item for item in channels["channels"] if item["name"] == "reborn-alerts"
        )
        await slack_post(
            client, emulate_url, "conversations.join", {"channel": channel["id"]}
        )
        await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {"channel": channel["id"], "text": "QA10 self-authored earlier message"},
        )
        await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {
                "channel": channel["id"],
                "text": "ENTITYMSG_1784643032040 QA10 searchable marker",
            },
        )
        root = await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {"channel": channel["id"], "text": "QA10 thread root"},
        )
        await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {
                "channel": channel["id"],
                "thread_ts": root["ts"],
                "text": "QA10 visible thread reply",
            },
        )
    return {
        "team_id": identity["team_id"],
        "user_id": identity["user_id"],
        "reviewer_id": by_name["qa-reviewer"]["id"],
        "channel_id": channel["id"],
        "channel_name": channel["name"],
        "thread_ts": root["ts"],
    }


async def _configure_slack(base_url: str, slack_state: dict[str, str]) -> None:
    client_id = "reborn-qa-emulate-slack-client"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        configured = await client.get(
            f"{base_url}/api/webchat/v2/operator/extension-configuration",
            timeout=30,
        )
        configured.raise_for_status()
        group = next(
            item
            for item in configured.json()["groups"]
            if item["group_id"] == "extension.slack"
        )
        response = await client.put(
            f"{base_url}/api/webchat/v2/operator/extension-configuration/extension.slack",
            json={
                "values": [
                    {"handle": "slack_bot_token", "value": EMULATE_SLACK_BEARER},
                    {"handle": "slack_signing_secret", "value": "emulate-signing-secret"},
                    {"handle": "slack_team_id", "value": slack_state["team_id"]},
                    {"handle": "slack_api_app_id", "value": client_id},
                    {"handle": "slack_installation_id", "value": slack_state["team_id"]},
                    {"handle": "slack_bot_user_id", "value": slack_state["user_id"]},
                    {"handle": "slack_oauth_client_id", "value": client_id},
                    {
                        "handle": "slack_oauth_client_secret",
                        "value": "emulate-slack-client-secret",
                    },
                ],
                "expected_revision": group["revision"],
                "idempotency_key": f"reborn-qa-emulate-{uuid.uuid4()}",
            },
            timeout=30,
        )
        response.raise_for_status()
        assert response.json()["complete"] is True, response.text


async def _seed_github_account(base_url: str) -> None:
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        response = await client.post(
            f"{base_url}/api/webchat/v2/extensions/github/setup",
            json={
                "action": "submit",
                "payload": {
                    "secrets": {"github_runtime_token": EMULATE_GITHUB_BEARER},
                    "fields": {},
                },
            },
            timeout=30,
        )
        response.raise_for_status()
        secret = next(
            item
            for item in response.json()["secrets"]
            if item["name"] == "github_runtime_token"
        )
        assert secret["provided"] is True, response.text


async def _seed_slack_account(
    base_url: str,
    emulate_url: str,
    slack_state: dict[str, str],
) -> None:
    expires_at = (datetime.now(UTC) + timedelta(minutes=5)).isoformat()
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        started = await client.post(
            f"{base_url}/api/webchat/v2/extensions/slack/setup/oauth/start",
            json={
                "provider": "slack",
                "account_label": "Emulate Slack account",
                "scopes": [],
                "expires_at": expires_at,
                "invocation_id": str(uuid.uuid4()),
            },
            timeout=30,
        )
        started.raise_for_status()
        body = started.json()
        query = parse_qs(urlparse(body["authorization_url"]).query)
        consent = await client.post(
            f"{emulate_url}/oauth/v2/authorize/callback",
            data={
                "user_id": slack_state["user_id"],
                "redirect_uri": query["redirect_uri"][0],
                "scope": query.get("scope", [""])[0],
                "user_scope": query.get("user_scope", [""])[0],
                "state": query["state"][0],
                "client_id": query["client_id"][0],
            },
            follow_redirects=False,
            timeout=30,
        )
        assert consent.status_code == 302, consent.text
        callback_query = parse_qs(urlparse(consent.headers["location"]).query)
        callback = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/slack/callback",
            params={key: values[0] for key, values in callback_query.items()},
            headers={"Accept": "application/json"},
            timeout=30,
        )
        assert callback.is_success, callback.text
        flow_status = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/flow/{body['flow_id']}/status",
            params={"invocation_id": body["callback_scope"]["invocation_id"]},
            timeout=30,
        )
        flow_status.raise_for_status()
        assert flow_status.json()["status"] == "completed", flow_status.text


async def _install_extensions(base_url: str, extension_ids: tuple[str, ...]) -> None:
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        for extension_id in extension_ids:
            installed = await client.post(
                f"{base_url}/api/webchat/v2/extensions/install",
                json={
                    "package_ref": {"kind": "extension", "id": extension_id}
                },
                timeout=30,
            )
            installed.raise_for_status()


async def _activate_extensions(base_url: str, extension_ids: tuple[str, ...]) -> None:
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        for extension_id in extension_ids:
            activated = await client.post(
                f"{base_url}/api/webchat/v2/extensions/{extension_id}/activate",
                timeout=30,
            )
            activated.raise_for_status()
            body = activated.json()
            assert body.get("activated") is True, body


def _provider_leg(trace: dict, prefixes: tuple[str, ...]) -> dict:
    """Keep the recorded provider decisions and final response in order."""
    provider_steps = []
    final_text = None
    for step in trace["steps"][1:]:
        response = step["response"]
        if response["type"] == "tool_calls":
            calls = [
                call
                for call in response["tool_calls"]
                if call["name"].startswith(prefixes)
                or call["name"] in TRACE_BOOTSTRAP_TOOLS
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


def _result_binding(tool: str, *fields: str) -> dict:
    return {"$trace_result": {"tool": tool, "fields": list(fields)}}


def _inject_deferred_tool_disclosure(trace: dict) -> None:
    """Translate the harvested extension flow to today's deferred-tool flow."""
    provider_names = list(
        dict.fromkeys(
            call["name"]
            for step in trace["steps"]
            for call in step["response"].get("tool_calls", [])
            if call["name"].startswith(PROVIDER_TOOL_PREFIXES)
        )
    )
    first_provider_step = next(
        index
        for index, step in enumerate(trace["steps"])
        if any(
            call["name"].startswith(PROVIDER_TOOL_PREFIXES)
            for call in step["response"].get("tool_calls", [])
        )
    )
    trace["steps"].insert(
        first_provider_step,
        {
            "response": {
                "type": "tool_calls",
                "tool_calls": [
                    {
                        "name": "capability_info",
                        "arguments": {"name": name.replace("__", ".")},
                    }
                    for name in provider_names
                ],
            }
        },
    )


def _coalesce_independent_provider_reads(trace: dict, batch_size: int = 25) -> None:
    """Fit QA 9C's independent Slack reads within today's loop-turn limit."""
    provider_indexes = [
        index
        for index, step in enumerate(trace["steps"])
        if any(
            call["name"].startswith(PROVIDER_TOOL_PREFIXES)
            for call in step["response"].get("tool_calls", [])
        )
    ]
    calls = [
        call
        for index in provider_indexes
        for call in trace["steps"][index]["response"]["tool_calls"]
    ]
    insertion_index = provider_indexes[0]
    trace["steps"] = [
        step
        for index, step in enumerate(trace["steps"])
        if index not in provider_indexes
    ]
    batches = [
        {
            "response": {
                "type": "tool_calls",
                "tool_calls": calls[start : start + batch_size],
            }
        }
        for start in range(0, len(calls), batch_size)
    ]
    trace["steps"][insertion_index:insertion_index] = batches


def _normalize_google_arguments(trace: dict, case: str) -> None:
    created_document = False
    created_spreadsheet = False
    seeded_spreadsheet = (
        "sheet_reborn_bug_tracker" if case.startswith("qa_7") else "sheet_reborn_abc"
    )

    for step in trace["steps"]:
        for call in step["response"].get("tool_calls", []):
            name = call["name"]
            arguments = call["arguments"]
            _replace_value(arguments, "EMAIL_REDACTED", "e2e.google@example.com")

            if name == "google-docs__create_document":
                created_document = True
            elif name.startswith("google-docs__") and "document_id" in arguments:
                arguments["document_id"] = (
                    _result_binding(
                        "google-docs__create_document",
                        "documentId",
                        "document_id",
                        "id",
                    )
                    if created_document
                    else "doc_reborn_strategy"
                )

            if name == "google-sheets__create_spreadsheet":
                created_spreadsheet = True
            elif name.startswith("google-sheets__"):
                if "spreadsheet_id" in arguments:
                    arguments["spreadsheet_id"] = (
                        _result_binding(
                            "google-sheets__create_spreadsheet",
                            "spreadsheetId",
                            "spreadsheet_id",
                            "id",
                        )
                        if created_spreadsheet
                        else seeded_spreadsheet
                    )
                if created_spreadsheet and "sheet_id" in arguments:
                    arguments["sheet_id"] = _result_binding(
                        "google-sheets__create_spreadsheet", "sheetId", "sheet_id"
                    )

            if name == "gmail__get_message":
                arguments["message_id"] = "msg_emulate_near_inbound"
            elif name == "google-drive__download_file":
                arguments["file_id"] = "drv_pepsico_account_brief"


def _normalize_slack_arguments(
    trace: dict, slack_state: dict[str, str], case: str
) -> None:
    for step in trace["steps"]:
        for call in step["response"].get("tool_calls", []):
            if not call["name"].startswith("slack__"):
                continue
            arguments = call["arguments"]
            if "channel" in arguments:
                arguments["channel"] = (
                    "C_REBORN_QA_10E_MISSING"
                    if case == "qa_10e_slack_error_honesty"
                    else slack_state["channel_id"]
                )
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


async def _load_trace(
    mock_llm_server: str,
    trace_path: Path,
    *,
    provider_prefixes: tuple[str, ...] | None = None,
    slack_state: dict[str, str] | None = None,
) -> dict:
    trace = json.loads(trace_path.read_text(encoding="utf-8"))
    if provider_prefixes is not None:
        trace = _provider_leg(trace, provider_prefixes)
    if trace_path.stem == "qa_9c_slack_digest_names_not_ids":
        _coalesce_independent_provider_reads(trace)
        _inject_deferred_tool_disclosure(trace)
    elif trace_path.stem == "qa_10e_slack_error_honesty":
        trace["steps"][-1]["request_hint"] = {
            "expected_failed_tool_result_contains": "channel_not_found"
        }
    if provider_prefixes is not None:
        _normalize_google_arguments(trace, trace_path.stem)
    if slack_state is not None:
        _normalize_slack_arguments(trace, slack_state, trace_path.stem)
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_trace",
            json={"source": trace_path.name, "trace": trace},
            timeout=15,
        )
        response.raise_for_status()
    return trace


async def _wait_for_trace_replay(mock_llm_server: str, timeout: float = 30) -> dict:
    state = {}
    async with httpx.AsyncClient() as client:
        for _ in range(int(timeout * 2)):
            response = await client.get(
                f"{mock_llm_server}/__mock/llm_trace",
                timeout=15,
            )
            response.raise_for_status()
            state = response.json()
            assert state["error"] is None, state["error"]
            if state["complete"]:
                return state
            await asyncio.sleep(0.5)
    raise AssertionError(
        f"recorded trace did not complete within {timeout} seconds: {state}"
    )


async def _fetch_all_timeline_pages_with_retry(
    client: httpx.AsyncClient, server: str, thread_id: str
) -> dict:
    timeline = None
    messages = []
    cursor = None
    seen_cursors = set()

    while True:
        params = {"limit": 200}
        if cursor is not None:
            params["cursor"] = cursor

        for _ in range(20):
            response = await client.get(
                f"{server}/api/webchat/v2/threads/{thread_id}/timeline",
                params=params,
                timeout=15,
            )
            if response.status_code != 429:
                response.raise_for_status()
                page = response.json()
                break
            await asyncio.sleep(0.5)
        else:
            raise AssertionError(
                "timeline remained rate-limited after replay completed"
            )

        if timeline is None:
            timeline = page
        messages = [*page.get("messages", []), *messages]
        cursor = page.get("next_cursor")
        if cursor is None:
            timeline["messages"] = messages
            timeline["next_cursor"] = None
            return timeline
        assert isinstance(cursor, str) and cursor, page
        assert cursor not in seen_cursors, f"timeline cursor repeated: {cursor}"
        seen_cursors.add(cursor)


def _recorded_provider_calls(trace: dict) -> list[dict]:
    return [
        call
        for step in trace["steps"]
        for call in step["response"].get("tool_calls", [])
        if call["name"].startswith(PROVIDER_TOOL_PREFIXES)
    ]


async def _assert_google_provider_outcome(
    emulate_url: str, case: str, trace: dict
) -> None:
    calls = _recorded_provider_calls(trace)
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        if case in {
            "qa_2f_calendar_prep_email_delivery",
            "qa_4e_github_release_email_delivery",
        }:
            send = next(call for call in calls if call["name"] == "gmail__send_message")
            subject = send["arguments"]["message"]["subject"]
            listed = await client.get(
                f"{emulate_url}/gmail/v1/users/me/messages",
                params={"q": f"subject:{subject}"},
            )
            listed.raise_for_status()
            assert listed.json().get("messages"), f"sent message missing for {case}"

        create_call = next(
            (
                call
                for call in calls
                if call["name"]
                in {
                    "google-docs__create_document",
                    "google-sheets__create_spreadsheet",
                }
            ),
            None,
        )
        if create_call is None:
            return

        title = create_call["arguments"]["title"]
        files = await client.get(
            f"{emulate_url}/drive/v3/files",
            params={"q": f"name = '{title}' and trashed = false"},
        )
        files.raise_for_status()
        matching = [item for item in files.json()["files"] if item["name"] == title]
        assert matching, f"created Google resource missing for {case}: {files.text}"
        resource_id = matching[-1]["id"]

        if create_call["name"] == "google-docs__create_document":
            document = await client.get(f"{emulate_url}/v1/documents/{resource_id}")
            document.raise_for_status()
            assert "QA5D-NONCE" in document.text, document.text
            return

        spreadsheet = await client.get(
            f"{emulate_url}/v4/spreadsheets/{resource_id}"
        )
        spreadsheet.raise_for_status()
        if case == "qa_6e_gmail_to_sheet_delivery":
            values = await client.get(
                f"{emulate_url}/v4/spreadsheets/{resource_id}/values/Sheet1!A1:C10"
            )
            values.raise_for_status()
            assert "REBORN_QA_6E_GMAIL_TO_SHEET_DELIVERY" in values.text, values.text
        elif case == "qa_7e_slack_bug_sheet_delivery":
            values = await client.get(
                f"{emulate_url}/v4/spreadsheets/{resource_id}/values/Bugs!A1:E10"
            )
            values.raise_for_status()
            assert "REBORN_QA_7E_BUG_ROW" in values.text, values.text


async def _assert_slack_provider_outcome(
    emulate_url: str,
    slack_state: dict[str, str],
    trace: dict,
) -> None:
    send = next(
        (
            call
            for call in _recorded_provider_calls(trace)
            if call["name"] == "slack__send_message"
        ),
        None,
    )
    if send is None:
        return
    async with httpx.AsyncClient(timeout=15) as client:
        history = await slack_post(
            client,
            emulate_url,
            "conversations.history",
            {"channel": slack_state["channel_id"], "limit": 100},
        )
    assert any(
        message.get("text") == send["arguments"]["text"]
        for message in history["messages"]
    ), history


@pytest.mark.parametrize(
    "case", PROVIDER_JOURNEY_CASES, ids=PROVIDER_JOURNEY_CASES
)
async def test_qa_journey_provider_leg_replays_through_emulate(
    reborn_qa_emulate_provider_server,
    mock_llm_server,
    case,
):
    """Every harvested provider journey executes through standalone Reborn."""
    server = reborn_qa_emulate_provider_server["base_url"]
    trace_path = TRACE_DIR / f"{case}.json"
    trace = await _load_trace(
        mock_llm_server,
        trace_path,
        provider_prefixes=PROVIDER_TOOL_PREFIXES,
        slack_state=reborn_qa_emulate_provider_server["slack_state"],
    )
    user_input = trace["steps"][0]["response"]["content"]
    expected_calls = _recorded_provider_calls(trace)

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, server)
        await send_message(client, server, thread_id, user_input)

        replay_timeout = 180 if case == "qa_9c_slack_digest_names_not_ids" else 120
        replay = await _wait_for_trace_replay(
            mock_llm_server, timeout=replay_timeout
        )
        assistant = await wait_for_assistant_message(
            client, server, thread_id, timeout=replay_timeout
        )
        timeline = await _fetch_all_timeline_pages_with_retry(
            client, server, thread_id
        )
        previews = [
            preview
            for message in timeline.get("messages", [])
            if (preview := capability_preview_payload(message)) is not None
        ]
        expected_counts = Counter(
            call["name"].replace("__", ".") for call in expected_calls
        )
        actual_counts = Counter(
            preview["capability_id"]
            for preview in previews
            if preview["capability_id"] in expected_counts
        )
        assert actual_counts == expected_counts, (actual_counts, expected_counts)
        for preview in previews:
            if preview["capability_id"] not in expected_counts:
                continue
            output = json.dumps(preview).lower()
            if case == "qa_10e_slack_error_honesty":
                assert preview["status"] == "failed", json.dumps(preview)
                assert "channel_not_found" in output, preview
                continue
            assert preview["status"] == "completed", json.dumps(preview)
            assert "auth_required" not in output, preview
            assert "not found" not in output, preview

        if case == "qa_2c_drive_connect":
            assert "Secondary Private Brief" not in json.dumps(timeline)
        elif case == "qa_10e_slack_error_honesty":
            assert "channel_not_found" in assistant["content"]

    await _assert_google_provider_outcome(
        reborn_qa_emulate_provider_server["emulate_google_url"], case, trace
    )
    await _assert_slack_provider_outcome(
        reborn_qa_emulate_provider_server["emulate_slack_url"],
        reborn_qa_emulate_provider_server["slack_state"],
        trace,
    )

    assert replay == {
        "source": trace_path.name,
        "next_response": len(trace["steps"]) - 1,
        "response_count": len(trace["steps"]) - 1,
        "complete": True,
        "error": None,
    }
