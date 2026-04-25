"""MCP prompts end-to-end coverage.

Two scenarios against the `/mcp-prompts` mock MCP server (no-auth, advertises
the `prompts` capability with `greet` + `summarize`):

1. `/prompts` slash command — install the mock server, activate it, send
   `/prompts` via `/api/chat/send`, and assert the response body lists
   both prompts grouped by server.
2. `/server:prompt-name` mention expansion — send a chat message that
   includes `/mcp_prompts:greet name=world`, subscribe to the SSE stream
   in parallel, assert the `mcp_prompts_expanded` event fires with the
   expected `prompt_names`, and assert the mock LLM's last `chat/completions`
   payload contains the `<mcp_prompt server="mcp_prompts" name="greet">`
   splice (proving the pre-LLM rewrite ran).

Uses the gateway's HTTP API throughout — no browser needed. The mock MCP
server is served by `mock_llm.py` on `/mcp-prompts` (registered alongside
the existing `/mcp` OAuth endpoint); localhost URLs skip the
`requires_auth` gate so install+activate go straight through.
"""

import asyncio
import json

import httpx
import pytest

from helpers import api_get, api_post, AUTH_TOKEN, OWNER_SCOPE_ID, auth_headers, sse_stream

MCP_SERVER_NAME = "mcp_prompts"


async def _install_and_activate_mcp_prompts(ironclaw_server: str, mock_llm_server: str) -> None:
    """Install and activate the no-auth mock MCP prompts server.

    Idempotent: if the server is already installed from a prior scenario
    in the same session, skip the install/activate calls and reuse the
    existing instance. Forcing a remove+reinstall between scenarios
    triggers a state-leakage bug in the MCP session manager that causes
    the second activation's `prompts/list` cache to come back empty —
    tracked as a follow-up outside this PR's scope.

    `auth_mcp` short-circuits when a stored access token is present
    (`is_authenticated` → true); the mock `/mcp-prompts` endpoint
    doesn't verify the Bearer value, so pre-planting a dummy token via
    the admin secrets API lets activation proceed without OAuth even
    though the current production code path defaults to OAuth-eligible
    for remote HTTPS servers. Localhost HTTP already returns false from
    `requires_auth()`, but `auth_mcp` still requires EITHER a custom
    Authorization header OR a stored token to skip the OAuth branch —
    so the token is the simpler plug.
    """
    # Idempotent: if mcp_prompts is already installed (from a prior
    # scenario in the same pytest session), reuse it. Remove+reinstall
    # triggers a separate MCP session-manager state-leakage bug.
    r = await api_get(ironclaw_server, "/api/extensions")
    if any(e["name"] == MCP_SERVER_NAME for e in r.json().get("extensions", [])):
        return

    # Pre-plant the access-token secret under the canonical name
    # (`mcp_<server>_access_token`) so `is_authenticated()` short-circuits
    # the OAuth discovery path during activation.
    async with httpx.AsyncClient() as client:
        put_secret = await client.put(
            f"{ironclaw_server}/api/admin/users/{OWNER_SCOPE_ID}"
            f"/secrets/mcp_{MCP_SERVER_NAME}_access_token",
            headers=auth_headers(),
            json={"value": "e2e-dummy-token"},
            timeout=15,
        )
    assert put_secret.status_code in (200, 201), (
        f"pre-plant secret failed: {put_secret.status_code} {put_secret.text}"
    )

    install = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={
            "name": MCP_SERVER_NAME,
            "url": f"{mock_llm_server}/mcp-prompts",
            "kind": "mcp_server",
        },
        timeout=30,
    )
    assert install.status_code == 200, f"install failed: {install.status_code} {install.text}"
    assert install.json().get("success") is True, install.text

    activate = await api_post(
        ironclaw_server,
        f"/api/extensions/{MCP_SERVER_NAME}/activate",
        timeout=30,
    )
    assert activate.status_code == 200, f"activate failed: {activate.status_code} {activate.text}"
    activate_body = activate.json()
    assert not activate_body.get("auth_url"), (
        f"activation should not trigger OAuth with pre-planted token, got: {activate_body}"
    )
    assert activate_body.get("success") is True, activate_body


async def test_prompts_slash_command_lists_prompts(ironclaw_server, mock_llm_server):
    """`/prompts` chat command returns a listing grouped by active server.

    Exercises the end-to-end path:
      user text → SubmissionParser → Agent::handle_prompts_list →
      ExtensionManager::list_prompts_for_user → McpClient::list_prompts →
      HTTP to the mock MCP server → prompts/list round-trip → response
      rendered and delivered back through the channel.
    """
    await _install_and_activate_mcp_prompts(ironclaw_server, mock_llm_server)

    # Confirm the HTTP API sees the prompts before driving the slash command
    # — this pins down whether activation or listing is at fault if the
    # slash-command assertion below fails.
    listing = await api_get(ironclaw_server, "/api/prompts", timeout=15)
    assert listing.status_code == 200, listing.text
    servers = listing.json().get("servers", [])
    assert any(s["server"] == MCP_SERVER_NAME for s in servers), (
        f"server '{MCP_SERVER_NAME}' should appear in /api/prompts, got: {servers}"
    )
    my = next(s for s in servers if s["server"] == MCP_SERVER_NAME)
    names = {p["name"] for p in my.get("prompts", [])}
    assert names == {"greet", "summarize"}, (
        f"expected greet+summarize, got: {names}"
    )

    # Subscribe to SSE before sending so we don't miss the response event.
    collected = []

    async def collect():
        try:
            async with sse_stream(ironclaw_server, timeout=30) as resp:
                while len(collected) < 40:
                    raw = await resp.content.readline()
                    if not raw:
                        break
                    line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
                    if line.startswith("data:"):
                        try:
                            collected.append(json.loads(line[5:].strip()))
                        except json.JSONDecodeError:
                            pass
        except asyncio.CancelledError:
            pass

    sse_task = asyncio.create_task(collect())
    await asyncio.sleep(0.5)  # let SSE subscribe

    # Create a fresh thread so the slash-command response is easy to find.
    thread = await api_post(ironclaw_server, "/api/chat/thread/new", json={}, timeout=15)
    assert thread.status_code == 200, thread.text
    thread_id = thread.json()["id"]

    send = await api_post(
        ironclaw_server,
        "/api/chat/send",
        json={"content": "/prompts", "thread_id": thread_id},
        timeout=30,
    )
    assert send.status_code == 202, send.text

    # `/prompts` is a SystemCommand — it bypasses turn creation and
    # delivers its rendered text through the channel as a `response`
    # SSE event, not into /api/chat/history. Poll the collected SSE
    # frames for that event and pull the text out.
    response_text = ""
    deadline = asyncio.get_running_loop().time() + 30
    while asyncio.get_running_loop().time() < deadline:
        for event in collected:
            if event.get("type") == "response":
                text = event.get("content") or event.get("text") or ""
                if "MCP prompts" in text:
                    response_text = text
                    break
        if response_text:
            break
        await asyncio.sleep(0.3)

    sse_task.cancel()
    try:
        await sse_task
    except (asyncio.CancelledError, Exception):
        pass

    assert response_text, (
        "`/prompts` response never appeared in history or SSE. "
        f"Collected SSE events: {collected[-5:]}"
    )
    assert MCP_SERVER_NAME in response_text, response_text
    assert "/mcp_prompts:greet" in response_text, response_text
    # `summarize` declares `topic` as required → formatter suffixes `*`
    # after the arg name.
    assert "/mcp_prompts:summarize" in response_text, response_text
    assert "topic*" in response_text, (
        f"summarize should render required arg with '*', got: {response_text!r}"
    )


async def test_mention_expansion_rewrites_user_message_and_fires_sse_event(
    ironclaw_server, mock_llm_server
):
    """A `/server:prompt-name key=value` mention is expanded before LLM dispatch.

    Asserts two observable side effects:
    1. `mcp_prompts_expanded` SSE event carries `["mcp_prompts:greet"]` in
       `prompt_names` (the UI activation-card hint).
    2. The mock LLM's last `chat/completions` payload contains the
       `<mcp_prompt server="mcp_prompts" name="greet">` splice plus the
       server-rendered text — proving the dispatcher rewrote the message
       between the user send and the LLM call.
    """
    await _install_and_activate_mcp_prompts(ironclaw_server, mock_llm_server)

    # Reset any prior captured LLM request so the assertion below only
    # sees this test's payload.
    async with httpx.AsyncClient() as client:
        await client.post(f"{mock_llm_server}/__mock/oauth/reset", timeout=5)

    collected = []

    async def collect():
        try:
            async with sse_stream(ironclaw_server, timeout=30) as resp:
                while len(collected) < 60:
                    raw = await resp.content.readline()
                    if not raw:
                        break
                    line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
                    if line.startswith("data:"):
                        try:
                            collected.append(json.loads(line[5:].strip()))
                        except json.JSONDecodeError:
                            pass
        except asyncio.CancelledError:
            pass

    sse_task = asyncio.create_task(collect())
    await asyncio.sleep(0.5)

    thread = await api_post(ironclaw_server, "/api/chat/thread/new", json={}, timeout=15)
    assert thread.status_code == 200, thread.text
    thread_id = thread.json()["id"]

    send = await api_post(
        ironclaw_server,
        "/api/chat/send",
        json={
            "content": "/mcp_prompts:greet name=world",
            "thread_id": thread_id,
        },
        timeout=30,
    )
    assert send.status_code == 202, send.text

    # Wait for the expansion SSE event OR the final assistant response,
    # whichever comes first. `mcp_prompts_expanded` fires pre-LLM, so it
    # should arrive well before any assistant text.
    expansion_event = None
    deadline = asyncio.get_running_loop().time() + 30
    while asyncio.get_running_loop().time() < deadline:
        for event in collected:
            if event.get("type") == "mcp_prompts_expanded":
                expansion_event = event
                break
        if expansion_event:
            break
        await asyncio.sleep(0.2)

    if expansion_event is None:
        # Debug: fetch the last LLM request to see what the dispatcher sent.
        async with httpx.AsyncClient() as dbg_client:
            dbg = await dbg_client.get(
                f"{mock_llm_server}/__mock/last_chat_request", timeout=5
            )
        raise AssertionError(
            "mcp_prompts_expanded SSE event never arrived. "
            f"Collected event types: {[e.get('type') for e in collected]}. "
            f"Last LLM request body: {dbg.text[:2000]}"
        )
    if expansion_event.get("prompt_names") != [f"{MCP_SERVER_NAME}:greet"]:
        async with httpx.AsyncClient() as dbg_client:
            mcp_state = await dbg_client.get(
                f"{mock_llm_server}/__mock/mcp/state", timeout=5
            )
            prompts_api = await api_get(ironclaw_server, "/api/prompts", timeout=5)
        raise AssertionError(
            f"Unexpected expansion_event: {expansion_event}. "
            f"Mock MCP state: {mcp_state.text[:3000]}. "
            f"/api/prompts: {prompts_api.text[:2000]}"
        )

    # Now poll the mock LLM for the rewritten payload. The dispatcher
    # splices `<mcp_prompt ...>` into the user message before the LLM
    # call, so the captured request must contain both the server-name
    # attribute and the server-rendered text (`Please greet world.`).
    rewritten_payload = None
    deadline = asyncio.get_running_loop().time() + 30
    expected_tag = f'<mcp_prompt server="{MCP_SERVER_NAME}" name="greet">'
    async with httpx.AsyncClient() as client:
        while asyncio.get_running_loop().time() < deadline:
            r = await client.get(f"{mock_llm_server}/__mock/last_chat_request", timeout=5)
            if r.status_code == 200 and r.json():
                # Search the message content strings directly — not the
                # re-serialised JSON, whose escaped quotes would break the
                # literal substring match.
                body = r.json()
                user_texts = []
                for msg in body.get("messages", []):
                    if msg.get("role") != "user":
                        continue
                    content = msg.get("content")
                    if isinstance(content, str):
                        user_texts.append(content)
                    elif isinstance(content, list):
                        user_texts.extend(
                            p.get("text", "") for p in content if isinstance(p, dict)
                        )
                joined = "\n".join(user_texts)
                if expected_tag in joined and "Please greet world." in joined:
                    rewritten_payload = body
                    break
            await asyncio.sleep(0.5)

    sse_task.cancel()
    try:
        await sse_task
    except (asyncio.CancelledError, Exception):
        pass

    assert rewritten_payload is not None, (
        "Mock LLM never received a chat request containing the expanded "
        "<mcp_prompt> splice."
    )
    # Regression guard: the raw mention literal must NOT survive into the
    # LLM payload. If it does, the dispatcher skipped the rewrite and the
    # LLM sees the user's raw slash form instead of the rendered text.
    last_user_text = ""
    for msg in rewritten_payload.get("messages", []):
        if msg.get("role") == "user":
            content = msg.get("content")
            if isinstance(content, str):
                last_user_text = content
            elif isinstance(content, list):
                last_user_text = " ".join(
                    p.get("text", "") for p in content if isinstance(p, dict)
                )
    assert "/mcp_prompts:greet" not in last_user_text, (
        "Raw mention literal leaked into the LLM payload — the dispatcher "
        f"did not rewrite. Last user text: {last_user_text!r}"
    )
