from __future__ import annotations

import time
from typing import Any
from urllib.parse import parse_qs, urlparse

from scripts.live_canary.common import CanaryError, ProbeResult, api_request


async def put_secret(
    base_url: str,
    token: str,
    *,
    user_id: str,
    name: str,
    value: str,
    provider: str | None = None,
) -> None:
    payload: dict[str, Any] = {"value": value}
    if provider is not None:
        payload["provider"] = provider
    response = await api_request(
        "PUT",
        base_url,
        f"/api/admin/users/{user_id}/secrets/{name}",
        token=token,
        json_body=payload,
    )
    if response.status_code != 200:
        raise CanaryError(f"Failed to seed secret {name}: {response.status_code} {response.text}")


async def list_extensions(base_url: str, token: str) -> list[dict[str, Any]]:
    response = await api_request("GET", base_url, "/api/extensions", token=token, timeout=30)
    response.raise_for_status()
    return response.json().get("extensions", [])


async def get_extension(base_url: str, token: str, name: str) -> dict[str, Any] | None:
    for extension in await list_extensions(base_url, token):
        if extension.get("name") == name:
            return extension
    return None


async def wait_for_extension(
    base_url: str,
    token: str,
    *,
    expected_display_name: str,
    timeout: float = 60.0,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for ext in await list_extensions(base_url, token):
            if ext.get("display_name") == expected_display_name or ext.get("name") == expected_display_name:
                return ext
        await _sleep()
    raise CanaryError(f"Timed out waiting for extension {expected_display_name}")


async def wait_for_extension_state(
    base_url: str,
    token: str,
    name: str,
    *,
    authenticated: bool | None = None,
    active: bool | None = None,
    timeout: float = 60.0,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        extension = await get_extension(base_url, token, name)
        if extension is not None:
            if authenticated is not None and extension.get("authenticated") != authenticated:
                await _sleep()
                continue
            if active is not None and extension.get("active") != active:
                await _sleep()
                continue
            return extension
        await _sleep()
    raise CanaryError(f"Timed out waiting for extension state: {name}")


async def install_extension(
    base_url: str,
    token: str,
    *,
    name: str,
    expected_display_name: str,
    install_kind: str | None = None,
    install_url: str | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {"name": name}
    if install_kind is not None:
        payload["kind"] = install_kind
    if install_url is not None:
        payload["url"] = install_url
    response = await api_request(
        "POST",
        base_url,
        "/api/extensions/install",
        token=token,
        json_body=payload,
        timeout=180,
    )
    if response.status_code != 200:
        raise CanaryError(f"Install failed for {name}: {response.status_code} {response.text}")
    body = response.json()
    if not body.get("success"):
        raise CanaryError(f"Install failed for {name}: {body}")
    return await wait_for_extension(
        base_url,
        token,
        expected_display_name=expected_display_name,
    )


async def activate_extension(
    base_url: str,
    token: str,
    *,
    extension_name: str,
    expected_display_name: str,
    timeout: float = 90.0,
) -> dict[str, Any]:
    response = await api_request(
        "POST",
        base_url,
        f"/api/extensions/{extension_name}/activate",
        token=token,
        timeout=60,
    )
    if response.status_code != 200:
        raise CanaryError(
            f"Activation failed for {extension_name}: {response.status_code} {response.text}"
        )
    body = response.json()
    if body.get("auth_url"):
        raise CanaryError(
            f"Activation unexpectedly required interactive auth for {extension_name}: {body['auth_url']}"
        )
    return await wait_for_extension_state(
        base_url,
        token,
        extension_name,
        authenticated=True,
        active=True,
        timeout=timeout,
    )


async def complete_oauth_flow(
    base_url: str,
    token: str,
    *,
    extension_name: str,
    code: str = "mock_auth_code",
    timeout: float = 90.0,
) -> dict[str, Any]:
    """Complete OAuth setup for an extension via the callback flow.

    Calls /api/extensions/{name}/setup to get an auth_url, extracts the
    state parameter, and completes the OAuth callback. The mock_llm
    exchange endpoint returns real or mock tokens depending on env vars.
    """
    import httpx

    setup_response = await api_request(
        "POST",
        base_url,
        f"/api/extensions/{extension_name}/setup",
        token=token,
        json_body={"secrets": {}},
        timeout=30,
    )
    if setup_response.status_code != 200:
        raise CanaryError(
            f"Setup failed for {extension_name}: {setup_response.status_code} {setup_response.text}"
        )
    auth_url = setup_response.json().get("auth_url")
    if not auth_url:
        raise CanaryError(f"No auth_url from setup for {extension_name}: {setup_response.json()}")

    state = parse_qs(urlparse(auth_url).query).get("state", [None])[0]
    if not state:
        raise CanaryError(f"auth_url missing state parameter: {auth_url}")

    async with httpx.AsyncClient(timeout=30.0) as client:
        callback_response = await client.get(
            f"{base_url}/oauth/callback",
            params={"code": code, "state": state},
            follow_redirects=True,
        )

    if callback_response.status_code != 200:
        raise CanaryError(
            f"OAuth callback failed for {extension_name}: "
            f"{callback_response.status_code} {callback_response.text[:500]}"
        )
    body_text = callback_response.text.lower()
    if "connected" not in body_text and "success" not in body_text:
        raise CanaryError(
            f"OAuth callback did not indicate success for {extension_name}: "
            f"{callback_response.text[:500]}"
        )

    return await wait_for_extension_state(
        base_url,
        token,
        extension_name,
        authenticated=True,
        active=True,
        timeout=timeout,
    )


async def create_responses_probe(
    *,
    base_url: str,
    token: str,
    provider: str,
    prompt: str,
    expected_tool_name: str,
    expected_text: str,
) -> ProbeResult:
    started = time.perf_counter()
    response = await api_request(
        "POST",
        base_url,
        "/v1/responses",
        token=token,
        json_body={"model": "default", "input": prompt},
        timeout=180,
    )
    latency_ms = int((time.perf_counter() - started) * 1000)
    if response.status_code != 200:
        return ProbeResult(
            provider=provider,
            mode="responses_api",
            success=False,
            latency_ms=latency_ms,
            details={"status_code": response.status_code, "body": response.text[:1000]},
        )

    body = response.json()
    tool_names = [item.get("name") for item in body.get("output", []) if item.get("type") == "function_call"]
    tool_outputs = [
        item.get("output", "")
        for item in body.get("output", [])
        if item.get("type") == "function_call_output"
    ]
    texts: list[str] = []
    for item in body.get("output", []):
        if item.get("type") != "message":
            continue
        for content in item.get("content", []):
            if content.get("type") == "output_text":
                texts.append(content.get("text", ""))
    response_text = "\n".join(texts)
    success = (
        body.get("status") == "completed"
        and expected_tool_name in tool_names
        and bool(tool_outputs)
        and expected_text in response_text
    )
    return ProbeResult(
        provider=provider,
        mode="responses_api",
        success=success,
        latency_ms=latency_ms,
        details={
            "status": body.get("status"),
            "tool_names": tool_names,
            "tool_outputs": tool_outputs,
            "response_text": response_text,
            "error": body.get("error"),
        },
    )


async def _sleep() -> None:
    import asyncio

    await asyncio.sleep(0.5)
