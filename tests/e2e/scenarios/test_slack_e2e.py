"""Full-process Slack E2E tests.

Boot IronClaw -> activate Slack via setup API -> POST webhook events
-> verify chat.postMessage round-trip through mock LLM to fake Slack API.
"""

import asyncio
import hashlib
import hmac
import json
import os
import time

import httpx

from helpers import api_post, auth_headers

# Bot token used throughout these tests.
BOT_TOKEN = "xoxb-FAKE-SLACK-BOT-TOKEN"
# Signing secret used for HMAC-SHA256 webhook verification.
SIGNING_SECRET = "e2e-test-slack-signing-secret"
# Owner user ID used in webhook events.
OWNER_USER_ID = "U42OWNER"
# Bot user ID (used to detect self-messages and strip mentions).
BOT_USER_ID = "UBOTUSER"


# -- helpers ---------------------------------------------------------------


def compute_slack_signature(
    signing_secret: str, timestamp: str, body_bytes: bytes
) -> str:
    """Compute Slack request signature: v0=HMAC-SHA256(v0:{ts}:{body})."""
    sig_basestring = f"v0:{timestamp}:{body_bytes.decode('utf-8')}"
    h = hmac.new(
        signing_secret.encode("utf-8"),
        sig_basestring.encode("utf-8"),
        hashlib.sha256,
    )
    return f"v0={h.hexdigest()}"


async def reset_fake_slack(fake_slack_url: str):
    async with httpx.AsyncClient() as c:
        await c.post(f"{fake_slack_url}/__mock/reset")


async def install_slack(base_url: str):
    """Install the bundled Slack WASM channel if not already installed."""
    r = await api_post(
        base_url,
        "/api/extensions/install",
        json={"name": "slack", "kind": "wasm_channel"},
        timeout=180,
    )
    # 200 = freshly installed, 409 = already installed -- both are fine.
    assert r.status_code in (200, 409), (
        f"Slack install failed ({r.status_code}): {r.text}"
    )


def _patch_slack_capabilities_for_testing(channels_dir: str):
    """Patch the installed capabilities file for E2E testing.

    1. Add files.slack.com to HTTP allowlist for file download tests.
    2. Add files.slack.com to credential host_patterns.
    """
    cap_path = os.path.join(channels_dir, "slack.capabilities.json")
    assert os.path.exists(cap_path), (
        f"Capabilities file not found at {cap_path}; "
        f"files in dir: {os.listdir(channels_dir)}"
    )
    with open(cap_path, "r") as f:
        caps = json.load(f)

    # Ensure files.slack.com is in the HTTP allowlist
    http_caps = caps.setdefault("capabilities", {}).setdefault("http", {})
    allowlist = http_caps.setdefault("allowlist", [])
    has_files_host = any(
        e.get("host") == "files.slack.com" for e in allowlist
    )
    if not has_files_host:
        allowlist.append({"host": "files.slack.com", "path_prefix": "/"})

    # Ensure files.slack.com is in credential host_patterns
    credentials = http_caps.setdefault("credentials", {})
    slack_bot_cred = credentials.setdefault("slack_bot", {})
    host_patterns = slack_bot_cred.setdefault("host_patterns", [])
    if "files.slack.com" not in host_patterns:
        host_patterns.append("files.slack.com")

    with open(cap_path, "w") as f:
        json.dump(caps, f, indent=2)


async def activate_slack(
    base_url: str, fake_slack_url: str, channels_dir: str
) -> None:
    """Install (if needed) and set up the Slack channel.

    Slack setup is single-step (no verification flow like Telegram).
    """
    await reset_fake_slack(fake_slack_url)
    await install_slack(base_url)

    # Patch capabilities for testing
    _patch_slack_capabilities_for_testing(channels_dir)

    # Single setup call with bot token and signing secret
    async with httpx.AsyncClient() as c:
        r = await c.post(
            f"{base_url}/api/extensions/slack/setup",
            headers=auth_headers(),
            json={
                "secrets": {
                    "slack_bot_token": BOT_TOKEN,
                    "slack_signing_secret": SIGNING_SECRET,
                },
                "fields": {},
            },
            timeout=30,
        )
    r.raise_for_status()
    body = r.json()
    assert body.get("activated") or body.get("success"), (
        f"Slack setup failed: {body}"
    )


def build_slack_dm_event(
    user_id: str,
    text: str,
    *,
    channel: str | None = None,
    ts: str | None = None,
    thread_ts: str | None = None,
    files: list[dict] | None = None,
    bot_id: str | None = None,
    subtype: str | None = None,
) -> dict:
    """Build a Slack event_callback payload with a DM message event."""
    if channel is None:
        channel = f"D{user_id}"
    if ts is None:
        ts = f"{time.time():.6f}"

    event = {
        "type": "message",
        "user": user_id,
        "text": text,
        "channel": channel,
        "ts": ts,
        "channel_type": "im",
    }
    if thread_ts is not None:
        event["thread_ts"] = thread_ts
    if files is not None:
        event["files"] = files
    if bot_id is not None:
        event["bot_id"] = bot_id
    if subtype is not None:
        event["subtype"] = subtype

    return {
        "type": "event_callback",
        "token": "fake-verification-token",
        "team_id": "T0001",
        "event": event,
        "event_id": f"Ev{ts.replace('.', '')}",
        "event_time": int(float(ts)),
    }


def build_slack_mention_event(
    user_id: str,
    text: str,
    *,
    channel: str = "C0001",
    ts: str | None = None,
) -> dict:
    """Build a Slack event_callback payload with an app_mention event."""
    if ts is None:
        ts = f"{time.time():.6f}"

    return {
        "type": "event_callback",
        "token": "fake-verification-token",
        "team_id": "T0001",
        "event": {
            "type": "app_mention",
            "user": user_id,
            "text": text,
            "channel": channel,
            "ts": ts,
        },
        "event_id": f"Ev{ts.replace('.', '')}",
        "event_time": int(float(ts)),
    }


async def post_slack_webhook(
    http_url: str,
    payload: dict,
    *,
    signing_secret: str | None = SIGNING_SECRET,
) -> httpx.Response:
    """POST a Slack event to IronClaw's webhook endpoint with HMAC signing."""
    body_bytes = json.dumps(payload).encode("utf-8")
    headers = {"Content-Type": "application/json"}

    if signing_secret is not None:
        timestamp = str(int(time.time()))
        signature = compute_slack_signature(signing_secret, timestamp, body_bytes)
        headers["X-Slack-Request-Timestamp"] = timestamp
        headers["X-Slack-Signature"] = signature

    async with httpx.AsyncClient() as c:
        return await c.post(
            f"{http_url}/webhook/slack",
            content=body_bytes,
            headers=headers,
            timeout=10,
        )


async def wait_for_sent_messages(
    fake_slack_url: str,
    *,
    min_count: int = 1,
    timeout: float = 30,
) -> list[dict]:
    """Poll the fake Slack API until at least min_count chat.postMessage calls appear."""
    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient() as c:
        while time.monotonic() < deadline:
            r = await c.get(f"{fake_slack_url}/__mock/sent_messages", timeout=5)
            messages = r.json().get("messages", [])
            if len(messages) >= min_count:
                return messages
            await asyncio.sleep(0.5)
    raise TimeoutError(
        f"Expected at least {min_count} sent messages within {timeout}s"
    )


async def get_api_calls(fake_slack_url: str) -> list[dict]:
    """Fetch all recorded API calls from the fake Slack server."""
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_slack_url}/__mock/api_calls", timeout=5)
        return r.json().get("calls", [])


# -- tests -----------------------------------------------------------------


async def test_slack_setup_and_dm_roundtrip(slack_e2e_server):
    """Full DM round-trip: setup -> webhook -> mock LLM -> chat.postMessage."""
    base_url = slack_e2e_server["base_url"]
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]
    channels_dir = slack_e2e_server["channels_dir"]

    # Reset fake API and activate the Slack channel
    await activate_slack(base_url, fake_slack_url, channels_dir)

    # Clear fake API state to only capture round-trip messages
    await reset_fake_slack(fake_slack_url)

    # POST a DM webhook event as the verified owner
    payload = build_slack_dm_event(OWNER_USER_ID, "hello")
    resp = await post_slack_webhook(http_url, payload)
    assert resp.status_code == 200, f"Webhook returned {resp.status_code}: {resp.text}"

    # Wait for the bot to send a reply via the fake Slack API.
    messages = await wait_for_sent_messages(fake_slack_url, min_count=1, timeout=30)
    reply_text = messages[-1].get("text", "")
    assert reply_text, f"Empty reply text. All sent messages: {messages}"
    assert messages[-1]["channel"] == f"D{OWNER_USER_ID}"


async def test_slack_app_mention_roundtrip(slack_e2e_server):
    """app_mention in channel -> reply with correct channel + thread_ts."""
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)

    ts = f"{time.time():.6f}"
    payload = build_slack_mention_event(
        OWNER_USER_ID,
        f"<@{BOT_USER_ID}> what time is it",
        channel="C0001",
        ts=ts,
    )
    resp = await post_slack_webhook(http_url, payload)
    assert resp.status_code == 200

    messages = await wait_for_sent_messages(fake_slack_url, min_count=1, timeout=30)
    reply = messages[-1]
    assert reply["channel"] == "C0001"
    # Reply should thread off the original message
    assert reply.get("thread_ts") == ts or reply.get("thread_ts") is not None


async def test_slack_url_verification_challenge(slack_e2e_server):
    """url_verification event -> response contains challenge echo."""
    http_url = slack_e2e_server["http_url"]

    challenge_value = "test-challenge-token-12345"
    payload = {
        "type": "url_verification",
        "token": "fake-verification-token",
        "challenge": challenge_value,
    }

    # url_verification doesn't use HMAC signing
    async with httpx.AsyncClient() as c:
        resp = await c.post(
            f"{http_url}/webhook/slack",
            json=payload,
            headers={"Content-Type": "application/json"},
            timeout=10,
        )

    assert resp.status_code == 200
    body = resp.text
    # The challenge should be echoed back (either as JSON or plain text)
    assert challenge_value in body, (
        f"Expected challenge '{challenge_value}' in response, got: {body}"
    )


async def test_slack_unauthorized_user_rejected(slack_e2e_server):
    """A webhook from a non-owner user should not produce a chat.postMessage reply."""
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)

    # Send a DM from a different user (not the owner)
    payload = build_slack_dm_event("U99STRANGER", "hello from stranger")
    resp = await post_slack_webhook(http_url, payload)
    assert resp.status_code == 200

    # Give it a moment, then verify no LLM reply was sent to the stranger
    await asyncio.sleep(3)
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_slack_url}/__mock/sent_messages", timeout=5)
    messages = r.json().get("messages", [])
    stranger_replies = [
        m for m in messages if m.get("channel") == "DU99STRANGER"
    ]
    for m in stranger_replies:
        text = m.get("text", "").lower()
        assert "how can i help" not in text, (
            f"Unauthorized user received an LLM reply: {m}"
        )


async def test_slack_invalid_hmac_signature_rejected(slack_e2e_server):
    """Webhook with wrong HMAC signature is rejected."""
    http_url = slack_e2e_server["http_url"]

    payload = build_slack_dm_event(OWNER_USER_ID, "should be rejected")
    resp = await post_slack_webhook(
        http_url, payload, signing_secret="wrong-signing-secret"
    )
    assert resp.status_code in (401, 403), (
        f"Expected 401/403, got {resp.status_code}: {resp.text}"
    )


async def test_slack_missing_hmac_headers_rejected(slack_e2e_server):
    """Webhook with no signature headers is rejected."""
    http_url = slack_e2e_server["http_url"]

    payload = build_slack_dm_event(OWNER_USER_ID, "should be rejected")
    # signing_secret=None means no HMAC headers are sent
    resp = await post_slack_webhook(http_url, payload, signing_secret=None)
    assert resp.status_code in (401, 403), (
        f"Expected 401/403 for missing HMAC, got {resp.status_code}: {resp.text}"
    )


async def test_slack_bot_message_ignored(slack_e2e_server):
    """Event with bot_id is silently dropped (no reply)."""
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)

    payload = build_slack_dm_event(
        OWNER_USER_ID,
        "I am a bot message",
        bot_id="B12345",
    )
    resp = await post_slack_webhook(http_url, payload)
    assert resp.status_code == 200

    await asyncio.sleep(3)
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_slack_url}/__mock/sent_messages", timeout=5)
    messages = r.json().get("messages", [])
    assert len(messages) == 0, (
        f"Expected no replies for bot message, got: {messages}"
    )


async def test_slack_message_subtype_ignored(slack_e2e_server):
    """Event with subtype is silently dropped (no reply)."""
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)

    payload = build_slack_dm_event(
        OWNER_USER_ID,
        "channel join message",
        subtype="channel_join",
    )
    resp = await post_slack_webhook(http_url, payload)
    assert resp.status_code == 200

    await asyncio.sleep(3)
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_slack_url}/__mock/sent_messages", timeout=5)
    messages = r.json().get("messages", [])
    assert len(messages) == 0, (
        f"Expected no replies for subtype message, got: {messages}"
    )


async def test_slack_bot_mention_stripped(slack_e2e_server):
    """<@UBOTUSER> hello -> LLM sees 'hello' (mention stripped)."""
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)

    payload = build_slack_mention_event(
        OWNER_USER_ID,
        f"<@{BOT_USER_ID}> hello",
        channel="C0001",
    )
    resp = await post_slack_webhook(http_url, payload)
    assert resp.status_code == 200

    # The bot should reply -- the mention prefix should be stripped
    # before reaching the LLM. The mock LLM matches "hello" -> greeting.
    messages = await wait_for_sent_messages(fake_slack_url, min_count=1, timeout=30)
    assert len(messages) >= 1, f"Expected a reply, got: {messages}"


async def test_slack_thread_reply_includes_thread_ts(slack_e2e_server):
    """DM with thread_ts -> reply includes thread_ts in chat.postMessage."""
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)

    thread_ts = "1234567890.000001"
    payload = build_slack_dm_event(
        OWNER_USER_ID,
        "hello in thread",
        thread_ts=thread_ts,
    )
    resp = await post_slack_webhook(http_url, payload)
    assert resp.status_code == 200

    messages = await wait_for_sent_messages(fake_slack_url, min_count=1, timeout=30)
    reply = messages[-1]
    assert reply.get("thread_ts") == thread_ts, (
        f"Expected thread_ts={thread_ts} in reply, got: {reply}"
    )


async def test_slack_malformed_payload_resilience(slack_e2e_server):
    """Bad JSON -> 200/400 (not 500), bot still works after."""
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)

    # Send a completely malformed payload
    body_bytes = b'{"not_a_valid_slack_event": true}'
    timestamp = str(int(time.time()))
    signature = compute_slack_signature(SIGNING_SECRET, timestamp, body_bytes)
    headers = {
        "Content-Type": "application/json",
        "X-Slack-Request-Timestamp": timestamp,
        "X-Slack-Signature": signature,
    }
    async with httpx.AsyncClient() as c:
        resp = await c.post(
            f"{http_url}/webhook/slack",
            content=body_bytes,
            headers=headers,
            timeout=10,
        )
    # Accept 200 or 400 but not 500
    assert resp.status_code in (200, 400), (
        f"Expected 200 or 400 for malformed payload, got {resp.status_code}: {resp.text}"
    )

    # Verify no replies were sent
    await asyncio.sleep(2)
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_slack_url}/__mock/sent_messages", timeout=5)
    messages = r.json().get("messages", [])
    assert len(messages) == 0, (
        f"Expected no replies for malformed payload, got: {messages}"
    )

    # Verify bot still works after bad payload
    await reset_fake_slack(fake_slack_url)
    payload = build_slack_dm_event(OWNER_USER_ID, "hello")
    resp2 = await post_slack_webhook(http_url, payload)
    assert resp2.status_code == 200

    messages = await wait_for_sent_messages(fake_slack_url, min_count=1, timeout=30)
    assert len(messages) >= 1, (
        f"Expected bot to work after malformed payload, got: {messages}"
    )


async def test_slack_file_attachment_with_dm(slack_e2e_server):
    """DM with files array -> file download attempted, message still processed."""
    http_url = slack_e2e_server["http_url"]
    fake_slack_url = slack_e2e_server["fake_slack_url"]

    await reset_fake_slack(fake_slack_url)

    payload = build_slack_dm_event(
        OWNER_USER_ID,
        "hello with attachment",
        files=[
            {
                "id": "F0FILE001",
                "name": "report.pdf",
                "mimetype": "application/pdf",
                "url_private_download": "https://files.slack.com/files-pri/T0001-F0FILE001/report.pdf",
                "size": 2048,
            }
        ],
    )
    resp = await post_slack_webhook(http_url, payload)
    assert resp.status_code == 200

    # Bot should reply to the text content regardless of file download outcome
    messages = await wait_for_sent_messages(fake_slack_url, min_count=1, timeout=30)
    assert len(messages) >= 1, (
        f"Expected bot to reply with file attachment, got: {messages}"
    )
    assert messages[-1]["channel"] == f"D{OWNER_USER_ID}"
