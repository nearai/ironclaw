"""Telegram hot-activation UI coverage."""

import asyncio
import json

from helpers import SEL

_CONFIGURE_SECRET_INPUT = "input[type='password']"
_CONFIGURE_SAVE_BUTTON = ".configure-actions button.btn-ext.activate"


_TELEGRAM_INSTALLED = {
    "name": "telegram",
    "display_name": "Telegram",
    "kind": "wasm_channel",
    "description": "Telegram Bot API channel",
    "url": None,
    "active": False,
    "authenticated": False,
    "has_auth": False,
    "needs_setup": True,
    "tools": [],
    "activation_status": "installed",
    "activation_error": None,
}

_TELEGRAM_ACTIVE = {
    **_TELEGRAM_INSTALLED,
    "active": True,
    "authenticated": True,
    "needs_setup": False,
    "activation_status": "active",
}


async def go_to_channels(page):
    """Navigate to Settings → Channels subtab (where wasm_channel extensions live)."""
    await page.locator(SEL["tab_button"].format(tab="settings")).click()
    await page.locator(SEL["settings_subtab"].format(subtab="channels")).click()
    await page.locator(SEL["settings_subpanel"].format(subtab="channels")).wait_for(
        state="visible", timeout=5000
    )
    # Wait for the Telegram card specifically (built-in cards render first)
    await page.locator(SEL["channels_ext_card"], has_text="Telegram").wait_for(
        state="visible", timeout=8000
    )


async def _default_gateway_status_handler(route):
    await route.fulfill(
        status=200,
        content_type="application/json",
        body=json.dumps({"enabled_channels": [], "sse_connections": 0, "ws_connections": 0}),
    )


async def mock_extension_lists(page, ext_handler, *, gateway_status_handler=None):
    async def handle_ext_list(route):
        path = route.request.url.split("?")[0]
        if path.endswith("/api/extensions"):
            await ext_handler(route)
        else:
            await route.continue_()

    async def handle_tools(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"tools": []}),
        )

    async def handle_registry(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"entries": []}),
        )

    # Register the broad route first so the specific endpoints below win.
    await page.route("**/api/extensions*", handle_ext_list)
    await page.route("**/api/extensions/tools", handle_tools)
    await page.route("**/api/extensions/registry", handle_registry)
    await page.route(
        "**/api/gateway/status",
        gateway_status_handler or _default_gateway_status_handler,
    )


async def wait_for_toast(page, text: str, *, timeout: int = 5000):
    await page.locator(SEL["toast"], has_text=text).wait_for(
        state="visible", timeout=timeout
    )


async def test_telegram_setup_modal_shows_bot_token_field(page):
    async def handle_ext_list(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"extensions": [_TELEGRAM_INSTALLED]}),
        )

    async def handle_setup(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "secrets": [
                        {
                            "name": "telegram_bot_token",
                            "prompt": "Enter your Telegram Bot API token (from @BotFather)",
                            "provided": False,
                            "optional": False,
                            "auto_generate": False,
                        }
                    ]
                }
            ),
        )

    await mock_extension_lists(page, handle_ext_list)
    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await go_to_channels(page)

    card = page.locator(SEL["channels_ext_card"], has_text="Telegram")
    await card.locator(SEL["ext_configure_btn"], has_text="Setup").click()

    modal = page.locator(SEL["configure_modal"])
    await modal.wait_for(state="visible", timeout=5000)
    assert "Telegram Bot API token" in await modal.text_content()
    assert "IronClaw will show a one-time code" in (
        await modal.text_content()
    )
    input_el = modal.locator(_CONFIGURE_SECRET_INPUT)
    assert await input_el.count() == 1


async def test_telegram_hot_activation_transitions_installed_to_active(page):
    phase = {"value": "installed"}
    captured_setup_payloads = []
    post_count = {"value": 0}
    second_request_started = asyncio.Event()
    allow_second_response = asyncio.Event()

    async def handle_ext_list(route):
        extensions = {
            "installed": [_TELEGRAM_INSTALLED],
            "active": [_TELEGRAM_ACTIVE],
        }[phase["value"]]
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"extensions": extensions}),
        )

    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    {
                        "secrets": [
                            {
                                "name": "telegram_bot_token",
                                "prompt": "Enter your Telegram Bot API token (from @BotFather)",
                                "provided": False,
                                "optional": False,
                                "auto_generate": False,
                            }
                        ]
                    }
                ),
            )
            return

        payload = json.loads(route.request.post_data or "{}")
        captured_setup_payloads.append(payload)
        post_count["value"] += 1
        await asyncio.sleep(0.05)
        if post_count["value"] == 1:
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    {
                        "success": True,
                        "activated": False,
                        "message": "Configuration saved for 'telegram'. Send `/start iclaw-7qk2m9` to @test_hot_bot in Telegram. IronClaw will finish setup automatically.",
                        "verification": {
                            "code": "iclaw-7qk2m9",
                            "instructions": "Send `/start iclaw-7qk2m9` to @test_hot_bot in Telegram. IronClaw will finish setup automatically.",
                            "deep_link": "https://t.me/test_hot_bot?start=iclaw-7qk2m9",
                        },
                    }
                ),
            )
        else:
            second_request_started.set()
            await allow_second_response.wait()
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    {
                        "success": True,
                        "activated": True,
                        "message": "Configuration saved, Telegram owner verified, and 'telegram' activated. Hot-activated WASM channel",
                    }
                ),
            )

    await mock_extension_lists(page, handle_ext_list)
    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await go_to_channels(page)

    card = page.locator(SEL["channels_ext_card"], has_text="Telegram")
    await card.locator(SEL["ext_configure_btn"], has_text="Setup").click()

    modal = page.locator(SEL["configure_modal"])
    await modal.wait_for(state="visible", timeout=5000)
    await modal.locator(_CONFIGURE_SECRET_INPUT).fill("123456789:ABCdefGhI")
    await modal.locator(_CONFIGURE_SAVE_BUTTON).click()
    await second_request_started.wait()
    await modal.locator(".configure-inline-status", has_text="Waiting for Telegram owner verification...").wait_for(
        state="visible", timeout=5000
    )
    assert "iclaw-7qk2m9" in (await modal.text_content())
    assert "/start iclaw-7qk2m9" in (await modal.text_content())
    assert await modal.locator(".configure-verification-link").count() == 1
    await modal.locator(_CONFIGURE_SAVE_BUTTON).wait_for(state="hidden", timeout=5000)

    await page.locator(SEL["configure_overlay"]).click(position={"x": 1, "y": 1})
    assert await page.locator(SEL["configure_overlay"]).is_visible()

    allow_second_response.set()
    await page.locator(SEL["configure_overlay"]).wait_for(state="hidden", timeout=5000)

    phase["value"] = "active"
    await page.evaluate(
        """
        handleAuthCompleted({
          extension_name: 'telegram',
          success: true,
          message: "Configuration saved, Telegram owner verified, and 'telegram' activated. Hot-activated WASM channel",
        });
        """
    )

    await wait_for_toast(page, "Telegram owner verified")
    await card.locator(SEL["ext_active_label"]).wait_for(state="visible", timeout=5000)
    assert await card.locator(SEL["ext_pairing_label"]).count() == 0

    assert captured_setup_payloads == [
        {"secrets": {"telegram_bot_token": "123456789:ABCdefGhI"}, "fields": {}},
        {"secrets": {}, "fields": {}},
    ]
