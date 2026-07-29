"""Attested-signing review page: routing, the auth wall, and the fail-closed
clear-signing UX (attested-signing §D5).

WebHID does not exist in a headless browser, so `navigator.hid` is stubbed at
the browser-API seam — the same seam the DMK transport will bind to. That stub
is deliberately *present but empty*: a page that decides what to offer by
sniffing for `navigator.hid` would look fine here, and this suite would miss it.
The descriptor answer is what must gate the ceremony, and these tests assert it
does.

The properties pinned here are the ones a unit test cannot reach, because they
are about the assembled page in a real browser:

* the approved transaction hash reaches the DOM **complete** — no CSS or layout
  truncation of the value the human compares against their device screen;
* with no descriptor, the page renders blocked and there is **no sign control
  in the DOM at all** (not merely disabled);
* the route sits behind the auth wall.
"""

import json

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)

INTENT_ID = "01J0000000000000000000REVIEW"
# 32 bytes, so the rendered form is 64 hex characters.
APPROVED_TX_HASH = "5e" * 32


def _intent_body(state: str = "pending") -> dict:
    return {
        "intent_id": INTENT_ID,
        "state": state,
        "chain_id": "eip155:11155111",
        "approved_tx_hash": APPROVED_TX_HASH,
        # Far future so the page never renders as expired.
        "expires_at_ms": 4_102_444_800_000,
        "decoded_tx": {
            "chain": "evm",
            "nonce": 7,
            "gas_limit": 21000,
            "to": "0x" + "22" * 20,
        },
    }


async def _open_review_page(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    clear_signing: str = "unavailable",
    authenticated: bool = True,
):
    """Open `/review/{INTENT_ID}` with the two backend reads stubbed."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    # WebHID is absent headless. Provide the API surface so the page cannot be
    # accidentally correct by feature-detecting it away — the descriptor answer
    # must be what gates signing.
    await page.add_init_script(
        """
        Object.defineProperty(navigator, 'hid', {
            configurable: true,
            value: {
                getDevices: async () => [],
                requestDevice: async () => [],
                addEventListener: () => {},
                removeEventListener: () => {},
            },
        });
        """
    )

    async def fulfill_json(route, body, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def handle_session(route):
        await fulfill_json(
            route,
            {"tenant_id": "reborn-v2-e2e", "user_id": USER_ID, "capabilities": {}},
        )

    async def handle_detail(route):
        await fulfill_json(route, _intent_body())

    async def handle_signing_context(route):
        body = {"clear_signing": clear_signing}
        if clear_signing == "available":
            body["descriptor"] = {"context": {"contract": {}}, "display": {"formats": {}}}
        await fulfill_json(route, body)

    await page.route("**/api/webchat/v2/session", handle_session)
    # Order matters: the more specific signing-context route is registered
    # first so the detail glob cannot swallow it.
    await page.route(
        f"**/api/webchat/v2/intents/{INTENT_ID}/signing-context", handle_signing_context
    )
    await page.route(f"**/api/webchat/v2/intents/{INTENT_ID}", handle_detail)

    suffix = f"?token={REBORN_V2_AUTH_TOKEN}" if authenticated else ""
    await page.goto(f"{reborn_v2_server}/review/{INTENT_ID}{suffix}")
    return context, page


async def test_review_page_renders_the_whole_hash(reborn_v2_server, reborn_v2_browser):
    """The value the human compares against the device must reach the DOM intact.

    A unit test can prove the component returns the full string; only a browser
    can prove nothing between the component and the screen shortened it.
    """
    context, page = await _open_review_page(reborn_v2_server, reborn_v2_browser)
    try:
        hash_node = page.get_by_test_id("review-approved-tx-hash")
        await expect(hash_node).to_be_visible(timeout=15000)

        rendered = (await hash_node.inner_text()).strip()
        assert rendered == APPROVED_TX_HASH, (
            f"the hash must render complete; got {rendered!r}"
        )
        assert "…" not in rendered and "..." not in rendered

        # And it must not be truncated by layout either — the text the user can
        # actually read has to be the whole value.
        overflows = await hash_node.evaluate(
            "node => node.scrollWidth > node.clientWidth + 1"
        )
        assert not overflows, "the hash is visually clipped, so it cannot be compared"
    finally:
        await context.close()


async def test_no_descriptor_leaves_no_sign_control_in_the_dom(
    reborn_v2_server, reborn_v2_browser
):
    """Fail closed, assembled: blocked UX renders and the control is ABSENT.

    Absent rather than disabled — a disabled control is one patch from enabled
    and reads as 'almost allowed'.
    """
    context, page = await _open_review_page(
        reborn_v2_server, reborn_v2_browser, clear_signing="unavailable"
    )
    try:
        await expect(page.get_by_test_id("review-ceremony-blocked")).to_be_visible(
            timeout=15000
        )
        await expect(page.get_by_test_id("review-sign-action")).to_have_count(0)
    finally:
        await context.close()


async def test_an_available_descriptor_offers_the_device_path(
    reborn_v2_server, reborn_v2_browser
):
    """The ready branch reaches the DOM, so the blocked branch is a real decision
    and not simply the only path that was ever wired."""
    context, page = await _open_review_page(
        reborn_v2_server, reborn_v2_browser, clear_signing="available"
    )
    try:
        await expect(page.get_by_test_id("review-sign-action")).to_be_visible(
            timeout=15000
        )
        await expect(page.get_by_test_id("review-ceremony-blocked")).to_have_count(0)
    finally:
        await context.close()


async def test_the_review_route_is_behind_the_auth_wall(
    reborn_v2_server, reborn_v2_browser
):
    """The public link only redirects here; the page itself demands a session.

    Without one the app must not render transaction detail — the whole point of
    the token showing nothing.
    """
    context, page = await _open_review_page(
        reborn_v2_server, reborn_v2_browser, authenticated=False
    )
    try:
        await page.wait_for_selector(SEL_V2["login_token"], timeout=15000)
        await expect(page.get_by_test_id("review-approved-tx-hash")).to_have_count(0)
    finally:
        await context.close()
