"""Owner-scope end-to-end scenarios.

These tests exercise the explicit owner model across:
- the web gateway chat UI
- the owner-scoped HTTP webhook channel
- routine tools / routines tab
- job creation via routine execution / jobs tab
"""

import asyncio
import json
import uuid

import httpx

from helpers import SEL, AUTH_TOKEN, signed_http_webhook_headers


async def _send_and_get_response(
    page,
    message: str,
    *,
    expected_fragment: str,
    timeout: int = 30000,
) -> str:
    """Send a chat message and return the newest assistant response text."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    assistant_sel = SEL["message_assistant"]
    before_count = await page.locator(assistant_sel).count()

    await chat_input.fill(message)
    await chat_input.press("Enter")

    expected = before_count + 1
    await page.wait_for_function(
        """({ assistantSelector, expectedCount, expectedFragment }) => {
            const messages = document.querySelectorAll(assistantSelector);
            if (messages.length < expectedCount) return false;
            const text = (messages[messages.length - 1].innerText || '').trim().toLowerCase();
            return text.includes(expectedFragment.toLowerCase());
        }""",
        arg={
            "assistantSelector": assistant_sel,
            "expectedCount": expected,
            "expectedFragment": expected_fragment,
        },
        timeout=timeout,
    )

    return await page.locator(assistant_sel).last.inner_text()


async def _post_http_webhook(
    http_channel_server: str,
    *,
    content: str,
    sender_id: str,
    thread_id: str,
) -> str:
    """Send a signed request to the owner-scoped HTTP webhook channel."""
    payload = {
        "user_id": sender_id,
        "thread_id": thread_id,
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
    data = response.json()
    assert data["status"] == "accepted", f"Unexpected webhook response: {data}"
    assert data["response"], f"Expected synchronous response body, got: {data}"
    return data["response"]


async def _open_tab(page, tab: str) -> None:
    btn = page.locator(SEL["tab_button"].format(tab=tab))
    await btn.click()
    await page.locator(SEL["tab_panel"].format(tab=tab)).wait_for(
        state="visible",
        timeout=5000,
    )


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
            routines = response.json()["routines"]
            for routine in routines:
                if routine["name"] == name:
                    return routine
            await _poll_sleep()
    raise AssertionError(f"Routine '{name}' was not created within {timeout}s")


async def _wait_for_job(base_url: str, title: str, timeout: float = 30.0) -> dict:
    """Poll the jobs API until the named job exists."""
    async with httpx.AsyncClient() as client:
        for _ in range(int(timeout * 2)):
            response = await client.get(
                f"{base_url}/api/jobs",
                headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
                timeout=10,
            )
            response.raise_for_status()
            jobs = response.json()["jobs"]
            for job in jobs:
                if job["title"] == title:
                    return job
            await _poll_sleep()
    raise AssertionError(f"Job '{title}' was not created within {timeout}s")


async def _poll_sleep() -> None:
    """Small shared backoff for API polling loops."""
    await asyncio.sleep(0.5)


async def test_http_channel_created_routine_is_visible_in_web_routines_tab(
    page,
    ironclaw_server,
    http_channel_server,
):
    """A routine created from the HTTP channel is visible in the web owner UI."""
    routine_name = f"owner-http-{uuid.uuid4().hex[:8]}"

    response_text = await _post_http_webhook(
        http_channel_server,
        content=f"create lightweight owner routine {routine_name}",
        sender_id="external-sender-alpha",
        thread_id="http-owner-routine-thread",
    )
    assert routine_name in response_text

    await _wait_for_routine(ironclaw_server, routine_name)

    await _open_tab(page, "routines")
    await page.locator(SEL["routine_row"]).filter(has_text=routine_name).first.wait_for(
        state="visible",
        timeout=15000,
    )


async def test_web_created_routine_is_listed_from_http_channel_across_senders(
    page,
    ironclaw_server,
    http_channel_server,
):
    """Routines created in web chat remain owner-global across HTTP senders/threads."""
    routine_name = f"owner-web-{uuid.uuid4().hex[:8]}"

    assistant_text = await _send_and_get_response(
        page,
        f"create lightweight owner routine {routine_name}",
        expected_fragment=routine_name,
    )
    assert routine_name in assistant_text

    await _wait_for_routine(ironclaw_server, routine_name)

    first_sender_text = await _post_http_webhook(
        http_channel_server,
        content="list owner routines",
        sender_id="http-sender-one",
        thread_id="owner-list-thread-a",
    )
    second_sender_text = await _post_http_webhook(
        http_channel_server,
        content="list owner routines",
        sender_id="http-sender-two",
        thread_id="owner-list-thread-b",
    )

    assert routine_name in first_sender_text, first_sender_text
    assert routine_name in second_sender_text, second_sender_text


async def test_http_created_full_job_routine_can_be_run_from_web_and_shows_in_jobs(
    page,
    ironclaw_server,
    http_channel_server,
):
    """A full-job routine created via HTTP can be run from the web UI and create a job."""
    routine_name = f"owner-job-{uuid.uuid4().hex[:8]}"

    response_text = await _post_http_webhook(
        http_channel_server,
        content=f"create full-job owner routine {routine_name}",
        sender_id="http-job-sender",
        thread_id="owner-job-thread",
    )
    assert routine_name in response_text

    await _wait_for_routine(ironclaw_server, routine_name)

    await _open_tab(page, "routines")
    routine_row = page.locator(SEL["routine_row"]).filter(has_text=routine_name).first
    await routine_row.wait_for(state="visible", timeout=15000)
    await routine_row.locator('button[data-action="trigger-routine"]').click()

    await _wait_for_job(ironclaw_server, routine_name, timeout=45.0)

    await _open_tab(page, "jobs")
    await page.locator(SEL["job_row"]).filter(has_text=routine_name).first.wait_for(
        state="visible",
        timeout=20000,
    )
