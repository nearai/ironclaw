"""Dedicated Reborn WebChat v2 smoke E2E.

This proves the *new* Reborn surface end-to-end: the `ironclaw-reborn serve`
binary (built with the `webui-v2-beta` feature) boots, serves the React SPA
under `/v2/`, authenticates a bearer caller, and runs one text turn through the
`/api/webchat/v2/*` endpoints against the deterministic mock LLM.

This is intentionally small and complements the Rust composition tests
(`crates/ironclaw_reborn_composition/tests/webui_v2_e2e.rs`), which drive the
same router in-process via `tower::ServiceExt::oneshot` with no real TCP
listener or browser. It also differs from `test_reborn_gateway_smoke.py`, which
exercises the legacy `ironclaw` web channel (`/api/chat/*`) under ENGINE_V2 —
NOT the `ironclaw-reborn` binary or the v2 webUI.

Wiring confirmed manually before this test existed:
- The v2 SPA + `serve` subcommand are gated behind `webui-v2-beta` (transitively
  enables `libsql`); the binary is `ironclaw-reborn`.
- LLM is selected via `$IRONCLAW_REBORN_HOME/config.toml` `[llm.default]`; the
  built-in `openai` provider (OpenAI `/v1/chat/completions`) is pointed at the
  mock with a `base_url` override and `api_key_env`.
- `IRONCLAW_REBORN_WEBUI_TOKEN` must be >= 32 bytes (it doubles as the SSO
  session-signing key); the user id maps the env-bearer caller.
- `NO_PROXY`/`no_proxy` must cover loopback so the provider's reqwest client
  does not route the mock request through a developer-local HTTP proxy.
"""

import asyncio
import json
import re
from enum import StrEnum
from urllib.parse import parse_qs, urlparse

import aiohttp
import httpx
import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    create_thread as _create_thread,
    fetch_timeline as _get_timeline,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
    send_and_settle as _send_and_settle,
    send_message as _send_message,
    wait_for_assistant_message as _wait_for_assistant_message,
)


class ToolPermissionState(StrEnum):
    DEFAULT = "default"
    DISABLED = "disabled"
    ASK_EACH_TIME = "ask_each_time"


async def _set_tool_permission(
    client: httpx.AsyncClient,
    base_url: str,
    capability_id: str,
    state: ToolPermissionState,
) -> None:
    response = await client.post(
        f"{base_url}/api/webchat/v2/settings/tools/{capability_id}",
        json={"state": state.value},
        timeout=15,
    )
    assert response.status_code == 200, (
        f"Failed to set {capability_id} to {state}: "
        f"{response.status_code} {response.text}"
    )


async def _restore_disabled_tool_policy(client: httpx.AsyncClient, base_url: str) -> None:
    errors = []
    for capability_id in ("builtin.echo", "builtin.shell"):
        try:
            await _set_tool_permission(
                client,
                base_url,
                capability_id,
                ToolPermissionState.DEFAULT,
            )
        except Exception as error:
            errors.append(f"{capability_id}: {error}")
    assert not errors, "Failed to restore tool permission defaults: " + "; ".join(errors)


@pytest.fixture
async def disabled_echo_shell_ask_policy(reborn_v2_server):
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        try:
            await _set_tool_permission(
                client,
                reborn_v2_server,
                "builtin.echo",
                ToolPermissionState.DISABLED,
            )
            await _set_tool_permission(
                client,
                reborn_v2_server,
                "builtin.shell",
                ToolPermissionState.ASK_EACH_TIME,
            )
            yield
        finally:
            await _restore_disabled_tool_policy(client, reborn_v2_server)


async def test_reborn_v2_serves_shell_and_gates_auth(reborn_v2_server, reborn_v2_browser):
    """The SPA renders the authed shell with a token, and the login view without one."""
    # With a valid token the authenticated chat shell renders.
    authed_ctx = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    authed_page = await authed_ctx.new_page()
    try:
        await authed_page.goto(f"{reborn_v2_server}/v2/?token={REBORN_V2_AUTH_TOKEN}")
        await expect(authed_page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
    finally:
        await authed_ctx.close()

    # Without a token the SPA falls back to the login/connect view.
    anon_ctx = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    anon_page = await anon_ctx.new_page()
    try:
        await anon_page.goto(f"{reborn_v2_server}/v2/")
        await expect(anon_page.locator(SEL_V2["login_token"])).to_be_visible(timeout=15000)
    finally:
        await anon_ctx.close()


async def test_reborn_v2_text_turn_persists(reborn_v2_server):
    """A text turn over /api/webchat/v2/* completes and persists one assistant reply."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

        prompt = "what is 2+2?"
        await _send_message(client, reborn_v2_server, thread_id, prompt)
        assistant = await _wait_for_assistant_message(client, reborn_v2_server, thread_id)
        assert "4" in assistant.get("content", "")

        # Exactly one finalized assistant message — no duplicate terminal response.
        timeline = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline",
            timeout=15,
        )
        timeline.raise_for_status()
        finalized = [
            message
            for message in timeline.json().get("messages", [])
            if message.get("kind") == "assistant"
            and message.get("status") == "finalized"
            and (message.get("content") or "").strip()
        ]
        assert len(finalized) == 1, (
            f"Expected one finalized assistant message, got {len(finalized)}: {finalized}"
        )


@pytest.mark.usefixtures("disabled_echo_shell_ask_policy")
async def test_reborn_v2_disabled_tool_does_not_route_through_shell(
    reborn_v2_server,
):
    """A named unavailable tool request should not route through another tool."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)
        prompt = "Use builtin.echo to print: disabled-test"
        await _send_message(client, reborn_v2_server, thread_id, prompt)

        assistant = await _wait_for_assistant_message(
            client,
            reborn_v2_server,
            thread_id,
            timeout=45,
        )
        assert "will not route it through another tool" in assistant.get("content", "")

        timeline = await _get_timeline(client, reborn_v2_server, thread_id)
        timeline_text = json.dumps(timeline, sort_keys=True)
        assert "builtin_shell" not in timeline_text
        assert "builtin.shell" not in timeline_text
        assert "echo \\\"disabled-test\\\"" not in timeline_text


async def test_reborn_v2_ui_send_renders_reply(reborn_v2_page, reborn_v2_server):
    """Typing in the composer and pressing Enter renders the assistant reply in the SPA."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("hello there")
    await composer.press("Enter")

    # The user bubble and the streamed assistant reply both render in the shell.
    await expect(reborn_v2_page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "hello there", timeout=15000
    )
    await expect(reborn_v2_page.locator(SEL_V2["msg_assistant"]).first).to_contain_text(
        "Hello", timeout=30000
    )


async def test_reborn_v2_composer_accepts_draft_while_run_is_processing(reborn_v2_page):
    """The composer stays editable while the current assistant run is still active."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("editable composer slow response")
    await composer.press("Enter")

    await expect(reborn_v2_page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "editable composer slow response", timeout=15000
    )
    await expect(
        reborn_v2_page.locator(SEL_V2["typing_indicator"])
    ).to_be_visible(timeout=15000)

    await expect(composer).to_be_enabled()
    await expect(composer).to_have_attribute("data-send-disabled", "true")
    await composer.fill("draft while the reply is still running")
    await expect(composer).to_have_value("draft while the reply is still running")
    await expect(composer).to_have_attribute("data-send-disabled", "true")

    await composer.press("Enter")
    await expect(reborn_v2_page.locator(SEL_V2["msg_user"])).to_have_count(1, timeout=1000)


async def test_reborn_v2_new_chat_sends_while_a_run_is_active(reborn_v2_page):
    """A new chat started while another thread's run is in flight must still send.

    Regression for the #5256 ``submitBusyRef`` deadlock: the send re-entrancy
    guard was released only on run *settlement*, delivered over the open thread's
    SSE. Starting a new chat tears down that SSE, so the settle event never
    reaches the hook, the guard stays held, and the new-chat send is silently
    dropped — "can't start a new chat while one is in progress". The guard must
    release when the send POST completes instead.

    This drives the *in-app* "+ New" button (client-side navigation), NOT a fresh
    ``page.goto`` — a full reload would remount the hook and reset the leaked ref,
    hiding the bug. Only same-instance navigation reproduces it. Unit tests cannot
    exercise the navigation + SSE-teardown lifecycle that this regression lives in.
    """
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])

    # Thread 1: a slow response keeps the run in flight (mock delays ~5s) so the
    # new chat is started before thread 1 settles — the deadlock window.
    await composer.fill("editable composer slow response")
    await composer.press("Enter")
    await expect(reborn_v2_page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "editable composer slow response", timeout=15000
    )
    await expect(
        reborn_v2_page.locator(SEL_V2["typing_indicator"])
    ).to_be_visible(timeout=15000)

    # Start a new chat via the in-app button while thread 1 is still running.
    await reborn_v2_page.locator(SEL_V2["new_chat"]).click()
    await expect(composer).to_be_visible(timeout=5000)
    # The fresh chat carries none of thread 1's messages.
    await expect(reborn_v2_page.locator(SEL_V2["msg_user"])).to_have_count(0, timeout=5000)

    # The crux: send in the new chat. With the deadlock this is dropped and no
    # bubble ever renders; with the fix it posts, creates the thread, and shows
    # the optimistic user bubble.
    await composer.fill("hi how are you")
    await composer.press("Enter")

    await expect(reborn_v2_page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "hi how are you", timeout=15000
    )
    await expect(reborn_v2_page).to_have_url(
        re.compile(r"/v2/chat/[0-9a-fA-F-]+"), timeout=15000
    )


async def test_reborn_v2_approval_gate_blocks_composer_send(
    reborn_v2_server, reborn_v2_browser
):
    """An open approval gate shows the warning and blocks new sends locally."""
    thread_id = "thread-approval-blocked"
    send_requests: list[dict] = []
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

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
                        "thread_id": thread_id,
                        "title": "Approval blocked regression",
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
                        "content": "trigger approval",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-25T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_send(route):
        send_requests.append(json.loads(route.request.post_data or "{}"))
        await fulfill_json(route, {"thread_id": thread_id}, status=202)

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/timeline**", handle_timeline)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/messages", handle_send)

    try:
        await page.goto(f"{reborn_v2_server}/v2/chat/{thread_id}?token={REBORN_V2_AUTH_TOKEN}")
        await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
        await expect(page.locator(SEL_V2["msg_user"]).first).to_contain_text(
            "trigger approval", timeout=15000
        )

        await page.evaluate(
            """
            () => window.__emitV2Sse("gate", {
              prompt: {
                turn_run_id: "run-gated",
                gate_ref: "gate-shell",
                invocation_id: "invoke-shell",
                headline: "Approval required",
                body: "Allow shell to inspect the workspace?",
                allow_always: false,
                approval_context: {
                  tool_name: "builtin.shell",
                  reason: "Allow shell to inspect the workspace?",
                  action: { label: "Run command" },
                  destination: { label: "Local workspace" },
                  details: [{ label: "Command", value: "pwd" }]
                }
              }
            })
            """
        )

        await expect(page.locator(SEL_V2["approval_card"]).first).to_be_visible(timeout=5000)
        await expect(
            page.get_by_text("Resolve the approval request before sending another message.")
        ).to_be_visible(timeout=5000)

        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("new message while approval is open")
        await composer.press("Enter")
        await expect(page.locator(SEL_V2["msg_user"])).to_have_count(1, timeout=1000)
        assert send_requests == []
    finally:
        await context.close()


async def test_reborn_v2_desktop_sidebar_can_collapse_and_persist(reborn_v2_page):
    """Desktop users can collapse the sidebar, and the preference survives reload."""
    sidebar = reborn_v2_page.locator(SEL_V2["sidebar"])
    toggle = reborn_v2_page.locator(SEL_V2["sidebar_toggle"])

    await expect(toggle).to_be_visible(timeout=15000)
    await expect(sidebar).to_be_visible(timeout=15000)

    await toggle.click()
    await expect(sidebar).to_be_hidden(timeout=5000)
    await reborn_v2_page.wait_for_function(
        "() => localStorage.getItem('ironclaw:v2-sidebar-open') === 'false'",
        timeout=5000,
    )

    await reborn_v2_page.reload()
    await expect(reborn_v2_page.locator(SEL_V2["chat_composer"])).to_be_visible(
        timeout=15000
    )
    await expect(sidebar).to_be_hidden(timeout=5000)

    await toggle.click()
    await expect(sidebar).to_be_visible(timeout=5000)
    await reborn_v2_page.wait_for_function(
        "() => localStorage.getItem('ironclaw:v2-sidebar-open') === 'true'",
        timeout=5000,
    )


async def test_reborn_v2_messages_omit_identity_labels(reborn_v2_page):
    """User and assistant messages render content without persistent identity labels."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("hello there")
    await composer.press("Enter")

    # Message bubbles retain content while omitting redundant identity labels.
    user_bubble = reborn_v2_page.locator(SEL_V2["msg_user"]).first
    await expect(user_bubble).to_contain_text("hello there", timeout=15000)
    await expect(user_bubble).not_to_contain_text("You")

    assistant_bubble = reborn_v2_page.locator(SEL_V2["msg_assistant"]).first
    await expect(assistant_bubble).to_contain_text("Hello", timeout=30000)
    await expect(assistant_bubble).not_to_contain_text("IronClaw")


async def test_reborn_v2_response_links_open_in_new_tab(reborn_v2_page):
    """Links inside an assistant reply open in a new tab."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("link test")
    await composer.press("Enter")

    link = (
        reborn_v2_page.locator(SEL_V2["msg_assistant"])
        .get_by_role("link", name="the pull request")
    )
    await expect(link).to_be_visible(timeout=30000)
    assert await link.get_attribute("target") == "_blank", "link must open in a new tab"
    rel = await link.get_attribute("rel") or ""
    assert "noopener" in rel, f"link must be noopener, got rel={rel!r}"


async def test_reborn_v2_logs_page_passes_scope_to_api_and_renders_context(
    reborn_v2_page, reborn_v2_server
):
    """The browser logs route passes URL scope to the API and renders scoped entries."""
    requested_queries: list[dict[str, list[str]]] = []
    logs_requested = asyncio.Event()

    async def handle_operator_logs(route):
        parsed = urlparse(route.request.url)
        requested_queries.append(parse_qs(parsed.query))
        logs_requested.set()
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "status": "available",
                    "logs": {
                        "source": "in_memory_tracing",
                        "entries": [
                            {
                                "id": "ui-log-1",
                                "timestamp": "2026-06-12T10:11:12.123Z",
                                "level": "info",
                                "target": "ironclaw::ui::logs",
                                "message": "scoped log from browser fixture",
                                "thread_id": "thread-ui",
                                "run_id": "run-ui",
                                "tool_call_id": "tool-call-ui",
                                "tool_name": "shell",
                                "source": "slack",
                            }
                        ],
                        "next_cursor": None,
                        "tail_supported": True,
                        "follow_supported": False,
                    },
                }
            ),
        )

    await reborn_v2_page.route("**/api/webchat/v2/operator/logs**", handle_operator_logs)
    await reborn_v2_page.goto(
        f"{reborn_v2_server}/v2/logs"
        "?thread_id=thread-ui&run_id=run-ui&tool_call_id=tool-call-ui&source=slack"
    )

    await asyncio.wait_for(logs_requested.wait(), timeout=10)
    first_query = requested_queries[0]
    assert first_query.get("thread_id") == ["thread-ui"], first_query
    assert first_query.get("run_id") == ["run-ui"], first_query
    assert first_query.get("tool_call_id") == ["tool-call-ui"], first_query
    assert first_query.get("source") == ["slack"], first_query
    assert first_query.get("limit") == ["500"], first_query

    await expect(
        reborn_v2_page.locator(SEL_V2["logs_scope_toolbar"])
    ).to_be_visible(timeout=10000)
    await expect(
        reborn_v2_page.locator(SEL_V2["logs_scope_chip"].format(key="thread_id"))
    ).to_contain_text("thread-ui")
    await expect(
        reborn_v2_page.locator(SEL_V2["logs_scope_chip"].format(key="run_id"))
    ).to_contain_text("run-ui")

    entry = reborn_v2_page.locator(SEL_V2["logs_entry"]).first
    await expect(entry.locator(SEL_V2["logs_entry_message"])).to_contain_text(
        "scoped log from browser fixture"
    )

    await entry.locator(SEL_V2["logs_entry_row"]).click()
    context = entry.locator(SEL_V2["logs_entry_context"])
    await expect(
        context.locator(SEL_V2["logs_context_chip"].format(key="tool_call_id"))
    ).to_contain_text("tool-call-ui")
    await expect(
        context.locator(SEL_V2["logs_context_chip"].format(key="tool_name"))
    ).to_contain_text("shell")
    await expect(
        context.locator(SEL_V2["logs_context_chip"].format(key="source"))
    ).to_contain_text("slack")


async def test_reborn_v2_thread_list_and_delete(reborn_v2_server):
    """Threads are listed for the caller and deletion removes the thread and transcript."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        keep_id = await _create_thread(client, reborn_v2_server)
        drop_id = await _create_thread(client, reborn_v2_server)

        listed = await client.get(f"{reborn_v2_server}/api/webchat/v2/threads", timeout=15)
        listed.raise_for_status()
        ids = {thread["thread_id"] for thread in listed.json().get("threads", [])}
        assert {keep_id, drop_id} <= ids, f"both threads should be listed, got {ids}"

        deleted = await client.request(
            "DELETE", f"{reborn_v2_server}/api/webchat/v2/threads/{drop_id}", timeout=15
        )
        assert deleted.status_code == 200, deleted.text

        # Transcript is gone (404, not an empty timeline) and re-delete is idempotent-404.
        gone = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{drop_id}/timeline", timeout=15
        )
        assert gone.status_code == 404, gone.text
        re_delete = await client.request(
            "DELETE", f"{reborn_v2_server}/api/webchat/v2/threads/{drop_id}", timeout=15
        )
        assert re_delete.status_code == 404, re_delete.text

        relisted = await client.get(f"{reborn_v2_server}/api/webchat/v2/threads", timeout=15)
        relisted.raise_for_status()
        remaining = {thread["thread_id"] for thread in relisted.json().get("threads", [])}
        assert drop_id not in remaining, "deleted thread must not reappear in the list"
        assert keep_id in remaining, "untouched thread must remain in the list"


async def test_reborn_v2_timeline_pagination(reborn_v2_server):
    """Timeline honors `limit` and pages older messages via the opaque `next_cursor`."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

        # Two settled turns -> >= 4 messages, enough to force a second page at limit=2.
        await _send_and_settle(client, reborn_v2_server, thread_id, "hello one", expected=1)
        await _send_and_settle(client, reborn_v2_server, thread_id, "hello two", expected=2)

        page1 = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline",
            params={"limit": 2},
            timeout=15,
        )
        page1.raise_for_status()
        page1_body = page1.json()
        assert len(page1_body["messages"]) == 2, page1_body
        cursor = page1_body.get("next_cursor")
        assert cursor, f"a thread with >2 messages must expose next_cursor: {page1_body}"

        # httpx URL-encodes the opaque cursor (it is JSON like {"before_message_sequence":N}).
        page2 = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline",
            params={"limit": 2, "cursor": cursor},
            timeout=15,
        )
        page2.raise_for_status()
        page2_body = page2.json()
        assert page2_body["messages"], f"cursor page must return older messages: {page2_body}"

        page1_seq = {m["sequence"] for m in page1_body["messages"]}
        page2_seq = {m["sequence"] for m in page2_body["messages"]}
        assert page1_seq.isdisjoint(page2_seq), (
            f"paged messages must not overlap: page1={page1_seq} page2={page2_seq}"
        )


async def test_reborn_v2_sse_streams_run_lifecycle(reborn_v2_server):
    """The SSE stream opens via the `?token=` shim and reports the run reaching completion.

    The browser's `EventSource` cannot set an `Authorization` header, so
    `GET /events` accepts `?token=` instead of a bearer (the only v2 route that
    does). The stream is projection-based: it carries run lifecycle status
    (`queued` -> `running` -> `completed`), not the reply text.
    """
    bearer = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=bearer) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

    events_url = (
        f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/events"
        f"?token={REBORN_V2_AUTH_TOKEN}"
    )
    client_timeout = aiohttp.ClientTimeout(total=45, sock_read=45)
    async with aiohttp.ClientSession(timeout=client_timeout) as session:
        # No Authorization header — only the `?token=` query shim authenticates.
        async with session.get(
            events_url, headers={"Accept": "text/event-stream"}
        ) as response:
            assert response.status == 200, (
                f"events stream must open via ?token= shim, got {response.status}"
            )

            # Submit the turn now that the stream is live, then read lifecycle frames.
            async with httpx.AsyncClient(headers=bearer) as client:
                await _send_message(client, reborn_v2_server, thread_id, "hello sse")

            async with asyncio.timeout(45):
                while True:
                    raw = await response.content.readline()
                    assert raw, "SSE stream closed before the run completed"
                    line = raw.decode("utf-8", errors="replace")
                    if '"status":"completed"' in line:
                        return


async def test_reborn_v2_bearer_auth_and_token_shim_scope(reborn_v2_server):
    """v2 routes require a bearer; the `?token=` shim authenticates only the events route."""
    bearer = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=bearer) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

    async with httpx.AsyncClient() as anon:
        # No credentials at all -> 401 on session, list, and timeline.
        for path in (
            "/api/webchat/v2/session",
            "/api/webchat/v2/threads",
            f"/api/webchat/v2/threads/{thread_id}/timeline",
        ):
            response = await anon.get(f"{reborn_v2_server}{path}", timeout=15)
            assert response.status_code == 401, f"{path} without bearer: {response.status_code}"

        # A malformed bearer is rejected.
        bad = await anon.get(
            f"{reborn_v2_server}/api/webchat/v2/session",
            headers={"Authorization": "Bearer not-a-valid-token"},
            timeout=15,
        )
        assert bad.status_code == 401, bad.text

        # The `?token=` shim must NOT authenticate a non-events route (timeline).
        shimmed = await anon.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline"
            f"?token={REBORN_V2_AUTH_TOKEN}",
            timeout=15,
        )
        assert shimmed.status_code == 401, (
            f"?token= must not authenticate timeline, got {shimmed.status_code}"
        )
