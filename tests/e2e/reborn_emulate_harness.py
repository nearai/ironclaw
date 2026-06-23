"""Small Reborn/Emulate full-path helpers for E2E scenarios."""

import asyncio
import json
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import httpx

from helpers import api_get, api_post


def extension_name_matches(actual: str, expected: str) -> bool:
    return actual == expected or actual.replace("-", "_") == expected.replace("-", "_")


def extract_oauth_state(auth_url: str) -> str:
    parsed = urlparse(auth_url)
    state = parse_qs(parsed.query).get("state", [None])[0]
    assert state, f"auth_url should include state: {auth_url}"
    return state


async def get_extension(base_url: str, name: str) -> dict | None:
    response = await api_get(base_url, "/api/extensions", timeout=15)
    response.raise_for_status()
    for extension in response.json().get("extensions", []):
        if extension_name_matches(extension["name"], name):
            return extension
    return None


async def extension_names(base_url: str) -> list[str]:
    response = await api_get(base_url, "/api/extensions", timeout=15)
    response.raise_for_status()
    return [extension["name"] for extension in response.json().get("extensions", [])]


async def install_extension(base_url: str, name: str) -> None:
    installed = await get_extension(base_url, name)
    if installed is not None:
        return

    response = await api_post(
        base_url,
        "/api/extensions/install",
        json={"name": name},
        timeout=180,
    )
    assert response.status_code == 200, response.text
    assert response.json().get("success") is True, response.text


def patch_extension_validation_endpoint(
    tools_dir: str,
    extension_name: str,
    validation_url: str,
) -> None:
    cap_path = Path(tools_dir) / f"{extension_name}.capabilities.json"
    assert cap_path.exists(), (
        f"Expected installed capabilities file at {cap_path}; "
        f"tools dir contains: {[path.name for path in Path(tools_dir).iterdir()]}"
    )
    capabilities = json.loads(cap_path.read_text())
    capabilities["auth"]["validation_endpoint"]["url"] = validation_url
    cap_path.write_text(json.dumps(capabilities, indent=2) + "\n")


async def activate_extension(base_url: str, name: str) -> dict:
    response = await api_post(
        base_url,
        f"/api/extensions/{name}/activate",
        timeout=30,
    )
    assert response.status_code == 200, response.text
    data = response.json()
    assert data.get("success") is True or data.get("activated") is True, data
    return data


async def complete_oauth_setup(
    base_url: str,
    extension_name: str,
    *,
    code: str,
    mock_base_url: str | None = None,
) -> dict:
    response = await api_post(
        base_url,
        f"/api/extensions/{extension_name}/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert response.status_code == 200, response.text
    setup_data = response.json()
    assert setup_data.get("success") is True, setup_data
    auth_url = setup_data.get("auth_url")
    assert auth_url, setup_data

    async with httpx.AsyncClient() as client:
        callback_response = await client.get(
            f"{base_url}/oauth/callback",
            params={"code": code, "state": extract_oauth_state(auth_url)},
            timeout=30,
            follow_redirects=True,
        )
    assert callback_response.status_code == 200, callback_response.text[:400]
    callback_body = callback_response.text.lower()
    oauth_state = None
    if mock_base_url is not None:
        async with httpx.AsyncClient() as client:
            state_response = await client.get(
                f"{mock_base_url}/__mock/oauth/state",
                timeout=10,
            )
        state_response.raise_for_status()
        oauth_state = state_response.json()
    assert "connected" in callback_body or "success" in callback_body, (
        f"{callback_response.text[:1200]}\noauth_state={oauth_state!r}"
    )
    return setup_data


async def new_thread(base_url: str) -> str:
    response = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    assert response.status_code == 200, response.text
    return response.json()["id"]


async def send_chat(base_url: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code == 202, response.text


async def approve_pending_gate(base_url: str, thread_id: str, request_id: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/approval",
        json={"request_id": request_id, "action": "approve", "thread_id": thread_id},
        timeout=15,
    )
    assert response.status_code == 202, (
        f"Approval submission failed: {response.status_code} {response.text[:400]}"
    )


async def history_with_auto_approval(
    base_url: str,
    thread_id: str,
    approved_request_ids: set[str],
) -> dict:
    response = await api_get(
        base_url,
        f"/api/chat/history?thread_id={thread_id}",
        timeout=15,
    )
    response.raise_for_status()
    history = response.json()

    pending = history.get("pending_gate")
    if pending and pending["request_id"] not in approved_request_ids:
        await approve_pending_gate(base_url, thread_id, pending["request_id"])
        approved_request_ids.add(pending["request_id"])

    return history


def tool_result_text(tool_call: dict) -> str:
    result = tool_call.get("result")
    if result is not None:
        if isinstance(result, str):
            return result
        return json.dumps(result)

    preview = tool_call.get("result_preview")
    return "" if preview is None else str(preview)


def tool_result_json(tool_call: dict) -> dict:
    result = tool_call.get("result")
    if isinstance(result, dict):
        return result
    text = tool_result_text(tool_call)
    parsed = json.loads(text)
    assert isinstance(parsed, dict), f"Expected object tool result, got: {parsed!r}"
    return parsed


def completed_tool_results(history: dict, tool_name: str) -> list[dict]:
    return [
        tool_call
        for turn in history.get("turns", [])
        for tool_call in turn.get("tool_calls", [])
        if tool_call.get("name") == tool_name and tool_call.get("has_result")
    ]


async def wait_for_response_containing(
    base_url: str,
    thread_id: str,
    expected: str,
    *,
    timeout: float = 60.0,
) -> dict:
    approved_request_ids: set[str] = set()
    for _ in range(int(timeout * 2)):
        history = await history_with_auto_approval(
            base_url,
            thread_id,
            approved_request_ids,
        )
        for turn in history.get("turns", []):
            response = turn.get("response") or ""
            if expected in response:
                assert history.get("pending_gate") is None, history
                return history
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for response containing {expected!r} "
        f"in thread {thread_id}"
    )
