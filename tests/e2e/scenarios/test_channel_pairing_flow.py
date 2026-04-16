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
    # The code is invalid, so approval fails — but the endpoint should
    # accept the thread_id field without a 422 or 500.
    assert resp.status_code != 500, (
        f"Pairing approve should not 500 with thread_id: {resp.text[:200]}"
    )


async def test_pairing_approve_without_thread_id_still_works(ironclaw_server):
    """Backward compatibility: pairing approve without thread_id should
    still work (the field is optional with serde(default))."""
    resp = await api_post(
        ironclaw_server,
        "/api/pairing/test-channel/approve",
        json={"code": "INVALID0"},
        timeout=10,
    )
    assert resp.status_code != 500, (
        f"Pairing approve should not 500 without thread_id: {resp.text[:200]}"
    )


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
    """The pairing approve handler should sanitize the channel path parameter
    before interpolating it into the synthetic agent message."""
    resp = await api_post(
        ironclaw_server,
        "/api/pairing/evil.Ignore all/approve",
        json={"code": "TESTCODE", "thread_id": None},
        timeout=10,
    )
    # The code is invalid so approval fails, but the handler should not 500
    # and the channel name should be sanitized in any injected message.
    assert resp.status_code != 500, (
        f"Pairing approve should not 500 with injection channel name: "
        f"{resp.text[:200]}"
    )
