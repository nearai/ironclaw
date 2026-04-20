"""Scenario 2: Chat message round-trip via SSE streaming."""

import asyncio
import base64
import io
import json
import zipfile
from pathlib import Path

import httpx
import pytest
from helpers import SEL, api_get, api_post, send_chat_and_wait_for_terminal_message

ROOT = Path(__file__).resolve().parents[3]
HELLO_PDF = ROOT / "tests" / "fixtures" / "hello.pdf"
ONE_BY_ONE_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO7Z0QAAAABJRU5ErkJggg=="
)


def _make_test_pptx(slide_text: str) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as archive:
        archive.writestr(
            "ppt/slides/slide1.xml",
            f"""<?xml version="1.0" encoding="UTF-8"?>
            <p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                   xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
              <p:cSld>
                <p:spTree>
                  <p:sp>
                    <p:txBody>
                      <a:p><a:r><a:t>{slide_text}</a:t></a:r></a:p>
                    </p:txBody>
                  </p:sp>
                </p:spTree>
              </p:cSld>
            </p:sld>""",
        )
    return buf.getvalue()


async def _wait_for_mock_llm_request_contains(mock_llm_url: str, needles: list[str], *, timeout: float = 30.0) -> dict:
    last_payload = {}
    async with httpx.AsyncClient() as client:
        for _ in range(int(timeout * 2)):
            response = await client.get(
                f"{mock_llm_url}/__mock/last_chat_request",
                timeout=15,
            )
            response.raise_for_status()
            payload = response.json()
            last_payload = payload
            haystack = json.dumps(payload).lower()
            if all(needle.lower() in haystack for needle in needles):
                return payload
            await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for mock LLM request containing {needles!r}. "
        f"Last payload: {json.dumps(last_payload)[:1200]}"
    )


async def _wait_for_thread_response(
    base_url: str,
    thread_id: str,
    *,
    expected_user_input: str,
    timeout: float = 45.0,
) -> dict:
    last_history = {}
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        response.raise_for_status()
        history = response.json()
        last_history = history
        turns = history.get("turns", [])
        if turns:
            last_turn = turns[-1]
            if expected_user_input in (last_turn.get("user_input") or "") and (
                last_turn.get("response") or ""
            ).strip():
                return history
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for assistant response in thread {thread_id}. "
        f"Last history: {json.dumps(last_history)[:1200]}"
    )


async def _wait_for_current_thread_id(page, *, timeout: int = 15000) -> str:
    await page.wait_for_function(
        "() => typeof currentThreadId !== 'undefined' && !!currentThreadId",
        timeout=timeout,
    )
    return await page.evaluate("() => currentThreadId")


async def _last_user_message_state(page) -> dict | None:
    return await page.evaluate(
        """
        () => {
          const users = document.querySelectorAll('#chat-messages .message.user');
          const lastUser = users.length ? users[users.length - 1] : null;
          if (!lastUser) return null;
          const content = lastUser.querySelector('.message-content');
          return {
            fileCards: lastUser.querySelectorAll('.message-attachment-file').length,
            imageCards: lastUser.querySelectorAll('.message-attachment-image').length,
            text: (lastUser.innerText || '').trim(),
            contentText: ((content && content.innerText) || '').trim(),
          };
        }
        """
    )


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


async def test_slash_autocomplete_shows_commands_and_skills(page, ironclaw_server):
    """Typing `/` should show built-in commands and installed skills in one menu."""
    response = await api_get(ironclaw_server, "/api/skills", timeout=10)
    response.raise_for_status()
    skills = response.json().get("skills", [])
    assert skills, "Expected at least one installed skill for slash autocomplete"
    skill_name = skills[0]["name"]

    chat_input = page.locator(SEL["chat_input"])
    autocomplete = page.locator(SEL["slash_autocomplete"])

    await chat_input.fill("/")
    await autocomplete.wait_for(state="visible", timeout=10000)
    await page.wait_for_function(
        """
        targetSkill => {
          const cmds = Array.from(
            document.querySelectorAll('#slash-autocomplete .slash-ac-cmd')
          ).map((el) => (el.textContent || '').trim());
          return cmds.includes('/help') && cmds.includes('/' + targetSkill);
        }
        """,
        arg=skill_name,
        timeout=10000,
    )

    commands = await page.evaluate(
        """
        () => Array.from(document.querySelectorAll('#slash-autocomplete .slash-ac-cmd'))
          .map((el) => (el.textContent || '').trim())
        """
    )
    assert "/help" in commands, commands
    assert f"/{skill_name}" in commands, commands

    skill_prefix = "/" if len(skill_name) == 1 else f"/{skill_name[:1]}"
    await chat_input.fill(skill_prefix)
    skill_item = page.locator(SEL["slash_item"]).filter(has_text=f"/{skill_name}").first
    await skill_item.wait_for(state="visible", timeout=10000)
    await skill_item.click()

    assert await chat_input.input_value() == f"/{skill_name} "


async def test_message_copy_button_writes_raw_text(page):
    """Clicking a message's per-message Copy button writes the raw text to
    navigator.clipboard and flashes the button label to "Copied".

    Covers the `.message-copy-btn` → `copyMessage(btn)` path in
    `core/render.js`. The existing `test_copy_from_chat_forces_plain_text`
    only exercises the Cmd+C selection handler, not the button click, so a
    regression that stopped wiring the button (or that broke the
    `data-copy-text` attribute on rebuilt history) would ship silently.
    """
    # Stub navigator.clipboard.writeText so the test doesn't depend on
    # browser permission prompts. Capture the last written text.
    await page.evaluate(
        """() => {
          window._copiedText = null;
          navigator.clipboard.writeText = (text) => {
            window._copiedText = text;
            return Promise.resolve();
          };
          // Seed one user message (plain text) and one assistant message
          // (markdown-rendered). Both go through addMessage, which wires
          // up the per-message Copy button.
          addMessage('user', 'hello from user');
          addMessage('assistant', '**bold** answer');
        }"""
    )

    # Click the Copy button on the user message. The button is only
    # visible on hover in CSS, so the parent intercepts pointer events
    # without `force=True`. The production click handler doesn't care
    # about pointer events — it's wired via addEventListener('click').
    user_copy = page.locator('#chat-messages .message.user .message-copy-btn').last
    await user_copy.wait_for(state="attached", timeout=5000)
    await user_copy.click(force=True)

    await page.wait_for_function(
        "() => window._copiedText === 'hello from user'",
        timeout=5000,
    )

    # Button label should flash "Copied!" then revert to "Copy" within 1.5s.
    assert (await user_copy.text_content()) == "Copied!"
    await page.wait_for_function(
        """() => {
          const userMsgs = document.querySelectorAll('#chat-messages .message.user');
          const lastUser = userMsgs[userMsgs.length - 1];
          const btn = lastUser && lastUser.querySelector('.message-copy-btn');
          return btn && (btn.textContent || '').trim() === 'Copy';
        }""",
        timeout=3000,
    )

    # Assistant copy should use the raw markdown (data-raw), not the
    # rendered HTML text.
    assistant_copy = page.locator('#chat-messages .message.assistant .message-copy-btn').last
    await assistant_copy.click(force=True)
    await page.wait_for_function(
        "() => window._copiedText === '**bold** answer'",
        timeout=5000,
    )


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


async def test_gateway_attachment_flow_renders_thread_and_reaches_llm(page, ironclaw_server, mock_llm_server):
    """Upload image/PDF/text/slides, render them in-thread, and verify the LLM payload."""
    attachment_input = page.locator(SEL["attachment_input"])
    chat_input = page.locator(SEL["chat_input"])

    await page.wait_for_function(
        "() => typeof currentThreadId !== 'undefined' && !!currentThreadId",
        timeout=15000,
    )
    thread_id = await page.evaluate("() => currentThreadId")

    await attachment_input.set_input_files(
        files=[
            {
                "name": "tiny.png",
                "mimeType": "image/png",
                "buffer": ONE_BY_ONE_PNG,
            },
            {
                "name": "hello.pdf",
                "mimeType": "application/pdf",
                "buffer": HELLO_PDF.read_bytes(),
            },
            {
                "name": "notes.txt",
                "mimeType": "text/plain",
                "buffer": b"Quarterly roadmap notes\nShip the gateway attachment flow.",
            },
            {
                "name": "roadmap.pptx",
                "mimeType": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                "buffer": _make_test_pptx("Gateway attachment roadmap slide"),
            },
        ]
    )

    await chat_input.fill("Please review these attachments.")
    await chat_input.press("Enter")

    history = await _wait_for_thread_response(
        ironclaw_server,
        thread_id,
        expected_user_input="Please review these attachments.",
        timeout=45.0,
    )

    attachment_state = await page.evaluate(
        """
        () => {
          const users = document.querySelectorAll('#chat-messages .message.user');
          const lastUser = users.length ? users[users.length - 1] : null;
          if (!lastUser) return null;
          return {
            fileCards: lastUser.querySelectorAll('.message-attachment-file').length,
            imageCards: lastUser.querySelectorAll('.message-attachment-image').length,
            text: (lastUser.innerText || '').trim(),
          };
        }
        """
    )
    assert attachment_state is not None, "Expected a user message in the thread"
    assert attachment_state["imageCards"] == 1, attachment_state
    assert attachment_state["fileCards"] >= 3, attachment_state
    assert "hello.pdf" in attachment_state["text"], attachment_state
    assert "notes.txt" in attachment_state["text"], attachment_state
    assert "roadmap.pptx" in attachment_state["text"], attachment_state

    last_turn = history["turns"][-1]
    assert "Please review these attachments." in (last_turn.get("user_input") or "")

    payload = await _wait_for_mock_llm_request_contains(
        mock_llm_server,
        ["Please review these attachments."],
        timeout=45.0,
    )
    serialized = json.dumps(payload)
    assert "hello.pdf" in serialized, serialized[:1200]
    assert "Quarterly roadmap notes" in serialized, serialized[:1200]
    assert "Gateway attachment roadmap slide" in serialized, serialized[:1200]
    assert "Ship the gateway attachment flow." in serialized, serialized[:1200]
    assert "data:image/png;base64," in serialized, serialized[:1200]


async def test_gateway_files_only_attachments_reload_from_history(page, ironclaw_server, mock_llm_server):
    """Files-only sends should persist and re-render from history without raw attachment markup."""
    thread_id = await _wait_for_current_thread_id(page)
    response = await api_post(
        ironclaw_server,
        "/api/chat/send",
        json={
            "content": "",
            "thread_id": thread_id,
            "attachments": [
                {
                    "mime_type": "application/pdf",
                    "filename": "files-only.pdf",
                    "data_base64": base64.b64encode(HELLO_PDF.read_bytes()).decode(),
                },
                {
                    "mime_type": "text/plain",
                    "filename": "files-only-notes.txt",
                    "data_base64": base64.b64encode(
                        b"Files-only attachment note.\nRendered from persisted history."
                    ).decode(),
                },
            ],
        },
        timeout=15,
    )
    response.raise_for_status()

    history = await _wait_for_thread_response(
        ironclaw_server,
        thread_id,
        expected_user_input="files-only-notes.txt",
        timeout=45.0,
    )

    payload = await _wait_for_mock_llm_request_contains(
        mock_llm_server,
        ["Files-only attachment note."],
        timeout=45.0,
    )
    serialized = json.dumps(payload)
    assert "files-only.pdf" in serialized, serialized[:1200]
    assert "files-only-notes.txt" in serialized, serialized[:1200]
    assert "Hello World" in serialized, serialized[:1200]

    await page.reload(wait_until="domcontentloaded")
    await page.locator(SEL["auth_screen"]).wait_for(state="hidden", timeout=15000)
    await page.wait_for_function(
        """targetThreadId => (
          typeof sseHasConnectedBefore !== 'undefined' &&
          sseHasConnectedBefore === true &&
          typeof currentThreadId !== 'undefined' &&
          currentThreadId === targetThreadId &&
          document.querySelectorAll('#chat-messages .message.user').length > 0
        )""",
        arg=thread_id,
        timeout=15000,
    )

    reloaded_state = await _last_user_message_state(page)
    assert reloaded_state is not None
    assert reloaded_state["fileCards"] >= 2, reloaded_state
    assert reloaded_state["imageCards"] == 0, reloaded_state
    assert "files-only.pdf" in reloaded_state["text"], reloaded_state
    assert "files-only-notes.txt" in reloaded_state["text"], reloaded_state
    assert "(files attached)" not in reloaded_state["text"], reloaded_state
    assert "<attachments>" not in reloaded_state["text"], reloaded_state
    assert reloaded_state["contentText"] == "", reloaded_state

    last_turn = history["turns"][-1]
    assert "Rendered from persisted history." in (last_turn.get("user_input") or "")


async def test_gateway_attachment_unextractable_file_uses_placeholder(page, ironclaw_server, mock_llm_server):
    """Unsupported documents should still reach the backend with a fallback attachment marker."""
    attachment_input = page.locator(SEL["attachment_input"])
    chat_input = page.locator(SEL["chat_input"])
    thread_id = await _wait_for_current_thread_id(page)

    await attachment_input.set_input_files(
        files=[
            {
                "name": "mystery.bin",
                "mimeType": "application/octet-stream",
                "buffer": b"\x00\x01\x02\x03binary-payload",
            }
        ]
    )

    await chat_input.fill("Please inspect this binary attachment.")
    await chat_input.press("Enter")

    history = await _wait_for_thread_response(
        ironclaw_server,
        thread_id,
        expected_user_input="Please inspect this binary attachment.",
        timeout=45.0,
    )

    attachment_state = await _last_user_message_state(page)
    assert attachment_state is not None
    assert attachment_state["fileCards"] >= 1, attachment_state
    assert "mystery.bin" in attachment_state["text"], attachment_state

    last_turn = history["turns"][-1]
    user_input = last_turn.get("user_input") or ""
    assert "mystery.bin" in user_input, user_input
    assert "failed to extract text" in user_input.lower(), user_input
    assert "unsupported document type" in user_input.lower(), user_input

    payload = await _wait_for_mock_llm_request_contains(
        mock_llm_server,
        ["Please inspect this binary attachment."],
        timeout=45.0,
    )
    serialized = json.dumps(payload)
    assert "mystery.bin" in serialized, serialized[:1200]
    assert "failed to extract text" in serialized.lower(), serialized[:1200]


async def test_gateway_attachment_limits_block_batched_uploads(page):
    """Batch validation should enforce per-file, count, and total-size limits."""
    await page.evaluate(
        """
        () => {
          window.__alerts = [];
          window.alert = (msg) => window.__alerts.push(String(msg));
          stagedAttachments = [];
          renderAttachmentPreviews();
        }
        """
    )

    await page.evaluate(
        """
        () => {
          const files = Array.from({ length: 6 }, (_, i) =>
            new File([new Uint8Array([i + 1])], `limit-${i + 1}.txt`, { type: 'text/plain' })
          );
          handleAttachmentFiles(files);
        }
        """
    )
    await page.wait_for_function(
        "() => stagedAttachments.length === 5 && window.__alerts.length >= 1",
        timeout=10000,
    )
    count_state = await page.evaluate(
        """
        () => ({
          staged: stagedAttachments.length,
          previews: document.querySelectorAll('#image-preview-strip .attachment-preview-container').length,
          alerts: [...window.__alerts],
        })
        """
    )
    assert count_state["staged"] == 5, count_state
    assert count_state["previews"] == 5, count_state
    assert any("5" in msg for msg in count_state["alerts"]), count_state

    await page.evaluate(
        """
        () => {
          window.__alerts = [];
          stagedAttachments = [];
          renderAttachmentPreviews();
        }
        """
    )

    await page.evaluate(
        """
        () => {
          const makeFile = (name, size) => new File([new Uint8Array(size)], name, { type: 'text/plain' });
          handleAttachmentFiles([
            makeFile('chunk-1.txt', 4 * 1024 * 1024),
            makeFile('chunk-2.txt', 4 * 1024 * 1024),
            makeFile('chunk-3.txt', 4 * 1024 * 1024),
          ]);
        }
        """
    )
    await page.wait_for_function(
        "() => stagedAttachments.length === 2 && window.__alerts.length >= 1",
        timeout=15000,
    )
    total_size_state = await page.evaluate(
        """
        () => ({
          staged: stagedAttachments.length,
          previews: document.querySelectorAll('#image-preview-strip .attachment-preview-container').length,
          alerts: [...window.__alerts],
        })
        """
    )
    assert total_size_state["staged"] == 2, total_size_state
    assert total_size_state["previews"] == 2, total_size_state
    assert any("10" in msg for msg in total_size_state["alerts"]), total_size_state

    await page.evaluate(
        """
        () => {
          window.__alerts = [];
          stagedAttachments = [];
          renderAttachmentPreviews();
        }
        """
    )

    await page.evaluate(
        """
        () => {
          const tooBig = new File(
            [new Uint8Array((5 * 1024 * 1024) + 1)],
            'too-big.txt',
            { type: 'text/plain' }
          );
          handleAttachmentFiles([tooBig]);
        }
        """
    )
    await page.wait_for_function(
        "() => window.__alerts.length === 1",
        timeout=10000,
    )
    oversized_state = await page.evaluate(
        """
        () => ({
          staged: stagedAttachments.length,
          previews: document.querySelectorAll('#image-preview-strip .attachment-preview-container').length,
          alerts: [...window.__alerts],
        })
        """
    )
    assert oversized_state["staged"] == 0, oversized_state
    assert oversized_state["previews"] == 0, oversized_state
    assert any("too-big.txt" in msg for msg in oversized_state["alerts"]), oversized_state
