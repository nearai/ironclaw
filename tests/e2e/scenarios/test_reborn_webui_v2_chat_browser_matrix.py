"""Stubbed-browser Reborn WebUI v2 chat matrix.

These tests drive the committed WebUI v2 bundle in Chromium while stubbing only
the `/api/webchat/v2/*` browser API contract. They sit between static JS unit
coverage and the live Reborn smoke suite: no live LLM calls, but real DOM,
routing, file input, paste/drop, and accessibility behavior.
"""

import json
import re
from urllib.parse import urlparse

import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import open_reborn_v2_page


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


DEFAULT_ATTACHMENTS = {
    "accept": ["text/plain", "image/png"],
    "max_count": 4,
    "max_file_bytes": 1024,
    "max_total_bytes": 4096,
}


def _thread(thread_id: str, title: str | None = None) -> dict:
    return {
        "thread_id": thread_id,
        "title": title or f"Thread {thread_id}",
        "created_at": "2026-06-27T00:00:00Z",
        "updated_at": "2026-06-27T00:00:00Z",
    }


def _user_message(message_id: str, content: str, *, attachments=None) -> dict:
    body = {
        "message_id": message_id,
        "kind": "user",
        "content": content,
        "sequence": 1,
        "status": "accepted",
        "created_at": "2026-06-27T00:00:00Z",
    }
    if attachments:
        body["attachments"] = attachments
    return body


class StubbedWebChatV2:
    def __init__(
        self,
        *,
        attachments: dict | None = None,
        threads: list[dict] | None = None,
        timelines: dict[str, list[dict]] | None = None,
        create_thread_id: str = "thread-created",
        send_response=None,
    ):
        self.attachments = attachments or DEFAULT_ATTACHMENTS
        self.threads = list(threads or [])
        self.timelines = dict(timelines or {})
        self.create_thread_id = create_thread_id
        self.send_response = send_response
        self.created_requests: list[dict] = []
        self.send_requests: list[dict] = []
        self.cancel_requests: list[dict] = []

    async def install(self, page) -> None:
        await page.add_init_script(
            """
            (() => {
              const streams = [];
              class FakeEventSource extends EventTarget {
                constructor(url) {
                  super();
                  this.url = url;
                  this.readyState = 0;
                  streams.push(this);
                  setTimeout(() => {
                    this.readyState = 1;
                    if (typeof this.onopen === "function") this.onopen(new Event("open"));
                  }, 0);
                }
                close() {
                  this.readyState = 2;
                }
              }
              window.EventSource = FakeEventSource;
              window.__emitV2Sse = (type, frame, id = "cursor-1") => {
                const stream = streams[streams.length - 1];
                if (!stream) throw new Error("no EventSource stream is open");
                const event = new MessageEvent(type, {
                  data: JSON.stringify({ type, ...frame }),
                  lastEventId: id,
                });
                stream.dispatchEvent(event);
              };
            })();
            """
        )
        await page.route(re.compile(r".*/api/webchat/v2/session$"), self._session)
        await page.route(
            re.compile(r".*/api/webchat/v2/threads(?:\?.*)?$"),
            self._threads,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/threads/[^/]+/timeline(?:\?.*)?$"),
            self._timeline,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/threads/[^/]+/messages$"),
            self._message,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/threads/[^/]+/runs/[^/]+/cancel$"),
            self._cancel,
        )

    async def _fulfill(self, route, body, status: int = 200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def _session(self, route) -> None:
        await self._fulfill(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": "reborn-v2-e2e-user",
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": self.attachments,
            },
        )

    async def _threads(self, route) -> None:
        if route.request.method == "POST":
            body = json.loads(route.request.post_data or "{}")
            self.created_requests.append(body)
            created = _thread(self.create_thread_id, "Created from browser")
            if not any(t["thread_id"] == self.create_thread_id for t in self.threads):
                self.threads.insert(0, created)
            await self._fulfill(route, {"thread": created})
            return
        await self._fulfill(route, {"threads": self.threads, "next_cursor": None})

    async def _timeline(self, route) -> None:
        thread_id = urlparse(route.request.url).path.split("/")[-2]
        await self._fulfill(
            route,
            {
                "messages": self.timelines.get(thread_id, []),
                "next_cursor": None,
            },
        )

    async def _message(self, route) -> None:
        thread_id = urlparse(route.request.url).path.split("/")[-2]
        body = json.loads(route.request.post_data or "{}")
        self.send_requests.append({"thread_id": thread_id, "body": body})
        if callable(self.send_response):
            response = self.send_response(thread_id, body, len(self.send_requests))
        elif self.send_response is not None:
            response = self.send_response
        else:
            response = {
                "thread_id": thread_id,
                "run_id": f"run-{len(self.send_requests)}",
                "status": "running",
                "accepted_message_ref": {"message_id": f"msg-{len(self.send_requests)}"},
            }
        if isinstance(response, tuple):
            status, payload = response
            await self._fulfill(route, payload, status=status)
            return
        await self._fulfill(route, response)

    async def _cancel(self, route) -> None:
        path = urlparse(route.request.url).path.split("/")
        body = json.loads(route.request.post_data or "{}")
        self.cancel_requests.append(
            {"thread_id": path[-4], "run_id": path[-2], "body": body}
        )
        await self._fulfill(route, {"thread_id": path[-4], "run_id": path[-2]})


async def _open_stubbed_chat(page, base_url: str, stub: StubbedWebChatV2, path="/v2/"):
    await stub.install(page)
    await open_reborn_v2_page(page, base_url, path=path)


async def _emit_final_reply(page, text="Stubbed final reply", run_id="run-1") -> None:
    await page.evaluate(
        """({ text, runId }) => window.__emitV2Sse("final_reply", {
          reply: {
            turn_run_id: runId,
            text,
            generated_at: "2026-06-27T00:00:02Z"
          }
        })""",
        {"text": text, "runId": run_id},
    )


async def _emit_running_projection(page, run_id="run-1") -> None:
    await page.evaluate(
        """(runId) => window.__emitV2Sse("projection_update", {
          state: { items: [{ run_status: { run_id: runId, status: "running" } }] }
        })""",
        run_id,
    )


async def _set_input_file(page, *, name, mime_type, text):
    await page.locator("input[type='file']").set_input_files(
        files=[{"name": name, "mimeType": mime_type, "buffer": text.encode()}]
    )


async def _paste_file(page, *, name, mime_type, text) -> None:
    await page.evaluate(
        """({ name, mimeType, text }) => {
          const file = new File([text], name, { type: mimeType });
          const data = new DataTransfer();
          data.items.add(file);
          const target = document.querySelector("[data-testid='chat-composer']");
          target.dispatchEvent(new ClipboardEvent("paste", {
            bubbles: true,
            cancelable: true,
            clipboardData: data,
          }));
        }""",
        {"name": name, "mimeType": mime_type, "text": text},
    )


async def _paste_files(page, files) -> None:
    await page.evaluate(
        """(files) => {
          const data = new DataTransfer();
          for (const item of files) {
            data.items.add(new File([item.text], item.name, { type: item.mimeType }));
          }
          const target = document.querySelector("[data-testid='chat-composer']");
          target.dispatchEvent(new ClipboardEvent("paste", {
            bubbles: true,
            cancelable: true,
            clipboardData: data,
          }));
        }""",
        files,
    )


async def _drop_file(page, *, name, mime_type, text) -> None:
    await page.evaluate(
        """({ name, mimeType, text }) => {
          const file = new File([text], name, { type: mimeType });
          const data = new DataTransfer();
          data.items.add(file);
          const target = document.querySelector("[data-testid='chat-composer']")
            .closest(".relative");
          target.dispatchEvent(new DragEvent("dragover", {
            bubbles: true,
            cancelable: true,
            dataTransfer: data,
          }));
          target.dispatchEvent(new DragEvent("drop", {
            bubbles: true,
            cancelable: true,
            dataTransfer: data,
          }));
        }""",
        {"name": name, "mimeType": mime_type, "text": text},
    )


async def _send_with_button(page, text: str) -> None:
    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill(text)
    send = page.get_by_label("Send message")
    await expect(send).to_be_enabled(timeout=5000)
    await send.click()


async def test_chat_browser_starter_prompt_creates_thread_and_renders_sse_reply(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(create_thread_id="thread-starter")
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)

        await page.get_by_role("button", name=re.compile("Inspect")).click()
        await expect(page.locator(SEL_V2["msg_user"]).first).to_be_visible(timeout=15000)
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-starter"), timeout=15000)
        await expect(
            page.get_by_role("button", name=re.compile("Created from browser")).first
        ).to_be_visible(timeout=15000)

        await _emit_final_reply(page, "Starter prompt complete")
        await expect(page.locator(SEL_V2["msg_assistant"]).first).to_contain_text(
            "Starter prompt complete", timeout=5000
        )
        assert len(stub.created_requests) == 1
        assert len(stub.send_requests) == 1
    finally:
        await context.close()


async def test_chat_browser_typed_first_message_button_path(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(create_thread_id="thread-typed")
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)

        await _send_with_button(page, "typed first message")
        await expect(page.locator(SEL_V2["msg_user"]).first).to_contain_text(
            "typed first message", timeout=15000
        )
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-typed"), timeout=15000)
        await _emit_final_reply(page, "Typed path complete")
        await expect(page.locator(SEL_V2["msg_assistant"]).first).to_contain_text(
            "Typed path complete", timeout=5000
        )
        assert stub.send_requests[0]["body"]["content"] == "typed first message"
    finally:
        await context.close()


async def test_chat_browser_existing_thread_followup_does_not_create_thread(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(
        threads=[_thread("thread-existing", "Existing thread")],
        timelines={"thread-existing": [_user_message("seed", "seed message")]},
    )
    try:
        await _open_stubbed_chat(
            page, reborn_v2_server, stub, path="/v2/chat/thread-existing"
        )

        await _send_with_button(page, "follow up")
        await expect(page.locator(SEL_V2["msg_user"])).to_have_count(2, timeout=15000)
        assert stub.created_requests == []
        assert stub.send_requests[0]["thread_id"] == "thread-existing"
        assert stub.send_requests[0]["body"]["content"] == "follow up"
    finally:
        await context.close()


@pytest.mark.parametrize(
    ("stage", "filename", "expected_name"),
    [
        ("picker", "picked.txt", "picked.txt"),
        ("drop", "dropped.txt", "dropped.txt"),
        ("paste", "pasted.txt", "pasted.txt"),
    ],
)
async def test_chat_browser_text_attachment_paths_submit_wire_shape(
    reborn_v2_server, reborn_v2_browser, stage, filename, expected_name
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(create_thread_id=f"thread-{stage}")
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        if stage == "picker":
            await _set_input_file(page, name=filename, mime_type="text/plain", text="hello")
        elif stage == "drop":
            await _drop_file(page, name=filename, mime_type="text/plain", text="hello")
        else:
            await _paste_file(page, name=filename, mime_type="text/plain", text="hello")

        await expect(page.get_by_text(expected_name)).to_be_visible(timeout=5000)
        await _send_with_button(page, f"send {stage} attachment")
        await expect(page).to_have_url(re.compile(rf"/v2/chat/thread-{stage}"), timeout=15000)

        attachments = stub.send_requests[0]["body"]["attachments"]
        assert attachments[0]["filename"] == expected_name
        assert attachments[0]["mime_type"] == "text/plain"
        assert attachments[0]["data_base64"]
    finally:
        await context.close()


async def test_chat_browser_image_attachment_renders_thumbnail_and_submits_wire_shape(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(create_thread_id="thread-image")
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        await _set_input_file(
            page,
            name="pixel.png",
            mime_type="image/png",
            text="not-really-a-png-but-good-enough-for-base64",
        )

        await expect(page.get_by_alt_text("pixel.png")).to_be_visible(timeout=5000)
        await _send_with_button(page, "send image")
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-image"), timeout=15000)
        attachments = stub.send_requests[0]["body"]["attachments"]
        assert attachments[0]["filename"] == "pixel.png"
        assert attachments[0]["mime_type"] == "image/png"
    finally:
        await context.close()


@pytest.mark.parametrize(
    ("method", "name", "mime_type", "text", "expected"),
    [
        ("picker", "bad.png", "image/png", "image", "not a supported"),
        ("picker", "huge.txt", "text/plain", "x" * 20, "too large"),
        ("paste", "bad.png", "image/png", "image", "not a supported"),
        ("paste", "huge.txt", "text/plain", "x" * 20, "too large"),
    ],
)
async def test_chat_browser_attachment_rejections_block_mutation(
    reborn_v2_server, reborn_v2_browser, method, name, mime_type, text, expected
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(
        attachments={
            "accept": ["text/plain"],
            "max_count": 4,
            "max_file_bytes": 8,
            "max_total_bytes": 64,
        }
    )
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        if method == "picker":
            await _set_input_file(page, name=name, mime_type=mime_type, text=text)
        else:
            await _paste_file(page, name=name, mime_type=mime_type, text=text)

        await expect(page.get_by_role("alert")).to_contain_text(expected, timeout=5000)
        await expect(page.get_by_label("Remove attachment")).to_have_count(0)
        assert stub.created_requests == []
        assert stub.send_requests == []
    finally:
        await context.close()


@pytest.mark.parametrize(
    ("attachments", "files", "expected_error"),
    [
        (
            {"accept": ["text/plain"], "max_count": 1, "max_file_bytes": 64, "max_total_bytes": 64},
            [
                {"name": "one.txt", "mimeType": "text/plain", "text": "one"},
                {"name": "two.txt", "mimeType": "text/plain", "text": "two"},
            ],
            "at most 1 file",
        ),
        (
            {"accept": ["text/plain"], "max_count": 4, "max_file_bytes": 64, "max_total_bytes": 5},
            [
                {"name": "fits.txt", "mimeType": "text/plain", "text": "1234"},
                {"name": "too-much.txt", "mimeType": "text/plain", "text": "1234"},
            ],
            "total limit",
        ),
    ],
)
async def test_chat_browser_paste_limit_errors_keep_only_admitted_files(
    reborn_v2_server, reborn_v2_browser, attachments, files, expected_error
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(attachments=attachments)
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        await _paste_files(page, files)

        await expect(page.get_by_text(files[0]["name"])).to_be_visible(timeout=5000)
        await expect(page.get_by_role("alert")).to_contain_text(expected_error)
        await expect(page.get_by_text(files[1]["name"])).to_have_count(0)
        await _send_with_button(page, "send admitted file")
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-created"), timeout=15000)
        assert len(stub.send_requests[0]["body"]["attachments"]) == 1
        assert stub.send_requests[0]["body"]["attachments"][0]["filename"] == files[0]["name"]
    finally:
        await context.close()


async def test_chat_browser_removed_attachment_and_dismissed_error_do_not_leak_to_send(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(
        attachments={
            "accept": ["text/plain"],
            "max_count": 4,
            "max_file_bytes": 64,
            "max_total_bytes": 64,
        }
    )
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        await _set_input_file(page, name="remove-me.txt", mime_type="text/plain", text="bye")
        await expect(page.get_by_text("remove-me.txt")).to_be_visible(timeout=5000)
        await page.get_by_label("Remove attachment").click()
        await expect(page.get_by_text("remove-me.txt")).to_have_count(0)

        await _paste_file(page, name="bad.png", mime_type="image/png", text="bad")
        await expect(page.get_by_role("alert")).to_contain_text("not a supported")
        await page.get_by_label("Dismiss").click()
        await expect(page.get_by_role("alert")).to_have_count(0)

        await _send_with_button(page, "send after cleanup")
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-created"), timeout=15000)
        assert "attachments" not in stub.send_requests[0]["body"]
    finally:
        await context.close()


async def test_chat_browser_busy_rejections_remain_visible_for_existing_and_new_threads(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    def busy(thread_id, _body, _count):
        return {
            "thread_id": thread_id,
            "outcome": "rejected_busy",
            "notice": "Thread is busy; try again soon.",
        }

    stub = StubbedWebChatV2(
        threads=[_thread("thread-busy", "Busy thread")],
        timelines={"thread-busy": [_user_message("seed", "seed message")]},
        create_thread_id="thread-new-busy",
        send_response=busy,
    )
    try:
        await _open_stubbed_chat(
            page, reborn_v2_server, stub, path="/v2/chat/thread-busy"
        )
        await _send_with_button(page, "existing busy")
        await expect(page.locator(SEL_V2["msg_system"]).last).to_contain_text(
            "Thread is busy", timeout=5000
        )

        await open_reborn_v2_page(page, reborn_v2_server, path="/v2/")
        await _send_with_button(page, "first busy")
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-new-busy"), timeout=15000)
        await expect(page.locator(SEL_V2["msg_system"]).last).to_contain_text(
            "Thread is busy", timeout=5000
        )
    finally:
        await context.close()


async def test_chat_browser_cancel_control_posts_run_cancel(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(create_thread_id="thread-cancel")
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        await _send_with_button(page, "start cancellable run")
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-cancel"), timeout=15000)
        await _emit_running_projection(page, "run-1")

        await page.get_by_label("Cancel").click()
        assert stub.cancel_requests == [
            {
                "thread_id": "thread-cancel",
                "run_id": "run-1",
                "body": {
                    "client_action_id": stub.cancel_requests[0]["body"]["client_action_id"],
                    "reason": "user_requested",
                },
            }
        ]
    finally:
        await context.close()


async def test_chat_browser_first_message_failure_can_retry_to_created_thread(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    def fail_once(thread_id, _body, count):
        if count == 1:
            return (503, {"error": {"message": "temporary outage"}})
        return {"thread_id": thread_id, "run_id": "run-retry", "status": "running"}

    stub = StubbedWebChatV2(create_thread_id="thread-retry", send_response=fail_once)
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        await _send_with_button(page, "retry me")
        await expect(page.locator(SEL_V2["msg_user"]).first).to_contain_text(
            "retry me", timeout=15000
        )
        await expect(page.get_by_label("Retry message")).to_be_visible(timeout=5000)

        await page.get_by_label("Retry message").click()
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-retry"), timeout=15000)
        assert [request["thread_id"] for request in stub.send_requests] == [
            "thread-retry",
            "thread-retry",
        ]
    finally:
        await context.close()


async def test_chat_browser_keyboard_submit_multiline_accessibility_and_focus(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedWebChatV2(create_thread_id="thread-keyboard")
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        composer = page.locator(SEL_V2["chat_composer"])
        await expect(page.get_by_role("main")).to_be_visible()
        await expect(page.get_by_label("Message IronClaw")).to_be_visible()
        await expect(page.get_by_label("Attach files")).to_be_visible()
        await expect(page.get_by_label("Send message")).to_be_disabled()

        await composer.focus()
        await composer.fill("line one")
        await page.keyboard.press("Shift+Enter")
        await page.keyboard.type("line two")
        await page.keyboard.press("Enter")

        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-keyboard"), timeout=15000)
        await expect(composer).to_be_focused(timeout=5000)
        assert stub.send_requests[0]["body"]["content"] == "line one\nline two"
    finally:
        await context.close()


async def test_chat_browser_mobile_first_message_has_no_horizontal_overflow(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 390, "height": 844})
    page = await context.new_page()
    stub = StubbedWebChatV2(create_thread_id="thread-mobile")
    try:
        await _open_stubbed_chat(page, reborn_v2_server, stub)
        await _send_with_button(page, "mobile send")
        await expect(page).to_have_url(re.compile(r"/v2/chat/thread-mobile"), timeout=15000)
        overflow = await page.evaluate(
            "() => document.documentElement.scrollWidth > document.documentElement.clientWidth"
        )
        assert overflow is False
    finally:
        await context.close()
