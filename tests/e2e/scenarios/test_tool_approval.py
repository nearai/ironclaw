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

    Tests the real flow: show approval card, then attempt to send input while approval is pending.
    Backend rejects with Pending result (not Error), and message includes tool context.
    """
    # First, inject an approval card to simulate the thread being in AwaitingApproval state
    await page.evaluate("""
        showApproval({
            request_id: 'test-req-waiting-approval',
            thread_id: currentThreadId,
            tool_name: 'shell',
            description: 'Execute: echo hello',
            parameters: '{"command": "echo hello"}'
        })
    """)

    # Wait for approval card to be visible (thread is now in AwaitingApproval state)
    card = page.locator('.approval-card[data-request-id="test-req-waiting-approval"]')
    await card.wait_for(state="visible", timeout=5000)

    # Record initial message count
    initial_count = await page.locator(SEL["message_assistant"]).count()

    # Now attempt to send input while approval is pending
    # (the backend will reject this and return the "Waiting for approval" status message)
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill("Test input while awaiting approval")
    await chat_input.press("Enter")

    # Wait for the status message from the backend rejection
    await page.wait_for_function(
        f"() => document.querySelectorAll('{SEL['message_assistant']}').length > {initial_count}",
        timeout=10000,
    )

    # Get the new status message
    last_msg = page.locator(SEL["message_assistant"]).last
    msg_text = await last_msg.text_content()

    # Verify no "Error:" prefix
    assert not msg_text.lower().startswith("error:"), (
        f"Approval rejection must NOT have 'Error:' prefix. Got: {msg_text!r}"
    )

    # Verify it contains "waiting for approval"
    assert "waiting for approval" in msg_text.lower(), (
        f"Expected 'Waiting for approval' text. Got: {msg_text!r}"
    )

    # Verify it contains the tool name and description
    assert "shell" in msg_text.lower(), (
        f"Expected tool name 'shell' in message. Got: {msg_text!r}"
    )
    assert "echo hello" in msg_text, (
        f"Expected tool description in message. Got: {msg_text!r}"
    )
