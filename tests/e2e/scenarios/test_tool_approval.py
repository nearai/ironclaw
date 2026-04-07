"""Scenario 6: Tool approval overlay UI behavior."""

import pytest
from helpers import SEL


INJECT_APPROVAL_JS = """
(data) => {
    // Simulate an approval_needed SSE event by calling showApproval directly
    showApproval(data);
}
"""


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
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

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


# -- Text-based approval interception tests ----------------------------------


async def test_text_yes_intercepts_approval(page):
    """Typing 'yes' in the chat input should resolve a pending approval card."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

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
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

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
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

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


async def test_text_aliases_intercepted(page):
    """Various approval aliases ('y', 'n', 'approve', 'deny') should be intercepted."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

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
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

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
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

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
