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
import os
import signal
import socket
import uuid
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import aiohttp
import httpx
import pytest
from playwright.async_api import async_playwright, expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2, wait_for_ready

USER_ID = "reborn-v2-e2e-user"


def _find_free_port() -> int:
    """Ask the OS for an available loopback port (startup hint; bind is retried)."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _read_log(path: Path, limit: int = 8192) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")[-limit:]
    except OSError:
        return ""


def _forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value


async def _stop_process(proc, *, sig=signal.SIGINT, timeout: float = 10) -> None:
    """Signal a subprocess and wait for exit without re-reading stdio pipes."""
    if proc.returncode is not None:
        return
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await asyncio.wait_for(proc.wait(), timeout=5)


def _write_config_toml(path: Path, mock_llm_server: str) -> None:
    """Seed a sparse Reborn config that selects the mock LLM via the `openai` provider.

    The built-in `openai` provider speaks the OpenAI Chat Completions wire shape
    (`/v1/chat/completions`) that `mock_llm.py` serves. The `base_url` override
    points it at the mock; `api_key_env` names an env var the server fixture sets.
    Secrets stay out of the file — only the env-var NAME is referenced.
    """
    path.write_text(
        f"""api_version = "ironclaw.runtime/v1"

[boot]
profile = "local-dev"

[identity]
default_owner = "{USER_ID}"
tenant = "reborn-v2-e2e"
default_agent = "reborn-v2-e2e-agent"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "mock-model"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "{mock_llm_server}/v1"
""",
        encoding="utf-8",
    )


@pytest.fixture(scope="module")
async def reborn_v2_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Start `ironclaw-reborn serve` with the v2 surface against the mock LLM."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-home")
    reborn_home = home_dir / "reborn-home"
    reborn_home.mkdir(parents=True, exist_ok=True)
    _write_config_toml(reborn_home / "config.toml", mock_llm_server)

    proc = None
    base_url = None
    last_stderr = ""
    last_port = None

    for attempt in range(1, 4):
        port = _find_free_port()
        last_port = port
        stdout_path = home_dir / f"reborn-v2-attempt-{attempt}.stdout.log"
        stderr_path = home_dir / f"reborn-v2-attempt-{attempt}.stderr.log"

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home_dir),
            "IRONCLAW_REBORN_HOME": str(reborn_home),
            "IRONCLAW_REBORN_PROFILE": "local-dev",
            "IRONCLAW_REBORN_WEBUI_TOKEN": REBORN_V2_AUTH_TOKEN,
            "IRONCLAW_REBORN_WEBUI_USER_ID": USER_ID,
            "MOCK_LLM_API_KEY": "mock-api-key",
            # Keep the provider's reqwest client off any developer-local HTTP
            # proxy so the loopback mock request is not intercepted (502).
            "NO_PROXY": "127.0.0.1,localhost,::1",
            "no_proxy": "127.0.0.1,localhost,::1",
            "RUST_LOG": "ironclaw=warn,ironclaw_reborn=warn",
            "RUST_BACKTRACE": "1",
        }
        _forward_coverage_env(env)

        with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
            proc = await asyncio.create_subprocess_exec(
                ironclaw_reborn_binary,
                "serve",
                "--host", "127.0.0.1",
                "--port", str(port),
                stdin=asyncio.subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                env=env,
            )
        base_url = f"http://127.0.0.1:{port}"

        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            break
        except TimeoutError:
            if proc.returncode is None:
                await _stop_process(proc, timeout=2)
            last_stderr = _read_log(stderr_path)
            proc = None
    else:
        pytest.fail(
            "Reborn WebUI v2 server failed to start after 3 attempts.\n"
            f"Last attempted port: {last_port}\n"
            f"stderr:\n{last_stderr}"
        )

    try:
        yield base_url
    finally:
        if proc is not None and proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


@pytest.fixture(scope="module")
async def reborn_v2_browser():
    """Chromium instance for the v2 smoke (independent of the legacy gateway).

    Chromium's cold start can contend with the binary build and server startup,
    so launch with a generous timeout and one retry rather than letting the
    default 30s connect window flake the whole module.
    """
    from playwright.async_api import Error as PlaywrightError

    headless = os.environ.get("HEADED", "").strip() not in ("1", "true")
    async with async_playwright() as p:
        browser = None
        for attempt in range(3):
            try:
                browser = await p.chromium.launch(headless=headless, timeout=60000)
                break
            except PlaywrightError:
                if attempt == 2:
                    raise
                await asyncio.sleep(1)
        yield browser
        await browser.close()


@pytest.fixture
async def reborn_v2_page(reborn_v2_server, reborn_v2_browser):
    """Fresh authed page on the v2 SPA, navigated past the login/connect view."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{reborn_v2_server}/v2/?token={REBORN_V2_AUTH_TOKEN}")
    # Authenticated landing route is /chat; the composer is the shell anchor.
    await page.wait_for_selector(SEL_V2["chat_composer"], timeout=15000)
    yield page
    await context.close()


def _client_action_id() -> str:
    """Idempotency key accepted by `webui_inbound::parse_client_action_id`."""
    return str(uuid.uuid4())


async def _create_thread(client: httpx.AsyncClient, base_url: str) -> str:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads",
        json={"client_action_id": _client_action_id()},
        timeout=15,
    )
    response.raise_for_status()
    return response.json()["thread"]["thread_id"]


async def _send_message(
    client: httpx.AsyncClient, base_url: str, thread_id: str, content: str
) -> None:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/messages",
        json={"client_action_id": _client_action_id(), "content": content},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text


async def _wait_for_assistant_message(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Poll the timeline until a finalized assistant message appears."""
    last_timeline: dict = {}
    for _ in range(int(timeout * 2)):
        response = await client.get(
            f"{base_url}/api/webchat/v2/threads/{thread_id}/timeline",
            timeout=15,
        )
        response.raise_for_status()
        last_timeline = response.json()
        finalized = [
            message
            for message in last_timeline.get("messages", [])
            if message.get("kind") == "assistant"
            and message.get("status") == "finalized"
            and (message.get("content") or "").strip()
        ]
        if finalized:
            return finalized[-1]
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for a finalized assistant message in thread {thread_id}. "
        f"Last timeline: {last_timeline}"
    )


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


async def test_reborn_v2_messages_show_identity_labels(reborn_v2_page):
    """User and assistant messages render a persistent identity label."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("hello there")
    await composer.press("Enter")

    # The user bubble carries the "You" identity alongside its content.
    user_bubble = reborn_v2_page.locator(SEL_V2["msg_user"]).first
    await expect(user_bubble).to_contain_text("hello there", timeout=15000)
    await expect(user_bubble).to_contain_text("You")

    # The assistant bubble carries the "IronClaw" identity (the canned reply
    # text itself never contains that string).
    assistant_bubble = reborn_v2_page.locator(SEL_V2["msg_assistant"]).first
    await expect(assistant_bubble).to_contain_text("IronClaw", timeout=30000)


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


def _finalized_assistant_count(timeline: dict) -> int:
    return sum(
        1
        for message in timeline.get("messages", [])
        if message.get("kind") == "assistant"
        and message.get("status") == "finalized"
        and (message.get("content") or "").strip()
    )


async def _send_and_settle(
    client: httpx.AsyncClient, base_url: str, thread_id: str, content: str, expected: int
) -> None:
    """Send a text turn and wait until `expected` assistant replies are finalized.

    Sending while a prior turn is still running defers the message
    (`deferred_busy`), so each turn must settle before the next is sent.
    """
    await _send_message(client, base_url, thread_id, content)
    for _ in range(90):
        response = await client.get(
            f"{base_url}/api/webchat/v2/threads/{thread_id}/timeline", timeout=15
        )
        response.raise_for_status()
        if _finalized_assistant_count(response.json()) >= expected:
            return
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Thread {thread_id} did not reach {expected} finalized assistant replies"
    )


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
