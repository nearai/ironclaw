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
