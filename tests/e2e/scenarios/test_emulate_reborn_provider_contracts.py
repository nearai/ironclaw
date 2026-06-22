"""Emulate-backed provider contract tests for Reborn-capable integrations.

These tests keep the provider fixture layer honest for integrations that
Reborn already exposes in the codebase. They assert seeded data and real
provider-side mutations so the fixtures cannot pass by only booting Emulate.
"""

import base64
import json

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


def _raw_mime(*, to: str, subject: str, body: str) -> str:
    message = (
        f"To: {to}\r\n"
        f"Subject: {subject}\r\n"
        "Content-Type: text/plain; charset=utf-8\r\n"
        "\r\n"
        f"{body}"
    )
    return base64.urlsafe_b64encode(message.encode("utf-8")).decode("ascii").rstrip("=")


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


async def test_emulate_google_covers_reborn_gsuite_read_inputs(emulate_google_server):
    base_url = emulate_google_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        messages_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages",
            headers=_google_headers(),
            params={"q": "is:unread"},
        )
        messages_response.raise_for_status()
        messages = messages_response.json()["messages"]
        assert any(item["id"] == "msg_emulate_unread" for item in messages)

        message_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages/msg_emulate_unread",
            headers=_google_headers(),
            params={"format": "full"},
        )
        message_response.raise_for_status()
        message = message_response.json()
        assert _gmail_header(message, "Subject") == "Emulate seeded unread"

        crm_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages",
            headers=_google_headers(),
            params={"q": "from:near.ai"},
        )
        crm_response.raise_for_status()
        crm_messages = crm_response.json()["messages"]
        assert any(item["id"] == "msg_emulate_near_inbound" for item in crm_messages)

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
        assert any(
            event["summary"] == "PepsiCo procurement sync"
            and any(
                attendee["email"] == "buyer@pepsico.example"
                for attendee in event.get("attendees", [])
            )
            for event in events
        )

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
        assert any(
            item["id"] == "drv_pepsico_account_brief"
            and item["name"] == "PepsiCo Account Brief"
            for item in files
        )
        assert any(
            item["id"] == "drv_near_ai_strategy"
            and item["name"] == "NEAR AI Strategy"
            for item in files
        )

        strategy_response = await client.get(
            f"{base_url}/drive/v3/files/drv_near_ai_strategy",
            headers=_google_headers(),
            params={"alt": "media"},
        )
        strategy_response.raise_for_status()
        assert "user-owned agents" in strategy_response.text


async def test_emulate_google_covers_reborn_gsuite_write_outputs(emulate_google_server):
    base_url = emulate_google_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        subject = "Reborn meeting prep summary"
        sent_response = await client.post(
            f"{base_url}/gmail/v1/users/me/messages/send",
            headers=_google_headers(),
            json={
                "raw": _raw_mime(
                    to="e2e.google@example.com",
                    subject=subject,
                    body="PepsiCo meeting prep from Calendar, Drive, and latest news.",
                )
            },
        )
        sent_response.raise_for_status()
        sent_message = sent_response.json()
        assert "SENT" in sent_message["labelIds"]

        sent_readback_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages/{sent_message['id']}",
            headers=_google_headers(),
            params={"format": "full"},
        )
        sent_readback_response.raise_for_status()
        sent_readback = sent_readback_response.json()
        assert _gmail_header(sent_readback, "Subject") == subject
        assert _gmail_header(sent_readback, "To") == "e2e.google@example.com"

        created_event_response = await client.post(
            f"{base_url}/calendar/v3/calendars/primary/events",
            headers=_google_headers(),
            json={
                "summary": "Reborn created prep follow-up",
                "description": "Created by the Reborn Emulate provider contract.",
                "start": {"dateTime": "2026-06-22T15:00:00.000Z"},
                "end": {"dateTime": "2026-06-22T15:30:00.000Z"},
                "attendees": [{"email": "buyer@pepsico.example"}],
            },
        )
        created_event_response.raise_for_status()
        created_event = created_event_response.json()
        assert created_event["summary"] == "Reborn created prep follow-up"

        listed_events_response = await client.get(
            f"{base_url}/calendar/v3/calendars/primary/events",
            headers=_google_headers(),
            params={"q": "Reborn created prep follow-up"},
        )
        listed_events_response.raise_for_status()
        assert any(
            event["id"] == created_event["id"]
            for event in listed_events_response.json()["items"]
        )

        delete_event_response = await client.delete(
            f"{base_url}/calendar/v3/calendars/primary/events/{created_event['id']}",
            headers=_google_headers(),
        )
        assert delete_event_response.status_code == 204

        boundary = "reborn-e2e-drive-upload"
        drive_content = "CRM row export generated by the Reborn Emulate provider contract."
        drive_metadata = {
            "name": "Reborn CRM Export",
            "mimeType": "text/plain",
            "parents": ["root"],
        }
        multipart_body = (
            f"--{boundary}\r\n"
            "Content-Type: application/json; charset=UTF-8\r\n"
            "\r\n"
            f"{json.dumps(drive_metadata)}\r\n"
            f"--{boundary}\r\n"
            "Content-Type: text/plain\r\n"
            "\r\n"
            f"{drive_content}\r\n"
            f"--{boundary}--\r\n"
        ).encode("utf-8")
        drive_create_response = await client.post(
            f"{base_url}/upload/drive/v3/files",
            headers={
                **_google_headers(),
                "Content-Type": f"multipart/related; boundary={boundary}",
            },
            content=multipart_body,
        )
        drive_create_response.raise_for_status()
        drive_file = drive_create_response.json()
        assert drive_file["name"] == "Reborn CRM Export"
        assert drive_file["mimeType"] == "text/plain"

        drive_media_response = await client.get(
            f"{base_url}/drive/v3/files/{drive_file['id']}",
            headers=_google_headers(),
            params={"alt": "media"},
        )
        drive_media_response.raise_for_status()
        assert drive_media_response.text == drive_content


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

        thread_reply_text = "Threaded Reborn follow-up"
        thread_reply = await _slack_post(
            client,
            base_url,
            "chat.postMessage",
            {
                "channel": channel["id"],
                "thread_ts": posted["ts"],
                "text": thread_reply_text,
            },
        )
        assert thread_reply["message"]["thread_ts"] == posted["ts"]

        history = await _slack_post(
            client,
            base_url,
            "conversations.history",
            {"channel": channel["id"]},
        )
        assert any(message["text"] == text for message in history["messages"])

        replies = await _slack_post(
            client,
            base_url,
            "conversations.replies",
            {"channel": channel["id"], "ts": posted["ts"]},
        )
        assert any(
            message["text"] == thread_reply_text for message in replies["messages"]
        )

        users = await _slack_post(client, base_url, "users.list")
        reviewer = next(
            member for member in users["members"] if member["name"] == "qa-reviewer"
        )
        dm = await _slack_post(
            client,
            base_url,
            "conversations.open",
            {"users": reviewer["id"], "return_im": True},
        )
        dm_channel = dm["channel"]["id"]
        dm_text = "Reborn Emulate Slack DM delivery contract"
        dm_posted = await _slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": dm_channel, "text": dm_text},
        )
        assert dm_posted["message"]["text"] == dm_text

        dm_history = await _slack_post(
            client,
            base_url,
            "conversations.history",
            {"channel": dm_channel},
        )
        assert any(message["text"] == dm_text for message in dm_history["messages"])


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

        release_response = await client.post(
            f"{base_url}/repos/nearai/ironclaw/releases",
            headers=_github_headers(),
            json={
                "tag_name": "reborn-emulate-v1",
                "name": "Reborn Emulate v1",
                "body": "Release seeded through the Emulate provider contract.",
            },
        )
        release_response.raise_for_status()
        release = release_response.json()
        assert release["tag_name"] == "reborn-emulate-v1"
        assert release["draft"] is False

        latest_release_response = await client.get(
            f"{base_url}/repos/nearai/ironclaw/releases/latest",
            headers=_github_headers(),
        )
        latest_release_response.raise_for_status()
        latest_release = latest_release_response.json()
        assert latest_release["tag_name"] == "reborn-emulate-v1"

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
