"""Emulate-backed provider contract tests for Reborn-capable integrations.

These tests keep the provider fixture layer honest for integrations that
Reborn already exposes in the codebase. They assert seeded data and real
provider-side mutations so the fixtures cannot pass by only booting Emulate.
"""

import base64
import json

import httpx
import pytest

from emulate_provider import (
    github_json,
    github_headers,
    gmail_header,
    google_headers,
    raw_mime,
    slack_headers,
    slack_post,
)
from helpers import (
    EMULATE_GITHUB_SECONDARY_BEARER,
    EMULATE_GOOGLE_SECONDARY_BEARER,
    EMULATE_SLACK_LIMITED_BEARER,
)

GITHUB_RELEASE_WRITE_UNAVAILABLE = {
    403,
    404,
}
GITHUB_RELEASE_WRITE_PROBE_PAYLOAD = {"tag_name": ""}


async def _skip_if_github_release_writes_unavailable(
    client: httpx.AsyncClient, base_url: str
) -> None:
    probe = await client.post(
        f"{base_url}/repos/nearai/ironclaw/releases",
        headers=github_headers(),
        json=GITHUB_RELEASE_WRITE_PROBE_PAYLOAD,
    )
    if probe.status_code in GITHUB_RELEASE_WRITE_UNAVAILABLE:
        pytest.skip("Selected Emulate GitHub fixture does not expose repo write APIs")
    if probe.status_code != 422:
        raise AssertionError(
            "GitHub release write probe must reject the invalid payload "
            f"without mutation; got {probe.status_code}: {probe.text}"
        )


async def test_emulate_google_covers_reborn_gsuite_read_inputs(emulate_google_server):
    base_url = emulate_google_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        messages_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages",
            headers=google_headers(),
            params={"q": "is:unread"},
        )
        messages_response.raise_for_status()
        messages = messages_response.json()["messages"]
        assert any(item["id"] == "msg_emulate_unread" for item in messages)

        message_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages/msg_emulate_unread",
            headers=google_headers(),
            params={"format": "full"},
        )
        message_response.raise_for_status()
        message = message_response.json()
        assert gmail_header(message, "Subject") == "Emulate seeded unread"

        crm_response = await client.get(
            f"{base_url}/gmail/v1/users/me/messages",
            headers=google_headers(),
            params={"q": "from:near.ai"},
        )
        crm_response.raise_for_status()
        crm_messages = crm_response.json()["messages"]
        assert any(item["id"] == "msg_emulate_near_inbound" for item in crm_messages)

        calendars_response = await client.get(
            f"{base_url}/calendar/v3/users/me/calendarList",
            headers=google_headers(),
        )
        calendars_response.raise_for_status()
        calendars = calendars_response.json()["items"]
        assert any(
            item["id"] == "primary" and item["summary"] == "E2E Primary Calendar"
            for item in calendars
        )

        events_response = await client.get(
            f"{base_url}/calendar/v3/calendars/primary/events",
            headers=google_headers(),
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
            headers=google_headers(),
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
            headers=google_headers(),
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
            headers=google_headers(),
            json={
                "raw": raw_mime(
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
            headers=google_headers(),
            params={"format": "full"},
        )
        sent_readback_response.raise_for_status()
        sent_readback = sent_readback_response.json()
        assert gmail_header(sent_readback, "Subject") == subject
        assert gmail_header(sent_readback, "To") == "e2e.google@example.com"

        created_event_response = await client.post(
            f"{base_url}/calendar/v3/calendars/primary/events",
            headers=google_headers(),
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
            headers=google_headers(),
            params={"q": "Reborn created prep follow-up"},
        )
        listed_events_response.raise_for_status()
        assert any(
            event["id"] == created_event["id"]
            for event in listed_events_response.json()["items"]
        )

        delete_event_response = await client.delete(
            f"{base_url}/calendar/v3/calendars/primary/events/{created_event['id']}",
            headers=google_headers(),
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
                **google_headers(),
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
            headers=google_headers(),
            params={"alt": "media"},
        )
        drive_media_response.raise_for_status()
        assert drive_media_response.text == drive_content


async def test_emulate_google_covers_reborn_docs_contract(emulate_google_server):
    """Cover create, insert, and read shapes used by harvested Docs traces."""
    base_url = emulate_google_server["url"]
    marker = "Reborn harvested Google Docs contract"
    async with httpx.AsyncClient(timeout=10) as client:
        created = await client.post(
            f"{base_url}/v1/documents",
            headers=google_headers(),
            json={"title": marker},
        )
        if created.status_code == 404:
            pytest.skip("Emulate 0.7.0 does not expose the Google Docs API")
        created.raise_for_status()
        document_id = created.json()["documentId"]
        assert created.json()["title"] == marker

        updated = await client.post(
            f"{base_url}/v1/documents/{document_id}:batchUpdate",
            headers=google_headers(),
            json={
                "requests": [
                    {
                        "insertText": {
                            "endOfSegmentLocation": {},
                            "text": marker,
                        }
                    }
                ]
            },
        )
        updated.raise_for_status()
        assert updated.json()["writeControl"]["requiredRevisionId"]

        document = await client.get(
            f"{base_url}/v1/documents/{document_id}",
            headers=google_headers(),
        )
        document.raise_for_status()
        text = "".join(
            element.get("textRun", {}).get("content", "")
            for structural in document.json()["body"]["content"]
            for element in structural.get("paragraph", {}).get("elements", [])
        )
        assert marker in text


async def test_emulate_google_covers_reborn_sheets_contract(emulate_google_server):
    """Cover create, metadata, rename, write, append, and read trace shapes."""
    base_url = emulate_google_server["url"]
    marker = "Reborn harvested Google Sheets contract"
    async with httpx.AsyncClient(timeout=10) as client:
        created = await client.post(
            f"{base_url}/v4/spreadsheets",
            headers=google_headers(),
            json={"properties": {"title": marker}},
        )
        if created.status_code == 404:
            pytest.skip("Emulate 0.7.0 does not expose the Google Sheets API")
        created.raise_for_status()
        spreadsheet = created.json()
        spreadsheet_id = spreadsheet["spreadsheetId"]
        sheet_id = spreadsheet["sheets"][0]["properties"]["sheetId"]
        assert spreadsheet["properties"]["title"] == marker

        renamed = await client.post(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}:batchUpdate",
            headers=google_headers(),
            json={
                "requests": [
                    {
                        "updateSheetProperties": {
                            "properties": {"sheetId": sheet_id, "title": "Results"},
                            "fields": "title",
                        }
                    }
                ]
            },
        )
        renamed.raise_for_status()

        written = await client.put(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}/values/Results!A1:B1",
            headers=google_headers(),
            json={"values": [[marker, "seed"]]},
        )
        written.raise_for_status()
        assert written.json()["updatedCells"] == 2

        appended = await client.post(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}/values/Results:append",
            headers=google_headers(),
            json={"values": [[marker, "appended"]]},
        )
        appended.raise_for_status()
        assert appended.json()["updates"]["updatedCells"] == 2

        metadata = await client.get(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}",
            headers=google_headers(),
        )
        metadata.raise_for_status()
        assert metadata.json()["sheets"][0]["properties"]["title"] == "Results"

        values = await client.get(
            f"{base_url}/v4/spreadsheets/{spreadsheet_id}/values/Results!A1:B2",
            headers=google_headers(),
        )
        values.raise_for_status()
        assert values.json()["values"] == [
            [marker, "seed"],
            [marker, "appended"],
        ]


async def test_emulate_slack_covers_reborn_delivery_surfaces(emulate_slack_server):
    base_url = emulate_slack_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        auth = await slack_post(client, base_url, "auth.test")
        assert auth["team"] == "Reborn E2E Workspace"
        assert auth["user"] == "reborn-user"

        channels = await slack_post(
            client,
            base_url,
            "conversations.list",
            {"types": "public_channel", "exclude_archived": True},
        )
        channel = next(
            item for item in channels["channels"] if item["name"] == "reborn-alerts"
        )

        text = "Reborn Emulate Slack delivery contract"
        posted = await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel["id"], "text": text},
        )
        assert posted["message"]["text"] == text

        thread_reply_text = "Threaded Reborn follow-up"
        thread_reply = await slack_post(
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

        history = await slack_post(
            client,
            base_url,
            "conversations.history",
            {"channel": channel["id"]},
        )
        assert any(message["text"] == text for message in history["messages"])

        replies = await slack_post(
            client,
            base_url,
            "conversations.replies",
            {"channel": channel["id"], "ts": posted["ts"]},
        )
        assert any(
            message["text"] == thread_reply_text for message in replies["messages"]
        )

        users = await slack_post(client, base_url, "users.list")
        reviewer = next(
            member for member in users["members"] if member["name"] == "qa-reviewer"
        )
        dm = await slack_post(
            client,
            base_url,
            "conversations.open",
            {"users": reviewer["id"], "return_im": True},
        )
        dm_channel = dm["channel"]["id"]
        dm_text = "Reborn Emulate Slack DM delivery contract"
        dm_posted = await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": dm_channel, "text": dm_text},
        )
        assert dm_posted["message"]["text"] == dm_text

        dm_history = await slack_post(
            client,
            base_url,
            "conversations.history",
            {"channel": dm_channel},
        )
        assert any(message["text"] == dm_text for message in dm_history["messages"])

        await slack_post(
            client,
            base_url,
            "reactions.add",
            {"channel": channel["id"], "timestamp": posted["ts"], "name": "eyes"},
        )
        reaction = await slack_post(
            client,
            base_url,
            "reactions.get",
            {"channel": channel["id"], "timestamp": posted["ts"]},
        )
        assert any(
            item["name"] == "eyes" and item["count"] == 1
            for item in reaction["message"]["reactions"]
        )

        reviewer_info = await slack_post(
            client,
            base_url,
            "users.info",
            {"user": reviewer["id"]},
        )
        assert reviewer_info["user"]["name"] == "qa-reviewer"


async def test_emulate_slack_covers_reborn_search_messages(emulate_slack_server):
    """Cover the search.messages result shape used by harvested Slack traces."""
    base_url = emulate_slack_server["url"]
    marker = "reborn-harvested-slack-search-contract"
    async with httpx.AsyncClient(timeout=10) as client:
        channels = await slack_post(
            client,
            base_url,
            "conversations.list",
            {"types": "public_channel"},
        )
        channel_id = next(
            channel["id"]
            for channel in channels["channels"]
            if channel["name"] == "reborn-alerts"
        )
        await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel_id, "text": marker},
        )

        response = await client.get(
            f"{base_url}/api/search.messages",
            headers=slack_headers(),
            params={"query": marker, "count": 20, "sort": "timestamp"},
        )
        if response.status_code == 404:
            pytest.skip("Emulate 0.7.0 does not expose Slack search.messages")
        response.raise_for_status()
        body = response.json()
        assert body["ok"] is True
        assert any(
            match["text"] == marker for match in body["messages"]["matches"]
        )


async def test_emulate_github_covers_reborn_repo_surfaces(emulate_github_server):
    base_url = emulate_github_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        await _skip_if_github_release_writes_unavailable(client, base_url)

        user = await github_json(client, base_url, "GET", "/user")
        assert user["login"] == "reborn-dev"

        created_repo = await github_json(
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

        user_repos = await github_json(client, base_url, "GET", "/user/repos")
        assert any(
            item["full_name"] == "reborn-dev/reborn-provider-contract"
            for item in user_repos
        )

        repo = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw",
        )
        assert repo["full_name"] == "nearai/ironclaw"
        assert repo["language"] == "Rust"
        assert "reborn" in repo["topics"]

        fork = await github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/forks",
            payload={"name": "ironclaw-reborn-fork"},
            expected_status=202,
        )
        assert fork["full_name"] == "reborn-dev/ironclaw-reborn-fork"

        forks = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/forks",
        )
        assert any(item["full_name"] == fork["full_name"] for item in forks)

        release = await github_json(
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

        latest_release = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/releases/latest",
        )
        assert latest_release["tag_name"] == "reborn-emulate-v1"

        releases = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/releases",
        )
        assert any(item["tag_name"] == "reborn-emulate-v1" for item in releases)

        issue_title = "Emulate Reborn provider contract issue"
        issue = await github_json(
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

        issue_readback = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/issues/{issue['number']}",
        )
        assert issue_readback["title"] == issue_title

        issue_comment = await github_json(
            client,
            base_url,
            "POST",
            f"/repos/nearai/ironclaw/issues/{issue['number']}/comments",
            payload={"body": "Issue comment from the provider contract."},
            expected_status=201,
        )
        assert issue_comment["body"] == "Issue comment from the provider contract."

        issue_comments = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/issues/{issue['number']}/comments",
        )
        assert any(item["id"] == issue_comment["id"] for item in issue_comments)

        issues = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/issues",
            params={"state": "open"},
        )
        assert any(item["title"] == issue_title for item in issues)

        issue_search = await github_json(
            client,
            base_url,
            "GET",
            "/search/issues",
            params={"q": "repo:nearai/ironclaw Emulate Reborn provider contract issue"},
        )
        assert any(item["number"] == issue["number"] for item in issue_search["items"])

        main_ref = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/git/ref/heads/main",
        )
        main_sha = main_ref["object"]["sha"]
        main_commit = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/git/commits/{main_sha}",
        )
        content = "Reborn provider contract git object payload."
        blob = await github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/git/blobs",
            payload={"content": content, "encoding": "utf-8"},
            expected_status=201,
        )
        blob_readback = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/git/blobs/{blob['sha']}",
        )
        assert base64.b64decode(blob_readback["content"]).decode("utf-8") == content

        tree = await github_json(
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
        tree_readback = await github_json(
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

        commit = await github_json(
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
        branch_ref = await github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/git/refs",
            payload={"ref": f"refs/heads/{branch_name}", "sha": commit["sha"]},
            expected_status=201,
        )
        assert branch_ref["ref"] == f"refs/heads/{branch_name}"

        branches = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/branches",
        )
        assert any(item["name"] == branch_name for item in branches)

        matching_refs = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/git/matching-refs/heads/reborn-emulate",
        )
        assert any(item["ref"] == f"refs/heads/{branch_name}" for item in matching_refs)

        pr_title = "Emulate Reborn provider contract PR"
        pr = await github_json(
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

        pull_requests = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/pulls",
            params={"state": "open"},
        )
        assert any(item["number"] == pr["number"] for item in pull_requests)

        pr_readback = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}",
        )
        assert pr_readback["head"]["ref"] == branch_name

        pr_files = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/files",
        )
        assert isinstance(pr_files, list)

        review = await github_json(
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

        reviews = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/reviews",
        )
        assert any(item["id"] == review["id"] for item in reviews)

        review_comments = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/reviews/{review['id']}/comments",
        )
        assert any(
            item["body"] == "Inline review comment from the provider contract."
            for item in review_comments
        )

        pr_comment = await github_json(
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

        pr_comments = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/nearai/ironclaw/pulls/{pr['number']}/comments",
        )
        assert any(item["id"] == pr_comment["id"] for item in pr_comments)

        merge = await github_json(
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

        repo_search = await github_json(
            client,
            base_url,
            "GET",
            "/search/repositories",
            params={"q": "org:nearai ironclaw"},
        )
        assert any(item["full_name"] == "nearai/ironclaw" for item in repo_search["items"])

        code_search = await github_json(
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

        workflow_runs = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/actions/runs",
        )
        assert workflow_runs["total_count"] == 1
        assert workflow_runs["workflow_runs"][0]["id"] == 1001

        workflows = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/actions/workflows",
        )
        assert [workflow["id"] for workflow in workflows["workflows"]] == [101]


async def test_emulate_google_keeps_seeded_accounts_isolated(emulate_google_server):
    """Provider fixtures must not make cross-account reads pass vacuously."""
    base_url = emulate_google_server["url"]
    primary = google_headers()
    secondary = google_headers(EMULATE_GOOGLE_SECONDARY_BEARER)

    async with httpx.AsyncClient(timeout=10) as client:
        primary_messages = await client.get(
            f"{base_url}/gmail/v1/users/me/messages",
            headers=primary,
        )
        secondary_messages = await client.get(
            f"{base_url}/gmail/v1/users/me/messages",
            headers=secondary,
        )
        primary_messages.raise_for_status()
        secondary_messages.raise_for_status()
        primary_ids = {
            item["id"] for item in primary_messages.json().get("messages", [])
        }
        secondary_ids = {
            item["id"] for item in secondary_messages.json().get("messages", [])
        }
        assert "msg_emulate_secondary_private" not in primary_ids
        assert "msg_emulate_secondary_private" in secondary_ids
        assert "msg_emulate_unread" in primary_ids
        assert "msg_emulate_unread" not in secondary_ids

        primary_events = await client.get(
            f"{base_url}/calendar/v3/calendars/primary/events",
            headers=primary,
        )
        secondary_events = await client.get(
            f"{base_url}/calendar/v3/calendars/primary/events",
            headers=secondary,
        )
        primary_events.raise_for_status()
        secondary_events.raise_for_status()
        primary_event_ids = {
            item["id"] for item in primary_events.json().get("items", [])
        }
        secondary_event_ids = {
            item["id"] for item in secondary_events.json().get("items", [])
        }
        assert "evt_secondary_private" not in primary_event_ids
        assert "evt_secondary_private" in secondary_event_ids

        primary_files = await client.get(
            f"{base_url}/drive/v3/files",
            headers=primary,
        )
        secondary_files = await client.get(
            f"{base_url}/drive/v3/files",
            headers=secondary,
        )
        primary_files.raise_for_status()
        secondary_files.raise_for_status()
        primary_file_ids = {
            item["id"] for item in primary_files.json().get("files", [])
        }
        secondary_file_ids = {
            item["id"] for item in secondary_files.json().get("files", [])
        }
        assert "drv_secondary_private" not in primary_file_ids
        assert "drv_secondary_private" in secondary_file_ids


async def test_emulate_google_covers_missing_resource_errors(emulate_google_server):
    async with httpx.AsyncClient(timeout=10) as client:
        response = await client.get(
            f"{emulate_google_server['url']}/gmail/v1/users/me/messages/missing-message",
            headers=google_headers(),
        )
    assert response.status_code == 404, response.text


async def test_emulate_slack_covers_qa9_and_qa10_provider_shapes(
    emulate_slack_server,
):
    """Pin the stateful Slack shapes used by the live QA 9/10 assertions."""
    base_url = emulate_slack_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        users = await slack_post(client, base_url, "users.list")
        by_name = {member["name"]: member for member in users.get("members", [])}
        reviewer = by_name["qa-reviewer"]
        no_email_user = by_name["no-email-user"]

        profile = await slack_post(
            client,
            base_url,
            "users.profile.get",
            {"user": reviewer["id"]},
        )
        assert profile["profile"]["status_text"] == "Reviewing the release candidate"

        no_email = await slack_post(
            client,
            base_url,
            "users.info",
            {"user": no_email_user["id"]},
        )
        assert not no_email.get("user", {}).get("profile", {}).get("email")

        channels = await slack_post(
            client,
            base_url,
            "conversations.list",
            {"types": "public_channel"},
        )
        alerts = next(
            (
                item
                for item in channels.get("channels", [])
                if item["name"] == "reborn-alerts"
            ),
            None,
        )
        assert alerts is not None, "reborn-alerts channel not found"

        missing_scope = await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": alerts["id"], "text": "must be denied"},
            token=EMULATE_SLACK_LIMITED_BEARER,
            expect_ok=False,
        )
        assert missing_scope["ok"] is False
        assert missing_scope["error"] == "missing_scope"
        assert "chat:write" in missing_scope["needed"]

        dm_channels: dict[str, str] = {}
        markers = {
            "qa-reviewer": "QA9 delivery target reviewer",
            "no-email-user": "QA9 delivery target no-email-user",
        }
        for user_name, marker in markers.items():
            opened = await slack_post(
                client,
                base_url,
                "conversations.open",
                {"users": by_name[user_name]["id"], "return_im": True},
            )
            dm_channel = opened["channel"]["id"]
            dm_channels[user_name] = dm_channel
            await slack_post(
                client,
                base_url,
                "chat.postMessage",
                {"channel": dm_channel, "text": marker},
            )

        for user_name, marker in markers.items():
            history = await slack_post(
                client,
                base_url,
                "conversations.history",
                {"channel": dm_channels[user_name]},
            )
            texts = [message["text"] for message in history["messages"]]
            assert texts.count(marker) == 1
            assert all(
                other_marker not in texts
                for other_marker in markers.values()
                if other_marker != marker
            )

        mention_text = f"Release review requested from <@{reviewer['id']}>"
        mention = await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": alerts["id"], "text": mention_text},
        )
        assert mention["message"]["text"] == mention_text

        root = await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": alerts["id"], "text": "QA10 thread root"},
        )
        await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {
                "channel": alerts["id"],
                "thread_ts": root["ts"],
                "text": "QA10 visible thread reply",
            },
        )
        replies = await slack_post(
            client,
            base_url,
            "conversations.replies",
            {"channel": alerts["id"], "ts": root["ts"]},
        )
        assert [message["text"] for message in replies.get("messages", [])].count(
            "QA10 visible thread reply"
        ) == 1


async def test_emulate_slack_covers_identity_membership_and_last_sent(
    emulate_slack_server,
):
    """Cover the remaining deterministic inputs consumed by Slack transforms."""
    base_url = emulate_slack_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        identity = await slack_post(client, base_url, "auth.test")
        assert identity["user"] == "reborn-user"
        assert identity["user_id"].startswith("U")

        channels = await slack_post(
            client,
            base_url,
            "conversations.list",
            {"types": "public_channel"},
        )
        alerts = next(
            channel
            for channel in channels["channels"]
            if channel["name"] == "reborn-alerts"
        )
        await slack_post(
            client,
            base_url,
            "conversations.join",
            {"channel": alerts["id"]},
        )
        joined_channels = await slack_post(
            client,
            base_url,
            "conversations.list",
            {"types": "public_channel"},
        )
        joined_alerts = next(
            channel
            for channel in joined_channels["channels"]
            if channel["id"] == alerts["id"]
        )
        assert joined_alerts["is_member"] is True

        first_marker = "QA10 self-authored earlier message"
        last_marker = "QA10 self-authored last-sent message"
        await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": alerts["id"], "text": first_marker},
        )
        posted = await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": alerts["id"], "text": last_marker},
        )
        history = await slack_post(
            client,
            base_url,
            "conversations.history",
            {"channel": alerts["id"], "limit": 10},
        )
        matching = [
            message
            for message in history["messages"]
            if message["text"] in {first_marker, last_marker}
        ]
        assert matching[0]["text"] == last_marker
        assert matching[0]["user"] == identity["user_id"]
        assert posted["message"]["user"] == identity["user_id"]


async def test_emulate_google_drive_update_roundtrips(emulate_google_server):
    """Pin Drive update semantics in addition to create/read coverage."""
    base_url = emulate_google_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        boundary = "reborn-e2e-drive-update"
        drive_metadata = {
            "name": "Reborn QA Update Fixture",
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
            "Drive update fixture content\r\n"
            f"--{boundary}--\r\n"
        ).encode("utf-8")
        created = await client.post(
            f"{base_url}/upload/drive/v3/files",
            headers={
                **google_headers(),
                "Content-Type": f"multipart/related; boundary={boundary}",
            },
            content=multipart_body,
        )
        created.raise_for_status()
        created_data = created.json()
        file_id = created_data.get("id")
        assert file_id, f"Created file response missing 'id': {created_data}"

        renamed = await client.patch(
            f"{base_url}/drive/v3/files/{file_id}",
            headers=google_headers(),
            json={"name": "Reborn QA Brief Updated"},
        )
        renamed.raise_for_status()
        assert renamed.json()["name"] == "Reborn QA Brief Updated"

        readback = await client.get(
            f"{base_url}/drive/v3/files/{file_id}",
            headers=google_headers(),
        )
        readback.raise_for_status()
        assert readback.json()["name"] == "Reborn QA Brief Updated"


async def test_emulate_github_distinguishes_repositories_and_private_accounts(
    emulate_github_server,
):
    base_url = emulate_github_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        await _skip_if_github_release_writes_unavailable(client, base_url)

        second_repo = await github_json(
            client,
            base_url,
            "POST",
            "/user/repos",
            payload={
                "name": "release-fixture-secondary",
                "private": True,
                "auto_init": True,
            },
            expected_status=201,
        )
        primary_release = await github_json(
            client,
            base_url,
            "POST",
            "/repos/nearai/ironclaw/releases",
            payload={"tag_name": "fixture-primary-v2", "name": "Primary v2"},
            expected_status=201,
        )
        secondary_release = await github_json(
            client,
            base_url,
            "POST",
            f"/repos/{second_repo['full_name']}/releases",
            payload={"tag_name": "fixture-secondary-v7", "name": "Secondary v7"},
            expected_status=201,
        )

        latest_primary = await github_json(
            client,
            base_url,
            "GET",
            "/repos/nearai/ironclaw/releases/latest",
        )
        latest_secondary = await github_json(
            client,
            base_url,
            "GET",
            f"/repos/{second_repo['full_name']}/releases/latest",
        )
        assert latest_primary["id"] == primary_release["id"]
        assert latest_secondary["id"] == secondary_release["id"]
        assert latest_primary["tag_name"] != latest_secondary["tag_name"]

        foreign_private = await client.get(
            f"{base_url}/repos/{second_repo['full_name']}",
            headers=github_headers(EMULATE_GITHUB_SECONDARY_BEARER),
        )
        assert foreign_private.status_code in (403, 404), foreign_private.text


async def test_emulate_github_covers_identity_and_negative_results(
    emulate_github_server,
):
    base_url = emulate_github_server["url"]
    async with httpx.AsyncClient(timeout=10) as client:
        primary = await github_json(client, base_url, "GET", "/user")
        secondary = await github_json(
            client,
            base_url,
            "GET",
            "/user",
            token=EMULATE_GITHUB_SECONDARY_BEARER,
        )
        assert primary["login"] == "reborn-dev"
        assert secondary["login"] == "reborn-reviewer"

        missing = await client.get(
            f"{base_url}/repos/nearai/does-not-exist",
            headers=github_headers(),
        )
        assert missing.status_code == 404

        empty_search = await github_json(
            client,
            base_url,
            "GET",
            "/search/issues",
            params={"q": "repo:nearai/ironclaw no-such-emulate-result"},
        )
        assert empty_search["total_count"] == 0
        assert empty_search["items"] == []
