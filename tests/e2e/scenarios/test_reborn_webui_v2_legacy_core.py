"""Legacy core Playwright scenarios ported to Reborn WebUI v2.

This file is the first migration slice for the legacy ``test_connection.py`` and
basic ``test_chat.py`` intent. It targets the real ``ironclaw-reborn serve``
surface rather than the legacy ``ironclaw`` gateway, so assertions use Reborn's
sidebar routes, token login view, and ``data-testid`` selectors.
"""

import re

import httpx
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    reborn_bearer_headers,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


async def test_reborn_legacy_core_shell_loads_and_navigates(reborn_v2_page):
    """Port of legacy connection/tab smoke to Reborn's sidebar shell."""
    await expect(reborn_v2_page.locator(SEL_V2["chat_composer"])).to_be_visible(
        timeout=15000
    )
    await expect(reborn_v2_page.locator(SEL_V2["sidebar"])).to_be_visible(timeout=15000)

    for label, path in (
        ("Workspace", "/workspace"),
        ("Automations", "/automations"),
        ("Extensions", "/extensions"),
        ("Settings", "/settings"),
    ):
        await reborn_v2_page.get_by_role("link", name=label).click()
        await expect(reborn_v2_page).to_have_url(re.compile(f".*{path}.*"), timeout=10000)

    base_url = reborn_v2_page.url.split("/v2", 1)[0]
    await reborn_v2_page.goto(f"{base_url}/v2/chat?token={REBORN_V2_AUTH_TOKEN}")
    await expect(reborn_v2_page.locator(SEL_V2["chat_composer"])).to_be_visible(
        timeout=15000
    )


async def test_reborn_legacy_v2_shell_hides_removed_work_tabs(reborn_v2_page):
    """Port of legacy v2 shell coverage for removed routines/activity tabs."""
    sidebar = reborn_v2_page.locator(SEL_V2["sidebar"])

    await expect(sidebar.get_by_role("link", name="Automations")).to_be_visible(
        timeout=5000
    )
    await expect(sidebar.get_by_role("link", name="Routines")).to_have_count(0)
    await expect(sidebar.get_by_role("link", name="Missions")).to_have_count(0)


async def test_reborn_legacy_core_auth_rejection(reborn_v2_server, reborn_v2_browser):
    """Port of legacy no-token auth rejection to the Reborn login view."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await page.goto(f"{reborn_v2_server}/v2/")
        await expect(page.locator(SEL_V2["login_token"])).to_be_visible(timeout=15000)
    finally:
        await context.close()


async def test_reborn_legacy_core_health_and_session_api(reborn_v2_server):
    """Port of legacy connection/ownership API checks to Reborn's v2 endpoints."""
    async with httpx.AsyncClient() as client:
        health = await client.get(f"{reborn_v2_server}/api/health", timeout=10)
        assert health.status_code == 200, health.text
        assert health.json()["status"] == "healthy"

        unauthenticated = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/session", timeout=10
        )
        assert unauthenticated.status_code in (401, 403), unauthenticated.text

        invalid = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/session",
            headers={"Authorization": "Bearer not-the-reborn-token"},
            timeout=10,
        )
        assert invalid.status_code in (401, 403), invalid.text

        session = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/session",
            headers=reborn_bearer_headers(),
            timeout=10,
        )
        assert session.status_code == 200, session.text

    payload = session.json()
    assert payload["tenant_id"] == "reborn-v2-e2e"
    assert payload["user_id"] == USER_ID
    assert payload["capabilities"]["operator_webui_config"] is True
    assert "reborn_projects" in payload["features"]

    attachments = payload["attachments"]
    assert ".pdf" in attachments["accept"]
    assert attachments["max_count"] >= 1
    assert attachments["max_file_bytes"] > 0
    assert attachments["max_total_bytes"] >= attachments["max_file_bytes"]


async def test_reborn_legacy_core_send_message_and_receive_response(reborn_v2_page):
    """Port of the legacy single-message chat round trip."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("What is 2+2?")
    await composer.press("Enter")

    await expect(reborn_v2_page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "What is 2+2?", timeout=15000
    )
    await expect(reborn_v2_page.locator(SEL_V2["msg_assistant"]).first).to_contain_text(
        "4", timeout=30000
    )


async def test_reborn_legacy_first_conversation_appears_in_sidebar(reborn_v2_page):
    """Port of the legacy first gateway conversation sidebar-row regression."""
    title = "sidebar label regression check"
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])

    await composer.fill(title)
    await composer.press("Enter")

    await expect(reborn_v2_page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        title, timeout=15000
    )
    await expect(reborn_v2_page.locator(SEL_V2["msg_assistant"]).first).to_be_visible(
        timeout=30000
    )

    sidebar_row = reborn_v2_page.locator("#gateway-sidebar").get_by_role(
        "button"
    ).filter(has_text=title).first
    await expect(sidebar_row).to_be_visible(timeout=15000)
    await expect(sidebar_row).to_contain_text(title)


async def test_reborn_legacy_core_multiple_messages(reborn_v2_page):
    """Port of the legacy two-message browser chat flow."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])

    await composer.fill("Hello")
    await composer.press("Enter")
    await expect(reborn_v2_page.locator(SEL_V2["msg_assistant"])).to_have_count(
        1, timeout=30000
    )

    await composer.fill("What is 2+2?")
    await composer.press("Enter")
    await expect(reborn_v2_page.locator(SEL_V2["msg_user"])).to_have_count(
        2, timeout=15000
    )
    await expect(reborn_v2_page.locator(SEL_V2["msg_assistant"])).to_have_count(
        2, timeout=30000
    )
    await expect(reborn_v2_page.locator(SEL_V2["msg_assistant"]).nth(1)).to_contain_text(
        "4", timeout=30000
    )


async def test_reborn_legacy_core_empty_message_not_sent(reborn_v2_page):
    """Port of the legacy empty-send suppression test."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    initial_user_count = await reborn_v2_page.locator(SEL_V2["msg_user"]).count()
    initial_assistant_count = await reborn_v2_page.locator(SEL_V2["msg_assistant"]).count()

    await composer.fill("   ")
    await composer.press("Enter")
    await reborn_v2_page.wait_for_timeout(750)

    assert await reborn_v2_page.locator(SEL_V2["msg_user"]).count() == initial_user_count
    assert (
        await reborn_v2_page.locator(SEL_V2["msg_assistant"]).count()
        == initial_assistant_count
    )
