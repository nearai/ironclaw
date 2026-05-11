"""Regression coverage for #3317 — Telegram pairing chat-claim flow.

The user-visible bug: the Telegram bot's pairing reply said
"Enter this code in IronClaw to pair your Telegram account: <code>" without
naming a specific surface, so users naturally pasted the code into their
TUI/CLI chat. The agent rejected it ("wrong place; send it in Telegram"),
leaving them stuck.

This scenario asserts both halves of the fix:

1. The pairing reply now lists every IronClaw surface explicitly
   (Settings → Channels, agent chat, terminal CLI), so the user knows
   exactly where to type the code.

2. Typing ``approve telegram <code>`` in any chat surface — including
   the gateway's `/api/chat/send` — actually completes the pairing,
   matching the bot reply's instructions.

Without this coverage, the surface-explicit reply could quietly regress
to a generic "Enter this code in IronClaw" wording, or the chat-claim
parser could be unhooked from the bridge handler, and #3317 would
silently come back.
"""

import asyncio
import json
import time

import pytest

from helpers import api_post, sse_stream

from .test_telegram_e2e import (
    _LOCAL_TELEGRAM_WASM,
    PAIRED_USER_ID,
    WEBHOOK_SECRET,
    _next_test_update_id,
    activate_telegram,
    extract_pairing_code,
    post_telegram_webhook,
    reset_fake_tg,
    wait_for_sent_messages,
)


async def _send_and_collect_response(
    base_url: str,
    *,
    thread_id: str,
    content: str,
    predicate,
    timeout: float = 30.0,
) -> str:
    """Send a chat message and return the matching `response` SSE event.

    `Submission::PairingClaim` is handled by the bridge layer and the reply
    is delivered through `WebChannel::respond` → `AppEvent::Response` over
    SSE only — no `Turn` is persisted, so polling `/api/chat/history`
    cannot see it. The chat-surface tests therefore have to listen on the
    same event stream the browser/TUI does, and the SSE stream must be
    open *before* the send so the response event isn't missed in the
    fan-out window.
    """
    matched: list[str] = []

    async def collect():
        async with sse_stream(base_url, timeout=timeout + 5) as resp:
            # Note we're connected; the bridge response is broadcast after
            # this point.
            collect_started.set()
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                try:
                    line_bytes = await asyncio.wait_for(
                        resp.content.readline(), timeout=remaining
                    )
                except asyncio.TimeoutError:
                    break
                if not line_bytes:
                    break
                line = line_bytes.decode("utf-8", errors="replace").strip()
                if not line.startswith("data:"):
                    continue
                try:
                    event = json.loads(line[5:].strip())
                except json.JSONDecodeError:
                    continue
                if event.get("type") != "response":
                    continue
                if event.get("thread_id") != thread_id:
                    continue
                content_field = event.get("content", "")
                if predicate(content_field):
                    matched.append(content_field)
                    return

    collect_started = asyncio.Event()
    collector = asyncio.create_task(collect())
    try:
        # Wait for the SSE stream to attach so the broadcast doesn't fan
        # out to zero subscribers before the send completes.
        await asyncio.wait_for(collect_started.wait(), timeout=10)

        send_r = await api_post(
            base_url,
            "/api/chat/send",
            json={"content": content, "thread_id": thread_id},
            timeout=30,
        )
        assert send_r.status_code in (200, 202), (
            f"chat send failed ({send_r.status_code}): {send_r.text}"
        )

        await asyncio.wait_for(collector, timeout=timeout + 5)
    finally:
        if not collector.done():
            collector.cancel()
            try:
                await collector
            except (asyncio.CancelledError, Exception):
                pass

    assert matched, (
        f"No matching `response` SSE event arrived within {timeout}s "
        f"for thread {thread_id}"
    )
    return matched[0]


async def test_telegram_pairing_reply_names_every_surface(
    isolated_telegram_e2e_server,
):
    """The bot's pairing reply must name web Settings, agent chat, and CLI.

    A regression that drops any of those three surfaces would re-create
    the ambiguity that #3317 surfaced (user pastes code into TUI, no
    handler matches, agent improvises an unhelpful reply).

    The reply text lives inside the Telegram WASM channel binary, so this
    assertion only runs when a locally-built WASM is available to overlay
    onto the registry-downloaded artifact. CI workflows that don't build
    `channels-src/telegram/` skip this scenario; the canary lane in
    `scripts/live_canary/auth_registry.py` covers the same wording
    against the deployed binary.
    """
    if not _LOCAL_TELEGRAM_WASM.exists():
        pytest.skip(
            "Locally-built Telegram WASM not present at "
            f"{_LOCAL_TELEGRAM_WASM} — pairing-reply wording asserts "
            "source-tree text and can't be exercised against the registry "
            "artifact."
        )
    base_url = isolated_telegram_e2e_server["base_url"]
    http_url = isolated_telegram_e2e_server["http_url"]
    fake_tg_url = isolated_telegram_e2e_server["fake_tg_url"]
    channels_dir = isolated_telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)
    await reset_fake_tg(fake_tg_url)

    # Trigger the pairing reply by DM'ing the bot from an unknown user.
    pairing_resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": _next_test_update_id(),
            "message": {
                "message_id": 5001,
                "from": {
                    "id": PAIRED_USER_ID,
                    "is_bot": False,
                    "first_name": "Pairing Tester",
                },
                "chat": {"id": PAIRED_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert pairing_resp.status_code == 200

    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    pairing_text = next(
        (m["text"] for m in reversed(messages) if "pair" in m.get("text", "").lower()),
        None,
    )
    assert pairing_text, f"No pairing-reply text found in: {messages}"

    # Every surface must be named so users know where the code is valid.
    assert "Settings" in pairing_text and "Channels" in pairing_text, (
        f"pairing reply must mention Settings → Channels: {pairing_text}"
    )
    assert "approve telegram" in pairing_text, (
        f"pairing reply must mention chat-surface command 'approve telegram': "
        f"{pairing_text}"
    )
    assert "ironclaw pairing approve telegram" in pairing_text, (
        f"pairing reply must mention CLI fallback: {pairing_text}"
    )

    # Telegram itself must NOT be advertised as a chat surface for the
    # `approve telegram CODE` command. The recipient is by definition
    # unpaired, so their DMs are intercepted by the allowlist gate in
    # the WASM channel before the agent parser ever sees the command —
    # they'd just get another pairing reply. Pairing approval requires
    # an already-authenticated IronClaw surface (web / TUI / CLI).
    # Reference: review on PR #3381.
    assert "TUI / web / Telegram" not in pairing_text, (
        "pairing reply must not list Telegram itself as a chat surface "
        f"for the approve command: {pairing_text}"
    )

    code = extract_pairing_code(messages)
    assert code, f"Expected pairing code in reply, got: {pairing_text}"
    assert code.isalnum() and code.isupper(), (
        f"pairing code must be alphanumeric uppercase, got: {code!r}"
    )


async def test_chat_surface_approves_pairing_code(
    isolated_telegram_e2e_server,
):
    """Typing `approve telegram CODE` in chat completes pairing end-to-end.

    The chat surface here is the web gateway's `/api/chat/send`, but the
    same parser runs for TUI/CLI/Telegram-itself. We then verify the
    paired user can actually exchange messages — proving the pairing
    propagated to the running WASM channel via
    `complete_pairing_approval`, not just the DB row.
    """
    base_url = isolated_telegram_e2e_server["base_url"]
    http_url = isolated_telegram_e2e_server["http_url"]
    fake_tg_url = isolated_telegram_e2e_server["fake_tg_url"]
    channels_dir = isolated_telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)
    await reset_fake_tg(fake_tg_url)

    # Step 1 — DM the bot from an unknown user to mint a pairing code.
    pairing_resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": _next_test_update_id(),
            "message": {
                "message_id": 6001,
                "from": {
                    "id": PAIRED_USER_ID,
                    "is_bot": False,
                    "first_name": "Chat-Claim Tester",
                },
                "chat": {"id": PAIRED_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert pairing_resp.status_code == 200

    pairing_messages = await wait_for_sent_messages(
        fake_tg_url, min_count=1, timeout=60
    )
    code = extract_pairing_code(pairing_messages)
    assert code, f"Expected pairing code, got messages: {pairing_messages}"
    await reset_fake_tg(fake_tg_url)

    # Step 2 — Submit the pairing claim through the chat surface that
    # users naturally try first. This is the exact path #3317 said was
    # rejected before the fix.
    thread_r = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    thread_r.raise_for_status()
    thread_id = thread_r.json()["id"]

    pairing_response = await _send_and_collect_response(
        base_url,
        thread_id=thread_id,
        content=f"approve telegram {code}",
        predicate=lambda c: (
            "Pairing approved" in c
            or "Pairing was approved" in c
            or "Invalid or expired pairing code" in c
        ),
        timeout=30,
    )

    assert "Pairing approved" in pairing_response, (
        f"Expected successful pairing, got: {pairing_response}"
    )
    assert "telegram" in pairing_response, (
        f"Pairing response must name the channel: {pairing_response}"
    )

    # Step 3 — Prove the pairing actually propagated: the previously-
    # unknown PAIRED_USER_ID should now exchange messages without
    # triggering another pairing reply.
    await reset_fake_tg(fake_tg_url)
    paired_resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": _next_test_update_id(),
            "message": {
                "message_id": 6002,
                "from": {
                    "id": PAIRED_USER_ID,
                    "is_bot": False,
                    "first_name": "Chat-Claim Tester",
                },
                "chat": {"id": PAIRED_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello again",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert paired_resp.status_code == 200

    follow_up_messages = await wait_for_sent_messages(
        fake_tg_url, min_count=1, timeout=60
    )
    follow_up_text = "\n".join(m.get("text", "") for m in follow_up_messages)
    assert "approve telegram" not in follow_up_text, (
        f"Paired user must not receive another pairing reply, got: {follow_up_text}"
    )
    assert any(
        m.get("chat_id") == PAIRED_USER_ID for m in follow_up_messages
    ), (
        f"Expected at least one reply addressed to PAIRED_USER_ID after pairing, "
        f"got: {follow_up_messages}"
    )


async def test_chat_surface_rejects_invalid_pairing_code(
    isolated_telegram_e2e_server,
):
    """Garbage codes get a clear 'invalid or expired' response, not a stuck thread.

    A second regression class #3317 hinted at: silent rejection. If the
    chat handler just routed bad codes back to the LLM, the user would
    again see an improvised "wrong place" reply. The handler must
    distinguish "valid syntax, unknown code" from "garbage input" and
    surface that as a normal turn response.
    """
    base_url = isolated_telegram_e2e_server["base_url"]
    http_url = isolated_telegram_e2e_server["http_url"]
    fake_tg_url = isolated_telegram_e2e_server["fake_tg_url"]
    channels_dir = isolated_telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)
    await reset_fake_tg(fake_tg_url)

    thread_r = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    thread_r.raise_for_status()
    thread_id = thread_r.json()["id"]

    invalid_response = await _send_and_collect_response(
        base_url,
        thread_id=thread_id,
        content="approve telegram NOSUCHCODE99",
        predicate=lambda c: "Invalid or expired pairing code" in c,
        timeout=30,
    )

    assert "Invalid or expired pairing code" in invalid_response, (
        f"Invalid pairing claim must surface a clear rejection, got: {invalid_response}"
    )


async def test_telegram_dm_approve_command_is_intercepted_by_allowlist_gate(
    isolated_telegram_e2e_server,
):
    """An unpaired Telegram user cannot complete pairing from Telegram itself.

    Reviewer concern on PR #3381: if the bot's pairing reply tells users
    they can type `approve telegram CODE` "in any IronClaw chat
    (TUI / web / Telegram)", a user following that instruction *back into
    Telegram* gets the message intercepted by `handle_message`'s
    allowlist gate before the agent parser ever sees it — they just get
    another pairing reply.

    The fix is to remove Telegram from the surfaces the bot promises.
    This test locks in the channel-layer behavior that motivates the
    fix: an unpaired DM containing `approve telegram CODE` must NOT
    complete pairing — it must be re-issued as a pairing reply (or, at
    worst, ignored). It exercises the Telegram webhook path, not
    `/api/chat/send`, so it covers exactly the layer the reviewer
    flagged.
    """
    if not _LOCAL_TELEGRAM_WASM.exists():
        pytest.skip(
            "Locally-built Telegram WASM not present at "
            f"{_LOCAL_TELEGRAM_WASM} — channel-layer interception is in "
            "the WASM binary and can't be exercised against the registry "
            "artifact."
        )
    base_url = isolated_telegram_e2e_server["base_url"]
    http_url = isolated_telegram_e2e_server["http_url"]
    fake_tg_url = isolated_telegram_e2e_server["fake_tg_url"]
    channels_dir = isolated_telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)
    await reset_fake_tg(fake_tg_url)

    # Step 1 — DM the bot to mint a pairing code.
    pairing_resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": _next_test_update_id(),
            "message": {
                "message_id": 7001,
                "from": {
                    "id": PAIRED_USER_ID,
                    "is_bot": False,
                    "first_name": "Allowlist Gate Tester",
                },
                "chat": {"id": PAIRED_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert pairing_resp.status_code == 200

    pairing_messages = await wait_for_sent_messages(
        fake_tg_url, min_count=1, timeout=60
    )
    code = extract_pairing_code(pairing_messages)
    assert code, f"Expected pairing code, got messages: {pairing_messages}"
    await reset_fake_tg(fake_tg_url)

    # Step 2 — Same unpaired user types `approve telegram CODE` back
    # into the Telegram DM. The allowlist gate in `handle_message`
    # must intercept this before the agent parser sees it.
    claim_resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": _next_test_update_id(),
            "message": {
                "message_id": 7002,
                "from": {
                    "id": PAIRED_USER_ID,
                    "is_bot": False,
                    "first_name": "Allowlist Gate Tester",
                },
                "chat": {"id": PAIRED_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": f"approve telegram {code}",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert claim_resp.status_code == 200

    follow_up_messages = await wait_for_sent_messages(
        fake_tg_url, min_count=1, timeout=60
    )
    follow_up_text = "\n".join(m.get("text", "") for m in follow_up_messages)

    # The user must NOT see "Pairing approved" — they're still unpaired,
    # the channel layer correctly didn't route their DM to the agent.
    assert "Pairing approved" not in follow_up_text, (
        "An unpaired Telegram DM must not complete pairing approval; "
        f"got: {follow_up_text}"
    )
    # And they should see *another* pairing reply (the channel layer
    # treats every unauthorized DM as a fresh pairing request).
    assert "Pair this Telegram account" in follow_up_text, (
        "Unpaired Telegram DM must produce another pairing reply, not "
        f"silent intake: {follow_up_text}"
    )
