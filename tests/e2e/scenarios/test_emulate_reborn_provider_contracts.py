"""Emulate-backed provider contract tests for Reborn-capable integrations.

These tests keep the provider fixture layer honest for integrations that
Reborn already exposes in the codebase. They assert seeded data and real
provider-side mutations so the fixtures cannot pass by only booting Emulate.
"""

import httpx

GOOGLE_TOKEN = "mock-refreshed-access-token"
SLACK_TOKEN = "emulate-slack-token"
GITHUB_TOKEN = "emulate-github-token"


def _google_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {GOOGLE_TOKEN}"}


def _slack_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {SLACK_TOKEN}"}


def _github_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {GITHUB_TOKEN}"}


def _gmail_header(message: dict, name: str) -> str | None:
    for header in message.get("payload", {}).get("headers", []):
        if header.get("name", "").lower() == name.lower():
            return header.get("value")
    return None


async def _slack_post(
    client: httpx.AsyncClient,
    base_url: str,
    method: str,
    payload: dict | None = None,
) -> dict:
    response = await client.post(
        f"{base_url}/api/{method}",
        headers=_slack_headers(),
        json=payload or {},
    )
    response.raise_for_status()
    body = response.json()
    assert body.get("ok") is True, f"Slack {method} failed: {body}"
    return body


async def test_emulate_google_covers_reborn_gsuite_surfaces(emulate_google_server):
    base_url = emulate_google_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        messages_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages",
            headers=_google_headers(),
            params={"q": "is:unread"},
        )
        messages_response.raise_for_status()
        messages = messages_response.json()["messages"]
        assert messages[0]["id"] == "msg_emulate_unread"

        message_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages/msg_emulate_unread",
            headers=_google_headers(),
            params={"format": "full"},
        )
        message_response.raise_for_status()
        message = message_response.json()
        assert _gmail_header(message, "Subject") == "Emulate seeded unread"

        calendars_response = await client.get(
            f"{base_url}/calendar/v3/users/me/calendarList",
            headers=_google_headers(),
        )
        calendars_response.raise_for_status()
        calendars = calendars_response.json()["items"]
        assert any(
            item["id"] == "primary" and item["summary"] == "E2E Primary Calendar"
            for item in calendars
        )

        events_response = await client.get(
            f"{base_url}/calendar/v3/calendars/primary/events",
            headers=_google_headers(),
        )
        events_response.raise_for_status()
        events = events_response.json()["items"]
        assert any(event["summary"] == "Reborn planning sync" for event in events)

        files_response = await client.get(
            f"{base_url}/drive/v3/files",
            headers=_google_headers(),
            params={"q": "'root' in parents and trashed=false", "orderBy": "name"},
        )
        files_response.raise_for_status()
        files = files_response.json()["files"]
        assert any(
            item["id"] == "drv_reborn_qa_brief"
            and item["name"] == "Reborn QA Brief"
            and item["mimeType"] == "text/plain"
            for item in files
        )


async def test_emulate_slack_covers_reborn_delivery_surfaces(emulate_slack_server):
    base_url = emulate_slack_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        auth = await _slack_post(client, base_url, "auth.test")
        assert auth["team"] == "Reborn E2E Workspace"
        assert auth["user"] == "reborn-user"

        channels = await _slack_post(
            client,
            base_url,
            "conversations.list",
            {"types": "public_channel", "exclude_archived": True},
        )
        channel = next(
            item for item in channels["channels"] if item["name"] == "reborn-alerts"
        )

        text = "Reborn Emulate Slack delivery contract"
        posted = await _slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel["id"], "text": text},
        )
        assert posted["message"]["text"] == text

        history = await _slack_post(
            client,
            base_url,
            "conversations.history",
            {"channel": channel["id"]},
        )
        assert any(message["text"] == text for message in history["messages"])


async def test_emulate_github_covers_reborn_repo_issue_surfaces(emulate_github_server):
    base_url = emulate_github_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        user_response = await client.get(f"{base_url}/user", headers=_github_headers())
        user_response.raise_for_status()
        assert user_response.json()["login"] == "reborn-dev"

        repo_response = await client.get(
            f"{base_url}/repos/nearai/ironclaw",
            headers=_github_headers(),
        )
        repo_response.raise_for_status()
        repo = repo_response.json()
        assert repo["full_name"] == "nearai/ironclaw"
        assert repo["language"] == "Rust"
        assert "reborn" in repo["topics"]

        title = "Emulate Reborn provider contract"
        issue_response = await client.post(
            f"{base_url}/repos/nearai/ironclaw/issues",
            headers=_github_headers(),
            json={
                "title": title,
                "body": "Created by the Reborn Emulate provider contract test.",
            },
        )
        issue_response.raise_for_status()
        issue = issue_response.json()
        assert issue["number"] == 1
        assert issue["title"] == title
        assert issue["state"] == "open"

        issues_response = await client.get(
            f"{base_url}/repos/nearai/ironclaw/issues",
            headers=_github_headers(),
            params={"state": "open"},
        )
        issues_response.raise_for_status()
        assert any(item["title"] == title for item in issues_response.json())
