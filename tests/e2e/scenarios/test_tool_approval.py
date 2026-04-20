"""Scenario 6: Tool approval overlay UI behavior."""

import asyncio
import json

from helpers import (
    AUTH_TOKEN,
    SEL,
    api_get,
    api_post,
    ensure_writable_chat_input,
    send_chat_and_wait_for_terminal_message,
)


INJECT_APPROVAL_JS = """
(data) => {
    // Simulate an approval_needed SSE event by calling showApproval directly
    showApproval(data);
}
"""


async def _create_thread(base_url: str) -> str:
    response = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    assert response.status_code == 200, response.text
    return response.json()["id"]


async def _send_chat_message(base_url: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code == 202, response.text


async def _wait_for_history(
    base_url: str,
    thread_id: str,
    *,
    expect_pending: bool | None = None,
    response_fragment: str | None = None,
    turn_count_at_least: int | None = None,
    timeout: float = 20.0,
) -> dict:
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=10,
        )
        assert response.status_code == 200, response.text
        history = response.json()
        pending = history.get("pending_gate")
        turns = history.get("turns", [])
        latest_response = turns[-1].get("response") if turns else None

        pending_ok = expect_pending is None or bool(pending) == expect_pending
        response_ok = response_fragment is None or (
            latest_response is not None and response_fragment in latest_response
        )
        turns_ok = turn_count_at_least is None or len(turns) >= turn_count_at_least
        if pending_ok and response_ok and turns_ok:
            return history

        await asyncio.sleep(0.25)

    raise AssertionError(
        f"Timed out waiting for history state: expect_pending={expect_pending}, "
        f"response_fragment={response_fragment!r}"
    )


async def test_approval_card_appears(page):
    """Injecting an approval event should show the approval card."""
    # Inject a fake approval_needed event
    await page.evaluate("""
        showApproval({
            request_id: 'test-req-001',
            thread_id: currentThreadId,
            tool_name: 'shell',
            description: 'Execute: echo hello world',
            parameters: '{"command": "echo hello world"}'
        })
    """)

    # Verify the approval card appeared
    card = page.locator(SEL["approval_card"])
    await card.wait_for(state="visible", timeout=5000)

    # Check card contents
    header = card.locator(SEL["approval_header"].replace(".approval-card ", ""))
    assert await header.text_content() == "Tool requires approval"

    tool_name = card.locator(".approval-tool-name")
    assert await tool_name.text_content() == "shell"

    desc = card.locator(".approval-description")
    assert "echo hello world" in await desc.text_content()

    # Verify all three buttons exist
    assert await card.locator("button.approve").count() == 1
    assert await card.locator("button.always").count() == 1
    assert await card.locator("button.deny").count() == 1


async def test_approval_approve_disables_buttons(page):
    """Clicking Approve should disable all buttons and show status."""
    # Inject approval card
    await page.evaluate("""
        showApproval({
            request_id: 'test-req-002',
            thread_id: currentThreadId,
            tool_name: 'http',
            description: 'GET https://example.com',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-req-002"]')
    await card.wait_for(state="visible", timeout=5000)

    # Click Approve
    await card.locator("button.approve").click()

    # Buttons should be disabled
    await page.wait_for_timeout(500)
    buttons = card.locator(".approval-actions button")
    count = await buttons.count()
    for i in range(count):
        is_disabled = await buttons.nth(i).is_disabled()
        assert is_disabled, f"Button {i} should be disabled after approval"

    # Resolved status should show
    resolved = card.locator(".approval-resolved")
    assert await resolved.text_content() == "Approved"


async def test_approval_deny_shows_denied(page):
    """Clicking Deny should show 'Denied' status."""
    await page.evaluate("""
        showApproval({
            request_id: 'test-req-003',
            thread_id: currentThreadId,
            tool_name: 'write_file',
            description: 'Write to /tmp/test.txt',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-req-003"]')
    await card.wait_for(state="visible", timeout=5000)

    # Click Deny
    await card.locator("button.deny").click()

    await page.wait_for_timeout(500)
    resolved = card.locator(".approval-resolved")
    assert await resolved.text_content() == "Denied"


async def test_approval_params_toggle(page):
    """Parameters toggle should show/hide the parameter details."""
    await page.evaluate("""
        showApproval({
            request_id: 'test-req-004',
            thread_id: currentThreadId,
            tool_name: 'shell',
            description: 'Run command',
            parameters: '{"command": "ls -la /tmp"}'
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-req-004"]')
    await card.wait_for(state="visible", timeout=5000)

    # Parameters should be hidden initially
    params = card.locator(".approval-params")
    assert await params.is_hidden(), "Parameters should be hidden initially"

    # Click toggle to show
    toggle = card.locator(".approval-params-toggle")
    await toggle.click()
    await page.wait_for_timeout(300)

    assert await params.is_visible(), "Parameters should be visible after toggle"
    text = await params.text_content()
    assert "ls -la /tmp" in text

    # Click toggle again to hide
    await toggle.click()
    await page.wait_for_timeout(300)
    assert await params.is_hidden(), "Parameters should be hidden after second toggle"


async def test_waiting_for_approval_message_no_error_prefix(page):
    """Verify that input submitted while awaiting approval shows non-error status with tool context.

    Trigger a real approval-needed tool call, then attempt to send another message while
    approval is pending. The backend should reject the second input with a non-error
    status that includes the pending tool context.
    """
    assistant_messages = page.locator(SEL["message_assistant"])
    chat_input = await ensure_writable_chat_input(page)

    # Trigger a real HTTP tool call that pauses for approval in the default E2E harness.
    await chat_input.fill("make approval post approval-required")
    await chat_input.press("Enter")

    card = page.locator(SEL["approval_card"]).last
    await card.wait_for(state="visible", timeout=10000)

    tool_name = await card.locator(".approval-tool-name").text_content()
    desc_text = await card.locator(".approval-description").text_content()
    assert tool_name == "http"
    assert desc_text is not None and "HTTP requests to external APIs" in desc_text

    # With the thread now genuinely awaiting approval, the next message should be rejected
    # as a non-error pending status.
    initial_count = await assistant_messages.count()
    await chat_input.fill("send another message now")
    await chat_input.press("Enter")

    await page.wait_for_function(
        f"() => document.querySelectorAll('{SEL['message_assistant']}').length > {initial_count}",
        timeout=10000,
    )

    last_msg = assistant_messages.last.locator(".message-content")
    msg_text = await last_msg.inner_text()

    # Verify no "Error:" prefix
    assert not msg_text.lower().startswith("error:"), (
        f"Approval rejection must NOT have 'Error:' prefix. Got: {msg_text!r}"
    )

    # Verify it contains "waiting for approval"
    assert "waiting for approval" in msg_text.lower(), (
        f"Expected 'Waiting for approval' text. Got: {msg_text!r}"
    )

    # Verify it contains the tool name and description
    assert "http" in msg_text.lower(), (
        f"Expected tool name 'http' in message. Got: {msg_text!r}"
    )
    assert "HTTP requests to external APIs" in msg_text, (
        f"Expected tool description in message. Got: {msg_text!r}"
    )


async def test_chat_reply_approve_resumes_pending_tool(ironclaw_server):
    """A plain chat reply of 'approve' should resume the pending tool call."""
    thread_id = await _create_thread(ironclaw_server)

    await _send_chat_message(ironclaw_server, thread_id, "make approval post approval-chat")
    await _wait_for_history(ironclaw_server, thread_id, expect_pending=True)

    await _send_chat_message(ironclaw_server, thread_id, "approve")
    history = await _wait_for_history(
        ironclaw_server,
        thread_id,
        expect_pending=False,
        response_fragment="The http tool returned:",
        turn_count_at_least=1,
    )

    assert history.get("pending_gate") is None
    assert history["turns"][-1]["response"] is not None


async def test_chat_reply_deny_rejects_pending_tool(ironclaw_server):
    """A plain chat reply of 'deny' should reject the pending tool call."""
    thread_id = await _create_thread(ironclaw_server)

    await _send_chat_message(ironclaw_server, thread_id, "make approval post approval-denied")
    await _wait_for_history(ironclaw_server, thread_id, expect_pending=True)

    await _send_chat_message(ironclaw_server, thread_id, "deny")
    history = await _wait_for_history(
        ironclaw_server,
        thread_id,
        expect_pending=False,
        response_fragment="was rejected",
        turn_count_at_least=1,
    )

    response_text = history["turns"][-1]["response"]
    assert response_text is not None
    assert "Tool 'http' was rejected" in response_text


async def test_text_approval_resolves_real_tool_call(page):
    """Typing 'yes' should resolve a real approval gate triggered by a tool call."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Trigger a real HTTP tool call that requires approval
    await chat_input.fill("make approval post text-approval-e2e")
    await chat_input.press("Enter")

    # Wait for the approval card to appear (from the SSE event)
    card = page.locator(SEL["approval_card"]).last
    await card.wait_for(state="visible", timeout=15000)

    tool_name = await card.locator(".approval-tool-name").text_content()
    assert tool_name == "http"

    # Type "yes" to approve — should be intercepted by the frontend
    await chat_input.fill("yes")
    await chat_input.press("Enter")

    # Card should show resolved status
    resolved = card.locator(".approval-resolved")
    await resolved.wait_for(state="visible", timeout=5000)
    assert await resolved.text_content() == "Approved"

    # Card should be removed after brief delay
    await card.wait_for(state="hidden", timeout=5000)


async def test_slash_approve_is_thread_scoped_api(ironclaw_server):
    """Sending '/approve' in thread A must not resolve a pending gate in thread B."""
    thread_a = await _create_thread(ironclaw_server)
    thread_b = await _create_thread(ironclaw_server)

    await _send_chat_message(
        ironclaw_server,
        thread_b,
        "make approval post slash-approve-thread-scope",
    )
    await _wait_for_history(ironclaw_server, thread_b, expect_pending=True)

    await _send_chat_message(ironclaw_server, thread_a, "/approve")
    await asyncio.sleep(1.0)

    history_a = await _wait_for_history(
        ironclaw_server,
        thread_a,
        expect_pending=False,
        timeout=5.0,
    )
    assert history_a.get("pending_gate") is None

    history_b = await _wait_for_history(
        ironclaw_server,
        thread_b,
        expect_pending=True,
        turn_count_at_least=1,
        timeout=5.0,
    )
    assert history_b.get("pending_gate") is not None


# NOTE: This test permanently auto-approves the `http` tool for the session.
# All tests that need the http approval gate to fire MUST run before this one.
async def test_chat_reply_always_auto_approves_next_same_tool(ironclaw_server):
    """A plain chat reply of 'always' should auto-approve the same tool next time."""
    thread_id = await _create_thread(ironclaw_server)

    await _send_chat_message(ironclaw_server, thread_id, "make approval post approval-always-a")
    await _wait_for_history(ironclaw_server, thread_id, expect_pending=True)

    await _send_chat_message(ironclaw_server, thread_id, "always")
    await _wait_for_history(
        ironclaw_server,
        thread_id,
        expect_pending=False,
        response_fragment="The http tool returned:",
        turn_count_at_least=1,
    )

    await _send_chat_message(ironclaw_server, thread_id, "make approval post approval-always-b")
    history = await _wait_for_history(
        ironclaw_server,
        thread_id,
        expect_pending=False,
        response_fragment="The http tool returned:",
        turn_count_at_least=2,
    )

    assert history.get("pending_gate") is None
    assert len(history["turns"]) >= 2


# -- Text-based approval interception tests ----------------------------------


async def test_text_yes_intercepts_approval(page):
    """Typing 'yes' in the chat input should resolve a pending approval card."""
    chat_input = await ensure_writable_chat_input(page)

    user_msg_count_before = await page.locator(SEL["message_user"]).count()

    await page.evaluate("""
        showApproval({
            request_id: 'test-text-yes',
            thread_id: currentThreadId,
            tool_name: 'http',
            description: 'GET https://example.com',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-text-yes"]')
    await card.wait_for(state="visible", timeout=5000)

    await chat_input.fill("yes")
    await chat_input.press("Enter")

    resolved = card.locator(".approval-resolved")
    await resolved.wait_for(state="visible", timeout=5000)
    assert await resolved.text_content() == "Approved"

    # Input should be cleared after interception
    assert await chat_input.input_value() == "", "Input should be cleared after keyword interception"

    # No user message bubble should appear for "yes"
    user_msg_count_after = await page.locator(SEL["message_user"]).count()
    assert user_msg_count_after == user_msg_count_before, (
        "Typing 'yes' should not create a user message bubble"
    )


async def test_text_no_intercepts_denial(page):
    """Typing 'no' in the chat input should deny a pending approval card."""
    chat_input = await ensure_writable_chat_input(page)

    await page.evaluate("""
        showApproval({
            request_id: 'test-text-no',
            thread_id: currentThreadId,
            tool_name: 'shell',
            description: 'Execute: rm -rf /',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-text-no"]')
    await card.wait_for(state="visible", timeout=5000)

    await chat_input.fill("no")
    await chat_input.press("Enter")

    resolved = card.locator(".approval-resolved")
    await resolved.wait_for(state="visible", timeout=5000)
    assert await resolved.text_content() == "Denied"

    assert await chat_input.input_value() == "", "Input should be cleared after keyword interception"


async def test_text_always_intercepts_always(page):
    """Typing 'always' in the chat input should always-approve a pending card."""
    chat_input = await ensure_writable_chat_input(page)

    await page.evaluate("""
        showApproval({
            request_id: 'test-text-always',
            thread_id: currentThreadId,
            tool_name: 'http',
            description: 'POST https://example.com/api',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-text-always"]')
    await card.wait_for(state="visible", timeout=5000)

    await chat_input.fill("always")
    await chat_input.press("Enter")

    resolved = card.locator(".approval-resolved")
    await resolved.wait_for(state="visible", timeout=5000)
    assert await resolved.text_content() == "Always approved"

    assert await chat_input.input_value() == "", "Input should be cleared after keyword interception"


async def test_text_skips_resolved_card_targets_unresolved(page):
    """Typing 'yes' should skip a resolved card and target the next unresolved one."""
    chat_input = await ensure_writable_chat_input(page)

    # Inject two approval cards
    await page.evaluate("""
        showApproval({
            request_id: 'test-resolved-older',
            thread_id: currentThreadId,
            tool_name: 'http',
            description: 'Older unresolved card',
        });
        showApproval({
            request_id: 'test-resolved-newer',
            thread_id: currentThreadId,
            tool_name: 'shell',
            description: 'Newer card (will be resolved)',
        });
    """)

    older_card = page.locator('.approval-card[data-request-id="test-resolved-older"]')
    newer_card = page.locator('.approval-card[data-request-id="test-resolved-newer"]')
    await older_card.wait_for(state="visible", timeout=5000)
    await newer_card.wait_for(state="visible", timeout=5000)

    # Resolve the newer card via button click (it stays in DOM for 1.5s)
    await newer_card.locator("button.approve").click()
    newer_resolved = newer_card.locator(".approval-resolved")
    await newer_resolved.wait_for(state="visible", timeout=5000)

    # Now type "yes" — should skip the resolved newer card, target the older unresolved one
    await chat_input.fill("yes")
    await chat_input.press("Enter")

    older_resolved = older_card.locator(".approval-resolved")
    await older_resolved.wait_for(state="visible", timeout=5000)
    assert await older_resolved.text_content() == "Approved"


async def test_text_aliases_intercepted(page):
    """Various approval aliases ('y', 'n', 'approve', 'deny') should be intercepted."""
    chat_input = await ensure_writable_chat_input(page)

    aliases = [
        ("y", "Approved"),
        ("n", "Denied"),
        ("approve", "Approved"),
        ("deny", "Denied"),
    ]

    for i, (text, expected_label) in enumerate(aliases):
        req_id = f"test-alias-{i}"
        await page.evaluate(
            f"""
            showApproval({{
                request_id: '{req_id}',
                thread_id: currentThreadId,
                tool_name: 'http',
                description: 'Test alias {text}',
            }})
            """
        )

        card = page.locator(f'.approval-card[data-request-id="{req_id}"]')
        await card.wait_for(state="visible", timeout=5000)

        await chat_input.fill(text)
        await chat_input.press("Enter")

        resolved = card.locator(".approval-resolved")
        await resolved.wait_for(state="visible", timeout=5000)
        actual = await resolved.text_content()
        assert actual == expected_label, (
            f"Alias '{text}' should resolve as '{expected_label}', got '{actual}'"
        )


async def test_text_approval_case_insensitive(page):
    """Approval keywords should be matched case-insensitively ('Yes', 'YES', 'No')."""
    chat_input = await ensure_writable_chat_input(page)

    cases = [
        ("Yes", "Approved"),
        ("YES", "Approved"),
        ("No", "Denied"),
        ("ALWAYS", "Always approved"),
    ]

    for i, (text, expected_label) in enumerate(cases):
        req_id = f"test-case-{i}"
        await page.evaluate(
            f"""
            showApproval({{
                request_id: '{req_id}',
                thread_id: currentThreadId,
                tool_name: 'http',
                description: 'Test case {text}',
            }})
            """
        )

        card = page.locator(f'.approval-card[data-request-id="{req_id}"]')
        await card.wait_for(state="visible", timeout=5000)

        await chat_input.fill(text)
        await chat_input.press("Enter")

        resolved = card.locator(".approval-resolved")
        await resolved.wait_for(state="visible", timeout=5000)
        actual = await resolved.text_content()
        assert actual == expected_label, (
            f"Case '{text}' should resolve as '{expected_label}', got '{actual}'"
        )


async def test_normal_text_not_intercepted_with_approval_card(page):
    """Regular text should still send as a normal message even when an approval card is visible."""
    chat_input = await ensure_writable_chat_input(page)

    user_msg_count_before = await page.locator(SEL["message_user"]).count()

    await page.evaluate("""
        showApproval({
            request_id: 'test-passthrough',
            thread_id: currentThreadId,
            tool_name: 'http',
            description: 'GET https://example.com',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-passthrough"]')
    await card.wait_for(state="visible", timeout=5000)

    # Type regular text that is not an approval keyword
    await chat_input.fill("hello world")
    await chat_input.press("Enter")

    # A user message bubble should appear (text was NOT intercepted)
    await page.wait_for_function(
        f"() => document.querySelectorAll('{SEL['message_user']}').length > {user_msg_count_before}",
        timeout=5000,
    )

    # The approval card should still be visible (not resolved)
    assert await card.is_visible(), "Approval card should remain visible after non-keyword text"
    assert await card.locator(".approval-resolved").count() == 0, (
        "Approval card should not show a resolved label"
    )


async def test_text_approval_resolves_real_tool_call(browser, managed_gateway_server):
    """Typing 'yes' should resolve a real approval gate triggered by a tool call."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await page.goto(f"{managed_gateway_server.base_url}/?token={AUTH_TOKEN}")
        await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
        chat_input = await ensure_writable_chat_input(page)

        # Trigger a real HTTP tool call that requires approval
        await chat_input.fill("make approval post text-approval-e2e")
        await chat_input.press("Enter")

        # Wait for the approval card to appear (from the SSE event)
        card = page.locator(SEL["approval_card"]).last
        await card.wait_for(state="visible", timeout=15000)

        tool_name = await card.locator(".approval-tool-name").text_content()
        assert tool_name == "http"

        # Type "yes" to approve — should be intercepted by the frontend
        await chat_input.fill("yes")
        await chat_input.press("Enter")

        # Card should show resolved status
        resolved = card.locator(".approval-resolved")
        await resolved.wait_for(state="visible", timeout=5000)
        assert await resolved.text_content() == "Approved"

        # Card should be removed after brief delay
        await card.wait_for(state="hidden", timeout=5000)
    finally:
        await context.close()
# -- Regression: bare keywords without pending approval ----------------------


async def test_bare_yes_treated_as_chat_when_no_approval_pending_api(ironclaw_server):
    """Sending 'yes' via API when no approval is pending should reach the LLM.

    Regression test for the bug where SubmissionParser unconditionally converted
    bare keywords like 'yes'/'no'/'always' into ApprovalResponse, causing the
    backend to return 'No pending approval for this thread.' instead of routing
    the message as normal user input.
    """
    thread_id = await _create_thread(ironclaw_server)

    # Send "yes" with no prior approval pending
    await _send_chat_message(ironclaw_server, thread_id, "yes")

    # The LLM should process it as a regular message and produce a response.
    # The mock LLM returns "I understand your request." for unrecognized input.
    # Critically, the response must NOT be "No pending approval for this thread."
    # Wait for a non-null response (not just turn existence) to avoid flakiness.
    history = await _wait_for_history(
        ironclaw_server,
        thread_id,
        turn_count_at_least=1,
        response_fragment="",  # any non-null response
        timeout=15.0,
    )

    response_text = history["turns"][-1].get("response") or ""
    assert "No pending approval" not in response_text, (
        f"Bare 'yes' was intercepted as approval instead of chat input. Got: {response_text!r}"
    )
    assert response_text, "Expected an LLM response for bare 'yes' input"


async def test_bare_no_treated_as_chat_when_no_approval_pending_api(ironclaw_server):
    """Sending 'no' via API when no approval is pending should reach the LLM."""
    thread_id = await _create_thread(ironclaw_server)

    await _send_chat_message(ironclaw_server, thread_id, "no")

    history = await _wait_for_history(
        ironclaw_server,
        thread_id,
        turn_count_at_least=1,
        response_fragment="",  # any non-null response
        timeout=15.0,
    )

    response_text = history["turns"][-1].get("response") or ""
    assert "No pending approval" not in response_text, (
        f"Bare 'no' was intercepted as approval instead of chat input. Got: {response_text!r}"
    )
    assert response_text, "Expected an LLM response for bare 'no' input"


async def test_bare_yes_treated_as_chat_in_browser_when_no_card(page):
    """Typing 'yes' in the browser when no approval card exists should send as chat.

    Regression: the frontend sendMessage() used to check for any .approval-card
    in the DOM without verifying it belonged to the current thread. With no cards
    at all, the backend's SubmissionParser would still convert 'yes' into an
    ApprovalResponse. After the fix, 'yes' should reach the LLM as normal input.
    """
    result = await send_chat_and_wait_for_terminal_message(page, "yes", timeout=15000)

    assert result["role"] == "assistant", (
        f"Expected assistant response, got {result['role']}: {result['text']!r}"
    )
    assert "No pending approval" not in result["text"], (
        f"Bare 'yes' was intercepted as approval instead of chat input. Got: {result['text']!r}"
    )


async def test_approval_card_from_other_thread_not_intercepted(page):
    """An approval card stamped with a different thread_id must not intercept 'yes'."""
    chat_input = await ensure_writable_chat_input(page)

    # Inject an approval card tagged with a DIFFERENT thread ID
    await page.evaluate("""
        showApproval({
            request_id: 'test-other-thread',
            thread_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
            tool_name: 'http',
            description: 'From another thread',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-other-thread"]')
    await card.wait_for(state="visible", timeout=5000)

    # Type "yes" — should NOT be intercepted because the card belongs to a
    # different thread. It should go through as a normal chat message.
    result = await send_chat_and_wait_for_terminal_message(page, "yes", timeout=15000)

    assert result["role"] == "assistant", (
        f"Expected assistant response, got {result['role']}: {result['text']!r}"
    )
    # The approval card should still be unresolved
    assert await card.locator(".approval-resolved").count() == 0, (
        "Approval card from another thread should NOT be resolved"
    )


async def test_approval_card_button_posts_card_thread_id(page):
    """Clicking an approval card must post the card's thread_id, not the active thread."""
    chat_input = await ensure_writable_chat_input(page)

    captured = {}

    async def handle_gate_resolve(route):
        captured["body"] = json.loads(route.request.post_data or "{}")
        await route.fulfill(status=200, content_type="application/json", body='{"ok":true}')

    await page.route("**/api/chat/gate/resolve", handle_gate_resolve)

    await page.evaluate("""
        showApproval({
            request_id: 'test-button-thread-id',
            thread_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
            tool_name: 'http',
            description: 'From another thread',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-button-thread-id"]')
    await card.wait_for(state="visible", timeout=5000)
    await card.locator("button.approve").click()

    for _ in range(20):
        if "body" in captured:
            break
        await page.wait_for_timeout(100)
    else:
        raise AssertionError("Expected /api/chat/gate/resolve request to be captured")

    assert captured["body"]["request_id"] == "test-button-thread-id"
    assert captured["body"]["thread_id"] == "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    assert captured["body"]["resolution"] == "approved"
    assert captured["body"]["always"] is False


async def test_slash_approve_does_not_intercept_other_thread_card(page):
    """Typing '/approve' must not resolve an approval card from another thread."""
    chat_input = await ensure_writable_chat_input(page)

    captured = {"count": 0}

    async def handle_gate_resolve(route):
        captured["count"] += 1
        await route.fulfill(status=200, content_type="application/json", body='{"ok":true}')

    await page.route("**/api/chat/gate/resolve", handle_gate_resolve)

    await page.evaluate("""
        showApproval({
            request_id: 'test-other-thread-slash',
            thread_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
            tool_name: 'http',
            description: 'From another thread',
        })
    """)

    card = page.locator('.approval-card[data-request-id="test-other-thread-slash"]')
    await card.wait_for(state="visible", timeout=5000)

    await chat_input.fill("/approve")
    await chat_input.press("Enter")
    await page.wait_for_timeout(1000)

    assert captured["count"] == 0, (
        "Approval card from another thread should NOT trigger gate resolution"
    )
    assert await card.locator(".approval-resolved").count() == 0, (
        "Approval card from another thread should NOT be resolved by /approve"
    )


async def test_slash_approve_is_thread_scoped_api(managed_gateway_server):
    """Sending '/approve' in thread A must not resolve a pending gate in thread B."""
    base_url = managed_gateway_server.base_url
    thread_a = await _create_thread(base_url)
    thread_b = await _create_thread(base_url)

    await _send_chat_message(
        base_url,
        thread_b,
        "make approval post slash-approve-thread-scope",
    )
    await _wait_for_history(base_url, thread_b, expect_pending=True)

    await _send_chat_message(base_url, thread_a, "/approve")
    await asyncio.sleep(1.0)

    history_a = await _wait_for_history(
        base_url,
        thread_a,
        expect_pending=False,
        timeout=5.0,
    )
    assert history_a.get("pending_gate") is None

    history_b = await _wait_for_history(
        base_url,
        thread_b,
        expect_pending=True,
        turn_count_at_least=1,
        timeout=5.0,
    )
    assert history_b.get("pending_gate") is not None
