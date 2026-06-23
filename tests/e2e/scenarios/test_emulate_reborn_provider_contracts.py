"""Emulate-backed provider contract tests for Reborn-capable integrations.

These tests keep the provider fixture layer honest for integrations that
Reborn already exposes in the codebase. They assert seeded data and real
provider-side mutations so the fixtures cannot pass by only booting Emulate.
"""

import base64
import json

import httpx

from helpers import (
    EMULATE_GITHUB_BEARER,
    EMULATE_GOOGLE_BEARER,
    EMULATE_SLACK_BEARER,
)


def _google_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {EMULATE_GOOGLE_BEARER}"}


def _slack_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {EMULATE_SLACK_BEARER}"}


def _github_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {EMULATE_GITHUB_BEARER}"}


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


async def _github_json(
    client: httpx.AsyncClient,
    base_url: str,
    method: str,
    path: str,
    *,
    payload: dict | None = None,
    params: dict | None = None,
    expected_status: int = 200,
) -> dict | list:
    response = await client.request(
        method,
        f"{base_url}{path}",
        headers=_github_headers(),
        json=payload,
        params=params,
    )
    assert response.status_code == expected_status, (
        f"GitHub {method} {path} returned {response.status_code}: {response.text}"
    )
    if not response.content:
        return {}
    return response.json()


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

        await _slack_post(
            client,
            base_url,
            "reactions.add",
            {"channel": channel["id"], "timestamp": posted["ts"], "name": "eyes"},
        )
        reaction = await _slack_post(
            client,
            base_url,
            "reactions.get",
            {"channel": channel["id"], "timestamp": posted["ts"]},
        )
        assert any(
            item["name"] == "eyes" and item["count"] == 1
            for item in reaction["message"]["reactions"]
        )

        reviewer_info = await _slack_post(
            client,
            base_url,
            "users.info",
            {"user": reviewer["id"]},
        )
        assert reviewer_info["user"]["name"] == "qa-reviewer"


async def test_emulate_github_covers_reborn_repo_surfaces(emulate_github_server):
    base_url = emulate_github_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        user = await _github_json(client, base_url, "GET", "/user")
        assert user["login"] == "reborn-dev"

        created_repo = await _github_json(
            client,
            base_url,
            "POST",
            "/user/repos",
            payload={
                "name": "reborn-provider-contract",
                "description": "Created by the Reborn Emulate provider contract.",
                "private": True,
                "auto_init": True,
            },
            expected_status=201,
        )
        assert created_repo["full_name"] == "reborn-dev/reborn-provider-contract"

        user_repos = await _github_json(client, base_url, "GET", "/user/repos")
        assert any(
            item["full_name"] == "reborn-dev/reborn-provider-contract"
            for item in user_repos
        )

        repo = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw",
        )
        assert repo["full_name"] == "nearai/ironclaw"
        assert repo["language"] == "Rust"
        assert "reborn" in repo["topics"]

        fork = await _github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/forks",
            payload={"name": "ironclaw-reborn-fork"},
            expected_status=202,
        )
        assert fork["full_name"] == "reborn-dev/ironclaw-reborn-fork"

        forks = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/forks",
        )
        assert any(item["full_name"] == fork["full_name"] for item in forks)

        release = await _github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/releases",
            payload={
                "tag_name": "reborn-emulate-v1",
                "name": "Reborn Emulate v1",
                "body": "Release seeded through the Emulate provider contract.",
            },
            expected_status=201,
        )
        assert release["tag_name"] == "reborn-emulate-v1"
        assert release["draft"] is False

        latest_release = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/releases/latest",
        )
        assert latest_release["tag_name"] == "reborn-emulate-v1"

        releases = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/releases",
        )
        assert any(item["tag_name"] == "reborn-emulate-v1" for item in releases)

        issue_title = "Emulate Reborn provider contract issue"
        issue = await _github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/issues",
            payload={
                "title": issue_title,
                "body": "Created by the Reborn Emulate provider contract test.",
            },
            expected_status=201,
        )
        assert issue["title"] == issue_title
        assert issue["state"] == "open"

        issue_readback = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/issues/{issue['number']}",
        )
        assert issue_readback["title"] == issue_title

        issue_comment = await _github_json(
            client,
            base_url,
            "POST",
            f"/repos/nearai/ironclaw/issues/{issue['number']}/comments",
            payload={"body": "Issue comment from the provider contract."},
            expected_status=201,
        )
        assert issue_comment["body"] == "Issue comment from the provider contract."

        issue_comments = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/issues/{issue['number']}/comments",
        )
        assert any(item["id"] == issue_comment["id"] for item in issue_comments)

        issues = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/issues",
            params={"state": "open"},
        )
        assert any(item["title"] == issue_title for item in issues)

        issue_search = await _github_json(
            client,
            base_url,
            "GET",
            "/search/issues",
            params={"q": "repo:nearai/ironclaw Emulate Reborn provider contract issue"},
        )
        assert any(item["number"] == issue["number"] for item in issue_search["items"])

        main_ref = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/git/ref/heads/main",
        )
        main_sha = main_ref["object"]["sha"]
        main_commit = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/git/commits/{main_sha}",
        )
        content = "Reborn provider contract git object payload."
        blob = await _github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/git/blobs",
            payload={"content": content, "encoding": "utf-8"},
            expected_status=201,
        )
        blob_readback = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/git/blobs/{blob['sha']}",
        )
        assert base64.b64decode(blob_readback["content"]).decode("utf-8") == content

        tree = await _github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/git/trees",
            payload={
                "base_tree": main_commit["commit"]["tree"]["sha"],
                "tree": [
                    {
                        "path": "docs/emulate-contract.md",
                        "mode": "100644",
                        "type": "blob",
                        "sha": blob["sha"],
                    }
                ],
            },
            expected_status=201,
        )
        tree_readback = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/git/trees/{tree['sha']}",
            params={"recursive": "1"},
        )
        assert any(
            item["path"] == "docs/emulate-contract.md"
            and item["sha"] == blob["sha"]
            for item in tree_readback["tree"]
        )

        commit = await _github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/git/commits",
            payload={
                "message": "docs: add emulate provider contract",
                "tree": tree["sha"],
                "parents": [main_sha],
            },
            expected_status=201,
        )
        branch_name = "reborn-emulate-provider-contract"
        branch_ref = await _github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/git/refs",
            payload={"ref": f"refs/heads/{branch_name}", "sha": commit["sha"]},
            expected_status=201,
        )
        assert branch_ref["ref"] == f"refs/heads/{branch_name}"

        branches = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/branches",
        )
        assert any(item["name"] == branch_name for item in branches)

        matching_refs = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/git/matching-refs/heads/reborn-emulate",
        )
        assert any(item["ref"] == f"refs/heads/{branch_name}" for item in matching_refs)

        pr_title = "Emulate Reborn provider contract PR"
        pr = await _github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/pulls",
            payload={
                "title": pr_title,
                "head": branch_name,
                "base": "main",
                "body": "PR created through the Emulate provider contract.",
                "draft": False,
            },
            expected_status=201,
        )
        assert pr["title"] == pr_title
        assert pr["state"] == "open"

        pull_requests = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/pulls",
            params={"state": "open"},
        )
        assert any(item["number"] == pr["number"] for item in pull_requests)

        pr_readback = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}",
        )
        assert pr_readback["head"]["ref"] == branch_name

        pr_files = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/files",
        )
        assert isinstance(pr_files, list)

        review = await _github_json(
            client,
            base_url,
            "POST",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/reviews",
            payload={
                "body": "Provider contract review comment.",
                "event": "COMMENT",
                "comments": [
                    {
                        "path": "docs/emulate-contract.md",
                        "position": 1,
                        "body": "Inline review comment from the provider contract.",
                    }
                ],
            },
            expected_status=201,
        )
        assert review["state"] == "COMMENTED"

        reviews = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/reviews",
        )
        assert any(item["id"] == review["id"] for item in reviews)

        review_comments = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/reviews/{review['id']}/comments",
        )
        assert any(
            item["body"] == "Inline review comment from the provider contract."
            for item in review_comments
        )

        pr_comment = await _github_json(
            client,
            base_url,
            "POST",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/comments",
            payload={
                "body": "Follow-up PR review comment from the provider contract.",
                "in_reply_to_id": review_comments[0]["id"],
            },
            expected_status=201,
        )
        assert pr_comment["in_reply_to_id"] == review_comments[0]["id"]

        pr_comments = await _github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/comments",
        )
        assert any(item["id"] == pr_comment["id"] for item in pr_comments)

        merge = await _github_json(
            client,
            base_url,
            "PUT",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/merge",
            payload={
                "commit_title": "Merge Reborn Emulate provider contract PR",
                "merge_method": "squash",
            },
        )
        assert merge["merged"] is True

        repo_search = await _github_json(
            client,
            base_url,
            "GET",
            "/search/repositories",
            params={"q": "org:nearai ironclaw"},
        )
        assert any(item["full_name"] == "nearai/ironclaw" for item in repo_search["items"])

        code_search = await _github_json(
            client,
            base_url,
            "GET",
            "/search/code",
            params={"q": "provider contract repo:nearai/ironclaw"},
        )
        assert any(
            item["path"] == "docs/emulate-contract.md"
            and item["sha"] == blob["sha"]
            for item in code_search["items"]
        )

        workflow_runs = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/actions/runs",
        )
        assert workflow_runs["total_count"] == 0

        workflows = await _github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/actions/workflows",
        )
        assert workflows["workflows"] == []
