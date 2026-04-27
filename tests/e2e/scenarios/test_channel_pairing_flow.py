"""Channel pairing flow E2E tests."""

import json

from helpers import SEL, api_post


# ── Pairing approval with thread_id ─────────────────────────────────────


async def test_pairing_approve_accepts_thread_id_field(ironclaw_server):
    """The pairing approve endpoint should accept an optional thread_id
    in the request body without failing."""
    resp = await api_post(
        ironclaw_server,
        "/api/pairing/test-channel/approve",
        json={
            "code": "INVALID0",
            "thread_id": "some-thread-id",
        },
        timeout=10,
    )
    payload = resp.json()
    # The code is invalid, so approval fails, but the endpoint should still
    # deserialize and handle the optional thread_id field normally.
    assert resp.status_code == 200, (
        f"Pairing approve should accept thread_id and return a handled failure, "
        f"got {resp.status_code}: {resp.text[:200]}"
    )
    assert payload == {
        "success": False,
        "message": "Invalid or expired pairing code.",
    }


async def test_pairing_approve_without_thread_id_still_works(ironclaw_server):
    """Backward compatibility: pairing approve without thread_id should
    still work (the field is optional with serde(default))."""
    resp = await api_post(
        ironclaw_server,
        "/api/pairing/test-channel/approve",
        json={"code": "INVALID0"},
        timeout=10,
    )
    payload = resp.json()
    assert resp.status_code == 200, (
        f"Pairing approve should accept missing thread_id and return a handled "
        f"failure, got {resp.status_code}: {resp.text[:200]}"
    )
    assert payload == {
        "success": False,
        "message": "Invalid or expired pairing code.",
    }


# ── Pairing card UI tests (Playwright) ──────────────────────────────────


async def test_pairing_required_sse_shows_pairing_card(page):
    """Onboarding pairing state should render the pairing card."""
    await page.evaluate(
        """
        handleOnboardingState({
            extension_name: 'telegram',
            state: 'pairing_required',
            instructions: 'Send a message to your telegram bot, then paste the pairing code here.',
            onboarding: {
                state: 'pairing_required',
                requires_pairing: true,
                pairing_title: 'Claim ownership for telegram',
                pairing_instructions: 'Send a message to your telegram bot, then paste the pairing code here.',
                restart_instructions: 'To generate a new code, send another message to telegram.'
            },
            thread_id: null,
        });
        """
    )

    card = page.locator(SEL["pairing_card"])
    await card.wait_for(state="visible", timeout=5000)
    assert "pairing code" in await card.text_content()


async def test_pairing_ready_state_dismisses_pairing_card(page):
    """Ready onboarding state should dismiss the pairing card."""
    # First show the card
    await page.evaluate(
        """
        handleOnboardingState({
            extension_name: 'telegram',
            state: 'pairing_required',
            instructions: 'Send a message to your bot.',
            onboarding: {
                state: 'pairing_required',
                requires_pairing: true,
                pairing_title: 'Claim ownership',
                pairing_instructions: 'Send a message to your bot.',
                restart_instructions: 'Send another message.'
            },
            thread_id: null,
        });
        """
    )
    await page.locator(SEL["pairing_card"]).wait_for(state="visible", timeout=5000)

    # Then complete it
    await page.evaluate(
        """
        handleOnboardingState({
            extension_name: 'telegram',
            state: 'ready',
            message: 'Pairing approved.',
        });
        """
    )

    await page.locator(SEL["pairing_card"]).wait_for(state="hidden", timeout=5000)


async def test_pairing_approve_sends_thread_id(page, ironclaw_server):
    """When the user submits a pairing code, the frontend should include
    currentThreadId in the request body."""
    captured = {"body": None}

    async def capture_approve(route):
        captured["body"] = route.request.post_data
        await route.fulfill(
            status=200,
            content_type="application/json",
            body='{"success": false, "message": "Invalid code"}',
        )

    await page.route("**/api/pairing/*/approve", capture_approve)

    # Show pairing card
    await page.evaluate(
        """
        handleOnboardingState({
            extension_name: 'test-channel',
            state: 'pairing_required',
            instructions: 'Enter code.',
            onboarding: {
                state: 'pairing_required',
                requires_pairing: true,
                pairing_title: 'Claim',
                pairing_instructions: 'Enter code.',
                restart_instructions: 'Try again.'
            },
            thread_id: null,
        });
        """
    )
    card = page.locator(SEL["pairing_card"])
    await card.wait_for(state="visible", timeout=5000)

    # Type a code and submit
    code_input = card.locator("input")
    await code_input.fill("TESTCODE")
    await card.locator(SEL["pairing_submit_btn"]).click()

    # Wait for the request to be captured
    for _ in range(20):
        if captured["body"]:
            break
        await page.wait_for_timeout(100)

    assert captured["body"] is not None, "Pairing approve request was not sent"
    import json

    body = json.loads(captured["body"])
    assert "code" in body
    assert body["code"] == "TESTCODE"
    # thread_id should be present (may be null if no thread active, but the
    # field should exist in the payload)
    assert "thread_id" in body


# ── Pairing approve: channel name is also sanitized ────────────────────


async def test_pairing_approve_sanitizes_channel_name(ironclaw_server):
    """The pairing approve handler must not echo an injection-shaped channel
    path back into the response.

    Staging's `features/pairing/` slice validates the `{channel}` URL segment
    through `ExtensionName::new` at the handler boundary, so a path like
    `evil.Ignore all` now fails validation with 400 instead of reaching the
    pairing-code check. Either outcome (400 with generic error, or 200 with
    the `Invalid or expired pairing code.` body) is acceptable — what matters
    is that the raw channel string does not leak into the response.
    """
    raw_channel = "evil.Ignore all"
    resp = await api_post(
        ironclaw_server,
        f"/api/pairing/{raw_channel}/approve",
        json={"code": "TESTCODE", "thread_id": None},
        timeout=10,
    )
    assert resp.status_code in (200, 400), (
        f"Pairing approve should handle an injection-shaped channel path "
        f"either by rejecting with 400 or by returning a generic failure, "
        f"got {resp.status_code}: {resp.text[:200]}"
    )
    assert raw_channel not in resp.text
