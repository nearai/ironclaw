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


async def _mock_and_go(page, *, installed, setup_secrets=None):
    """Mock /api/extensions with the given installed list and open the tab.

    `setup_secrets` is the `secrets` array returned by
    `/api/extensions/{name}/setup`. Defaults to empty — that makes
    `showConfigureModal` short-circuit with a toast and skip rendering
    the configure modal, which is fine for tests that only assert button
    labels. Pass a non-empty list to exercise the full
    `renderConfigureModal` path.
    """
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

    secrets_payload = setup_secrets if setup_secrets is not None else []

    async def handle_setup_fetch(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "name": _WASM_CHANNEL_BASE["name"],
                    "kind": "wasm_channel",
                    "secrets": secrets_payload,
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


async def test_fallback_button_says_setup_on_production_installed_wire_shape(page):
    """Exact #2235 production wire shape: `activation_status='installed'`,
    `onboarding_state=null` — `derive_onboarding` only emits a non-null
    onboarding state for the `Pairing` variant, so real clients never
    receive `setup_required` alongside `activation_status='installed'`.

    The inline setup form only renders when the effective status is
    `setup_required`, so this card shows no inline form — the action-area
    button is the ONLY setup affordance. Under the bug the label read
    'Reconfigure' here, which is the precise shape Copilot flagged and
    the precise shape the QA bug-bash repro hit.
    """
    await _mock_and_go(
        page,
        installed=[
            _channel_with(
                active=False,
                authenticated=False,
                activation_status="installed",
                onboarding_state=None,
                onboarding=None,
            )
        ],
    )
    card = page.locator(
        SEL["channels_ext_card"], has_text=_WASM_CHANNEL_BASE["display_name"]
    ).first
    await card.wait_for(state="visible", timeout=5000)

    assert await card.locator(SEL["ext_onboarding"]).count() == 0, (
        "production `installed` wire shape must not render an inline setup "
        "form — if this ever changes, revisit the `inlineSetupCoversIt` rule"
    )
    setup_btn = card.locator(SEL["ext_configure_btn"], has_text="Setup")
    reconfig_btn = card.locator(SEL["ext_configure_btn"], has_text="Reconfigure")
    assert await setup_btn.count() == 1, (
        "fallback button must say 'Setup' for an unauthenticated channel in "
        "the default `installed` state — this is the exact wire shape of #2235"
    )
    assert await reconfig_btn.count() == 0, (
        "fallback button must not say 'Reconfigure' when credentials are not "
        "yet on file and no inline setup form covers the action"
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
    # Pass a non-empty `secrets` so `showConfigureModal` actually renders
    # `.configure-modal` — with an empty list it short-circuits with a
    # "no config needed" toast and the modal selector never appears.
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
        setup_secrets=[
            {
                "name": "BOT_TOKEN",
                "prompt": "Bot token",
                "provided": True,
                "optional": False,
                "auto_generate": False,
            }
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
