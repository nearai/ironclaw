"""Helpers for exercising the production-wired notification Inbox in E2E tests."""

import asyncio
import math

import httpx

from reborn_webui_harness import create_thread, send_message, wait_for_assistant_message


async def retry_after_rate_limit(response: httpx.Response, *, deadline: float) -> bool:
    """Sleep for a bounded Retry-After delay when the API throttles polling."""
    if response.status_code != 429:
        return False
    try:
        retry_after = float(response.headers.get("retry-after", "1"))
    except (TypeError, ValueError):
        retry_after = 1.0
    if not math.isfinite(retry_after):
        retry_after = 1.0
    remaining = max(deadline - asyncio.get_running_loop().time(), 0.0)
    await asyncio.sleep(min(max(retry_after, 0.5), remaining))
    return True


def notification_automation_name(kind: str, label: str) -> str:
    return f"E2E notification {kind} {label}"


async def create_notification_automation(
    client: httpx.AsyncClient,
    base_url: str,
    kind: str,
    label: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Create a far-future automation through the real agent/tool path."""
    thread_id = await create_thread(client, base_url)
    await send_message(
        client,
        base_url,
        thread_id,
        f"reborn create notification {kind} automation {label}",
    )
    await wait_for_assistant_message(client, base_url, thread_id, timeout=timeout)
    expected_name = notification_automation_name(kind, label)
    deadline = asyncio.get_running_loop().time() + timeout
    last_body: dict = {}
    while asyncio.get_running_loop().time() < deadline:
        response = await client.get(
            f"{base_url}/api/webchat/v2/automations",
            params={"include_completed": "true", "limit": 100, "run_limit": 0},
            timeout=10,
        )
        if await retry_after_rate_limit(response, deadline=deadline):
            continue
        response.raise_for_status()
        last_body = response.json()
        match = next(
            (
                item
                for item in last_body.get("automations", [])
                if item.get("name") == expected_name
            ),
            None,
        )
        if match is not None:
            return match
        await asyncio.sleep(0.25)
    raise AssertionError(
        f"Timed out waiting for automation {expected_name!r}; last={last_body}"
    )


async def run_notification_automation(
    client: httpx.AsyncClient,
    base_url: str,
    automation_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Run an automation and return its projected run with thread identity."""
    response = await client.post(
        f"{base_url}/api/webchat/v2/automations/{automation_id}/run",
        timeout=15,
    )
    assert response.status_code == 200, response.text
    body = response.json()
    run_id = (body.get("run_result") or {}).get("run_id")
    assert isinstance(run_id, str) and run_id, body

    deadline = asyncio.get_running_loop().time() + timeout
    last_body: dict = {}
    while asyncio.get_running_loop().time() < deadline:
        listed = await client.get(
            f"{base_url}/api/webchat/v2/automations",
            params={"include_completed": "true", "limit": 100, "run_limit": 10},
            timeout=10,
        )
        if await retry_after_rate_limit(listed, deadline=deadline):
            continue
        listed.raise_for_status()
        last_body = listed.json()
        automation = next(
            (
                item
                for item in last_body.get("automations", [])
                if item.get("automation_id") == automation_id
            ),
            None,
        )
        run = next(
            (
                item
                for item in (automation or {}).get("recent_runs", [])
                if item.get("run_id") == run_id and item.get("thread_id")
            ),
            None,
        )
        if run is not None:
            return run
        await asyncio.sleep(0.25)
    raise AssertionError(
        f"Timed out waiting for automation run {run_id!r}; last={last_body}"
    )


async def wait_for_notification(
    client: httpx.AsyncClient,
    base_url: str,
    *,
    kind: str,
    run_id: str,
    resolved: bool | None = None,
    timeout: float = 90.0,
) -> dict:
    """Wait for one production Inbox record and optionally its resolution state."""
    deadline = asyncio.get_running_loop().time() + timeout
    last_body: dict = {}
    while asyncio.get_running_loop().time() < deadline:
        response = await client.get(
            f"{base_url}/api/webchat/v2/notifications",
            params={"limit": 100},
            timeout=10,
        )
        if await retry_after_rate_limit(response, deadline=deadline):
            continue
        response.raise_for_status()
        last_body = response.json()
        notification = next(
            (
                item
                for item in last_body.get("notifications", [])
                if item.get("kind") == kind and item.get("turn_run_id") == run_id
            ),
            None,
        )
        if notification is not None:
            is_resolved = notification.get("resolved_at") is not None
            if resolved is None or is_resolved is resolved:
                return notification
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for {kind!r} notification for {run_id!r}; last={last_body}"
    )


async def delete_notification_automation(
    client: httpx.AsyncClient,
    base_url: str,
    automation_id: str | None,
) -> None:
    if automation_id is None:
        return
    response = await client.delete(
        f"{base_url}/api/webchat/v2/automations/{automation_id}",
        timeout=15,
    )
    assert response.status_code == 200, response.text
