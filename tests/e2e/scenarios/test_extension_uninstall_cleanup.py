"""Extension uninstall secret cleanup E2E tests.

Exercises real install/setup/auth/remove flows and verifies the backing
secrets table is cleaned up when extensions are uninstalled.
"""

import sqlite3
from urllib.parse import parse_qs, urlparse

import httpx

from helpers import api_get, api_post


def _extract_state(auth_url: str) -> str:
    parsed = urlparse(auth_url)
    state = parse_qs(parsed.query).get("state", [None])[0]
    assert state, f"auth_url should include state: {auth_url}"
    return state


def _secret_exists(db_path: str, user_id: str, name: str) -> bool:
    with sqlite3.connect(db_path) as conn:
        row = conn.execute(
            "SELECT 1 FROM secrets WHERE user_id = ? AND name = ? LIMIT 1",
            (user_id, name),
        ).fetchone()
    return row is not None


def _secret_names(db_path: str, user_id: str) -> set[str]:
    with sqlite3.connect(db_path) as conn:
        rows = conn.execute(
            "SELECT name FROM secrets WHERE user_id = ?",
            (user_id,),
        ).fetchall()
    return {row[0] for row in rows}


async def _get_extension(base_url: str, name: str) -> dict | None:
    response = await api_get(base_url, "/api/extensions", timeout=15)
    response.raise_for_status()
    for extension in response.json().get("extensions", []):
        if extension["name"] == name:
            return extension
    return None


async def _ensure_removed(base_url: str, name: str) -> None:
    extension = await _get_extension(base_url, name)
    if extension is not None:
        response = await api_post(base_url, f"/api/extensions/{name}/remove", timeout=30)
        assert response.status_code == 200, response.text
        assert response.json().get("success") is True, response.text


async def _install_extension(
    base_url: str,
    name: str,
    *,
    kind: str | None = None,
    url: str | None = None,
) -> None:
    payload = {"name": name}
    if kind is not None:
        payload["kind"] = kind
    if url is not None:
        payload["url"] = url

    response = await api_post(
        base_url,
        "/api/extensions/install",
        json=payload,
        timeout=180,
    )
    assert response.status_code == 200, response.text
    assert response.json().get("success") is True, response.text


async def test_remove_wasm_tool_deletes_unique_secret(extension_cleanup_server):
    server = extension_cleanup_server["base_url"]
    db_path = extension_cleanup_server["db_path"]
    user_id = extension_cleanup_server["gateway_user_id"]

    await _ensure_removed(server, "web-search")

    await _install_extension(server, "web-search")

    setup_response = await api_post(
        server,
        "/api/extensions/web-search/setup",
        json={"secrets": {"brave_api_key": "cleanup-test-key"}},
        timeout=30,
    )
    assert setup_response.status_code == 200, setup_response.text
    assert setup_response.json().get("success") is True, setup_response.text
    assert _secret_exists(db_path, user_id, "brave_api_key")

    remove_response = await api_post(
        server,
        "/api/extensions/web-search/remove",
        timeout=30,
    )
    assert remove_response.status_code == 200, remove_response.text
    assert remove_response.json().get("success") is True, remove_response.text
    assert not _secret_exists(db_path, user_id, "brave_api_key")


async def test_remove_wasm_channel_deletes_setup_secrets(extension_cleanup_server):
    server = extension_cleanup_server["base_url"]
    db_path = extension_cleanup_server["db_path"]
    user_id = extension_cleanup_server["gateway_user_id"]

    await _ensure_removed(server, "discord")

    await _install_extension(server, "discord", kind="wasm_channel")

    setup_response = await api_post(
        server,
        "/api/extensions/discord/setup",
        json={
            "secrets": {
                "discord_bot_token": "cleanup-discord-bot-token",
                "discord_public_key": "cleanup-discord-public-key",
            }
        },
        timeout=30,
    )
    assert setup_response.status_code == 200, setup_response.text
    assert setup_response.json().get("success") is True, setup_response.text
    assert _secret_exists(db_path, user_id, "discord_bot_token")
    assert _secret_exists(db_path, user_id, "discord_public_key")

    remove_response = await api_post(
        server,
        "/api/extensions/discord/remove",
        timeout=30,
    )
    assert remove_response.status_code == 200, remove_response.text
    assert remove_response.json().get("success") is True, remove_response.text
    assert not _secret_exists(db_path, user_id, "discord_bot_token")
    assert not _secret_exists(db_path, user_id, "discord_public_key")


async def test_remove_shared_google_oauth_secrets_after_last_tool(extension_cleanup_server):
    server = extension_cleanup_server["base_url"]
    db_path = extension_cleanup_server["db_path"]
    user_id = extension_cleanup_server["gateway_user_id"]

    await _ensure_removed(server, "gmail")
    await _ensure_removed(server, "google-drive")

    await _install_extension(server, "gmail")
    await _install_extension(server, "google-drive")

    setup_response = await api_post(
        server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert setup_response.status_code == 200, setup_response.text
    auth_url = setup_response.json().get("auth_url")
    assert auth_url, setup_response.text

    async with httpx.AsyncClient() as client:
        callback_response = await client.get(
            f"{server}/oauth/callback",
            params={"code": "mock_auth_code", "state": _extract_state(auth_url)},
            timeout=30,
            follow_redirects=True,
        )
    assert callback_response.status_code == 200, callback_response.text[:400]

    shared_secrets = [
        "google_oauth_token",
        "google_oauth_token_refresh_token",
        "google_oauth_token_scopes",
    ]
    for secret_name in shared_secrets:
        assert _secret_exists(db_path, user_id, secret_name), f"expected {secret_name} to exist"

    gmail_remove_response = await api_post(
        server,
        "/api/extensions/gmail/remove",
        timeout=30,
    )
    assert gmail_remove_response.status_code == 200, gmail_remove_response.text
    assert gmail_remove_response.json().get("success") is True, gmail_remove_response.text
    for secret_name in shared_secrets:
        assert _secret_exists(db_path, user_id, secret_name), (
            f"{secret_name} should remain while google-drive is still installed"
        )

    drive_remove_response = await api_post(
        server,
        "/api/extensions/google-drive/remove",
        timeout=30,
    )
    assert drive_remove_response.status_code == 200, drive_remove_response.text
    assert drive_remove_response.json().get("success") is True, drive_remove_response.text
    for secret_name in shared_secrets:
        assert not _secret_exists(db_path, user_id, secret_name), (
            f"{secret_name} should be deleted after the last Google tool is removed"
        )


async def test_remove_mcp_server_deletes_stored_secrets(extension_cleanup_server):
    server = extension_cleanup_server["base_url"]
    db_path = extension_cleanup_server["db_path"]
    user_id = extension_cleanup_server["gateway_user_id"]
    mcp_url = f"{extension_cleanup_server['mock_llm_url']}/mcp"

    await _ensure_removed(server, "mock-mcp")

    await _install_extension(server, "mock-mcp", kind="mcp_server", url=mcp_url)

    setup_response = await api_post(
        server,
        "/api/extensions/mock-mcp/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert setup_response.status_code == 200, setup_response.text
    auth_url = setup_response.json().get("auth_url")
    if auth_url is None:
        activate_response = await api_post(
            server,
            "/api/extensions/mock-mcp/activate",
            timeout=30,
        )
        assert activate_response.status_code == 200, activate_response.text
        auth_url = activate_response.json().get("auth_url")
    assert auth_url, "mock-mcp should require OAuth in E2E"

    async with httpx.AsyncClient() as client:
        callback_response = await client.get(
            f"{server}/oauth/callback",
            params={"code": "mock_mcp_code", "state": _extract_state(auth_url)},
            timeout=30,
            follow_redirects=True,
        )
    assert callback_response.status_code == 200, callback_response.text[:400]

    expected_mcp_secrets = [
        "mcp_mock_mcp_access_token",
        "mcp_mock_mcp_client_id",
    ]
    stored_secret_names = _secret_names(db_path, user_id)
    for secret_name in expected_mcp_secrets:
        assert secret_name in stored_secret_names, (
            f"expected {secret_name} to exist; stored secrets were {sorted(stored_secret_names)}"
        )

    remove_response = await api_post(
        server,
        "/api/extensions/mock-mcp/remove",
        timeout=30,
    )
    assert remove_response.status_code == 200, remove_response.text
    assert remove_response.json().get("success") is True, remove_response.text
    remaining_secret_names = _secret_names(db_path, user_id)
    assert not any(name.startswith("mcp_mock-mcp_") for name in remaining_secret_names), (
        f"mock-mcp secrets should be deleted on remove; remaining secrets were "
        f"{sorted(remaining_secret_names)}"
    )
