"""Legacy approval-card browser coverage ported to Reborn WebChat v2."""

import asyncio
import json
from urllib.parse import unquote, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-legacy-approval"
THREAD_B_ID = "thread-legacy-approval-other"
RUN_ID = "run-legacy-approval"
GATE_REF = "gate-legacy-approval"


async def _open_stubbed_approval_thread(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    resolve_release: asyncio.Event | None = None,
    resolve_response: dict | None = None,
    thread_records: list[dict] | None = None,
    timelines: dict[str, list[dict]] | None = None,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    resolve_requests: list[dict] = []
    default_thread_records = [
        {
            "thread_id": THREAD_ID,
            "title": "Legacy approval port",
            "created_at": "2026-06-25T00:00:00Z",
            "updated_at": "2026-06-25T00:00:00Z",
        }
    ]
    default_timelines = {
        THREAD_ID: [
            {
                "message_id": "seed-user",
                "kind": "user",
                "content": "run a gated command",
                "sequence": 1,
                "status": "accepted",
                "created_at": "2026-06-25T00:00:00Z",
            }
        ]
    }
    thread_records = thread_records or default_thread_records
    timelines = timelines or default_timelines

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
          window.__emitV2Sse = (type, frame, id = "cursor-approval") => {
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

    async def fulfill_json(route, body, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def handle_session(route):
        await fulfill_json(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": USER_ID,
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_count": 4,
                    "max_file_bytes": 1048576,
                    "max_total_bytes": 4194304,
                },
            },
        )

    async def handle_threads(route):
        await fulfill_json(
            route,
            {
                "threads": thread_records,
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        parsed = urlparse(route.request.url)
        thread_id = unquote(parsed.path.split("/threads/", 1)[1].split("/timeline", 1)[0])
        await fulfill_json(
            route,
            {
                "messages": timelines.get(thread_id, []),
                "next_cursor": None,
            },
        )

    async def handle_resolve(route):
        resolve_requests.append(
            {
                "url": route.request.url,
                "body": json.loads(route.request.post_data or "{}"),
            }
        )
        if resolve_release is not None:
            await resolve_release.wait()
        await fulfill_json(
            route,
            resolve_response
            or {
                "thread_id": THREAD_ID,
                "run_id": RUN_ID,
                "status": "completed",
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route("**/api/webchat/v2/threads/*/timeline**", handle_timeline)
    await page.route(
        f"**/api/webchat/v2/threads/{THREAD_ID}/runs/**/gates/**/resolve",
        handle_resolve,
    )

    await page.goto(f"{reborn_v2_server}/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
    await expect(page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "run a gated command", timeout=15000
    )

    return context, page, resolve_requests


async def _emit_approval_gate(page, *, allow_always=True, gate_ref=GATE_REF):
    long_command = "python - <<'PY'\n" + "print('approval payload line')\n" * 28 + "PY"
    await page.evaluate(
        """
        (prompt) => window.__emitV2Sse("gate", { prompt })
        """,
        {
            "turn_run_id": RUN_ID,
            "gate_ref": gate_ref,
            "invocation_id": "invoke-legacy-approval",
            "headline": "Approval required",
            "body": "Allow shell to inspect the workspace?",
            "allow_always": allow_always,
            "approval_context": {
                "tool_name": "builtin.shell",
                "reason": "Allow shell to inspect the workspace?",
                "action": {"label": "Run command", "preview": long_command},
                "destination": {"label": "Local workspace"},
                "scope": {"label": "Workspace"},
                "details": [
                    {"label": "Command", "value": long_command},
                    {"label": "Directory", "value": "[workspace]/reborn-approval"},
                ],
            },
        },
    )
    return long_command


async def test_reborn_legacy_approval_card_renders_details_and_expands_payload(
    reborn_v2_server, reborn_v2_browser
):
    context, page, _resolve_requests = await _open_stubbed_approval_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        long_command = await _emit_approval_gate(page)

        card = page.locator(SEL_V2["approval_card"]).first
        await expect(card).to_be_visible(timeout=5000)
        await expect(card).to_contain_text("Approval required")
        await expect(card).to_contain_text("builtin.shell")
        await expect(card).to_contain_text("Allow shell to inspect the workspace?")
        await expect(card).to_contain_text("Command")
        await expect(card).to_contain_text("Directory")
        await expect(card).to_contain_text("[workspace]/reborn-approval")
        await expect(card.get_by_role("button", name="Approve")).to_be_visible()
        await expect(card.get_by_role("button", name="Deny")).to_be_visible()
        await expect(card.get_by_label("Always allow builtin.shell without asking")).to_be_visible()

        await expect(card).not_to_contain_text(long_command[-40:])
        await card.get_by_role("button", name="View full command").click()
        await expect(card.get_by_role("button", name="Show preview")).to_be_visible()
        await expect(card).to_contain_text(long_command[-40:])
        await card.get_by_role("button", name="Show preview").click()
        await expect(card.get_by_role("button", name="View full command")).to_be_visible()
        await expect(card).not_to_contain_text(long_command[-40:])
    finally:
        await context.close()


async def test_reborn_legacy_approval_buttons_resolve_gate(
    reborn_v2_server, reborn_v2_browser
):
    context, page, resolve_requests = await _open_stubbed_approval_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_approval_gate(page, allow_always=True, gate_ref="gate-approve")
        card = page.locator(SEL_V2["approval_card"]).first
        await card.get_by_role("button", name="Approve").click()
        await expect(card).to_be_hidden(timeout=5000)

        assert len(resolve_requests) == 1
        assert f"/threads/{THREAD_ID}/runs/{RUN_ID}/gates/gate-approve/resolve" in (
            resolve_requests[0]["url"]
        )
        assert resolve_requests[0]["body"]["resolution"] == "approved"
        assert resolve_requests[0]["body"]["always"] is False
        assert resolve_requests[0]["body"]["client_action_id"]

        await _emit_approval_gate(page, allow_always=True, gate_ref="gate-always")
        card = page.locator(SEL_V2["approval_card"]).first
        await card.get_by_label("Always allow builtin.shell without asking").check()
        await card.get_by_role("button", name="Approve & always allow").click()
        await expect(card).to_be_hidden(timeout=5000)

        assert len(resolve_requests) == 2
        assert f"/threads/{THREAD_ID}/runs/{RUN_ID}/gates/gate-always/resolve" in (
            resolve_requests[1]["url"]
        )
        assert resolve_requests[1]["body"]["resolution"] == "approved"
        assert resolve_requests[1]["body"]["always"] is True

        await _emit_approval_gate(page, allow_always=False, gate_ref="gate-deny")
        card = page.locator(SEL_V2["approval_card"]).first
        await card.get_by_role("button", name="Deny").click()
        await expect(card).to_be_hidden(timeout=5000)

        assert len(resolve_requests) == 3
        assert f"/threads/{THREAD_ID}/runs/{RUN_ID}/gates/gate-deny/resolve" in (
            resolve_requests[2]["url"]
        )
        assert resolve_requests[2]["body"]["resolution"] == "declined"
        assert resolve_requests[2]["body"]["always"] is False
    finally:
        await context.close()


async def test_reborn_legacy_approval_deny_shows_declined_activity(
    reborn_v2_server, reborn_v2_browser
):
    context, page, resolve_requests = await _open_stubbed_approval_thread(
        reborn_v2_server,
        reborn_v2_browser,
        resolve_response={
            "thread_id": THREAD_ID,
            "run_id": RUN_ID,
            "status": "queued",
            "outcome": "resumed",
        },
    )
    try:
        await _emit_approval_gate(page, allow_always=False, gate_ref="gate-denied-visible")

        card = page.locator(SEL_V2["approval_card"]).first
        await card.get_by_role("button", name="Deny").click()
        await expect(card).to_be_hidden(timeout=5000)

        declined_activity = page.locator(
            SEL_V2["tool_activity_card_for"].format(name="shell")
        ).filter(has_text="declined")
        await expect(declined_activity).to_be_visible(timeout=5000)
        await expect(declined_activity).to_have_attribute("data-tool-status", "declined")
        await expect(declined_activity).to_contain_text("gate_declined")

        assert len(resolve_requests) == 1
        assert f"/threads/{THREAD_ID}/runs/{RUN_ID}/gates/gate-denied-visible/resolve" in (
            resolve_requests[0]["url"]
        )
        assert resolve_requests[0]["body"]["resolution"] == "declined"
        assert resolve_requests[0]["body"]["always"] is False
    finally:
        await context.close()


async def test_reborn_legacy_approval_actions_disable_while_resolving(
    reborn_v2_server, reborn_v2_browser
):
    resolve_release = asyncio.Event()
    context, page, resolve_requests = await _open_stubbed_approval_thread(
        reborn_v2_server,
        reborn_v2_browser,
        resolve_release=resolve_release,
    )
    try:
        await _emit_approval_gate(page, allow_always=True, gate_ref="gate-disable")
        card = page.locator(SEL_V2["approval_card"]).first

        always = card.get_by_label("Always allow builtin.shell without asking")
        await always.check()
        approve = card.get_by_role("button", name="Approve & always allow")
        deny = card.get_by_role("button", name="Deny")
        await approve.click()

        await expect(approve).to_be_disabled(timeout=5000)
        await expect(deny).to_be_disabled()
        await expect(always).to_be_disabled()
        assert len(resolve_requests) == 1
        assert resolve_requests[0]["body"]["resolution"] == "approved"
        assert resolve_requests[0]["body"]["always"] is True

        resolve_release.set()
        await expect(card).to_be_hidden(timeout=5000)
    finally:
        resolve_release.set()
        await context.close()


async def test_reborn_legacy_pending_approval_blocks_send_without_error_message(
    reborn_v2_server, reborn_v2_browser
):
    """Port pending-approval send rejection to Reborn's local gate blocker."""
    context, page, _resolve_requests = await _open_stubbed_approval_thread(
        reborn_v2_server, reborn_v2_browser
    )
    send_requests: list[dict] = []

    async def handle_send(route):
        send_requests.append(json.loads(route.request.post_data or "{}"))
        await route.fulfill(
            status=500,
            content_type="application/json",
            body=json.dumps({"error": "send should stay locally blocked"}),
        )

    await page.route(f"**/api/webchat/v2/threads/{THREAD_ID}/messages", handle_send)

    try:
        await _emit_approval_gate(page, allow_always=False, gate_ref="gate-block-send")
        await expect(page.locator(SEL_V2["approval_card"]).first).to_be_visible(
            timeout=5000
        )

        status_text = page.get_by_text(
            "Resolve the approval request before sending another message.",
            exact=True,
        ).first
        await expect(status_text).to_be_visible(timeout=5000)
        await expect(page.locator(SEL_V2["busy_gate_notice"])).to_have_count(0)
        await expect(
            page.locator(SEL_V2["msg_system"]).filter(
                has_text="Resolve the approval request"
            )
        ).to_have_count(0)
        await expect(
            page.locator(SEL_V2["msg_assistant"]).filter(
                has_text="Resolve the approval request"
            )
        ).to_have_count(0)
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(
                has_text="Resolve the approval request"
            )
        ).to_have_count(0)

        composer = page.locator(SEL_V2["chat_composer"])
        await expect(composer).to_have_attribute("data-send-disabled", "true")
        await composer.fill("send while approval is pending")
        await composer.press("Enter")

        await expect(composer).to_have_value("send while approval is pending")
        await expect(page.locator(SEL_V2["msg_user"])).to_have_count(1)
        await expect(page.locator(SEL_V2["msg_system"])).to_have_count(0)
        await expect(page.locator(SEL_V2["msg_assistant"])).to_have_count(0)
        assert send_requests == []
    finally:
        await context.close()


async def test_reborn_legacy_pending_approval_does_not_block_other_thread(
    reborn_v2_server, reborn_v2_browser
):
    """Port other-thread approval isolation to Reborn's per-thread gate state."""
    context, page, _resolve_requests = await _open_stubbed_approval_thread(
        reborn_v2_server,
        reborn_v2_browser,
        thread_records=[
            {
                "thread_id": THREAD_ID,
                "title": "Approval Thread A",
                "created_at": "2026-06-25T00:00:00Z",
                "updated_at": "2026-06-25T00:00:00Z",
            },
            {
                "thread_id": THREAD_B_ID,
                "title": "Approval Thread B",
                "created_at": "2026-06-25T00:01:00Z",
                "updated_at": "2026-06-25T00:01:00Z",
            },
        ],
        timelines={
            THREAD_ID: [
                {
                    "message_id": "thread-a-user",
                    "kind": "user",
                    "content": "run a gated command",
                    "sequence": 1,
                    "status": "accepted",
                    "created_at": "2026-06-25T00:00:00Z",
                }
            ],
            THREAD_B_ID: [
                {
                    "message_id": "thread-b-user",
                    "kind": "user",
                    "content": "thread b is clear",
                    "sequence": 1,
                    "status": "accepted",
                    "created_at": "2026-06-25T00:01:00Z",
                }
            ],
        },
    )
    send_requests: list[dict] = []

    async def handle_send_b(route):
        send_requests.append(json.loads(route.request.post_data or "{}"))
        await route.fulfill(
            status=202,
            content_type="application/json",
            body=json.dumps(
                {
                    "thread_id": THREAD_B_ID,
                    "run_id": "run-thread-b-send",
                    "status": "queued",
                }
            ),
        )

    await page.route(f"**/api/webchat/v2/threads/{THREAD_B_ID}/messages", handle_send_b)

    try:
        await _emit_approval_gate(page, allow_always=False, gate_ref="gate-thread-a")
        await expect(page.locator(SEL_V2["approval_card"]).first).to_be_visible(
            timeout=5000
        )
        await expect(page.locator(SEL_V2["chat_composer"])).to_have_attribute(
            "data-send-disabled",
            "true",
        )

        await page.locator(SEL_V2["sidebar_button"]).filter(
            has_text="Approval Thread B"
        ).first.click()
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="thread b is clear")
        ).to_be_visible(timeout=15000)
        await expect(page.locator(SEL_V2["approval_card"])).to_have_count(0)

        composer = page.locator(SEL_V2["chat_composer"])
        await expect(composer).to_have_attribute(
            "data-send-disabled",
            "false",
            timeout=5000,
        )
        await composer.fill("send from thread b")
        await composer.press("Enter")

        await expect(page.locator(SEL_V2["msg_user"]).last).to_contain_text(
            "send from thread b",
            timeout=10000,
        )
        assert len(send_requests) == 1
        assert send_requests[0]["content"] == "send from thread b"
    finally:
        await context.close()


async def test_reborn_legacy_bare_approval_keywords_send_as_chat_without_gate(
    reborn_v2_page,
):
    """Port of the no-pending-gate approval-keyword regression to Reborn."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])

    for keyword in ("yes", "no", "always"):
        user_count = await reborn_v2_page.locator(SEL_V2["msg_user"]).count()
        assistant_count = await reborn_v2_page.locator(SEL_V2["msg_assistant"]).count()

        await expect(composer).to_have_attribute(
            "data-send-disabled",
            "false",
            timeout=15000,
        )
        await composer.fill(keyword)
        await composer.press("Enter")

        await expect(reborn_v2_page.locator(SEL_V2["msg_user"])).to_have_count(
            user_count + 1,
            timeout=10000,
        )
        await expect(reborn_v2_page.locator(SEL_V2["msg_user"]).last).to_contain_text(
            keyword
        )
        await expect(reborn_v2_page.locator(SEL_V2["msg_assistant"])).to_have_count(
            assistant_count + 1,
            timeout=15000,
        )
        await expect(reborn_v2_page.locator(SEL_V2["approval_card"])).to_have_count(0)
