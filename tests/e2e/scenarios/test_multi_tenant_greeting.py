"""Multi-tenant assistant greeting regression coverage."""

import httpx
import uuid

from helpers import AUTH_TOKEN, SEL

GREETING_MARKER = "always-on chief of staff"


def _auth_headers(token: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


async def _create_user(base_url: str, display_name: str, email: str) -> str:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{base_url}/api/admin/users",
            headers=_auth_headers(AUTH_TOKEN),
            json={"display_name": display_name, "email": email},
            timeout=15,
        )
    assert response.status_code == 200, response.text
    return response.json()["token"]


async def _assistant_history(base_url: str, token: str) -> tuple[str, list[dict]]:
    async with httpx.AsyncClient() as client:
        threads = await client.get(
            f"{base_url}/api/chat/threads",
            headers=_auth_headers(token),
            timeout=15,
        )
        threads.raise_for_status()
        assistant_thread_id = threads.json()["assistant_thread"]["id"]
        history = await client.get(
            f"{base_url}/api/chat/history",
            headers=_auth_headers(token),
            params={"thread_id": assistant_thread_id},
            timeout=15,
        )
        history.raise_for_status()
    return assistant_thread_id, history.json()["turns"]


async def _open_user_page(browser, base_url: str, token: str):
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{base_url}/?token={token}")
    await page.wait_for_selector(SEL["auth_screen"], state="hidden", timeout=15000)
    return context, page


async def _assert_single_greeting(page, expected_count: int = 1) -> None:
    assistant_sel = SEL["message_assistant"]
    await page.wait_for_function(
        """({ selector, expectedCount, marker }) => {
            const messages = document.querySelectorAll(selector);
            if (messages.length !== expectedCount) return false;
            const text = (messages[messages.length - 1].innerText || '').toLowerCase();
            return text.includes(marker.toLowerCase());
        }""",
        arg={
            "selector": assistant_sel,
            "expectedCount": expected_count,
            "marker": GREETING_MARKER,
        },
        timeout=15000,
    )


async def test_multi_tenant_initial_greeting_is_persisted_once(browser, ironclaw_server):
    alice_suffix = uuid.uuid4().hex[:8]
    bob_suffix = uuid.uuid4().hex[:8]
    charlie_suffix = uuid.uuid4().hex[:8]

    alice_token = await _create_user(
        ironclaw_server,
        display_name="Alice Tenant",
        email=f"alice-tenant-{alice_suffix}@example.com",
    )
    bob_token = await _create_user(
        ironclaw_server,
        display_name="Bob Tenant",
        email=f"bob-tenant-{bob_suffix}@example.com",
    )
    charlie_token = await _create_user(
        ironclaw_server,
        display_name="Charlie Tenant",
        email=f"charlie-tenant-{charlie_suffix}@example.com",
    )

    alice_context, alice_page = await _open_user_page(browser, ironclaw_server, alice_token)
    try:
        await _assert_single_greeting(alice_page)
        alice_thread_id, alice_turns = await _assistant_history(ironclaw_server, alice_token)
        assert len(alice_turns) == 1
        assert GREETING_MARKER in (alice_turns[0].get("response") or "").lower()
    finally:
        await alice_context.close()

    bob_context, bob_page = await _open_user_page(browser, ironclaw_server, bob_token)
    try:
        await _assert_single_greeting(bob_page)
        bob_thread_id, bob_turns = await _assistant_history(ironclaw_server, bob_token)
        assert len(bob_turns) == 1
        assert GREETING_MARKER in (bob_turns[0].get("response") or "").lower()
    finally:
        await bob_context.close()

    charlie_context, charlie_page = await _open_user_page(browser, ironclaw_server, charlie_token)
    try:
        await _assert_single_greeting(charlie_page)
        charlie_thread_id, charlie_turns = await _assistant_history(
            ironclaw_server, charlie_token
        )
        assert len(charlie_turns) == 1
        assert GREETING_MARKER in (charlie_turns[0].get("response") or "").lower()
    finally:
        await charlie_context.close()

    assert len({alice_thread_id, bob_thread_id, charlie_thread_id}) == 3

    alice_reload_context, alice_reload_page = await _open_user_page(
        browser, ironclaw_server, alice_token
    )
    try:
        await _assert_single_greeting(alice_reload_page)
        _, alice_turns_after = await _assistant_history(ironclaw_server, alice_token)
        assert len(alice_turns_after) == 1
    finally:
        await alice_reload_context.close()

    bob_reload_context, bob_reload_page = await _open_user_page(
        browser, ironclaw_server, bob_token
    )
    try:
        await _assert_single_greeting(bob_reload_page)
        _, bob_turns_after = await _assistant_history(ironclaw_server, bob_token)
        assert len(bob_turns_after) == 1
    finally:
        await bob_reload_context.close()
