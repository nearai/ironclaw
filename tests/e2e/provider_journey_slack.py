"""Slack world setup, provider readback, and cleanup for journey replays."""

import os
import uuid
from datetime import UTC, datetime, timedelta
from urllib.parse import parse_qs, urlparse

import httpx
from emulate_provider import slack_post
from reborn_webui_harness import (
    fetch_extension_oauth_requirement,
    reborn_bearer_headers,
)

EMULATE_SLACK_CHANNEL_BEARER_ENV = "IRONCLAW_E2E_EMULATE_SLACK_CHANNEL_BEARER"


def emulate_slack_channel_bearer() -> str:
    return os.environ[EMULATE_SLACK_CHANNEL_BEARER_ENV]


async def seed_slack_workspace(emulate_url: str) -> dict[str, str]:
    """Create deterministic Slack data and return its provider-issued IDs."""
    async with httpx.AsyncClient(timeout=15) as client:
        identity = await slack_post(client, emulate_url, "auth.test")
        users = await slack_post(client, emulate_url, "users.list")
        by_name = {member["name"]: member for member in users["members"]}
        channels = await slack_post(
            client,
            emulate_url,
            "conversations.list",
            {"types": "public_channel"},
        )
        channel = next(
            item for item in channels["channels"] if item["name"] == "reborn-alerts"
        )
        await slack_post(
            client, emulate_url, "conversations.join", {"channel": channel["id"]}
        )
        await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {"channel": channel["id"], "text": "QA10 self-authored earlier message"},
        )
        await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {
                "channel": channel["id"],
                "text": "ENTITYMSG_1784643032040 QA10 searchable marker",
            },
        )
        root = await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {"channel": channel["id"], "text": "QA10 thread root"},
        )
        await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {
                "channel": channel["id"],
                "thread_ts": root["ts"],
                "text": "QA10 visible thread reply",
            },
        )
    return {
        "team_id": identity["team_id"],
        "user_id": identity["user_id"],
        "reviewer_id": by_name["qa-reviewer"]["id"],
        "channel_id": channel["id"],
        "channel_name": channel["name"],
        "thread_ts": root["ts"],
    }


async def configure_slack(base_url: str, slack_state: dict[str, str]) -> None:
    client_id = "reborn-qa-emulate-slack-client"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        configured = await client.get(
            f"{base_url}/api/webchat/v2/operator/extension-configuration",
            timeout=30,
        )
        configured.raise_for_status()
        group = next(
            item
            for item in configured.json()["groups"]
            if item["group_id"] == "extension.slack"
        )
        response = await client.put(
            f"{base_url}/api/webchat/v2/operator/extension-configuration/"
            "extension.slack",
            json={
                "values": [
                    {
                        "handle": "slack_bot_token",
                        "value": emulate_slack_channel_bearer(),
                    },
                    {
                        "handle": "slack_signing_secret",
                        "value": "emulate-signing-secret",
                    },
                    {"handle": "slack_team_id", "value": slack_state["team_id"]},
                    {"handle": "slack_api_app_id", "value": client_id},
                    {
                        "handle": "slack_installation_id",
                        "value": slack_state["team_id"],
                    },
                    {
                        "handle": "slack_bot_user_id",
                        "value": slack_state["user_id"],
                    },
                    {"handle": "slack_oauth_client_id", "value": client_id},
                    {
                        "handle": "slack_oauth_client_secret",
                        "value": "emulate-slack-client-secret",
                    },
                ],
                "expected_revision": group["revision"],
                "idempotency_key": f"reborn-qa-emulate-{uuid.uuid4()}",
            },
            timeout=30,
        )
        response.raise_for_status()
        assert response.json()["complete"] is True, response.text


async def seed_slack_account(
    base_url: str,
    emulate_url: str,
    slack_state: dict[str, str],
) -> None:
    expires_at = (datetime.now(UTC) + timedelta(minutes=5)).isoformat()
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        requirement = await fetch_extension_oauth_requirement(client, base_url, "slack")
        started = await client.post(
            f"{base_url}/api/webchat/v2/extensions/slack/setup/oauth/start",
            json={
                "requirement": requirement["name"],
                "expires_at": expires_at,
                "invocation_id": requirement["setup"].get("invocation_id"),
            },
            timeout=30,
        )
        assert started.is_success, started.text
        body = started.json()
        query = parse_qs(urlparse(body["authorization_url"]).query)
        consent = await client.post(
            f"{emulate_url}/oauth/v2/authorize/callback",
            data={
                "user_id": slack_state["user_id"],
                "redirect_uri": query["redirect_uri"][0],
                "scope": query.get("scope", [""])[0],
                "user_scope": query.get("user_scope", [""])[0],
                "state": query["state"][0],
                "client_id": query["client_id"][0],
            },
            follow_redirects=False,
            timeout=30,
        )
        assert consent.status_code == 302, consent.text
        callback_query = parse_qs(urlparse(consent.headers["location"]).query)
        callback = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/slack/callback",
            params={key: values[0] for key, values in callback_query.items()},
            headers={"Accept": "application/json"},
            timeout=30,
        )
        assert callback.is_success, callback.text
        flow_status = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/flow/{body['flow_id']}/status",
            params={"invocation_id": body["callback_scope"]["invocation_id"]},
            timeout=30,
        )
        flow_status.raise_for_status()
        assert flow_status.json()["status"] == "completed", flow_status.text


async def _messages_for_send(
    client: httpx.AsyncClient,
    emulate_url: str,
    slack_state: dict[str, str],
    send: dict,
) -> list[dict]:
    payload = {"channel": slack_state["channel_id"], "limit": 100}
    thread_ts = send["arguments"].get("thread_ts")
    if thread_ts is None:
        method = "conversations.history"
    else:
        method = "conversations.replies"
        payload["ts"] = thread_ts
    page = await slack_post(client, emulate_url, method, payload)
    return page.get("messages", [])


def _send_calls(calls: list[dict]) -> list[dict]:
    return [call for call in calls if call["name"] == "slack__send_message"]


async def assert_slack_provider_outcome(
    emulate_url: str,
    slack_state: dict[str, str],
    calls: list[dict],
) -> None:
    async with httpx.AsyncClient(timeout=15) as client:
        for send in _send_calls(calls):
            messages = await _messages_for_send(client, emulate_url, slack_state, send)
            assert any(
                message.get("text") == send["arguments"]["text"] for message in messages
            ), messages


async def assert_slack_provider_baseline(
    emulate_url: str,
    slack_state: dict[str, str],
    calls: list[dict],
) -> None:
    async with httpx.AsyncClient(timeout=15) as client:
        for send in _send_calls(calls):
            messages = await _messages_for_send(client, emulate_url, slack_state, send)
            assert not any(
                message.get("text") == send["arguments"]["text"] for message in messages
            ), "provider world already contains the expected Slack delivery"


async def cleanup_slack_provider_mutations(
    emulate_url: str,
    slack_state: dict[str, str],
    calls: list[dict],
) -> None:
    sends = _send_calls(calls)
    if not sends:
        return
    async with httpx.AsyncClient(timeout=15) as client:
        matches = {}
        for send in sends:
            messages = await _messages_for_send(client, emulate_url, slack_state, send)
            expected_text = send["arguments"]["text"]
            matches.update(
                {
                    message["ts"]: message
                    for message in messages
                    if message.get("text") == expected_text
                }
            )
        for message in matches.values():
            await slack_post(
                client,
                emulate_url,
                "chat.delete",
                {"channel": slack_state["channel_id"], "ts": message["ts"]},
            )
