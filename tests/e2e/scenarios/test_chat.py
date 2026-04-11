"""Scenario 2: Chat message round-trip via SSE streaming."""

import pytest
from helpers import SEL, send_chat_and_wait_for_terminal_message


async def test_send_message_and_receive_response(page):
    """Type a message, receive a streamed response from mock LLM."""
    result = await send_chat_and_wait_for_terminal_message(page, "What is 2+2?")

    assert result["role"] == "assistant"
    assert "4" in result["text"], f"Expected '4' in response, got: '{result['text']}'"


async def test_multiple_messages(page):
    """Send two messages, verify both get responses."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # First message
    await chat_input.fill("Hello")
    await chat_input.press("Enter")

    # Wait for first response
    await page.locator(SEL["message_assistant"]).first.wait_for(
        state="visible", timeout=15000
    )

    # Second message
    await chat_input.fill("What is 2+2?")
    await chat_input.press("Enter")

    # Wait for second response (at least 2 assistant messages)
    await page.wait_for_function(
        """() => document.querySelectorAll('#chat-messages .message.assistant').length >= 2""",
        timeout=15000,
    )

    # Verify counts
    user_count = await page.locator(SEL["message_user"]).count()
    assistant_count = await page.locator(SEL["message_assistant"]).count()
    assert user_count >= 2, f"Expected >= 2 user messages, got {user_count}"
    assert assistant_count >= 2, f"Expected >= 2 assistant messages, got {assistant_count}"


async def test_empty_message_not_sent(page):
    """Pressing Enter with empty input should not create a message."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    initial_count = await page.locator(f"{SEL['message_user']}, {SEL['message_assistant']}").count()

    # Press Enter with empty input
    await chat_input.press("Enter")

    # Wait a moment and verify no new messages
    await page.wait_for_timeout(2000)
    final_count = await page.locator(f"{SEL['message_user']}, {SEL['message_assistant']}").count()
    assert final_count == initial_count, "Empty message should not create new messages"


async def test_copy_from_chat_forces_plain_text(page):
    """Copying selected chat text should populate plain text clipboard data only."""
    await page.evaluate("addMessage('assistant', 'Copy me into Sheets')")

    copied = await page.evaluate(
        """
        () => {
          const content = Array.from(document.querySelectorAll('#chat-messages .message.assistant .message-content'))
            .find((el) => (el.textContent || '').includes('Copy me into Sheets'));
          if (!content) return {ok: false, reason: 'no content'};
          const range = document.createRange();
          range.selectNodeContents(content);
          const sel = window.getSelection();
          sel.removeAllRanges();
          sel.addRange(range);

          const store = {};
          const evt = new Event('copy', { bubbles: true, cancelable: true });
          evt.clipboardData = {
            clearData: () => { Object.keys(store).forEach((k) => delete store[k]); },
            setData: (t, v) => { store[t] = v; },
            getData: (t) => store[t] || '',
          };

          content.dispatchEvent(evt);
          return {
            ok: true,
            defaultPrevented: evt.defaultPrevented,
            text: store['text/plain'] || '',
            html: store['text/html'] || '',
          };
        }
        """
    )

    assert copied["ok"], copied.get("reason", "copy setup failed")
    assert copied["defaultPrevented"] is True
    assert "Copy me into Sheets" in copied["text"]
    assert copied["html"] == ""


async def test_turn_cost_event_does_not_render_message_badge(page):
    """Usage SSE events should not append token/cost footers to chat messages."""
    await page.evaluate(
        """
        () => {
          currentThreadId = 'thread-turn-cost-test';
          addMessage('assistant', 'No footer please');
        }
        """
    )

    badge_count = await page.evaluate(
        """
        () => {
          const before = document.querySelectorAll('.turn-cost-badge').length;
          const hasEventSource =
            typeof eventSource !== 'undefined' && !!eventSource && !!eventSource.dispatchEvent;
          if (!hasEventSource) {
            return {
              before,
              after: document.querySelectorAll('.turn-cost-badge').length,
              text: document.querySelector('#chat-messages .message.assistant:last-child')?.textContent || '',
              hasEventSource,
            };
          }
          eventSource.dispatchEvent(new MessageEvent('turn_cost', {
            data: JSON.stringify({
              thread_id: currentThreadId || 'thread-turn-cost-test',
              input_tokens: 632101,
              output_tokens: 0,
              cost_usd: '$1.6296',
            }),
          }));
          return {
            before,
            after: document.querySelectorAll('.turn-cost-badge').length,
            text: document.querySelector('#chat-messages .message.assistant:last-child')?.textContent || '',
            hasEventSource,
          };
        }
        """
    )

    assert badge_count["hasEventSource"] is True
    assert badge_count["before"] == 0
    assert badge_count["after"] == 0
    assert "632,101 tokens" not in badge_count["text"]
    assert "$1.6296" not in badge_count["text"]
