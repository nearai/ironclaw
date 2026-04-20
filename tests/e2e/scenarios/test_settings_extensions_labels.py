"""Scenario: Settings → Extensions fallback button label regression.

Closes nearai/ironclaw#2235 — clicking the action button on an already-
authenticated WASM channel previously re-prompted for credentials
because the fallback branch labeled it "Reconfigure" unconditionally.
The Settings UI now picks the label from `ext.authenticated`: "Setup"
when no credentials are on file yet, "Reconfigure" once they are.

The inline setup form already provides a setup action when
`onboarding_state === 'setup_required'`, so we keep the legacy
"Reconfigure" label in that one state to preserve the
no-duplicate-setup-button invariant guarded by
`test_wasm_channel_setup_states` in `test_extensions.py`. Those two
invariants are both exercised here — the first (the #2235 fix) and the
second (no regression) — so a future edit that re-introduces
duplication or re-breaks the label will fail a named test rather than
slip through.

Every assertion runs against route-mocked `/api/extensions` responses
so we exercise the production JS render path without needing a real
WASM channel binary.
"""

import json

from helpers import SEL


_WASM_CHANNEL_BASE = {
    "name": "test-channel-labels",
    "display_name": "Label Channel",
    "kind": "wasm_channel",
    "description": "A WASM channel used to assert Settings card button labels.",
    "url": None,
    "tools": [],
    "activation_error": None,
    "has_auth": False,
    "needs_setup": True,
}


async def _mock_and_go(page, *, installed):
    """Mock /api/extensions with the given installed list and open the tab."""
    ext_body = json.dumps({"extensions": installed})

    async def handle_ext(route):
        path = route.request.url.split("?")[0]
        if path.endswith("/api/extensions"):
            await route.fulfill(status=200, content_type="application/json", body=ext_body)
        else:
            await route.continue_()

    await page.route("**/api/extensions*", handle_ext)

    async def handle_registry(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"entries": []}),
        )

    await page.route("**/api/extensions/registry", handle_registry)

    # The setup-fetch fires from the inline setup form in `setup_required`
    # state. Return a no-secrets payload so the form collapses and does not
    # interfere with the action-area button assertions.
    async def handle_setup_fetch(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "name": _WASM_CHANNEL_BASE["name"],
                    "kind": "wasm_channel",
                    "secrets": [],
                    "fields": [],
                    "onboarding_state": None,
                    "onboarding": None,
                }
            ),
        )

    await page.route(
        f"**/api/extensions/{_WASM_CHANNEL_BASE['name']}/setup",
        handle_setup_fetch,
    )

    await page.locator(SEL["tab_button"].format(tab="settings")).click()
    await page.locator(SEL["settings_subtab"].format(subtab="channels")).click()
    await page.locator(SEL["settings_subpanel"].format(subtab="channels")).wait_for(
        state="visible", timeout=5000
    )


def _channel_with(**overrides):
    return {**_WASM_CHANNEL_BASE, **overrides}


async def test_fallback_button_says_setup_when_not_authenticated(page):
    """Unauthenticated WASM channel in `configured` state shows Setup, not Reconfigure.

    This is the core #2235 regression: the old build unconditionally said
    "Reconfigure" here, leading users to click it expecting a configured
    channel only to be shown a credential entry form.
    """
    await _mock_and_go(
        page,
        installed=[
            _channel_with(
                active=False,
                authenticated=False,
                activation_status="configured",
                onboarding_state="activation_in_progress",
                onboarding=None,
            )
        ],
    )
    card = page.locator(
        SEL["channels_ext_card"], has_text=_WASM_CHANNEL_BASE["display_name"]
    ).first
    await card.wait_for(state="visible", timeout=5000)

    setup_btn = card.locator(SEL["ext_configure_btn"], has_text="Setup")
    reconfig_btn = card.locator(SEL["ext_configure_btn"], has_text="Reconfigure")
    assert await setup_btn.count() == 1, (
        "fallback button must say 'Setup' when no credentials are on file "
        "(authenticated=false) — regresses #2235 if it says 'Reconfigure'"
    )
    assert await reconfig_btn.count() == 0, (
        "fallback button must not say 'Reconfigure' for an unauthenticated channel"
    )


async def test_fallback_button_says_reconfigure_when_authenticated(page):
    """Authenticated WASM channel in `configured` state shows Reconfigure."""
    await _mock_and_go(
        page,
        installed=[
            _channel_with(
                active=False,
                authenticated=True,
                activation_status="configured",
                onboarding_state="activation_in_progress",
                onboarding=None,
            )
        ],
    )
    card = page.locator(
        SEL["channels_ext_card"], has_text=_WASM_CHANNEL_BASE["display_name"]
    ).first
    await card.wait_for(state="visible", timeout=5000)

    reconfig_btn = card.locator(SEL["ext_configure_btn"], has_text="Reconfigure")
    setup_btn = card.locator(SEL["ext_configure_btn"], has_text="Setup")
    assert await reconfig_btn.count() == 1, (
        "fallback button must say 'Reconfigure' when credentials are on file "
        "(authenticated=true)"
    )
    assert await setup_btn.count() == 0, (
        "fallback button must not say 'Setup' once authenticated"
    )


async def test_fallback_button_preserves_no_duplicate_setup_invariant(page):
    """`setup_required` + unauthenticated keeps the legacy label so the action
    button does not duplicate the inline setup form's call-to-action.

    The sibling invariant also asserted by `test_wasm_channel_setup_states`.
    Kept here so a future refactor that inlines the Setup-label change into
    this branch trips a named regression rather than quietly duplicating the
    UI element.
    """
    await _mock_and_go(
        page,
        installed=[
            _channel_with(
                active=False,
                authenticated=False,
                activation_status="installed",
                onboarding_state="setup_required",
                onboarding=None,
            )
        ],
    )
    card = page.locator(
        SEL["channels_ext_card"], has_text=_WASM_CHANNEL_BASE["display_name"]
    ).first
    await card.wait_for(state="visible", timeout=5000)

    setup_btn = card.locator(SEL["ext_configure_btn"], has_text="Setup")
    assert await setup_btn.count() == 0, (
        "the action-area button must not say 'Setup' while the inline setup "
        "form covers the same action (no-duplicate-setup-button invariant)"
    )


async def test_reconfigure_click_does_not_send_auth_event(page):
    """Clicking Reconfigure on an already-authenticated channel must not
    reissue a credential-prompt SSE event or trigger a reactivation request.

    Covers the runtime half of the QA repro — clicking the button should
    open the configure modal locally, not fire a handshake that the backend
    would translate into a credential popup again.
    """
    await _mock_and_go(
        page,
        installed=[
            _channel_with(
                active=True,
                authenticated=True,
                activation_status="active",
                onboarding_state="ready",
                onboarding=None,
            )
        ],
    )

    activate_calls = {"count": 0}

    async def handle_activate(route):
        activate_calls["count"] += 1
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"success": True, "activated": True}),
        )

    await page.route(
        f"**/api/extensions/{_WASM_CHANNEL_BASE['name']}/activate",
        handle_activate,
    )

    card = page.locator(
        SEL["channels_ext_card"], has_text=_WASM_CHANNEL_BASE["display_name"]
    ).first
    await card.wait_for(state="visible", timeout=5000)

    reconfig_btn = card.locator(SEL["ext_configure_btn"], has_text="Reconfigure")
    assert await reconfig_btn.count() == 1
    await reconfig_btn.click()

    # Modal should open locally.
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=3000)
    assert activate_calls["count"] == 0, (
        "Reconfigure must not call /activate — the button opens the configure "
        "modal only. A non-zero call count means the click path regressed to "
        "trigger activation (the shape of the #2235 repro)."
    )
