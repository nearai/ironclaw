"""Legacy channel pairing UI coverage ported to Reborn WebUI v2."""

import json

import pytest
from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture dependency
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture dependency
)

pytestmark = pytest.mark.skip(
    reason=(
        "Current main routes channel setup through Extensions instead of chat; "
        "reactivate with nearai/ironclaw#5362."
    )
)


async def test_reborn_legacy_generic_connect_command_redeems_pairing_code(
    reborn_v2_page,
):
    page = reborn_v2_page
    redeem_requests: list[dict] = []
    blocked_message_sends: list[str] = []

    async def handle_connectable(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "channels": [
                        {
                            "channel": "telegram",
                            "display_name": "Telegram",
                            "strategy": "inbound_proof_code",
                            "command_aliases": ["telegram", "telegram account"],
                            "action": {
                                "title": "Claim your Telegram account",
                                "instructions": "Paste the proof code from Telegram.",
                                "input_placeholder": "Telegram proof code",
                                "submit_label": "Connect Telegram",
                                "success_message": "Telegram account connected.",
                                "error_message": "Telegram pairing failed.",
                            },
                        }
                    ]
                }
            ),
        )

    async def handle_redeem(route):
        redeem_requests.append(json.loads(route.request.post_data or "{}"))
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "provider": "telegram",
                    "provider_user_id": "123456789",
                }
            ),
        )

    async def block_message_send(route):
        blocked_message_sends.append(route.request.url)
        await route.fulfill(
            status=500,
            content_type="application/json",
            body=json.dumps({"error": "connect command should not send a chat message"}),
        )

    await page.route("**/api/webchat/v2/channels/connectable", handle_connectable)
    await page.route("**/api/webchat/v2/extensions/pairing/redeem", handle_redeem)
    await page.route("**/api/webchat/v2/threads/*/messages", block_message_send)

    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill("connect telegram")
    await composer.press("Enter")

    card = page.locator(
        SEL_V2["channel_connect_card_for"].format(
            channel="telegram",
            strategy="inbound_proof_code",
        )
    )
    await expect(card).to_be_visible(timeout=10000)
    await expect(card).to_contain_text("Connect Telegram")
    await expect(card).to_contain_text("Claim your Telegram account")

    section = card.locator(SEL_V2["pairing_section"])
    await expect(section).to_be_visible(timeout=5000)
    input_field = section.locator(SEL_V2["pairing_code_input"])
    await input_field.fill("  pair-2468  ")
    await section.locator(SEL_V2["pairing_submit"]).click()

    await expect(section.locator(SEL_V2["pairing_success"])).to_contain_text(
        "Telegram account connected.", timeout=5000
    )
    await expect(input_field).to_have_value("")
    assert redeem_requests == [{"channel": "telegram", "code": "PAIR-2468"}]
    assert blocked_message_sends == []
