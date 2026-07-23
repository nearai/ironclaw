"""Google provider operation cases and provider-side oracles."""

import json

import httpx

from emulate_provider import gmail_header, google_headers, raw_mime
from provider_operation_types import ProviderOperationCase

CALENDAR_CREATE_MARKER = "REBORN_PROVIDER_CASE_CREATED_EVENT"
DRIVE_FOLDER_MARKER = "REBORN_PROVIDER_CASE_CREATED_FOLDER"
DRIVE_UPLOAD_CONTENT = "Uploaded through the reusable provider operation runner."
DRIVE_UPLOAD_MARKER = "REBORN_PROVIDER_CASE_UPLOADED_FILE.txt"
GMAIL_REPLY_MARKER = "REBORN_PROVIDER_CASE_REPLY"
SEEDED_GMAIL_THREAD_ID = "thr_emulate_unread"


async def _drive_file(emulate_url: str, file_id: str) -> dict:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(f"{emulate_url}/drive/v3/files/{file_id}")
    response.raise_for_status()
    return response.json()


async def _drive_files_named(emulate_url: str, name: str) -> list[dict]:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(
            f"{emulate_url}/drive/v3/files",
            params={"q": f"name = '{name}' and trashed = false", "pageSize": 100},
        )
    response.raise_for_status()
    return response.json().get("files", [])


async def _assert_drive_get_baseline(emulate_url: str) -> None:
    file = await _drive_file(emulate_url, "drv_reborn_qa_brief")
    assert file["name"] == "Reborn QA Brief", file


async def _assert_drive_get_outcome(emulate_url: str, preview: dict) -> None:
    await _assert_drive_get_baseline(emulate_url)
    assert "Reborn QA Brief" in json.dumps(preview), preview


async def _assert_drive_update_baseline(emulate_url: str) -> None:
    await _assert_drive_get_baseline(emulate_url)


async def _assert_drive_update_outcome(emulate_url: str, preview: dict) -> None:
    file = await _drive_file(emulate_url, "drv_reborn_qa_brief")
    assert file["name"] == "REBORN_PROVIDER_CASE_UPDATED_FILE", file
    assert "REBORN_PROVIDER_CASE_UPDATED_FILE" in json.dumps(preview), preview


async def _assert_drive_folder_baseline(emulate_url: str) -> None:
    assert not await _drive_files_named(emulate_url, DRIVE_FOLDER_MARKER)


async def _assert_drive_folder_outcome(emulate_url: str, preview: dict) -> None:
    matches = await _drive_files_named(emulate_url, DRIVE_FOLDER_MARKER)
    assert len(matches) == 1, matches
    assert matches[0]["mimeType"] == "application/vnd.google-apps.folder", matches[0]
    assert matches[0]["parents"] == ["root"], matches[0]
    assert DRIVE_FOLDER_MARKER in json.dumps(preview), preview


async def _assert_drive_upload_baseline(emulate_url: str) -> None:
    assert not await _drive_files_named(emulate_url, DRIVE_UPLOAD_MARKER)


async def _assert_drive_upload_outcome(emulate_url: str, preview: dict) -> None:
    matches = await _drive_files_named(emulate_url, DRIVE_UPLOAD_MARKER)
    assert len(matches) == 1, matches
    uploaded = matches[0]
    assert uploaded["mimeType"] == "text/plain", uploaded
    assert uploaded["parents"] == ["root"], uploaded
    assert uploaded["size"] == str(len(DRIVE_UPLOAD_CONTENT.encode())), uploaded

    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(
            f"{emulate_url}/drive/v3/files/{uploaded['id']}",
            params={"alt": "media"},
        )
    response.raise_for_status()
    assert response.text == DRIVE_UPLOAD_CONTENT
    assert DRIVE_UPLOAD_MARKER in json.dumps(preview), preview


async def _drafts(emulate_url: str) -> list[dict]:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(
            f"{emulate_url}/gmail/v1/users/me/drafts",
            params={"maxResults": 100},
        )
    response.raise_for_status()
    return response.json().get("drafts", [])


async def _assert_gmail_draft_baseline(emulate_url: str) -> None:
    assert not await _drafts(emulate_url), "seeded provider unexpectedly has drafts"


async def _assert_gmail_draft_outcome(emulate_url: str, preview: dict) -> None:
    drafts = await _drafts(emulate_url)
    assert len(drafts) == 1, drafts
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(
            f"{emulate_url}/gmail/v1/users/me/drafts/{drafts[0]['id']}",
            params={"format": "full"},
        )
    response.raise_for_status()
    draft = response.json()
    assert gmail_header(draft["message"], "Subject") == "REBORN_PROVIDER_CASE_DRAFT"
    assert "REBORN_PROVIDER_CASE_DRAFT" in json.dumps(preview), preview


async def _gmail_message(emulate_url: str, message_id: str) -> dict:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(
            f"{emulate_url}/gmail/v1/users/me/messages/{message_id}",
            params={"format": "full"},
        )
    response.raise_for_status()
    return response.json()


async def _gmail_messages_with_subject(emulate_url: str, subject: str) -> list[dict]:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(
            f"{emulate_url}/gmail/v1/users/me/messages",
            params={
                "q": f"subject:{subject}",
                "includeSpamTrash": "true",
                "maxResults": 100,
            },
        )
    response.raise_for_status()
    messages = response.json().get("messages", [])
    return [
        await _gmail_message(emulate_url, message["id"]) for message in messages
    ]


async def _assert_gmail_reply_baseline(emulate_url: str) -> None:
    seeded = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert seeded["threadId"] == SEEDED_GMAIL_THREAD_ID, seeded
    assert not await _gmail_messages_with_subject(emulate_url, GMAIL_REPLY_MARKER)


async def _assert_gmail_reply_outcome(emulate_url: str, preview: dict) -> None:
    matches = await _gmail_messages_with_subject(emulate_url, GMAIL_REPLY_MARKER)
    assert len(matches) == 1, matches
    reply = matches[0]
    assert reply["threadId"] == SEEDED_GMAIL_THREAD_ID, reply
    assert "SENT" in reply["labelIds"], reply
    assert gmail_header(reply, "To") == "qa-sender@example.com", reply
    assert GMAIL_REPLY_MARKER in json.dumps(preview), preview


async def _assert_gmail_trash_baseline(emulate_url: str) -> None:
    message = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert "TRASH" not in message["labelIds"], message


async def _assert_gmail_trash_outcome(emulate_url: str, preview: dict) -> None:
    message = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert "TRASH" in message["labelIds"], message
    assert "msg_emulate_unread" in json.dumps(preview), preview


async def _calendar_events(emulate_url: str, query: str | None = None) -> list[dict]:
    params = {"maxResults": 100}
    if query is not None:
        params["q"] = query
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(
            f"{emulate_url}/calendar/v3/calendars/primary/events",
            params=params,
        )
    response.raise_for_status()
    return response.json().get("items", [])


async def _assert_calendar_create_baseline(emulate_url: str) -> None:
    assert not await _calendar_events(emulate_url, CALENDAR_CREATE_MARKER)


async def _assert_calendar_create_outcome(emulate_url: str, preview: dict) -> None:
    matches = await _calendar_events(emulate_url, CALENDAR_CREATE_MARKER)
    assert len(matches) == 1, matches
    event = matches[0]
    assert event["summary"] == CALENDAR_CREATE_MARKER, event
    assert event["description"] == "Created by the provider operation runner.", event
    assert event["start"]["dateTime"] == "2026-07-30T09:00:00.000Z", event
    assert event["end"]["dateTime"] == "2026-07-30T09:30:00.000Z", event
    assert [attendee["email"] for attendee in event["attendees"]] == [
        "teammate@example.com"
    ], event
    assert CALENDAR_CREATE_MARKER in json.dumps(preview), preview


async def _assert_calendar_delete_baseline(emulate_url: str) -> None:
    matching_ids = {
        event["id"] for event in await _calendar_events(emulate_url)
    }
    assert "evt_reborn_planning_sync" in matching_ids, matching_ids


async def _assert_calendar_delete_outcome(emulate_url: str, preview: dict) -> None:
    matching_ids = {
        event["id"] for event in await _calendar_events(emulate_url)
    }
    assert "evt_reborn_planning_sync" not in matching_ids, matching_ids
    assert "evt_reborn_planning_sync" in json.dumps(preview), preview


GOOGLE_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="google_drive_get_file",
        provider_service="google",
        capability_id="google-drive.get_file",
        arguments={"file_id": "drv_reborn_qa_brief"},
        assert_baseline=_assert_drive_get_baseline,
        assert_outcome=_assert_drive_get_outcome,
    ),
    ProviderOperationCase(
        case_id="gmail_create_draft",
        provider_service="google",
        capability_id="gmail.create_draft",
        arguments={
            "draft": {
                "message": {
                    "raw": raw_mime(
                        to="draft-recipient@example.com",
                        subject="REBORN_PROVIDER_CASE_DRAFT",
                        body="Created through the reusable provider operation runner.",
                    )
                }
            }
        },
        assert_baseline=_assert_gmail_draft_baseline,
        assert_outcome=_assert_gmail_draft_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_update_file",
        provider_service="google",
        capability_id="google-drive.update_file",
        arguments={
            "file_id": "drv_reborn_qa_brief",
            "name": "REBORN_PROVIDER_CASE_UPDATED_FILE",
        },
        assert_baseline=_assert_drive_update_baseline,
        assert_outcome=_assert_drive_update_outcome,
    ),
    ProviderOperationCase(
        case_id="gmail_trash_message",
        provider_service="google",
        capability_id="gmail.trash_message",
        arguments={"message_id": "msg_emulate_unread"},
        assert_baseline=_assert_gmail_trash_baseline,
        assert_outcome=_assert_gmail_trash_outcome,
    ),
    ProviderOperationCase(
        case_id="gmail_reply_to_message",
        provider_service="google",
        capability_id="gmail.reply_to_message",
        arguments={
            "message": {
                "raw": raw_mime(
                    to="qa-sender@example.com",
                    subject=GMAIL_REPLY_MARKER,
                    body="Reply sent through the reusable provider operation runner.",
                ),
                "threadId": SEEDED_GMAIL_THREAD_ID,
            }
        },
        assert_baseline=_assert_gmail_reply_baseline,
        assert_outcome=_assert_gmail_reply_outcome,
    ),
    ProviderOperationCase(
        case_id="google_calendar_create_event",
        provider_service="google",
        capability_id="google-calendar.create_event",
        arguments={
            "calendar_id": "primary",
            "event": {
                "summary": CALENDAR_CREATE_MARKER,
                "description": "Created by the provider operation runner.",
                "start": {
                    "dateTime": "2026-07-30T09:00:00.000Z",
                    "timeZone": "UTC",
                },
                "end": {
                    "dateTime": "2026-07-30T09:30:00.000Z",
                    "timeZone": "UTC",
                },
                "attendees": [{"email": "teammate@example.com"}],
            },
        },
        assert_baseline=_assert_calendar_create_baseline,
        assert_outcome=_assert_calendar_create_outcome,
    ),
    ProviderOperationCase(
        case_id="google_calendar_delete_event",
        provider_service="google",
        capability_id="google-calendar.delete_event",
        arguments={
            "calendar_id": "primary",
            "event_id": "evt_reborn_planning_sync",
        },
        assert_baseline=_assert_calendar_delete_baseline,
        assert_outcome=_assert_calendar_delete_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_create_folder",
        provider_service="google",
        capability_id="google-drive.create_folder",
        arguments={"name": DRIVE_FOLDER_MARKER, "parent_id": "root"},
        assert_baseline=_assert_drive_folder_baseline,
        assert_outcome=_assert_drive_folder_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_upload_file",
        provider_service="google",
        capability_id="google-drive.upload_file",
        arguments={
            "name": DRIVE_UPLOAD_MARKER,
            "content": DRIVE_UPLOAD_CONTENT,
            "mime_type": "text/plain",
            "parent_id": "root",
        },
        assert_baseline=_assert_drive_upload_baseline,
        assert_outcome=_assert_drive_upload_outcome,
    ),
)
