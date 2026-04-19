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
    "onboarding_state": "setup_required",
    "onboarding": {
        "state": "setup_required",
        "requires_pairing": True,
        "credential_title": "Configure credentials for Telegram",
        "credential_instructions": "Enter your Telegram Bot API token from @BotFather. After you save it, IronClaw will start the bot in polling mode and wait for you to claim ownership.",
        "credential_next_step": "Next: open your Telegram bot, send it any message, wait for the pairing code reply, then paste that code into IronClaw.",
        "setup_url": "https://t.me/BotFather",
        "pairing_title": "Claim ownership for Telegram",
        "pairing_instructions": "Open your Telegram bot, send it any message such as hi or /start, wait for the pairing code reply, then paste that code into IronClaw. Telegram bots cannot message you first.",
        "restart_instructions": "If you close this claim step, send another message in the channel to get a new pairing code.",
    },
}

_TELEGRAM_ACTIVE = {
    **_TELEGRAM_INSTALLED,
    "active": True,
    "authenticated": True,
    "needs_setup": False,
    "activation_status": "active",
    "onboarding_state": "ready",
    "onboarding": {**_TELEGRAM_INSTALLED["onboarding"], "state": "ready"},
}

_TELEGRAM_PAIRING = {
    **_TELEGRAM_ACTIVE,
    "activation_status": "pairing",
    "onboarding_state": "pairing_required",
    "onboarding": {**_TELEGRAM_INSTALLED["onboarding"], "state": "pairing_required"},
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


async def _active_thread_id(page):
    await page.wait_for_function("() => !!currentThreadId", timeout=10000)
    return await page.evaluate("() => currentThreadId")


async def test_telegram_setup_card_shows_bot_token_field(page):
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
                    "name": "telegram",
                    "kind": "wasm_channel",
                    "secrets": [
                        {
                            "name": "telegram_bot_token",
                            "prompt": "Enter your Telegram Bot API token (from @BotFather)",
                            "provided": False,
                            "optional": False,
                            "auto_generate": False,
                        }
                    ],
                    "fields": [],
                    "onboarding_state": "setup_required",
                    "onboarding": _TELEGRAM_INSTALLED["onboarding"],
                }
            ),
        )

    await mock_extension_lists(page, handle_ext_list)
    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await go_to_channels(page)

    card = page.locator(SEL["channels_ext_card"], has_text="Telegram")
    await card.locator(SEL["ext_onboarding"]).wait_for(state="visible", timeout=5000)
    assert "Telegram Bot API token" in await card.locator(SEL["ext_onboarding"]).text_content()
    assert "pairing code" in await card.locator(SEL["ext_onboarding"]).text_content()
    assert await card.locator(SEL["setup_input"]).count() == 1
    assert await card.locator(SEL["setup_next_step"]).count() == 1
    link = card.locator(SEL["ext_onboarding"]).locator("a", has_text="Get your token")
    assert await link.count() == 1


async def test_telegram_hot_activation_transitions_installed_to_pairing(page):
    phase = {"value": "installed"}
    captured_setup_payloads = []
    thread_id = await _active_thread_id(page)

    async def handle_ext_list(route):
        extensions = {
            "installed": [_TELEGRAM_INSTALLED],
            "pairing": [_TELEGRAM_PAIRING],
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
                        "name": "telegram",
                        "kind": "wasm_channel",
                        "secrets": [
                            {
                                "name": "telegram_bot_token",
                                "prompt": "Enter your Telegram Bot API token (from @BotFather)",
                                "provided": False,
                                "optional": False,
                                "auto_generate": False,
                            }
                        ],
                        "fields": [],
                        "onboarding_state": "setup_required",
                        "onboarding": _TELEGRAM_INSTALLED["onboarding"],
                    }
                ),
            )
            return

        payload = json.loads(route.request.post_data or "{}")
        captured_setup_payloads.append(payload)
        phase["value"] = "pairing"
        await asyncio.sleep(0.05)
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "success": True,
                    "activated": True,
                    "message": "Configuration saved and 'telegram' activated. Credentials are saved, but ownership is still required before the channel is ready. Open your Telegram bot, send it any message such as hi or /start, wait for the pairing code reply, then paste that code into IronClaw. Telegram bots cannot message you first.",
                    "onboarding_state": "pairing_required",
                    "onboarding": _TELEGRAM_PAIRING["onboarding"],
                }
            ),
        )

    await mock_extension_lists(page, handle_ext_list)
    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await go_to_channels(page)

    card = page.locator(SEL["channels_ext_card"], has_text="Telegram")
    input_el = card.locator(SEL["setup_input"])
    await input_el.wait_for(state="visible", timeout=5000)
    await input_el.fill("123456789:ABCdefGhI")
    await card.locator(".ext-onboarding .btn-ext.activate", has_text="Save").click()

    await page.evaluate(
        """(threadId) => {
          handleOnboardingState({
            extension_name: 'telegram',
            state: 'pairing_required',
            request_id: 'req-telegram-hot-activation',
            thread_id: threadId,
            instructions: 'Open your Telegram bot, send it any message, then paste the pairing code here.',
            onboarding: {
              state: 'pairing_required',
              requires_pairing: true,
              pairing_title: 'Claim ownership for Telegram',
              pairing_instructions: 'Open your Telegram bot, send it any message, then paste the pairing code here.',
              restart_instructions: 'If you close this claim step, send another message in the channel to get a new pairing code.'
            }
          });
        }""",
        thread_id,
    )

    await page.locator(SEL["pairing_card"]).wait_for(state="attached", timeout=5000)
    await card.locator(SEL["ext_pairing_label"]).wait_for(state="visible", timeout=5000)
    assert await card.locator(SEL["pairing_help"]).count() >= 1
    assert await page.locator(SEL["pairing_restart"]).count() >= 1

    assert captured_setup_payloads == [
        {"secrets": {"telegram_bot_token": "123456789:ABCdefGhI"}, "fields": {}}
    ]


async def test_telegram_auth_required_shows_configure_modal_and_can_cancel(page):
    setup_hits = {"count": 0}
    cancel_bodies = []

    async def handle_setup(route):
        setup_hits["count"] += 1
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "name": "telegram",
                    "kind": "wasm_channel",
                    "secrets": [
                        {
                            "name": "telegram_bot_token",
                            "prompt": "Enter your Telegram Bot API token (from @BotFather)",
                            "provided": False,
                            "optional": False,
                            "auto_generate": False,
                        }
                    ],
                    "fields": [],
                    "onboarding_state": "setup_required",
                    "onboarding": _TELEGRAM_INSTALLED["onboarding"],
                }
            ),
        )

    async def handle_cancel(route):
        cancel_bodies.append(json.loads(route.request.post_data or "{}"))
        await route.fulfill(status=200, content_type="application/json", body="{}")

    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await page.route("**/api/chat/gate/resolve", handle_cancel)

    await page.evaluate(
        """
        handleOnboardingState({
          extension_name: 'telegram',
          state: 'auth_required',
          instructions: 'Enter your Telegram Bot API token (from @BotFather)',
          auth_url: null,
          request_id: 'req-telegram-auth',
        });
        """
    )

    # Auth-required now opens the unified configure modal, not the old setup card
    modal = page.locator(SEL["configure_overlay"])
    await modal.wait_for(state="visible", timeout=5000)
    assert "Telegram Bot API token" in await modal.text_content()
    assert await modal.locator(SEL["configure_input"]).count() == 1
    assert await page.locator(SEL["setup_card"]).count() == 0

    await modal.locator(".btn-ext.remove").click()
    await modal.wait_for(state="hidden", timeout=5000)
    assert cancel_bodies[0]["request_id"] == "req-telegram-auth"
    assert cancel_bodies[0]["resolution"] == "cancelled"
    assert cancel_bodies[0].get("thread_id")

    await page.evaluate(
        """
        handleOnboardingState({
          extension_name: 'telegram',
          state: 'auth_required',
          instructions: 'Enter your Telegram Bot API token (from @BotFather)',
          auth_url: null,
          request_id: 'req-telegram-auth-2',
        });
        """
    )
    await page.locator(SEL["configure_overlay"]).wait_for(state="visible", timeout=5000)
    assert setup_hits["count"] == 2


async def test_telegram_auth_gate_uses_extension_name_for_configure_modal(page):
    setup_hits = {"count": 0}

    async def handle_setup(route):
        setup_hits["count"] += 1
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "name": "telegram",
                    "kind": "wasm_channel",
                    "secrets": [
                        {
                            "name": "telegram_bot_token",
                            "prompt": "Enter your Telegram Bot API token (from @BotFather)",
                            "provided": False,
                            "optional": False,
                            "auto_generate": False,
                        },
                        {
                            "name": "webhook_secret",
                            "prompt": "Webhook secret (leave empty to auto-generate)",
                            "provided": False,
                            "optional": True,
                            "auto_generate": True,
                        },
                    ],
                    "fields": [],
                    "onboarding_state": "setup_required",
                    "onboarding": _TELEGRAM_INSTALLED["onboarding"],
                }
            ),
        )

    await page.route("**/api/extensions/telegram/setup", handle_setup)

    await page.evaluate(
        """
        handleGateRequired({
          request_id: 'req-telegram-gate',
          gate_name: 'authentication',
          tool_name: 'telegram',
          description: 'Tool telegram requires authentication',
          parameters: '{}',
          extension_name: 'telegram',
          resume_kind: {
            Authentication: {
              credential_name: 'telegram_bot_token',
              instructions: 'Enter your Telegram Bot API token (from @BotFather)',
              auth_url: null
            }
          },
          thread_id: null,
        });
        """
    )

    modal = page.locator(SEL["configure_overlay"])
    await modal.wait_for(state="visible", timeout=5000)
    assert "Telegram Bot API token" in await modal.text_content()
    assert "Webhook secret" in await modal.text_content()
    assert await page.locator(SEL["auth_card"]).count() == 0
    assert setup_hits["count"] == 1


async def test_telegram_configure_modal_submit_then_cancel_pairing_and_restart(page):
    setup_payloads = []
    pairing_request_id = "req-telegram-pairing"
    thread_id = await _active_thread_id(page)

    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    {
                        "name": "telegram",
                        "kind": "wasm_channel",
                        "secrets": [
                            {
                                "name": "telegram_bot_token",
                                "prompt": "Enter your Telegram Bot API token (from @BotFather)",
                                "provided": False,
                                "optional": False,
                                "auto_generate": False,
                            }
                        ],
                        "fields": [],
                        "onboarding_state": "setup_required",
                        "onboarding": _TELEGRAM_INSTALLED["onboarding"],
                    }
                ),
            )
            return

        setup_payloads.append(json.loads(route.request.post_data or "{}"))
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "success": True,
                    "activated": True,
                    "message": "Configuration saved and 'telegram' activated. Credentials are saved, but ownership is still required before the channel is ready.",
                    "onboarding_state": "pairing_required",
                    "onboarding": _TELEGRAM_PAIRING["onboarding"],
                }
            ),
        )

    await page.route("**/api/extensions/telegram/setup", handle_setup)

    await page.evaluate(
        """
        handleOnboardingState({
          extension_name: 'telegram',
          state: 'auth_required',
          instructions: 'Enter your Telegram Bot API token (from @BotFather)',
          auth_url: null,
          request_id: 'req-telegram-configure',
        });
        """
    )

    # Auth-required now opens the unified configure modal
    modal = page.locator(SEL["configure_overlay"])
    await modal.wait_for(state="visible", timeout=5000)
    await modal.locator(SEL["configure_input"]).fill("123456789:ABCdefGhI")
    await modal.locator(".btn-ext.activate").click()
    await modal.wait_for(state="hidden", timeout=5000)

    await page.evaluate(
        """({ threadId, requestId }) => {
          handleOnboardingState({
            extension_name: 'telegram',
            state: 'pairing_required',
            request_id: requestId,
            thread_id: threadId,
            instructions: 'Open your Telegram bot, send it any message such as hi or /start, wait for the pairing code reply, then paste that code here.',
            onboarding: {
              state: 'pairing_required',
              requires_pairing: true,
              pairing_title: 'Claim ownership for Telegram',
              pairing_instructions: 'Open your Telegram bot, send it any message such as hi or /start, wait for the pairing code reply, then paste that code into IronClaw. Telegram bots cannot message you first.',
              restart_instructions: 'If you close this claim step, send another message in the channel to get a new pairing code.'
            }
          });
        }""",
        {"threadId": thread_id, "requestId": pairing_request_id},
    )

    pairing_card = page.locator(SEL["pairing_card"])
    await pairing_card.wait_for(state="visible", timeout=5000)
    assert "pairing code" in await pairing_card.text_content()
    assert await pairing_card.locator(SEL["pairing_restart"]).count() == 1
    assert await pairing_card.get_attribute("data-request-id") == pairing_request_id

    await pairing_card.locator(SEL["pairing_cancel_btn"]).click()
    await pairing_card.wait_for(state="hidden", timeout=5000)
    await wait_for_toast(page, "send another message in the channel to get a new pairing code")

    await page.evaluate(
        """
        handleOnboardingState({
          extension_name: 'telegram',
          state: 'pairing_required',
          instructions: 'Open your Telegram bot, send it any message such as hi or /start, wait for the pairing code reply, then paste that code here.',
          onboarding: {
            state: 'pairing_required',
            requires_pairing: true,
            pairing_title: 'Claim ownership for Telegram',
            pairing_instructions: 'Open your Telegram bot, send it any message such as hi or /start, wait for the pairing code reply, then paste that code into IronClaw. Telegram bots cannot message you first.',
            restart_instructions: 'If you close this claim step, send another message in the channel to get a new pairing code.'
          }
        });
        """
    )
    await page.locator(SEL["pairing_card"]).wait_for(state="visible", timeout=5000)
    assert setup_payloads == [
        {
            "request_id": "req-telegram-configure",
            "thread_id": thread_id,
            "secrets": {"telegram_bot_token": "123456789:ABCdefGhI"},
            "fields": {},
        }
    ]


async def test_telegram_pairing_sse_request_id_wins_over_setup_response(page):
    captured_pairing_payloads = []
    thread_id = await _active_thread_id(page)
    pairing_request_id = "req-telegram-pairing-sse"

    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    {
                        "name": "telegram",
                        "kind": "wasm_channel",
                        "secrets": [
                            {
                                "name": "telegram_bot_token",
                                "prompt": "Enter your Telegram Bot API token (from @BotFather)",
                                "provided": False,
                                "optional": False,
                                "auto_generate": False,
                            }
                        ],
                        "fields": [],
                        "onboarding_state": "setup_required",
                        "onboarding": _TELEGRAM_INSTALLED["onboarding"],
                    }
                ),
            )
            return

        await page.evaluate(
            """({ threadId, requestId }) => {
              handleOnboardingState({
                extension_name: 'telegram',
                state: 'pairing_required',
                request_id: requestId,
                thread_id: threadId,
                instructions: 'Open your Telegram bot and enter the pairing code.',
                onboarding: {
                  state: 'pairing_required',
                  requires_pairing: true,
                  pairing_title: 'Claim ownership for Telegram',
                  pairing_instructions: 'Open your Telegram bot and enter the pairing code.',
                  restart_instructions: 'Send another Telegram message to get a new code.'
                }
              });
            }""",
            {"threadId": thread_id, "requestId": pairing_request_id},
        )
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "success": True,
                    "activated": True,
                    "message": "Configuration saved and 'telegram' activated.",
                    "onboarding_state": "pairing_required",
                    "onboarding": _TELEGRAM_PAIRING["onboarding"],
                }
            ),
        )

    async def handle_pairing_approve(route):
        captured_pairing_payloads.append(json.loads(route.request.post_data or "{}"))
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"success": True, "message": "Pairing approved."}),
        )

    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await page.route("**/api/pairing/telegram/approve", handle_pairing_approve)

    await page.evaluate(
        """(threadId) => {
          handleOnboardingState({
            extension_name: 'telegram',
            state: 'auth_required',
            instructions: 'Enter your Telegram Bot API token (from @BotFather)',
            auth_url: null,
            request_id: 'req-telegram-configure-race',
            thread_id: threadId,
          });
        }""",
        thread_id,
    )

    modal = page.locator(SEL["configure_overlay"])
    await modal.wait_for(state="visible", timeout=5000)
    await modal.locator(SEL["configure_input"]).fill("123456789:ABCdefGhI")
    await modal.locator(".btn-ext.activate").click()
    await modal.wait_for(state="hidden", timeout=5000)

    pairing_card = page.locator(SEL["pairing_card"])
    await pairing_card.wait_for(state="visible", timeout=5000)
    assert await pairing_card.get_attribute("data-request-id") == pairing_request_id

    await pairing_card.locator(SEL["auth_token_input"]).fill("PAIR-1234")
    await pairing_card.locator(SEL["pairing_submit_btn"]).click()
    await pairing_card.wait_for(state="hidden", timeout=5000)

    assert captured_pairing_payloads == [
        {
            "code": "PAIR-1234",
            "thread_id": thread_id,
            "request_id": pairing_request_id,
        }
    ]
