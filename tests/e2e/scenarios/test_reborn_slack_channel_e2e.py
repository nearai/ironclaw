"""Reborn generic channel-stack Slack E2E (extension-runtime P6 §10).

ONE scenario over the existing harness, driving the GENERIC surfaces of the
`ironclaw-reborn serve` binary end to end — never the retiring per-vendor
lane routes (`/api/webchat/v2/channels/slack/*`):

  configure  — save the manifest-declared deployment configuration through
               the operator API while Slack is still uninstalled. The signed
               ingress is live immediately and an unconnected DM gets a static
               connect notice without admitting a model turn.
  install    — the user installs Slack separately from deployment setup.
  connect    — the generic recipe-driven personal OAuth connect: start via
               `/api/webchat/v2/extensions/slack/setup/oauth/start`, complete
               via `/api/reborn/product-auth/oauth/slack/callback`; the token
               exchange lands on the fake Slack API through the env-gated
               vendor-egress rewrite seam and the generic post-OAuth channel
               identity hook binds the proven Slack user to the caller.
               Activation follows (its credential gate requires the account
               this connect creates); the channel host assembly reconciles
               the live ingress registration.
  message in — a v0-signed DM event on the canonical generic ingress route
               `/webhooks/extensions/slack/events` admits a real turn.
  reply out  — the coordinated reply reaches `chat.postMessage` on the fake
               Slack API (`/__mock/sent_messages`), via the delivery
               coordinator and host-side credential injection. The retired
               `/webhooks/slack/events` compatibility alias stays unmounted.
  remove     — after `/api/webchat/v2/extensions/slack/remove`, deployment
               ingress remains live, the user's personal connection is gone,
               and a subsequent DM gets the static connect notice without a
               new model reply.

Vendor egress reaches the fake through `IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP`
(the loopback-only, debug-build-only transport rewrite seam in
`ironclaw_network`); policy checks still run against the real vendor URL, so
DNS resolution of the vendor host must succeed (any networked environment).
"""

import asyncio
import hashlib
import hmac
import json
import time
import uuid
from urllib.parse import parse_qs, quote, urlparse

import httpx
import pytest

from reborn_webui_harness import (
    client_action_id,
    close_reborn_server,
    fetch_extension_oauth_requirement,
    reborn_bearer_headers,
    start_reborn_webui_v2_server,
)

SLACK_PACKAGE_REF = {"kind": "extension", "id": "slack"}
SIGNING_SECRET = "e2e-reborn-slack-signing-secret"
BOT_TOKEN = "xoxb-FAKE-REBORN-BOT-TOKEN"
OAUTH_CLIENT_ID = "e2e-reborn-client-id"
OAUTH_CLIENT_SECRET = "e2e-reborn-client-secret"
APP_ID = "A0001"
BOT_USER_ID = "U00BOT"
# Identity minted by the fake Slack OAuth v2 token endpoint.
OWNER_USER_ID = "U42OWNER"
TEAM_ID = "T0001"
DM_CHANNEL = f"D{OWNER_USER_ID}"

CANONICAL_EVENTS_PATH = "/webhooks/extensions/slack/events"
RETIRED_EVENTS_PATH = "/webhooks/slack/events"


@pytest.fixture(scope="module")
async def reborn_slack_channel_server(
    ironclaw_reborn_binary,
    mock_llm_server,
    fake_slack_server,
    tmp_path_factory,
):
    """Reborn serve with vendor egress rewritten onto the fake Slack API.

    The standard `webui-v2-beta` binary carries the whole generic channel
    stack (ingress, delivery coordinator, channel host assembly) — the old
    `slack-v2-host-beta` compile gate is gone.
    """
    fake = urlparse(fake_slack_server)
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-slack-channel-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        log_prefix="reborn-slack-channel",
        extra_env={
            "IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP": (
                f"slack.com={fake.hostname}:{fake.port}"
            ),
        },
    )
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


# -- helpers -----------------------------------------------------------------


def slack_signature(signing_secret: str, timestamp: str, body: bytes) -> str:
    digest = hmac.new(
        signing_secret.encode("utf-8"),
        f"v0:{timestamp}:{body.decode('utf-8')}".encode("utf-8"),
        hashlib.sha256,
    )
    return f"v0={digest.hexdigest()}"


def dm_event(event_id: str, text: str, *, user: str = OWNER_USER_ID) -> bytes:
    ts = f"{time.time():.6f}"
    return json.dumps(
        {
            "type": "event_callback",
            "event_id": event_id,
            "team_id": TEAM_ID,
            "event": {
                "type": "message",
                "user": user,
                "channel": DM_CHANNEL,
                "channel_type": "im",
                "text": text,
                "ts": ts,
            },
        }
    ).encode("utf-8")


async def post_signed_event(
    client: httpx.AsyncClient,
    base_url: str,
    path: str,
    body: bytes,
    *,
    signing_secret: str = SIGNING_SECRET,
) -> httpx.Response:
    timestamp = str(int(time.time()))
    return await client.post(
        f"{base_url}{path}",
        content=body,
        headers={
            "Content-Type": "application/json",
            "X-Slack-Request-Timestamp": timestamp,
            "X-Slack-Signature": slack_signature(signing_secret, timestamp, body),
        },
        timeout=15,
    )


async def wait_for_route_status(
    client: httpx.AsyncClient,
    base_url: str,
    path: str,
    accepted: set[int],
    *,
    timeout: float = 45,
) -> int:
    """Poll `path` with throwaway signed bot events (silently dropped when
    admitted) until its status lands in `accepted`. Proves route registration
    follows the activation/removal reconcile without admitting turns."""
    deadline = time.monotonic() + timeout
    status = None
    while time.monotonic() < deadline:
        probe = dm_event(f"Ev-probe-{time.monotonic_ns()}", "route probe")
        payload = json.loads(probe)
        payload["event"]["bot_id"] = "B00PROBE"
        probe = json.dumps(payload).encode("utf-8")
        response = await post_signed_event(client, base_url, path, probe)
        status = response.status_code
        if status in accepted:
            return status
        await asyncio.sleep(0.5)
    raise TimeoutError(
        f"route {path} did not reach status in {accepted} within {timeout}s; "
        f"last status: {status}"
    )


async def fake_sent_messages(client: httpx.AsyncClient, fake_slack_url: str) -> list:
    response = await client.get(f"{fake_slack_url}/__mock/sent_messages", timeout=5)
    return response.json().get("messages", [])


async def wait_for_message_containing(
    client: httpx.AsyncClient,
    fake_slack_url: str,
    needle: str,
    min_count: int,
    *,
    timeout: float = 30,
) -> list:
    deadline = time.monotonic() + timeout
    messages = []
    while time.monotonic() < deadline:
        messages = await fake_sent_messages(client, fake_slack_url)
        matching = [message for message in messages if needle in message.get("text", "")]
        if len(matching) >= min_count:
            return matching
        await asyncio.sleep(0.25)
    raise TimeoutError(
        f"expected at least {min_count} message(s) containing {needle!r}; "
        f"got {messages}"
    )


MOCK_GREETING = "Hello! How can I help you today?"


def final_replies(messages: list) -> list:
    """The coordinated final replies (the delivery path also posts a
    transient "thinking" acknowledgement per admitted turn)."""
    return [m for m in messages if MOCK_GREETING in m.get("text", "")]


async def wait_for_final_replies(
    client: httpx.AsyncClient,
    fake_slack_url: str,
    min_count: int,
    *,
    timeout: float = 60,
) -> list:
    deadline = time.monotonic() + timeout
    messages = []
    while time.monotonic() < deadline:
        messages = await fake_sent_messages(client, fake_slack_url)
        if len(final_replies(messages)) >= min_count:
            return final_replies(messages)
        await asyncio.sleep(0.5)
    raise TimeoutError(
        f"expected at least {min_count} final replies within {timeout}s; "
        f"got {len(final_replies(messages))} of {len(messages)}: {messages}"
    )


# -- the scenario --------------------------------------------------------------


async def test_reborn_slack_channel_configure_connect_roundtrip_remove(
    reborn_slack_channel_server, fake_slack_server
):
    base_url = reborn_slack_channel_server
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        await client.post(f"{fake_slack_server}/__mock/reset")

        # ── configure: deployment-owned values, still zero installations ──
        configured = await client.get(
            f"{base_url}/api/webchat/v2/operator/extension-configuration",
            timeout=30,
        )
        configured.raise_for_status()
        slack_group = next(
            group
            for group in configured.json()["groups"]
            if group["group_id"] == "extension.slack"
        )
        assert all(not usage["installed"] for usage in slack_group["used_by"])
        setup = await client.put(
            f"{base_url}/api/webchat/v2/operator/extension-configuration/extension.slack",
            json={
                "values": [
                    {"handle": "slack_bot_token", "value": BOT_TOKEN},
                    {"handle": "slack_signing_secret", "value": SIGNING_SECRET},
                    {"handle": "slack_team_id", "value": TEAM_ID},
                    {"handle": "slack_api_app_id", "value": APP_ID},
                    {"handle": "slack_installation_id", "value": TEAM_ID},
                    {"handle": "slack_bot_user_id", "value": BOT_USER_ID},
                    {"handle": "slack_oauth_client_id", "value": OAUTH_CLIENT_ID},
                    {
                        "handle": "slack_oauth_client_secret",
                        "value": OAUTH_CLIENT_SECRET,
                    },
                ],
                "expected_revision": slack_group["revision"],
                "idempotency_key": f"slack-e2e-{uuid.uuid4()}",
            },
            timeout=30,
        )
        setup.raise_for_status()
        setup_body = setup.json()
        assert setup_body["complete"] is True, setup_body
        assert OAUTH_CLIENT_SECRET not in setup.text

        installed_before = await client.get(
            f"{base_url}/api/webchat/v2/extensions", timeout=30
        )
        installed_before.raise_for_status()
        assert installed_before.json()["extensions"] == [], installed_before.text

        # Admin configuration owns deployment ingress. An unconnected sender
        # gets a static connect notice; the event must not reach the model.
        unconnected = await post_signed_event(
            client,
            base_url,
            CANONICAL_EVENTS_PATH,
            dm_event("Ev-before-install", "hello before install"),
        )
        assert unconnected.status_code == 200, unconnected.text
        notices = await wait_for_message_containing(
            client, fake_slack_server, "connect it in the Ironclaw web app", 1
        )
        assert notices[0]["channel"] == DM_CHANNEL, notices
        assert final_replies(await fake_sent_messages(client, fake_slack_server)) == []

        # ── install: user lifecycle remains separate from admin setup ──
        install = await client.post(
            f"{base_url}/api/webchat/v2/extensions/install",
            json={
                "package_ref": SLACK_PACKAGE_REF,
                "client_action_id": client_action_id(),
            },
            timeout=60,
        )
        install.raise_for_status()
        assert install.json()["success"] is True

        # ── connect: generic personal OAuth → channel identity binding ──
        # The installed member remains setup-needed until this user-level
        # OAuth connection exists. Completing OAuth is the lifecycle
        # transition; there is no separate public activation action.
        requirement = await fetch_extension_oauth_requirement(client, base_url, "slack")
        start = await client.post(
            f"{base_url}/api/webchat/v2/extensions/slack/setup/oauth/start",
            json={
                "requirement": requirement["name"],
                "expires_at": time.strftime(
                    "%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + 300)
                ),
                "invocation_id": requirement["setup"].get("invocation_id"),
            },
            timeout=30,
        )
        assert start.status_code == 200, start.text
        start_body = start.json()
        authorization_url = start_body["authorization_url"]
        state = parse_qs(urlparse(authorization_url).query)["state"][0]

        callback = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/slack/callback"
            f"?state={quote(state, safe='')}&code=e2e-auth-code",
            timeout=30,
            follow_redirects=False,
        )
        assert callback.status_code in (200, 302, 303), callback.text
        # The token exchange crossed the rewrite seam onto the fake vendor.
        calls = (
            await client.get(f"{fake_slack_server}/__mock/api_calls", timeout=5)
        ).json()["calls"]
        exchange_calls = [c for c in calls if c["method"] == "oauth.v2.access"]
        assert exchange_calls, f"token exchange must hit the fake vendor: {calls}"
        assert "client_secret" not in json.dumps(exchange_calls), exchange_calls
        # The flow settled Completed (identity hook bound without error).
        flow_status = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/flow/{start_body['flow_id']}/status"
            f"?invocation_id={start_body['callback_scope']['invocation_id']}",
            timeout=15,
        )
        assert flow_status.status_code == 200, flow_status.text
        assert flow_status.json()["status"] == "completed", flow_status.text

        # OAuth completion makes the member active and the assembly reconciles
        # its live ingress registration without a second activation request.
        await wait_for_route_status(client, base_url, CANONICAL_EVENTS_PATH, {200})
        forged = await post_signed_event(
            client,
            base_url,
            CANONICAL_EVENTS_PATH,
            dm_event("Ev-forged-sig", "should be rejected"),
            signing_secret="wrong-signing-secret",
        )
        assert forged.status_code in (401, 403), forged.text

        # ── message in / reply out: canonical generic ingress route ──
        inbound = await post_signed_event(
            client,
            base_url,
            CANONICAL_EVENTS_PATH,
            dm_event("Ev-canonical-dm", "hello"),
        )
        assert inbound.status_code == 200, inbound.text
        replies = await wait_for_final_replies(client, fake_slack_server, 1)
        assert replies[0]["channel"] == DM_CHANNEL, replies

        # ── retired compatibility alias: only the manifest route remains ──
        retired_inbound = await post_signed_event(
            client,
            base_url,
            RETIRED_EVENTS_PATH,
            dm_event("Ev-retired-path", "this route must stay retired"),
        )
        assert retired_inbound.status_code == 404, retired_inbound.text
        assert len(await wait_for_final_replies(client, fake_slack_server, 1)) == 1

        # ── remove: personal state clears; deployment ingress remains ──
        remove = await client.post(
            f"{base_url}/api/webchat/v2/extensions/slack/remove",
            json={"client_action_id": client_action_id()},
            timeout=60,
        )
        remove.raise_for_status()
        stale = await post_signed_event(
            client,
            base_url,
            CANONICAL_EVENTS_PATH,
            dm_event("Ev-after-remove", "hello after remove"),
        )
        assert stale.status_code == 200, stale.text
        await wait_for_message_containing(
            client, fake_slack_server, "connect it in the Ironclaw web app", 2
        )
        final_messages = await fake_sent_messages(client, fake_slack_server)
        assert len(final_replies(final_messages)) == 1, (
            f"no model reply may be delivered after removal: {final_messages}"
        )
