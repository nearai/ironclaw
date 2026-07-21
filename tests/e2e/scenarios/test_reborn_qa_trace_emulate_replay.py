"""Replay harvested Reborn QA model traces against real Emulate providers.

The trace controls model decisions only. Capability execution still crosses the
served Reborn runtime, first-party extension, credential boundary, HTTP rewrite,
and Emulate.dev provider. Assertions intentionally target those boundaries, not
the recorded model's final wording.
"""

import asyncio
import json
import uuid
from datetime import UTC, datetime, timedelta
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import httpx
import pytest

from reborn_webui_harness import (
    YOLO_PROFILE,
    close_reborn_server,
    create_thread,
    enable_reborn_global_auto_approve,
    reborn_bearer_headers,
    send_message,
    start_reborn_webui_v2_server,
    wait_for_assistant_message,
    wait_for_capability_preview,
)

pytest_plugins = ["reborn_webui_harness"]

ROOT = Path(__file__).resolve().parents[3]
DRIVE_CONNECT_TRACE = (
    ROOT
    / "tests/fixtures/llm_traces/reborn_qa/live_canary/qa_2c_drive_connect.json"
)
GOOGLE_DRIVE_READONLY_SCOPE = "https://www.googleapis.com/auth/drive.readonly"
GOOGLE_DRIVE_SCOPE = "https://www.googleapis.com/auth/drive"


@pytest.fixture(scope="module")
async def reborn_qa_emulate_google_server(
    ironclaw_reborn_binary,
    mock_llm_server,
    emulate_google_server,
    tmp_path_factory,
):
    """Start standalone Reborn with Google API traffic routed to Emulate."""
    home_dir = tmp_path_factory.mktemp("reborn-qa-emulate-google-home")
    rewrite_map = {
        "oauth2.googleapis.com": mock_llm_server,
        "www.googleapis.com": emulate_google_server["url"],
    }
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-qa-emulate-google",
        extra_env={
            "IRONCLAW_TEST_HTTP_REWRITE_MAP": json.dumps(rewrite_map),
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "reborn-qa-emulate-client",
            "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI": (
                "http://127.0.0.1/api/reborn/product-auth/oauth/google/callback"
            ),
        },
    )
    await enable_reborn_global_auto_approve(base_url)
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


async def _seed_google_account(base_url: str) -> None:
    expires_at = (datetime.now(UTC) + timedelta(minutes=5)).isoformat()
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        started = await client.post(
            f"{base_url}/api/webchat/v2/extensions/google-drive/setup/oauth/start",
            json={
                "provider": "google",
                "account_label": "Emulate Google account",
                "scopes": [GOOGLE_DRIVE_READONLY_SCOPE, GOOGLE_DRIVE_SCOPE],
                "expires_at": expires_at,
                "invocation_id": str(uuid.uuid4()),
            },
            timeout=15,
        )
        started.raise_for_status()
        started_body = started.json()
        authorization_url = started_body["authorization_url"]
        state = parse_qs(urlparse(authorization_url).query)["state"][0]

        callback = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/google/callback",
            params={
                "state": state,
                "code": "mock_auth_code",
                "scope": f"{GOOGLE_DRIVE_READONLY_SCOPE} {GOOGLE_DRIVE_SCOPE}",
            },
            headers={"Accept": "application/json"},
            timeout=30,
        )
        assert callback.is_success, callback.text
        reconciled = await client.post(
            f"{base_url}/api/reborn/product-auth/oauth/flow/"
            f"{started_body['flow_id']}/reconcile",
            params={
                "invocation_id": started_body["callback_scope"]["invocation_id"]
            },
            timeout=30,
        )
        reconciled.raise_for_status()
        invocation_id = started_body["callback_scope"]["invocation_id"]
        listed = await client.post(
            f"{base_url}/api/reborn/product-auth/accounts/list",
            json={
                "provider": "google",
                "requester_extension": "google-drive",
                "invocation_id": invocation_id,
            },
            timeout=30,
        )
        listed.raise_for_status()
        accounts = listed.json()["accounts"]
        assert len(accounts) == 1, listed.text
        selected = await client.post(
            f"{base_url}/api/reborn/product-auth/accounts/select",
            json={
                "provider": "google",
                "requester_extension": "google-drive",
                "account_id": accounts[0]["id"],
                "invocation_id": invocation_id,
            },
            timeout=30,
        )
        selected.raise_for_status()


async def _install_drive(base_url: str) -> None:
    package_ref = {"kind": "extension", "id": "google-drive"}
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        installed = await client.post(
            f"{base_url}/api/webchat/v2/extensions/install",
            json={"package_ref": package_ref},
            timeout=30,
        )
        installed.raise_for_status()


async def _activate_drive(base_url: str) -> None:
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        activated = await client.post(
            f"{base_url}/api/webchat/v2/extensions/google-drive/activate",
            timeout=30,
        )
        activated.raise_for_status()
        assert activated.json().get("activated") is True, activated.text


async def _load_trace(
    mock_llm_server: str,
    trace_path: Path,
    *,
    start_at_tool: str | None = None,
) -> dict:
    trace = json.loads(trace_path.read_text(encoding="utf-8"))
    if start_at_tool is not None:
        start_index = next(
            index
            for index, step in enumerate(trace["steps"])
            if any(
                call["name"] == start_at_tool
                for call in step["response"].get("tool_calls", [])
            )
        )
        trace["steps"] = [trace["steps"][0], *trace["steps"][start_index:]]
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


async def test_qa_2c_drive_connect_replays_through_emulate(
    reborn_qa_emulate_google_server,
    mock_llm_server,
):
    """The harvested choices execute locally and read Emulate's Drive state."""
    server = reborn_qa_emulate_google_server
    await _install_drive(server)
    await _seed_google_account(server)
    await _activate_drive(server)
    trace = await _load_trace(
        mock_llm_server,
        DRIVE_CONNECT_TRACE,
        start_at_tool="google-drive__list_files",
    )
    user_input = trace["steps"][0]["response"]["content"]

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, server)
        await send_message(client, server, thread_id, user_input)

        replay = await _wait_for_trace_replay(mock_llm_server)
        await wait_for_assistant_message(client, server, thread_id, timeout=120)
        preview = await wait_for_capability_preview(
            client,
            server,
            thread_id,
            "google-drive.list_files",
            output_fragment="Reborn QA Brief",
            timeout=120,
        )

        assert "Secondary Private Brief" not in json.dumps(preview)

    assert replay == {
        "source": DRIVE_CONNECT_TRACE.name,
        "next_response": 2,
        "response_count": 2,
        "complete": True,
        "error": None,
    }
