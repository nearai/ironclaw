"""Legacy channel pairing UI coverage ported to Reborn WebUI v2."""

import json

from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture dependency
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture dependency
)


async def test_reborn_legacy_slack_connect_command_renders_pairing_card_and_redeems_code(
    reborn_v2_page,
):
    """A Slack connect command opens Reborn's pairing card instead of sending a chat turn."""
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
                            "channel": "slack",
                            "display_name": "Slack",
                            "strategy": "inbound_proof_code",
                            "command_aliases": ["slack", "slack account", "slack pairing"],
                            "action": {
                                "title": "Claim your Slack account",
                                "instructions": "Paste the proof code from Slack.",
                                "input_placeholder": "Slack proof code",
                                "submit_label": "Connect Slack",
                                "success_message": "Slack account connected.",
                                "error_message": "Slack pairing failed.",
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
                    "provider": "slack",
                    "provider_user_id": "U123PAIR",
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

    user_count_before = await page.locator(SEL_V2["msg_user"]).count()
    assistant_count_before = await page.locator(SEL_V2["msg_assistant"]).count()

    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill("connect slack")
    await composer.press("Enter")

    card = page.locator(
        SEL_V2["channel_connect_card_for"].format(
            channel="slack",
            strategy="inbound_proof_code",
        )
    )
    await expect(card).to_be_visible(timeout=10000)
    await expect(card).to_contain_text("Connect Slack")
    await expect(card.locator(SEL_V2["slack_pairing_section"])).to_be_visible()
    await expect(card).to_contain_text("Claim your Slack account")

    await card.locator(SEL_V2["slack_pairing_code_input"]).fill("  PAIR1234  ")
    await card.locator(SEL_V2["slack_pairing_submit"]).click()
    await expect(card.locator(SEL_V2["slack_pairing_success"])).to_contain_text(
        "Slack account connected.", timeout=5000
    )

    assert redeem_requests == [{"channel": "slack", "code": "PAIR1234"}]
    assert blocked_message_sends == []
    assert await page.locator(SEL_V2["msg_user"]).count() == user_count_before
    assert await page.locator(SEL_V2["msg_assistant"]).count() == assistant_count_before

    await card.locator(SEL_V2["channel_connect_dismiss"]).click()
    await expect(card).to_be_hidden(timeout=5000)
