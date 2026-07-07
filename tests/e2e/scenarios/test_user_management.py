"""User Management browser regressions."""

import asyncio
import json
import uuid

from helpers import SEL


async def _open_settings_users(page) -> None:
    settings_tab = page.locator(SEL["tab_button"].format(tab="settings"))
    await settings_tab.click()
    await page.locator(SEL["tab_panel"].format(tab="settings")).wait_for(
        state="visible", timeout=5000
    )
    users_tab = page.locator(SEL["settings_subtab"].format(subtab="users"))
    await users_tab.wait_for(state="visible", timeout=5000)
    await users_tab.click()
    await page.locator(SEL["settings_subpanel"].format(subtab="users")).wait_for(
        state="visible", timeout=5000
    )


async def _fill_create_user_form(page, suffix: str) -> None:
    await page.locator(SEL["users_create_btn"]).click()
    await page.locator(SEL["users_display_name"]).fill(f"Debounce User {suffix}")
    await page.locator(SEL["users_email"]).fill(f"debounce-{suffix}@example.test")


async def _wait_for_create_enabled(page) -> None:
    await page.wait_for_function(
        """selector => {
            const button = document.querySelector(selector);
            return !!button && !button.disabled && button.getAttribute("aria-busy") === "false";
        }""",
        SEL["users_create_submit"],
        timeout=5000,
    )


def _capture_dialogs(page, messages: list[str]) -> None:
    def on_dialog(dialog):
        messages.append(dialog.message)
        asyncio.create_task(dialog.accept())

    page.on("dialog", on_dialog)


async def test_user_create_button_disables_during_submission(page):
    """Rapid clicks on Create submit only one user creation request."""
    post_count = 0
    suffix = uuid.uuid4().hex[:8]

    async def users_route(route):
        nonlocal post_count
        if route.request.method != "POST":
            await route.continue_()
            return

        post_count += 1
        await asyncio.sleep(0.3)
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "id": f"debounce-{suffix}",
                    "display_name": f"Debounce User {suffix}",
                    "email": f"debounce-{suffix}@example.test",
                    "role": "member",
                    "status": "active",
                    "token": f"token-{suffix}",
                }
            ),
        )

    await page.route("**/api/admin/users", users_route)
    await _open_settings_users(page)
    await _fill_create_user_form(page, suffix)

    await page.evaluate(
        """selector => {
            const button = document.querySelector(selector);
            button.click();
            button.click();
            button.click();
        }""",
        SEL["users_create_submit"],
    )

    submit = page.locator(SEL["users_create_submit"])
    await submit.wait_for(state="visible", timeout=5000)
    assert await submit.is_disabled(), "Create button should disable while request is in flight"

    await page.wait_for_timeout(100)
    assert post_count == 1, f"expected one create request while pending, got {post_count}"

    await page.locator(SEL["users_create_form"]).wait_for(state="hidden", timeout=5000)
    assert post_count == 1


async def test_user_create_reenables_after_server_error(page):
    """Create form recovers after the API rejects the request."""
    post_count = 0
    suffix = uuid.uuid4().hex[:8]
    dialogs: list[str] = []
    _capture_dialogs(page, dialogs)

    async def users_route(route):
        nonlocal post_count
        if route.request.method != "POST":
            await route.continue_()
            return

        post_count += 1
        await asyncio.sleep(0.1)
        await route.fulfill(status=500, content_type="text/plain", body="server exploded")

    await page.route("**/api/admin/users", users_route)
    await _open_settings_users(page)
    await _fill_create_user_form(page, suffix)

    submit = page.locator(SEL["users_create_submit"])
    await submit.click()
    assert await submit.is_disabled(), "Create button should disable while request is in flight"

    await _wait_for_create_enabled(page)
    await page.locator(SEL["users_create_form"]).wait_for(state="visible", timeout=5000)
    assert post_count == 1
    assert any("server exploded" in message for message in dialogs), dialogs


async def test_user_create_timeout_reenables_form(page):
    """Create form recovers when the request exceeds the client timeout."""
    post_count = 0
    suffix = uuid.uuid4().hex[:8]
    dialogs: list[str] = []
    _capture_dialogs(page, dialogs)

    async def users_route(route):
        nonlocal post_count
        if route.request.method != "POST":
            await route.continue_()
            return

        post_count += 1
        await asyncio.sleep(1)
        try:
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"id": f"late-{suffix}", "token": f"token-{suffix}"}),
            )
        except Exception:
            pass

    await page.route("**/api/admin/users", users_route)
    await _open_settings_users(page)
    await page.evaluate("window.__IRONCLAW_USER_CREATE_TIMEOUT_MS = 150")
    await _fill_create_user_form(page, suffix)

    submit = page.locator(SEL["users_create_submit"])
    await submit.click()
    assert await submit.is_disabled(), "Create button should disable while request is in flight"

    await _wait_for_create_enabled(page)
    await page.locator(SEL["users_create_form"]).wait_for(state="visible", timeout=5000)
    assert post_count == 1
    assert any("timed out" in message.lower() for message in dialogs), dialogs
