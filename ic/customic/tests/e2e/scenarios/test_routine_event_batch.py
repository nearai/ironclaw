"""E2E tests for event-triggered routines over the HTTP channel."""

import asyncio
import json
import uuid

import httpx
import pytest

from helpers import AUTH_TOKEN, SEL, signed_http_webhook_headers


async def _send_chat_message(page, message: str) -> None:
    """Send a chat message and wait for the assistant turn to appear."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)
    assistant_messages = page.locator(SEL["message_assistant"])
    before_count = await assistant_messages.count()

    await chat_input.fill(message)
    await chat_input.press("Enter")

    await page.wait_for_function(
        """({ selector, expectedCount }) => {
            return document.querySelectorAll(selector).length >= expectedCount;
        }""",
        arg={
            "selector": SEL["message_assistant"],
            "expectedCount": before_count + 1,
        },
        timeout=30000,
    )


async def _create_event_routine(
    page,
    base_url: str,
    *,
    name: str,
    pattern: str,
    channel: str = "http",
) -> dict:
    """Create an event routine through chat and return its API record."""
    await _send_chat_message(
        page,
        f"create event routine {name} channel {channel} pattern {pattern}",
    )
    return await _wait_for_routine(base_url, name)


async def _post_http_message(
    http_channel_server: str,
    *,
    content: str,
    sender_id: str | None = None,
    thread_id: str | None = None,
) -> dict:
    """Send a signed HTTP-channel message and return the JSON body."""
    payload = {
        "user_id": sender_id or f"sender-{uuid.uuid4().hex[:8]}",
        "thread_id": thread_id or f"thread-{uuid.uuid4().hex[:8]}",
        "content": content,
        "wait_for_response": True,
    }
    body = json.dumps(payload).encode("utf-8")

    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{http_channel_server}/webhook",
            content=body,
            headers=signed_http_webhook_headers(body),
            timeout=90,
        )

    assert response.status_code == 200, (
        f"HTTP webhook failed: {response.status_code} {response.text[:400]}"
    )
    return response.json()


async def _wait_for_routine(base_url: str, name: str, timeout: float = 20.0) -> dict:
    """Poll the routines API until the named routine exists."""
    async with httpx.AsyncClient() as client:
        for _ in range(int(timeout * 2)):
            response = await client.get(
                f"{base_url}/api/routines",
                headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
                timeout=10,
            )
            response.raise_for_status()
            for routine in response.json()["routines"]:
                if routine["name"] == name:
                    return routine
            await asyncio.sleep(0.5)
    raise AssertionError(f"Routine '{name}' was not created within {timeout}s")


async def _get_routine_runs(base_url: str, routine_id: str) -> list[dict]:
    """Fetch recent routine runs from the web API."""
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{base_url}/api/routines/{routine_id}/runs",
            headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
            timeout=10,
        )
    response.raise_for_status()
    return response.json()["runs"]


async def _wait_for_run_count(
    base_url: str,
    routine_id: str,
    *,
    expected_at_least: int,
    timeout: float = 20.0,
) -> list[dict]:
    """Poll until the routine has at least the expected run count."""
    for _ in range(int(timeout * 2)):
        runs = await _get_routine_runs(base_url, routine_id)
        if len(runs) >= expected_at_least:
            return runs
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Routine '{routine_id}' did not reach {expected_at_least} runs within {timeout}s"
    )


async def _wait_for_completed_run(
    base_url: str,
    routine_id: str,
    *,
    timeout: float = 30.0,
) -> dict:
    """Poll until the newest run is no longer marked running."""
    for _ in range(int(timeout * 2)):
        runs = await _get_routine_runs(base_url, routine_id)
        if runs and runs[0]["status"].lower() != "running":
            return runs[0]
        await asyncio.sleep(0.5)
    raise AssertionError(f"Routine '{routine_id}' did not complete within {timeout}s")


@pytest.mark.asyncio
async def test_create_event_trigger_routine(page, ironclaw_server):
    """Event routines can be created through the supported chat flow."""
    name = f"evt-{uuid.uuid4().hex[:8]}"
    routine = await _create_event_routine(
        page,
        ironclaw_server,
        name=name,
        pattern="test|demo",
    )

    assert routine["id"]
    assert routine["trigger_type"] == "event"
    assert "test|demo" in routine["trigger_summary"]


@pytest.mark.asyncio
async def test_event_trigger_fires_on_matching_message(
    page,
    ironclaw_server,
    http_channel_server,
):
    """Matching HTTP-channel messages create routine runs."""
    name = f"evt-{uuid.uuid4().hex[:8]}"
    routine = await _create_event_routine(
        page,
        ironclaw_server,
        name=name,
        pattern="urgent|critical|alert",
    )

    response = await _post_http_message(
        http_channel_server,
        content="urgent: server down",
    )
    assert response["status"] == "accepted"

    await _wait_for_run_count(
        ironclaw_server,
        routine["id"],
        expected_at_least=1,
    )
    completed_run = await _wait_for_completed_run(ironclaw_server, routine["id"])

    assert completed_run["status"].lower() == "attention"
    assert completed_run["trigger_type"] == "event"


@pytest.mark.asyncio
async def test_event_trigger_skips_non_matching_message(
    page,
    ironclaw_server,
    http_channel_server,
):
    """Non-matching messages do not create routine runs."""
    name = f"evt-{uuid.uuid4().hex[:8]}"
    routine = await _create_event_routine(
        page,
        ironclaw_server,
        name=name,
        pattern="urgent|critical|alert",
    )

    await _post_http_message(
        http_channel_server,
        content="hello there",
    )
    await asyncio.sleep(2)

    assert await _get_routine_runs(ironclaw_server, routine["id"]) == []


@pytest.mark.asyncio
async def test_multiple_routines_fire_on_matching_message(
    page,
    ironclaw_server,
    http_channel_server,
):
    """A single matching message can fire multiple event routines."""
    routines = []
    for _ in range(3):
        name = f"evt-{uuid.uuid4().hex[:8]}"
        routines.append(
            await _create_event_routine(
                page,
                ironclaw_server,
                name=name,
                pattern="error|warning|alert",
            )
        )

    await _post_http_message(
        http_channel_server,
        content="error: database connection failed",
    )

    for routine in routines:
        await _wait_for_run_count(
            ironclaw_server,
            routine["id"],
            expected_at_least=1,
        )
        completed_run = await _wait_for_completed_run(ironclaw_server, routine["id"])
        assert completed_run["status"].lower() == "attention"


@pytest.mark.asyncio
async def test_channel_filter_applied_correctly(
    page,
    ironclaw_server,
    http_channel_server,
):
    """Channel filters prevent HTTP messages from firing non-HTTP routines."""
    http_routine = await _create_event_routine(
        page,
        ironclaw_server,
        name=f"evt-{uuid.uuid4().hex[:8]}",
        pattern="alert",
        channel="http",
    )
    telegram_routine = await _create_event_routine(
        page,
        ironclaw_server,
        name=f"evt-{uuid.uuid4().hex[:8]}",
        pattern="alert",
        channel="telegram",
    )

    await _post_http_message(
        http_channel_server,
        content="alert from webhook",
    )

    await _wait_for_run_count(
        ironclaw_server,
        http_routine["id"],
        expected_at_least=1,
    )
    http_run = await _wait_for_completed_run(ironclaw_server, http_routine["id"])
    await asyncio.sleep(2)
    telegram_runs = await _get_routine_runs(ironclaw_server, telegram_routine["id"])

    assert http_run["status"].lower() == "attention"
    assert telegram_runs == []


@pytest.mark.asyncio
async def test_routine_execution_history_is_available(
    page,
    ironclaw_server,
    http_channel_server,
):
    """Routine run history is exposed by the routines runs API."""
    routine = await _create_event_routine(
        page,
        ironclaw_server,
        name=f"evt-{uuid.uuid4().hex[:8]}",
        pattern="history",
    )

    await _post_http_message(
        http_channel_server,
        content="history event",
    )

    await _wait_for_run_count(
        ironclaw_server,
        routine["id"],
        expected_at_least=1,
    )
    completed_run = await _wait_for_completed_run(ironclaw_server, routine["id"])

    assert completed_run["id"]
    assert completed_run["started_at"]
    assert completed_run["status"].lower() == "attention"
