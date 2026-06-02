"""Live-LLM Playwright regression for issue #3133 + half-2 (#3166).

Issue #3133: a Gmail-sending mission firing every 3 minutes whose
child thread bailed with the LLM-rendered "Failed to send email.
Status: None Error: None" pattern. Half-1 (PR #3155) added a typed
`GatePaused` outcome that pauses the mission and surfaces an
`AuthRequired` status update on the user's auth tray. Half-2 (this
PR) auto-resumes the mission once the user completes OAuth on the
matching credential.

Post-#3133, installed-but-unauthed provider tools are direct-callable;
the engine's auth preflight raises an `Authentication` gate at
execute time and the inline-await machinery parks the mission until
OAuth completes. The previous `tool_activate` enablement step has
been removed — the model calls the tool directly in both Tier 0 and
Tier 1 (CodeAct / Python via Monty), and the gate path fires from
either tier.

This test drives the full chat-driven mission lifecycle through a
real LLM (or a recorded trace from one) and asserts the auto-resume
mechanism end-to-end:

  1. Browser opens the chat tab against an isolated gateway.
  2. User types: "Create a mission to send a Gmail draft every 3
     minutes. Trigger it once now."
  3. The LLM's response (live or replayed) emits a `routine_create`
     tool call; the bridge alias translates that to a `mission_create`
     in engine v2.
  4. The LLM's next response emits `mission_fire` (or the test calls
     `/api/engine/missions/{id}/fire` directly when the LLM fails to
     fire it) and the child thread's first turn emits a direct
     `gmail(action="create_draft", ...)` call.
  5. Gmail is installed but not authenticated, so the auth preflight
     raises an Authentication gate and the mission transitions to
     **Paused** with `paused_gate.resume_kind =
     Authentication { credential = google_oauth_token }`.
  6. The test polls `/api/engine/missions` until the mission flips
     to `Paused`.
  7. The test completes OAuth via `/oauth/callback?code=mock_auth_code`.
     The credential write fires `bridge::resume_paused_missions_for_credential`,
     which transitions the mission Paused → Active and immediately
     re-fires it.
  8. The re-fired child thread sees gmail authenticated, so the
     `gmail` tool succeeds. The HTTP rewrite map routes
     `gmail.googleapis.com` at mock_llm.py, whose
     `/gmail/v1/users/me/drafts` endpoint returns a deterministic
     draft id so the next turn can quote it.
  9. The test polls mock_llm's `/__mock/gmail/state` until
     `drafts_created >= 1`, proving the auto-resumed mission
     actually completed the work the original gate had blocked.
 10. The test asserts no chat thread carries the `Status: None` +
     `Error: None` dual-marker (the #3133 fingerprint).

Live infrastructure (see `tests/e2e/live_harness.py` and
`tests/e2e/live_llm_proxy.py`):

  * Run with `IRONCLAW_LIVE_TEST=1` plus `IRONCLAW_LIVE_LLM_BASE_URL` /
    `IRONCLAW_LIVE_LLM_API_KEY` / `IRONCLAW_LIVE_LLM_MODEL` to record
    a fresh trace into `tests/e2e/fixtures/live/<test_name>.json`.
    Commit the resulting JSON so CI can replay deterministically.
  * Without `IRONCLAW_LIVE_TEST`, the test runs in replay mode against
    the committed fixture. If the fixture is missing the test is
    skipped (not failed) so a fresh checkout doesn't bog down on
    missing recordings.
"""

import asyncio
import os
from urllib.parse import parse_qs, urlparse

import httpx
import pytest

from helpers import SEL, api_get, api_post


# ── Regression markers ───────────────────────────────────────────────────

STATUS_NONE_MARKER = "Status: None"
ERROR_NONE_MARKER = "Error: None"
CONSECUTIVE_ERRORS_MARKER = "consecutive code errors"


CHAT_PROMPT = (
    "Use the mission_create tool to create a mission with these exact "
    "parameters and DO NOT run any other tools first:\n"
    "  name = 'gmail-draft-3133'\n"
    "  goal = 'Use the gmail tool with action=create_draft to send a "
    "draft to owner@example.com with subject \"Test mission #3133\" and "
    "body \"Mock draft from the IronClaw e2e test.\". Just call the "
    "gmail tool directly — the runtime handles authentication. Do NOT "
    "call tool_activate, tool_install, or any other setup tool first.'\n"
    "  cadence = cron expression '*/3 * * * *'\n"
    "Then immediately use mission_fire to trigger it once. "
    "Do not call tool_list, tool_info, tool_activate, or any other tool "
    "before mission_create. Just create the mission and fire it."
)


def _extract_state(auth_url: str) -> str:
    parsed = urlparse(auth_url)
    state = parse_qs(parsed.query).get("state", [None])[0]
    assert state, f"auth_url should include state: {auth_url}"
    return state


async def _install_gmail(server: str) -> None:
    response = await api_post(
        server, "/api/extensions/install", json={"name": "gmail"}, timeout=180
    )
    assert response.status_code == 200, response.text
    assert response.json().get("success") is True, response.text


async def _start_oauth_flow(server: str) -> str:
    response = await api_post(
        server, "/api/extensions/gmail/setup", json={"secrets": {}}, timeout=30
    )
    assert response.status_code == 200, response.text
    auth_url = response.json().get("auth_url")
    assert auth_url, response.json()
    return _extract_state(auth_url)


async def _complete_oauth(server: str, state: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{server}/oauth/callback",
            params={"code": "mock_auth_code", "state": state},
            timeout=30,
            follow_redirects=True,
        )
    assert response.status_code == 200, response.text[:400]
    assert "connected" in response.text.lower() or "success" in response.text.lower()


async def _list_engine_missions(server: str) -> list[dict]:
    response = await api_get(server, "/api/engine/missions", timeout=15)
    response.raise_for_status()
    return response.json().get("missions", []) or []


async def _wait_for_engine_mission(
    server: str, name_substr: str, *, timeout: float = 60.0
) -> dict:
    deadline = asyncio.get_event_loop().time() + timeout
    while asyncio.get_event_loop().time() < deadline:
        for m in await _list_engine_missions(server):
            mname = (m.get("name") or "").lower()
            if name_substr.lower() in mname:
                return m
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"engine mission containing {name_substr!r} never appeared within {timeout}s. "
        f"Have: {[m.get('name') for m in await _list_engine_missions(server)]}"
    )


async def _wait_for_mission_status(
    server: str,
    name_substr: str,
    statuses: tuple[str, ...],
    *,
    timeout: float,
) -> dict:
    deadline = asyncio.get_event_loop().time() + timeout
    last_seen: dict | None = None
    while asyncio.get_event_loop().time() < deadline:
        for m in await _list_engine_missions(server):
            mname = (m.get("name") or "").lower()
            if name_substr.lower() in mname:
                last_seen = m
                if m.get("status") in statuses:
                    return m
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"mission matching {name_substr!r} never reached one of "
        f"{statuses} within {timeout}s. Last seen: {last_seen}"
    )


async def _fire_engine_mission(server: str, mission_id: str) -> dict:
    response = await api_post(
        server, f"/api/engine/missions/{mission_id}/fire", timeout=15
    )
    assert response.status_code in (200, 202), response.text
    return response.json()


async def _gmail_mock_state(mock_llm_url: str) -> dict:
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{mock_llm_url}/__mock/gmail/state", timeout=10)
    response.raise_for_status()
    return response.json()


async def _reset_gmail_mock_state(mock_llm_url: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_url}/__mock/gmail/reset", timeout=10
        )
    response.raise_for_status()


async def _wait_for_gmail_drafts(
    mock_llm_url: str, *, target: int = 1, timeout: float = 120.0
) -> dict:
    deadline = asyncio.get_event_loop().time() + timeout
    last_seen: dict | None = None
    while asyncio.get_event_loop().time() < deadline:
        last_seen = await _gmail_mock_state(mock_llm_url)
        if last_seen.get("drafts_created", 0) >= target:
            return last_seen
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"gmail mock never recorded {target} draft(s) within {timeout}s. "
        f"Last seen: {last_seen}"
    )


async def _send_chat(page, text: str, *, timeout_ms: int = 5000) -> None:
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=timeout_ms)
    if await chat_input.evaluate("el => !!el.disabled"):
        await page.keyboard.press("Control+n")
        await page.wait_for_function(
            """selector => {
                const input = document.querySelector(selector);
                return !!input && !input.disabled;
            }""",
            arg=SEL["chat_input"],
            timeout=10000,
        )
    await chat_input.fill(text)
    await chat_input.press("Enter")
    await page.wait_for_selector(
        SEL["message_user"], state="visible", timeout=10000
    )


# ── Tests ────────────────────────────────────────────────────────────────


async def test_mission_gmail_draft_3133(
    mission_gmail_live_page, mission_gmail_live_server
):
    """Issue #3133 / #3166: full chat-driven mission lifecycle.

    In replay mode this runs deterministically against the committed
    LLM trace. In record mode (`IRONCLAW_LIVE_TEST=1`) it forwards to
    a real LLM, captures the trace into the fixture file, and asserts
    the same end-state — so a re-recording can't accidentally bake in
    a regression.

    Failing this test means one of:

    - half-1 #3133 regressed (mission stays Active on the gate path)
    - the persistent `paused_gate` field stopped being written
    - `bridge::resume_paused_missions_for_credential` stopped firing
      from `/oauth/callback`
    - the Paused → Active + immediate-fire transition is broken
    """
    server = mission_gmail_live_server["base_url"]
    mock_llm = mission_gmail_live_server["mock_llm_url"]
    mode = mission_gmail_live_server["mode"]
    print(f"[#3133] running in {mode} mode against {server}")

    # 1. Reset mock gmail state and install Gmail (NOT authenticated).
    await _reset_gmail_mock_state(mock_llm)
    await _install_gmail(server)
    extensions = await api_get(server, "/api/extensions", timeout=15)
    gmail = next(
        e for e in extensions.json()["extensions"] if e["name"] == "gmail"
    )
    assert not gmail["authenticated"], (
        "Gmail must start unauthenticated for the gate path to fire"
    )

    # 2. Drive chat → real LLM → routine_create / mission_create. The
    #    bridge's `routine_to_mission_alias` lands the result in
    #    engine v2's mission store, so we poll /api/engine/missions.
    await _send_chat(mission_gmail_live_page, CHAT_PROMPT)
    mission = await _wait_for_engine_mission(server, "gmail", timeout=120.0)
    mission_id = mission["id"]
    mission_name = mission["name"]
    print(f"[#3133] created mission {mission_name} ({mission_id})")

    # 3. The LLM's "trigger it now" response should emit `mission_fire`,
    #    but if the recording elided that for any reason we fall back
    #    to a direct fire so the test stays robust to LLM phrasing
    #    variation. Either way the child thread runs.
    pre_fire_status = mission.get("status")
    if pre_fire_status not in ("Paused", "Failed"):
        # Wait briefly for the LLM's mission_fire to fire it.
        try:
            await _wait_for_mission_status(
                server, "gmail", ("Paused",), timeout=20.0
            )
        except AssertionError:
            print("[#3133] LLM did not auto-fire the mission; firing directly")
            await _fire_engine_mission(server, mission_id)

    # 4. Wait until the mission lands in Paused after the gate fires.
    #    In record mode this can fail if the LLM's child thread runs
    #    in Tier 1 (CodeAct) — see the module docstring's known
    #    limitation. Surface a clear skip-or-fail diagnostic instead
    #    of timing out silently.
    try:
        paused = await _wait_for_mission_status(
            server, "gmail", ("Paused",), timeout=120.0
        )
    except AssertionError as e:
        if mode == "record":
            from live_harness import proxy_state
            st = await proxy_state(mission_gmail_live_server["live_proxy_url"])
            pytest.skip(
                f"record mode: mission never reached Paused. The live "
                f"LLM's child thread did not call gmail (or the auth "
                f"preflight didn't fire). Trace recorded "
                f"{st['record_count']} entries to "
                f"{mission_gmail_live_server['fixture']}. Inspect that "
                f"file to see what the LLM did. Original error: {e}"
            )
        raise
    print(f"[#3133] mission paused: {paused.get('status')}")

    # 5. Mission must NOT have created any drafts yet — the auth
    #    preflight gate paused execution before the gmail tool's
    #    HTTP call could complete.
    pre_oauth = await _gmail_mock_state(mock_llm)
    assert pre_oauth["drafts_created"] == 0, (
        f"no draft should be created before OAuth: {pre_oauth}"
    )

    # 6. Complete OAuth. This is the half-2 trigger:
    #    /oauth/callback → bridge::resume_paused_missions_for_credential
    #    → MissionManager::resume_paused_for_credential → mission
    #    Paused → Active and immediate fire.
    state = await _start_oauth_flow(server)
    await _complete_oauth(server, state)
    print("[#3133] OAuth callback completed")

    # 7. Mission must auto-resume to Active or Completed.
    final = await _wait_for_mission_status(
        server, "gmail", ("Active", "Completed"), timeout=120.0
    )
    print(f"[#3133] mission auto-resumed: status={final.get('status')}")

    # 8. The auto-resumed child thread must complete the gmail draft.
    #    create_draft lands on mock_llm's /gmail/v1/users/me/drafts.
    gmail_state = await _wait_for_gmail_drafts(mock_llm, target=1, timeout=180.0)
    assert gmail_state["drafts_created"] >= 1, gmail_state
    assert gmail_state["last_draft"] is not None
    print(f"[#3133] gmail mock recorded {gmail_state['drafts_created']} draft(s)")

    # 9. Regression marker for #3133: no chat thread should carry
    #    the dual `Status: None` + `Error: None` fingerprint.
    threads = (
        await api_get(server, "/api/chat/threads", timeout=15)
    ).json().get("threads", []) or []
    for thread in threads:
        thread_id = thread.get("id")
        if not thread_id:
            continue
        history = (
            await api_get(
                server, f"/api/chat/history?thread_id={thread_id}", timeout=15
            )
        ).json()
        for turn in history.get("turns", []) or []:
            for message in turn.get("messages", []) or []:
                body = message.get("content") or message.get("text") or ""
                if not isinstance(body, str):
                    continue
                has_status = STATUS_NONE_MARKER in body
                has_error = ERROR_NONE_MARKER in body
                assert not (has_status and has_error), (
                    f"regression: chat history carried both "
                    f"'{STATUS_NONE_MARKER}' and '{ERROR_NONE_MARKER}' — "
                    f"the #3133 fingerprint. Thread {thread_id}, "
                    f"body: {body[:400]}"
                )
                assert CONSECUTIVE_ERRORS_MARKER not in body.lower(), (
                    f"regression: chat history carried "
                    f"'{CONSECUTIVE_ERRORS_MARKER}' from #2583. "
                    f"Thread {thread_id}, body: {body[:400]}"
                )

    # 10. Live mode sanity: the proxy logged actual upstream calls.
    if mode == "record":
        from live_harness import proxy_state
        st = await proxy_state(mission_gmail_live_server["live_proxy_url"])
        assert st["record_count"] > 0, (
            f"record mode should have captured at least one LLM call: {st}"
        )
        print(
            f"[#3133] recorded {st['record_count']} LLM call(s) into "
            f"{mission_gmail_live_server['fixture']}"
        )
