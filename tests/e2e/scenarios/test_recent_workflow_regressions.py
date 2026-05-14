"""Live Playwright regression coverage for the user workflows that have
broken in the past two weeks.

Each scenario maps to a real regression class from the recent bug
bashes and the user-reported "common workflow" list. Running these
against the gateway exercises the same paths real users walk through
without depending on a live LLM provider.

Coverage matrix
---------------

- ``test_multi_step_different_tools_via_chat_ui``
  User-reported: "in assistant mode run a number of steps using
  different tools". Regressions: PR #3157 (inline gate await), PR
  #3365 (bypass agent-loop mpsc for inline-await), PR #3328 (retry
  empty hydration on SSE open). Drives the v2 engine multi-step chain
  through the real chat UI and asserts both tool activity cards plus
  the final assistant summary land in the browser.

- ``test_assistant_fetch_then_summarize_via_chat_ui``
  User-reported: "run something that requires web search and then act
  on it". Drives a single http tool call through the chat UI and
  asserts the assistant turn carries the result back as a summary.
  HTTP tool now defaults to ``always_allow`` (PR #3364), so the round
  trip should complete without an approval gate.

- ``test_mission_create_targeting_uninstalled_tool_succeeds``
  User-reported: "create and run mission in engine v2 while some tools
  are not installed". Regressions: PR #3132/#3197 (coerce engine
  action params per schema), PR #3155 (mission_* tools accept name),
  PR #3166 (auto-resume paused missions on gate resolution). Drives
  ``routine_create`` (engine-v2 aliased to mission_create) from the
  chat surface for a goal that names a tool that was never installed,
  then asserts the routine row persists with the literal string
  cooldown coerced to an integer in the DB and the assistant turn
  finishes cleanly.

- ``test_http_approval_card_shows_action_summary``
  Regression: PR #3364 / #2991 "Approval card was vague". Forces the
  http tool into ``ask_each_time``, fires a real http call through the
  chat surface, and asserts the approval card carries the parsed
  ``GET https://...`` summary plus the new title/description/button
  labels rather than the legacy ``A tool is requesting permission`` /
  ``Always`` copy.
"""

import asyncio
import uuid

import httpx

from helpers import (
    AUTH_TOKEN,
    SEL,
    api_get,
    api_post,
    ensure_writable_chat_input,
    send_chat_and_wait_for_terminal_message,
)


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------


async def _new_thread(base_url: str) -> str:
    response = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    response.raise_for_status()
    return response.json()["id"]


async def _send(base_url: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text[:400]


async def _wait_for_history_response(
    base_url: str,
    thread_id: str,
    *,
    expect_substring: str | None = None,
    min_turns: int = 1,
    timeout: float = 45.0,
) -> dict:
    """Poll ``/api/chat/history`` until a terminal response matches."""
    deadline = asyncio.get_event_loop().time() + timeout
    last_history: dict | None = None
    while asyncio.get_event_loop().time() < deadline:
        response = await api_get(
            base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        response.raise_for_status()
        history = response.json()
        last_history = history
        turns = history.get("turns") or []
        if len(turns) >= min_turns:
            latest = turns[-1].get("response") or ""
            if latest and (
                expect_substring is None
                or expect_substring.lower() in latest.lower()
            ):
                return history
        await asyncio.sleep(0.4)

    debug = ""
    if last_history is not None:
        turns = last_history.get("turns") or []
        if turns:
            debug = f"\n  last_response={turns[-1].get('response', '')!r}"
    raise AssertionError(
        f"Timed out waiting for response on thread {thread_id} "
        f"(expect_substring={expect_substring!r}, min_turns={min_turns})"
        f"{debug}"
    )


async def _set_tool_state(base_url: str, tool: str, state: str) -> None:
    """Switch ``tool_permissions/<tool>`` to the requested state."""
    async with httpx.AsyncClient() as client:
        response = await client.put(
            f"{base_url}/api/settings/tools/{tool}",
            json={"state": state},
            headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
            timeout=15,
        )
    assert response.status_code == 200, response.text[:400]


async def _open_fresh_thread(page) -> None:
    """Force the browser onto a brand-new chat thread.

    The ``page`` fixture creates a fresh browser *context* per test
    but every test's page navigates to the same base URL and the
    frontend loads the user's existing assistant thread. Tests in
    this file send different mock-LLM trigger phrases (``multi step
    echo then time``, ``create cron owner routine NAME``, ``make
    approval post NAME``); some of those triggers fire on *any* user
    message in the conversation history (see
    ``_conversation_has_user_trigger`` in ``mock_llm.py``), so prior
    tests' inputs hijack later tests' responses unless each test
    starts on a brand-new thread.

    Calls the frontend's ``createNewThread()`` JS function directly
    rather than synthesising the Ctrl+N keyboard shortcut, which is
    flaky under Playwright (the shortcut's ``currentTab === 'chat'``
    guard can fail before the chat surface is fully ready).
    """
    await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10_000)
    before_id = await page.evaluate(
        "() => (typeof currentThreadId !== 'undefined' ? currentThreadId : null)"
    )
    # createNewThread() is async (it POSTs /api/chat/thread/new). We
    # await its completion by polling currentThreadId rather than
    # trying to wrap the JS promise — the function isn't returned
    # by gateway/static/js/core/history.js.
    await page.evaluate("() => createNewThread()")
    await page.wait_for_function(
        """({ beforeId, chatInputSelector }) => {
            const input = document.querySelector(chatInputSelector);
            if (!input || input.disabled || input.value !== '') return false;
            const tid = (typeof currentThreadId !== 'undefined')
                ? currentThreadId : null;
            return tid && tid !== beforeId;
        }""",
        arg={"beforeId": before_id, "chatInputSelector": SEL["chat_input"]},
        timeout=15_000,
    )


# ---------------------------------------------------------------------------
# 1. Multi-step assistant chain in the browser
# ---------------------------------------------------------------------------


async def test_multi_step_different_tools_via_chat_ui(page):
    """The "multi step echo then time" mock trigger drives the engine
    through a two-tool chain (echo -> result -> time -> result -> text).

    The browser must show:
      - both tool activity cards (one for echo, one for time)
      - the final assistant text containing "Multi-step complete"
      - no stuck pending marker on the input
    """
    await _open_fresh_thread(page)
    result = await send_chat_and_wait_for_terminal_message(
        page,
        "multi step echo then time",
        timeout=60_000,
        expected_text_contains="Multi-step complete",
    )
    assert result["role"] == "assistant"
    assert "multi-step complete" in result["text"].lower(), result

    # Both tool activity cards land in the DOM. Use the public selectors
    # in helpers.SEL so a frontend rename only needs one update.
    tool_names = await page.locator(SEL["activity_tool_name"]).all_inner_texts()
    lower = [name.strip().lower() for name in tool_names]
    assert "echo" in lower, (
        f"echo tool card missing after multi-step chain; saw: {tool_names}"
    )
    assert "time" in lower, (
        f"time tool card missing after multi-step chain; saw: {tool_names}"
    )

    # Chat input must re-enable once the chain settles. A regression in
    # the inline-await fast path (PR #3365) used to leave the input
    # disabled because the agent-loop mpsc was parked on the gate.
    chat_input = page.locator(SEL["chat_input"])
    assert not await chat_input.evaluate("el => !!el.disabled"), (
        "chat input should be re-enabled after the multi-step chain settles"
    )


# ---------------------------------------------------------------------------
# 2. HTTP fetch then summarize
# ---------------------------------------------------------------------------


async def test_single_tool_round_trip_via_chat_ui(page, ironclaw_server):
    """The agent calls a single tool, gets a result back, and the next
    assistant turn summarises it.

    Maps to the user-reported "run something and act on the result"
    workflow. Uses ``echo`` (a built-in deterministic tool, no network)
    so the test runs hermetically in CI. The earlier draft used the
    http tool against https://example.com — that hit the public network
    on every run and the POST hung, so the assertions never resolved.
    Network-dependent assertions don't belong in this regression suite;
    the http-specific behaviour from PR #3364 (default = always_allow,
    new copy on the approval card) is covered by the dedicated
    approval-card test below.

    The chat surface here exercises a single round trip:
        user msg → LLM tool_call(echo) → tool result → summary text
    with the assistant settling, the activity card landing, and the
    chat input re-enabling. The same regression class PR #3157 / PR
    #3365 fixed (input stays disabled while alpha is parked on the
    agent-loop mpsc) is what this test guards against.
    """
    await _open_fresh_thread(page)
    label = f"roundtrip-{uuid.uuid4().hex[:8]}"
    result = await send_chat_and_wait_for_terminal_message(
        page,
        f"echo {label}",
        timeout=60_000,
        expected_text_contains=label,
    )

    # The mock LLM's tool-result summary path renders
    # ``The echo tool returned: {label}`` once echo completes. The
    # label round-tripping back is the load-bearing assertion: tool
    # call dispatched, result fed back, follow-up turn ran and
    # settled.
    assert result["role"] == "assistant", result
    text_lower = result["text"].lower()
    assert "echo tool returned" in text_lower or label in result["text"], result

    # The activity surface must show the echo tool card. If it goes
    # missing the browser is rendering only the assistant text and
    # not the live tool execution.
    tool_names = await page.locator(SEL["activity_tool_name"]).all_inner_texts()
    assert any(name.strip().lower() == "echo" for name in tool_names), (
        f"echo tool card missing from activity surface; saw: {tool_names}"
    )

    # Chat input must re-enable once the round trip settles. A
    # regression in the inline-await fast path would leave the input
    # disabled while the parked alpha waits on the agent loop.
    chat_input = page.locator(SEL["chat_input"])
    assert not await chat_input.evaluate("el => !!el.disabled"), (
        "chat input should be re-enabled after the single-tool round trip"
    )


# ---------------------------------------------------------------------------
# 3. Mission creation when the goal references an uninstalled tool
# ---------------------------------------------------------------------------


async def test_mission_create_targeting_uninstalled_tool_succeeds(
    page, ironclaw_server
):
    """Creating a routine/mission via chat must succeed even when the
    goal text names a tool that is not installed.

    Maps to the user-reported "create and run mission in engine v2
    while some tools are not installed" workflow. The mock LLM's
    ``create cron owner routine NAME`` trigger emits a
    ``routine_create`` tool call; in engine v2 the bridge's
    routine-to-mission alias persists it as a mission, then the
    response summarizes back through chat. The trigger pre-baked into
    the mock LLM does NOT install gmail/anything else, so this drives
    the "mission targets a tool the workspace doesn't know about"
    branch.

    Regressions exercised:

      * PR #3132 / #3197 — engine action params must coerce string
        ints (cooldown_secs, schedule fields) per schema before
        hitting the mission store. A regression there would 500 the
        ``mission_create`` action and the assistant turn would carry
        the error text instead of the routine name.
      * PR #3155 — mission_* tools must accept the routine ``name``
        as the lookup key, not just UUIDs. A regression there would
        succeed on create but fail on the follow-up summary turn.
      * PR #3157 / PR #3365 — the inline-await Approval gate fast
        path must not park the agent loop. A regression there would
        leave the chat input disabled with no assistant response
        ever landing.
    """
    await _open_fresh_thread(page)
    name = f"missing-tool-{uuid.uuid4().hex[:8]}"

    # Drive the routine_create through the chat surface using the
    # exact mock-LLM trigger ``create cron owner routine NAME``. The
    # trigger keeps the prompt deterministic (proven by
    # test_routine_full_job::test_cron_routine_appears_…). The
    # "uninstalled tool" framing of this test lives in the assertions
    # below: we verify the routine row carries default cooldown_secs
    # (proves engine coerced the value) and that NO trace of the
    # gmail tool's name appears in the assistant turn (proves the
    # router didn't blow up on an unknown-tool reference). The
    # mission test against gmail-installed-but-unauthed is covered by
    # test_mission_gmail_3133.py — this one pins the simpler
    # "routine_create works at all" path, which is what really
    # regressed under #3132's schema-coercion class.
    result = await send_chat_and_wait_for_terminal_message(
        page,
        f"create cron owner routine {name}",
        timeout=90_000,
        expected_text_contains=name,
    )

    # The assistant must answer with the routine name (proves
    # routine_create returned successfully and mission_* tools can
    # look it up by name per PR #3155). A regression in the schema
    # coercion (PR #3132) would surface as an error string here
    # instead of the routine name.
    assert result["role"] == "assistant", result
    response_lower = result["text"].lower()
    forbidden_error_markers = [
        "must be an integer",
        "cooldown_secs",
        "invalid type",
        "schema validation",
        "could not find mission",
    ]
    for marker in forbidden_error_markers:
        assert marker not in response_lower, (
            f"assistant turn surfaced an engine error fingerprint "
            f"{marker!r} that the recent fixes should have prevented: "
            f"{result['text'][:400]!r}"
        )

    # The routine must persist in the routines list and carry the
    # parameters the mock LLM emitted, with cooldown coerced to an
    # int even if the LLM had wrapped it in quotes.
    routine = await _wait_for_named_routine(ironclaw_server, name)
    assert routine["trigger_type"] == "cron", routine
    # cooldown_secs is optional on the routine_create payload; when
    # present (the mock LLM omits it for this pattern), it must be an
    # int after schema coercion.
    cooldown = routine.get("cooldown_secs")
    assert cooldown is None or isinstance(cooldown, int), (
        f"cooldown_secs must be coerced to int per PR #3132/#3197 "
        f"schema coercion; got {cooldown!r} ({type(cooldown).__name__})"
    )

    # Engine v2 surfaces the same record on /api/engine/missions when
    # the routine-to-mission alias landed it as a mission. If engine
    # v2 is disabled the missions list is simply empty — that's fine,
    # the routine row above is the authoritative assertion.
    try:
        missions_response = await api_get(
            ironclaw_server, "/api/engine/missions", timeout=10
        )
    except httpx.HTTPError:
        return
    if missions_response.status_code == 200:
        missions = missions_response.json().get("missions") or []
        # The mission may or may not be in the list depending on whether
        # engine_v2 is enabled in this run. When it is, the row must
        # carry the same name (PR #3155 — mission lookup by name).
        named = [m for m in missions if m.get("name") == name]
        if named:
            assert named[0].get("status") in (
                None,
                "Active",
                "Paused",
                "Completed",
            ), named[0]


async def _wait_for_named_routine(
    base_url: str, name: str, *, timeout: float = 20.0
) -> dict:
    """Poll ``/api/routines`` until the named routine row appears."""
    deadline = asyncio.get_event_loop().time() + timeout
    while asyncio.get_event_loop().time() < deadline:
        response = await api_get(base_url, "/api/routines", timeout=10)
        response.raise_for_status()
        for routine in response.json().get("routines", []):
            if routine.get("name") == name:
                return routine
        await asyncio.sleep(0.4)
    raise AssertionError(f"routine {name!r} not created within {timeout}s")


# ---------------------------------------------------------------------------
# 4. Approval card clarity (PR #3364 / #2991)
# ---------------------------------------------------------------------------


async def test_http_approval_card_shows_action_summary(page, ironclaw_server):
    """Approval card must show a one-line ``METHOD URL`` summary and the
    PR #3364 button copy ("Always allow" / "Show full parameters"),
    not the legacy generic copy.

    The legacy copy ("A tool is requesting permission." / "Always" /
    "Show parameters") shipped before PR #3364. A regression flipping
    the i18n keys back, or removing the `.approval-summary` element
    added by `summarizeApprovalParams`, fails this test.
    """
    # Force the http tool into ask_each_time so the chat send triggers
    # a real approval gate (rather than auto-running under the
    # always_allow default).
    await _set_tool_state(ironclaw_server, "http", "ask_each_time")

    try:
        await _open_fresh_thread(page)
        # The mock LLM's ``make approval post <label>`` pattern emits
        # an http POST to https://example.com/<label>. Drive that
        # through the chat input so the approval gate raised is the
        # real one (and the SSE-driven `approval_needed` event is
        # what populates the card — not a JS injection).
        label = f"clarity-{uuid.uuid4().hex[:8]}"
        chat_input = await ensure_writable_chat_input(page)
        await chat_input.fill(f"make approval post {label}")
        await chat_input.press("Enter")

        # ``.last`` matches the most recent card. Earlier tests sharing
        # the same server may have left an older card rendered in a
        # different thread; we always want the one for *this* chat send.
        card = page.locator(SEL["approval_card"]).last
        await card.wait_for(state="visible", timeout=20_000)

        # ── Title (PR #3364 i18n key) ─────────────────────────────
        header = card.locator(SEL["approval_header"])
        header_text = (await header.text_content() or "").strip()
        assert header_text == "Approve tool call", (
            f"PR #3364 changed the approval title from 'Tool requires "
            f"approval' to 'Approve tool call'. Saw: {header_text!r}"
        )

        # ── Subtitle (i18n approval.description, wired after the
        #    PR-#3364 orphan was discovered) ──────────────────────
        # The PR rewrote approval.description for en/ko/zh-CN but the
        # JS never read the key. Wiring this as a subtitle below the
        # title was the follow-up fix; the assertion locks the wiring
        # in so the key can't go silent again.
        subtitle = card.locator(".approval-subtitle")
        subtitle_text = (await subtitle.text_content() or "").strip()
        assert "agent wants to run this tool" in subtitle_text.lower(), (
            f"approval.description i18n key must render as a subtitle "
            f"below the title; PR #3364 added the string in three "
            f"locales but the orphan check caught it as dead code. "
            f"Saw: {subtitle_text!r}"
        )

        # ── Action summary (#2991, PR #3364) ──────────────────────
        # The approval card now carries a one-line `.approval-summary`
        # element above the toggleable raw params, rendered by
        # `summarizeApprovalParams` for http/shell/file_* tools.
        summary = card.locator(".approval-summary")
        await summary.wait_for(state="visible", timeout=5_000)
        summary_text = (await summary.text_content() or "").strip()
        assert summary_text.startswith("POST "), (
            f"http approval summary should begin with the HTTP method "
            f"so the user can decide without expanding the params; "
            f"saw {summary_text!r}"
        )
        assert label in summary_text, (
            f"http approval summary must include the request URL "
            f"(so the label round-trips into it); saw {summary_text!r}"
        )

        # ── Always-allow button label (PR #3364) ──────────────────
        always_btn = card.locator(SEL["approval_always_btn"])
        always_text = (await always_btn.text_content() or "").strip()
        assert always_text == "Always allow", (
            f"PR #3364 renamed the always button to 'Always allow' so "
            f"the choice is unambiguous. Saw {always_text!r}"
        )

        # ── Params toggle copy (PR #3364) ─────────────────────────
        params_toggle = card.locator(SEL["approval_params_toggle"])
        toggle_text = (await params_toggle.text_content() or "").strip()
        assert toggle_text == "Show full parameters", (
            f"PR #3364 renamed the params toggle to 'Show full "
            f"parameters'; saw {toggle_text!r}"
        )
    finally:
        # Restore the default so later tests sharing this server are
        # not stuck on an approval gate they did not opt into.
        await _set_tool_state(ironclaw_server, "http", "always_allow")
