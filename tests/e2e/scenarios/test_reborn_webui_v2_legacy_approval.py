"""Legacy approval-card browser coverage ported to Reborn WebChat v2."""

import json

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-legacy-approval"
RUN_ID = "run-legacy-approval"
GATE_REF = "gate-legacy-approval"


async def _open_stubbed_approval_thread(reborn_v2_server, reborn_v2_browser):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    resolve_requests: list[dict] = []

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
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def handle_threads(route):
        await fulfill_json(
            route,
            {
                "threads": [
                    {
                        "thread_id": THREAD_ID,
                        "title": "Legacy approval port",
                        "created_at": "2026-06-25T00:00:00Z",
                        "updated_at": "2026-06-25T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "seed-user",
                        "kind": "user",
                        "content": "run a gated command",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-25T00:00:00Z",
                    }
                ],
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
        await fulfill_json(
            route,
            {
                "thread_id": THREAD_ID,
                "run_id": RUN_ID,
                "status": "completed",
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{THREAD_ID}/timeline**", handle_timeline)
    await page.route(
        f"**/api/webchat/v2/threads/{THREAD_ID}/runs/**/gates/**/resolve",
        handle_resolve,
    )

    await page.goto(f"{reborn_v2_server}/v2/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")
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
                    {"label": "Directory", "value": "/tmp/reborn-approval"},
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
        await expect(card).to_contain_text("/tmp/reborn-approval")
        await expect(card.get_by_role("button", name="Approve")).to_be_visible()
        await expect(card.get_by_role("button", name="Deny")).to_be_visible()
        await expect(card.get_by_label("Always allow builtin.shell without asking")).to_be_visible()

        await expect(card).not_to_contain_text(long_command[-40:])
        await card.get_by_role("button", name="View full command").click()
        await expect(card.get_by_role("button", name="Show preview")).to_be_visible()
        await expect(card).to_contain_text(long_command[-40:])
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
        assert resolve_requests[2]["body"]["resolution"] == "denied"
        assert resolve_requests[2]["body"]["always"] is False
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
