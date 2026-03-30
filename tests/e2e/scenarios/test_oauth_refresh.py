"""Hosted OAuth refresh HTTP regression test.

Runs a real ironclaw binary in hosted mode, expires a stored Gmail access
token in the libSQL database, triggers a real gmail tool call through the
chat API, and verifies that refresh uses the hosted proxy endpoint.
"""

import asyncio
import sqlite3
from datetime import datetime, timezone
from urllib.parse import parse_qs, urlparse

import httpx

from helpers import api_get, api_post


def _extract_state(auth_url: str) -> str:
    parsed = urlparse(auth_url)
    state = parse_qs(parsed.query).get("state", [None])[0]
    assert state, f"auth_url should include state: {auth_url}"
    return state


def _parse_timestamp(value: str | None) -> datetime | None:
    if value is None:
        return None
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def _expire_access_token(db_path: str, user_id: str, secret_name: str) -> None:
    with sqlite3.connect(db_path) as conn:
        cursor = conn.execute(
            """
            UPDATE secrets
            SET expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 hour')
            WHERE user_id = ?1 AND name = ?2
            """,
            (user_id, secret_name),
        )
        conn.commit()
    assert cursor.rowcount == 1, f"Expected one secret row for {user_id}/{secret_name}"


def _find_secret_row(
    db_path: str,
    secret_name: str,
) -> tuple[str, str | None, str | None]:
    with sqlite3.connect(db_path) as conn:
        row = conn.execute(
            """
            SELECT user_id, expires_at, updated_at
            FROM secrets
            WHERE name = ?1
            ORDER BY updated_at DESC
            LIMIT 1
            """,
            (secret_name,),
        ).fetchone()
    assert row is not None, f"Missing secret row for {secret_name}"
    return row[0], row[1], row[2]


async def _get_extension(base_url: str, name: str) -> dict | None:
    response = await api_get(base_url, "/api/extensions", timeout=15)
    response.raise_for_status()
    for extension in response.json().get("extensions", []):
        if extension["name"] == name:
            return extension
    return None


async def _reset_mock_oauth_state(mock_base_url: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(f"{mock_base_url}/__mock/oauth/reset", timeout=10)
    response.raise_for_status()


async def _get_mock_oauth_state(mock_base_url: str) -> dict:
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{mock_base_url}/__mock/oauth/state", timeout=10)
    response.raise_for_status()
    return response.json()


async def _approve_pending_request(base_url: str, thread_id: str, request_id: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/approval",
        json={"request_id": request_id, "action": "approve", "thread_id": thread_id},
        timeout=15,
    )
    assert response.status_code == 202, (
        f"Approval submission failed: {response.status_code} {response.text[:400]}"
    )


async def _wait_for_gmail_tool_call(base_url: str, thread_id: str, timeout: float = 30.0) -> dict:
    approved_request_ids = set()
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        response.raise_for_status()
        history = response.json()

        pending = history.get("pending_approval")
        if pending and pending["request_id"] not in approved_request_ids:
            await _approve_pending_request(base_url, thread_id, pending["request_id"])
            approved_request_ids.add(pending["request_id"])

        for turn in history.get("turns", []):
            for tool_call in turn.get("tool_calls", []):
                if tool_call.get("name") == "gmail":
                    return history

        await asyncio.sleep(0.5)

    raise AssertionError(f"Timed out waiting for gmail tool call in thread {thread_id}")


async def _wait_for_tool_call(
    base_url: str,
    thread_id: str,
    tool_name: str,
    timeout: float = 30.0,
) -> dict:
    approved_request_ids = set()
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        response.raise_for_status()
        history = response.json()

        pending = history.get("pending_approval")
        if pending and pending["request_id"] not in approved_request_ids:
            await _approve_pending_request(base_url, thread_id, pending["request_id"])
            approved_request_ids.add(pending["request_id"])

        for turn in history.get("turns", []):
            for tool_call in turn.get("tool_calls", []):
                if tool_call.get("name") == tool_name:
                    return history

        await asyncio.sleep(0.5)

    raise AssertionError(f"Timed out waiting for {tool_name} tool call in thread {thread_id}")


async def _wait_for_refresh_request(mock_base_url: str, timeout: float = 20.0) -> dict:
    for _ in range(int(timeout * 2)):
        state = await _get_mock_oauth_state(mock_base_url)
        if state.get("refresh_count") == 1:
            return state
        await asyncio.sleep(0.5)
    raise AssertionError("Timed out waiting for exactly one OAuth refresh request")


async def test_hosted_gmail_oauth_refresh_uses_proxy(hosted_oauth_refresh_server):
    server = hosted_oauth_refresh_server["base_url"]
    db_path = hosted_oauth_refresh_server["db_path"]
    mock_base_url = hosted_oauth_refresh_server["mock_llm_url"]

    install_response = await api_post(
        server,
        "/api/extensions/install",
        json={"name": "gmail"},
        timeout=180,
    )
    assert install_response.status_code == 200, install_response.text
    assert install_response.json().get("success") is True

    setup_response = await api_post(
        server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert setup_response.status_code == 200, setup_response.text
    setup_data = setup_response.json()
    assert setup_data.get("success") is True, setup_data
    auth_url = setup_data.get("auth_url")
    assert auth_url, setup_data
    auth_params = parse_qs(urlparse(auth_url).query)
    assert auth_params.get("client_id") == ["hosted-google-client-id"]

    async with httpx.AsyncClient() as client:
        callback_response = await client.get(
            f"{server}/oauth/callback",
            params={"code": "mock_auth_code", "state": _extract_state(auth_url)},
            timeout=30,
            follow_redirects=True,
        )

    assert callback_response.status_code == 200, callback_response.text[:400]
    callback_body = callback_response.text.lower()
    assert "connected" in callback_body or "success" in callback_body

    gmail = await _get_extension(server, "gmail")
    assert gmail is not None, "gmail should be installed"
    assert gmail["authenticated"] is True, gmail
    assert "gmail" in gmail.get("tools", []), gmail

    await _reset_mock_oauth_state(mock_base_url)

    stored_user_id, expires_before, updated_before = _find_secret_row(
        db_path, "google_oauth_token"
    )
    assert _parse_timestamp(expires_before) is not None
    assert _parse_timestamp(updated_before) is not None

    await asyncio.sleep(0.1)
    _expire_access_token(db_path, stored_user_id, "google_oauth_token")

    thread_response = await api_post(server, "/api/chat/thread/new", timeout=15)
    assert thread_response.status_code == 200, thread_response.text
    thread_id = thread_response.json()["id"]

    send_response = await api_post(
        server,
        "/api/chat/send",
        json={"content": "check gmail unread", "thread_id": thread_id},
        timeout=30,
    )
    assert send_response.status_code == 202, send_response.text

    history = await _wait_for_gmail_tool_call(server, thread_id)
    assert any(
        tool_call.get("name") == "gmail"
        for turn in history.get("turns", [])
        for tool_call in turn.get("tool_calls", [])
    ), history

    oauth_state = await _wait_for_refresh_request(mock_base_url)
    assert oauth_state["refresh_count"] == 1, oauth_state
    last_refresh = oauth_state["last_refresh"]
    assert last_refresh is not None, oauth_state
    assert last_refresh["authorization"] == "Bearer e2e-test-token"
    assert last_refresh["form"]["client_id"] == "hosted-google-client-id"
    assert "client_secret" not in last_refresh["form"], last_refresh

    refreshed_user_id, expires_after, updated_after = _find_secret_row(
        db_path, "google_oauth_token"
    )
    assert refreshed_user_id == stored_user_id
    expires_after_dt = _parse_timestamp(expires_after)
    updated_after_dt = _parse_timestamp(updated_after)
    updated_before_dt = _parse_timestamp(updated_before)
    assert expires_after_dt is not None
    assert updated_after_dt is not None
    assert updated_before_dt is not None
    assert expires_after_dt > datetime.now(timezone.utc)
    assert updated_after_dt > updated_before_dt


async def test_hosted_mcp_oauth_refresh_uses_proxy(hosted_oauth_refresh_server):
    server = hosted_oauth_refresh_server["base_url"]
    db_path = hosted_oauth_refresh_server["db_path"]
    mock_base_url = hosted_oauth_refresh_server["mock_llm_url"]
    mcp_url = f"{mock_base_url}/mcp"

    install_response = await api_post(
        server,
        "/api/extensions/install",
        json={"name": "mock-mcp", "url": mcp_url, "kind": "mcp_server"},
        timeout=30,
    )
    assert install_response.status_code == 200, install_response.text
    assert install_response.json().get("success") is True, install_response.text

    activate_response = await api_post(
        server,
        "/api/extensions/mock-mcp/activate",
        timeout=30,
    )
    assert activate_response.status_code == 200, activate_response.text
    auth_url = activate_response.json().get("auth_url")
    assert auth_url, activate_response.json()

    async with httpx.AsyncClient() as client:
        callback_response = await client.get(
            f"{server}/oauth/callback",
            params={"code": "mock_mcp_code", "state": _extract_state(auth_url)},
            timeout=30,
            follow_redirects=True,
        )

    assert callback_response.status_code == 200, callback_response.text[:400]
    callback_body = callback_response.text.lower()
    assert "connected" in callback_body or "success" in callback_body

    mock_mcp = await _get_extension(server, "mock-mcp")
    assert mock_mcp is not None, "mock-mcp should be installed"
    assert mock_mcp["authenticated"] is True, mock_mcp
    assert "mock-mcp_mock_search" in mock_mcp.get("tools", []), mock_mcp

    client_secret_user_id, _, _ = _find_secret_row(db_path, "mcp_mock-mcp_client_secret")
    stored_user_id, expires_before, updated_before = _find_secret_row(
        db_path, "mcp_mock-mcp_access_token"
    )
    assert client_secret_user_id == stored_user_id
    assert _parse_timestamp(expires_before) is not None
    assert _parse_timestamp(updated_before) is not None

    await _reset_mock_oauth_state(mock_base_url)
    await asyncio.sleep(0.1)
    _expire_access_token(db_path, stored_user_id, "mcp_mock-mcp_access_token")

    thread_response = await api_post(server, "/api/chat/thread/new", timeout=15)
    assert thread_response.status_code == 200, thread_response.text
    thread_id = thread_response.json()["id"]

    send_response = await api_post(
        server,
        "/api/chat/send",
        json={"content": "check mock mcp", "thread_id": thread_id},
        timeout=30,
    )
    assert send_response.status_code == 202, send_response.text

    history = await _wait_for_tool_call(server, thread_id, "mock-mcp_mock_search")
    assert any(
        tool_call.get("name") == "mock-mcp_mock_search"
        for turn in history.get("turns", [])
        for tool_call in turn.get("tool_calls", [])
    ), history

    oauth_state = await _wait_for_refresh_request(mock_base_url)
    assert oauth_state["refresh_count"] == 1, oauth_state
    last_refresh = oauth_state["last_refresh"]
    assert last_refresh is not None, oauth_state
    assert last_refresh["authorization"] == "Bearer e2e-test-token"
    assert last_refresh["form"]["provider"] == "mcp:mock-mcp"
    assert last_refresh["form"]["client_id"] == "mock-mcp-client-id"
    assert last_refresh["form"]["client_secret"] == "mock-mcp-client-secret"
    assert last_refresh["form"]["token_url"].endswith("/oauth/token")

    refreshed_user_id, expires_after, updated_after = _find_secret_row(
        db_path, "mcp_mock-mcp_access_token"
    )
    refresh_user_id, _, refresh_updated_after = _find_secret_row(
        db_path, "mcp_mock-mcp_access_token_refresh_token"
    )
    assert refreshed_user_id == stored_user_id
    assert refresh_user_id == stored_user_id
    expires_after_dt = _parse_timestamp(expires_after)
    updated_after_dt = _parse_timestamp(updated_after)
    updated_before_dt = _parse_timestamp(updated_before)
    refresh_updated_after_dt = _parse_timestamp(refresh_updated_after)
    assert expires_after_dt is not None
    assert updated_after_dt is not None
    assert updated_before_dt is not None
    assert refresh_updated_after_dt is not None
    assert expires_after_dt > datetime.now(timezone.utc)
    assert updated_after_dt > updated_before_dt
    assert refresh_updated_after_dt >= updated_after_dt
