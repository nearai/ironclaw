"""End-to-end approval-gate tests for non-web channels.

Inline mid-turn approval gates (PR #3157) pause the engine inside the
running CodeAct script and surface the prompt back to the originating
channel via ``BridgeGateController::emit_gate_prompt``. These tests
verify the flow for the two bundled WASM channels that the e2e harness
already wires up — Telegram and Slack — covering:

- DM ``yes`` / ``no`` / ``always`` reply resolves the gate.
- ``always`` persists ``always_allow`` to the DB so a second tool call
  to the same tool auto-approves without a prompt.
- Cross-channel resolution: a gate raised from a Telegram or Slack
  message can be approved from the web ``/api/chat/approval`` endpoint.

The non-web web-equivalent file is ``test_v2_engine_approval_flow.py``;
this file deliberately reuses the ``"make approval post <label>"`` mock
LLM pattern so the same shape (``http`` POST → approval) exercises
non-web channel surface.
"""

from __future__ import annotations

import asyncio
import time
from typing import Any

import httpx
import pytest

from helpers import api_get, api_post, AUTH_TOKEN, auth_headers

# Reuse helpers from the per-channel scenarios so this file stays focused
# on the gate-resolution assertions and doesn't reimplement webhook
# plumbing or pairing.
from .test_telegram_e2e import (
    OWNER_USER_ID as TG_OWNER_USER_ID,
    WEBHOOK_SECRET as TG_WEBHOOK_SECRET,
    _next_test_update_id,
    activate_telegram,
    post_telegram_webhook,
    reset_fake_tg,
)
from .test_slack_e2e import (
    OWNER_USER_ID as SL_OWNER_USER_ID,
    activate_slack,
    build_slack_dm_event,
    build_slack_mention_event,
    post_slack_webhook,
    reset_fake_slack,
)


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------


async def _set_tool_permission(base_url: str, tool_name: str, state: str) -> None:
    """Pin the given tool to ``state`` so approval prompts are deterministic.

    The seeded default for ``http`` is ``always_allow``. Channel approval
    tests need the explicit ``ask_each_time`` path, just like the web-only
    flow in ``test_v2_engine_approval_flow.py``.
    """
    async with httpx.AsyncClient() as client:
        response = await client.put(
            f"{base_url}/api/settings/tools/{tool_name}",
            json={"state": state},
            headers=auth_headers(),
            timeout=15,
        )
    assert response.status_code == 200, (
        f"Failed to set {tool_name} permission to {state}: "
        f"{response.status_code} {response.text}"
    )


async def _find_channel_thread(
    base_url: str,
    *,
    channel: str,
    timeout: float = 30,
) -> str:
    """Return the most recently active thread id whose channel matches.

    Telegram/Slack open one thread per chat under the same user. Tests
    don't know the engine-assigned UUID up-front, so look it up by the
    ``channel`` field that ``/api/chat/threads`` already exposes.
    """
    deadline = time.monotonic() + timeout
    last_payload: Any = None
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            r = await client.get(
                f"{base_url}/api/chat/threads",
                headers=auth_headers(),
                timeout=10,
            )
            if r.status_code == 200:
                data = r.json()
                last_payload = data
                threads = data.get("threads", [])
                matching = [t for t in threads if t.get("channel") == channel]
                if matching:
                    matching.sort(
                        key=lambda t: t.get("updated_at") or "", reverse=True
                    )
                    return matching[0]["id"]
            await asyncio.sleep(0.5)
    raise AssertionError(
        f"No thread with channel='{channel}' appeared within {timeout}s. "
        f"Last /api/chat/threads payload: {last_payload!r}"
    )


async def _wait_for_pending_gate(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 60,
) -> dict:
    """Poll history until ``pending_gate`` is set; return the gate dict."""
    deadline = time.monotonic() + timeout
    last_history: Any = None
    while time.monotonic() < deadline:
        r = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        r.raise_for_status()
        last_history = r.json()
        pending = last_history.get("pending_gate")
        if pending and pending.get("request_id"):
            return pending
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for pending_gate on thread {thread_id}. "
        f"Last history: {last_history!r}"
    )


async def _wait_for_no_pending_gate(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 60,
) -> dict:
    """Poll history until ``pending_gate`` clears; return the final history."""
    deadline = time.monotonic() + timeout
    last_history: Any = None
    while time.monotonic() < deadline:
        r = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        r.raise_for_status()
        last_history = r.json()
        if not last_history.get("pending_gate"):
            return last_history
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for pending_gate to clear on thread {thread_id}. "
        f"Last history: {last_history!r}"
    )


async def _read_tool_permission(base_url: str, tool_name: str) -> str | None:
    """Return the persisted ``tool_permissions.<tool>`` value or None."""
    r = await api_get(
        base_url, f"/api/settings/tool_permissions.{tool_name}", timeout=15
    )
    if r.status_code != 200:
        return None
    return r.json().get("value")


# ---------------------------------------------------------------------------
# Telegram-specific helpers
# ---------------------------------------------------------------------------


def _telegram_message_update(text: str, *, message_id: int) -> dict:
    """Build a Telegram webhook update for the test owner DM."""
    return {
        "update_id": _next_test_update_id(),
        "message": {
            "message_id": message_id,
            "from": {
                "id": TG_OWNER_USER_ID,
                "is_bot": False,
                "first_name": "E2E Tester",
            },
            "chat": {"id": TG_OWNER_USER_ID, "type": "private"},
            "date": int(time.time()),
            "text": text,
        },
    }


async def _send_tg(http_url: str, text: str, *, message_id: int) -> None:
    resp = await post_telegram_webhook(
        http_url,
        _telegram_message_update(text, message_id=message_id),
        secret=TG_WEBHOOK_SECRET,
    )
    assert resp.status_code == 200, (
        f"Telegram webhook returned {resp.status_code}: {resp.text}"
    )


# ---------------------------------------------------------------------------
# Slack-specific helpers
# ---------------------------------------------------------------------------


async def _send_slack_dm(http_url: str, text: str) -> None:
    resp = await post_slack_webhook(
        http_url, build_slack_dm_event(SL_OWNER_USER_ID, text)
    )
    assert resp.status_code == 200, (
        f"Slack webhook returned {resp.status_code}: {resp.text}"
    )


async def _send_slack_mention(http_url: str, text: str, *, channel: str = "C0001") -> str:
    ts = f"{time.time():.6f}"
    resp = await post_slack_webhook(
        http_url,
        build_slack_mention_event(
            SL_OWNER_USER_ID, text, channel=channel, ts=ts
        ),
    )
    assert resp.status_code == 200, (
        f"Slack mention webhook returned {resp.status_code}: {resp.text}"
    )
    return ts


# ---------------------------------------------------------------------------
# Telegram fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
async def telegram_with_ask_each_time(telegram_e2e_server):
    """Activate Telegram and pin the http tool to ask_each_time."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)
    await reset_fake_tg(fake_tg_url)
    # The seeded default for `http` is `always_allow`; force the
    # ask-each-time path so these tests deterministically exercise the
    # inline-await gate prompt instead of the auto-approve fast path.
    await _set_tool_permission(base_url, "http", "ask_each_time")
    return telegram_e2e_server


# ---------------------------------------------------------------------------
# Slack fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
async def slack_with_ask_each_time(slack_e2e_server):
    """Activate Slack and pin the http tool to ask_each_time."""
    base_url = slack_e2e_server["base_url"]
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]
    channels_dir = slack_e2e_server["channels_dir"]

    await activate_slack(base_url, http_url, fake_slack_url, channels_dir)
    await reset_fake_slack(fake_slack_url)
    await _set_tool_permission(base_url, "http", "ask_each_time")
    return slack_e2e_server


# ---------------------------------------------------------------------------
# Telegram approval-gate tests
# ---------------------------------------------------------------------------


async def test_telegram_dm_approval_yes_resolves_inline_gate(
    telegram_with_ask_each_time,
):
    """Telegram DM 'yes' reply approves a paused inline gate."""
    base_url = telegram_with_ask_each_time["base_url"]
    http_url = telegram_with_ask_each_time["http_url"]

    await _send_tg(http_url, "make approval post tg-alpha", message_id=100)
    thread_id = await _find_channel_thread(base_url, channel="telegram")
    pending = await _wait_for_pending_gate(base_url, thread_id)
    assert pending["tool_name"] == "http", pending

    await _send_tg(http_url, "yes", message_id=101)
    history = await _wait_for_no_pending_gate(base_url, thread_id)
    # Sanity: a turn record exists for the original prompt.
    assert history.get("turns"), history


async def test_telegram_dm_approval_no_denies_inline_gate(
    telegram_with_ask_each_time,
):
    """Telegram DM 'no' reply denies the gate; pending_gate clears."""
    base_url = telegram_with_ask_each_time["base_url"]
    http_url = telegram_with_ask_each_time["http_url"]

    await _send_tg(http_url, "make approval post tg-deny", message_id=110)
    thread_id = await _find_channel_thread(base_url, channel="telegram")
    await _wait_for_pending_gate(base_url, thread_id)

    await _send_tg(http_url, "no", message_id=111)
    await _wait_for_no_pending_gate(base_url, thread_id)

    # `no` must NOT install always_allow.
    perm = await _read_tool_permission(base_url, "http")
    assert perm in (None, "ask_each_time"), (
        f"Deny must not persist always_allow, got {perm!r}"
    )


async def test_telegram_dm_approval_always_persists_to_db(
    telegram_with_ask_each_time,
):
    """Telegram DM 'always' clears the gate AND persists always_allow."""
    base_url = telegram_with_ask_each_time["base_url"]
    http_url = telegram_with_ask_each_time["http_url"]

    await _send_tg(http_url, "make approval post tg-always", message_id=120)
    thread_id = await _find_channel_thread(base_url, channel="telegram")
    await _wait_for_pending_gate(base_url, thread_id)

    await _send_tg(http_url, "always", message_id=121)
    await _wait_for_no_pending_gate(base_url, thread_id)

    perm = await _read_tool_permission(base_url, "http")
    assert perm == "always_allow", (
        f"'always' approval must persist always_allow, got {perm!r}"
    )

    # Reset back to ask_each_time so subsequent tests sharing the
    # session-scoped server start from a clean slate.
    await _set_tool_permission(base_url, "http", "ask_each_time")


async def test_telegram_gate_can_be_resolved_via_web_api(
    telegram_with_ask_each_time,
):
    """A gate fired from Telegram is also resolvable via /api/chat/approval.

    This covers the cross-channel resolve path: prompt was surfaced on
    Telegram, but the human chose to approve from the web dashboard.
    """
    base_url = telegram_with_ask_each_time["base_url"]
    http_url = telegram_with_ask_each_time["http_url"]

    await _send_tg(http_url, "make approval post tg-cross", message_id=130)
    thread_id = await _find_channel_thread(base_url, channel="telegram")
    pending = await _wait_for_pending_gate(base_url, thread_id)

    approval = await api_post(
        base_url,
        "/api/chat/approval",
        json={
            "request_id": pending["request_id"],
            "action": "approve",
            "thread_id": thread_id,
        },
        timeout=15,
    )
    assert approval.status_code == 202, (
        f"Cross-channel approve failed: {approval.status_code} {approval.text}"
    )
    await _wait_for_no_pending_gate(base_url, thread_id)


# ---------------------------------------------------------------------------
# Slack approval-gate tests
# ---------------------------------------------------------------------------


@pytest.mark.xfail(
    reason=(
        "Slack WASM bundle does not currently propagate same-channel "
        "'yes/no/always' text replies into the agent loop while a gate is "
        "parked on a sibling engine thread (each Slack event ts opens its "
        "own engine thread, and the parked thread blocks dispatch of the "
        "reply). Cross-channel resolution via /api/chat/approval works — "
        "see test_slack_gate_can_be_resolved_via_web_api. Track as a "
        "channel-native approval gap."
    ),
    strict=True,
)
async def test_slack_dm_approval_yes_resolves_inline_gate(
    slack_with_ask_each_time,
):
    """Slack DM 'yes' reply approves a paused inline gate."""
    base_url = slack_with_ask_each_time["base_url"]
    http_url = slack_with_ask_each_time["http_url"]

    await _send_slack_dm(http_url, "make approval post sl-alpha")
    thread_id = await _find_channel_thread(base_url, channel="slack")
    pending = await _wait_for_pending_gate(base_url, thread_id)
    assert pending["tool_name"] == "http", pending

    await _send_slack_dm(http_url, "yes")
    history = await _wait_for_no_pending_gate(base_url, thread_id)
    assert history.get("turns"), history


@pytest.mark.xfail(
    reason=(
        "Same channel-native gap as the matching 'yes' test — see the "
        "xfail note on test_slack_dm_approval_yes_resolves_inline_gate."
    ),
    strict=True,
)
async def test_slack_dm_approval_no_denies_inline_gate(
    slack_with_ask_each_time,
):
    """Slack DM 'no' reply denies the gate; pending_gate clears."""
    base_url = slack_with_ask_each_time["base_url"]
    http_url = slack_with_ask_each_time["http_url"]

    await _send_slack_dm(http_url, "make approval post sl-deny")
    thread_id = await _find_channel_thread(base_url, channel="slack")
    await _wait_for_pending_gate(base_url, thread_id)

    await _send_slack_dm(http_url, "no")
    await _wait_for_no_pending_gate(base_url, thread_id)

    perm = await _read_tool_permission(base_url, "http")
    assert perm in (None, "ask_each_time"), (
        f"Deny must not persist always_allow, got {perm!r}"
    )


@pytest.mark.xfail(
    reason=(
        "Same channel-native gap as the matching 'yes' test — see the "
        "xfail note on test_slack_dm_approval_yes_resolves_inline_gate."
    ),
    strict=True,
)
async def test_slack_dm_approval_always_persists_to_db(
    slack_with_ask_each_time,
):
    """Slack DM 'always' clears the gate AND persists always_allow."""
    base_url = slack_with_ask_each_time["base_url"]
    http_url = slack_with_ask_each_time["http_url"]

    await _send_slack_dm(http_url, "make approval post sl-always")
    thread_id = await _find_channel_thread(base_url, channel="slack")
    await _wait_for_pending_gate(base_url, thread_id)

    await _send_slack_dm(http_url, "always")
    await _wait_for_no_pending_gate(base_url, thread_id)

    perm = await _read_tool_permission(base_url, "http")
    assert perm == "always_allow", (
        f"'always' approval must persist always_allow, got {perm!r}"
    )

    await _set_tool_permission(base_url, "http", "ask_each_time")


async def test_slack_gate_can_be_resolved_via_web_api(
    slack_with_ask_each_time,
):
    """Cross-channel: Slack gate resolved via /api/chat/approval."""
    base_url = slack_with_ask_each_time["base_url"]
    http_url = slack_with_ask_each_time["http_url"]

    await _send_slack_dm(http_url, "make approval post sl-cross")
    thread_id = await _find_channel_thread(base_url, channel="slack")
    pending = await _wait_for_pending_gate(base_url, thread_id)

    approval = await api_post(
        base_url,
        "/api/chat/approval",
        json={
            "request_id": pending["request_id"],
            "action": "approve",
            "thread_id": thread_id,
        },
        timeout=15,
    )
    assert approval.status_code == 202, (
        f"Cross-channel approve failed: {approval.status_code} {approval.text}"
    )
    await _wait_for_no_pending_gate(base_url, thread_id)


async def test_slack_app_mention_approval_does_not_post_to_dm(
    slack_with_ask_each_time,
):
    """app_mention firing a gate must not post an approval card to a DM.

    Engine v2 + RelayChannel skips approval rendering for non-DM events
    (`relay/channel.rs:435-447`). The WASM Slack channel + fake API used
    here does not post a Block Kit approval card either; the closest
    observable signal is "no chat.postMessage to D<owner> within a few
    seconds" while the gate sits parked in `pending_gates`.

    We don't wait for the 30-min `expires_at` to fire — we just verify
    the prompt didn't leak to the owner DM, then clean up by approving
    via the web API so the parked engine task gets unstuck before the
    next test runs.
    """
    base_url = slack_with_ask_each_time["base_url"]
    http_url = slack_with_ask_each_time["http_url"]
    fake_slack_url = slack_with_ask_each_time["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)
    await _send_slack_mention(http_url, "make approval post sl-mention")

    thread_id = await _find_channel_thread(base_url, channel="slack")
    pending = await _wait_for_pending_gate(base_url, thread_id)
    assert pending["tool_name"] == "http", pending

    # Give the engine a few seconds to (incorrectly) push to the owner DM.
    await asyncio.sleep(3)
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_slack_url}/__mock/sent_messages", timeout=5)
    messages = r.json().get("messages", [])
    dm_replies = [m for m in messages if m.get("channel") == f"D{SL_OWNER_USER_ID}"]
    assert not dm_replies, (
        f"app_mention-fired gate must not post an approval card to the owner DM, "
        f"got: {dm_replies}"
    )

    # Clean up so the parked engine task doesn't bleed into later tests.
    approval = await api_post(
        base_url,
        "/api/chat/approval",
        json={
            "request_id": pending["request_id"],
            "action": "deny",
            "thread_id": thread_id,
        },
        timeout=15,
    )
    assert approval.status_code == 202, approval.text
    await _wait_for_no_pending_gate(base_url, thread_id)
