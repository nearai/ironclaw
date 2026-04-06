"""Scenario 6: Tool approval overlay UI behavior."""

import asyncio

from helpers import SEL, api_get, api_post


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
        pending = history.get("pending_gate") or history.get("pending_approval")
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

    assert (history.get("pending_gate") or history.get("pending_approval")) is None
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

    assert (history.get("pending_gate") or history.get("pending_approval")) is None
    assert len(history["turns"]) >= 2
