"""Gmail full-path provider operation cases."""

import json

import httpx

from emulate_provider import gmail_header, google_headers, raw_mime
from provider_operation_types import ProviderOperationCase

GMAIL_REPLY_MARKER = "REBORN_PROVIDER_CASE_REPLY"
SEEDED_GMAIL_MESSAGE_ID = "<msg_emulate_unread@ironclaw.test>"
SEEDED_GMAIL_SUBJECT = "Emulate seeded unread"
SEEDED_GMAIL_THREAD_ID = "thr_emulate_unread"


async def _get(
    emulate_url: str,
    path: str,
    *,
    params: dict[str, str | int] | None = None,
) -> httpx.Response:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(f"{emulate_url}{path}", params=params)
    response.raise_for_status()
    return response


async def _drafts(emulate_url: str) -> list[dict]:
    response = await _get(
        emulate_url,
        "/gmail/v1/users/me/drafts",
        params={"maxResults": 100},
    )
    return response.json().get("drafts", [])


async def _assert_gmail_draft_baseline(emulate_url: str) -> None:
    assert not await _drafts(emulate_url), "seeded provider unexpectedly has drafts"


async def _assert_gmail_draft_outcome(emulate_url: str, preview: dict) -> None:
    drafts = await _drafts(emulate_url)
    assert len(drafts) == 1, drafts
    response = await _get(
        emulate_url,
        f"/gmail/v1/users/me/drafts/{drafts[0]['id']}",
        params={"format": "full"},
    )
    draft = response.json()
    assert gmail_header(draft["message"], "Subject") == "REBORN_PROVIDER_CASE_DRAFT"
    assert "REBORN_PROVIDER_CASE_DRAFT" in json.dumps(preview), preview


async def _gmail_message(emulate_url: str, message_id: str) -> dict:
    response = await _get(
        emulate_url,
        f"/gmail/v1/users/me/messages/{message_id}",
        params={"format": "full"},
    )
    return response.json()


async def _gmail_thread_messages(emulate_url: str, thread_id: str) -> list[dict]:
    response = await _get(
        emulate_url,
        "/gmail/v1/users/me/messages",
        params={"includeSpamTrash": "true", "maxResults": 100},
    )
    messages = [
        message
        for message in response.json().get("messages", [])
        if message["threadId"] == thread_id
    ]
    return [
        await _gmail_message(emulate_url, message["id"]) for message in messages
    ]


async def _assert_gmail_reply_baseline(emulate_url: str) -> None:
    seeded = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert seeded["threadId"] == SEEDED_GMAIL_THREAD_ID, seeded
    assert gmail_header(seeded, "Message-ID") == SEEDED_GMAIL_MESSAGE_ID, seeded
    assert gmail_header(seeded, "Subject") == SEEDED_GMAIL_SUBJECT, seeded
    thread_messages = await _gmail_thread_messages(
        emulate_url, SEEDED_GMAIL_THREAD_ID
    )
    assert [message["id"] for message in thread_messages] == [
        "msg_emulate_unread"
    ], thread_messages


async def _assert_gmail_reply_outcome(emulate_url: str, preview: dict) -> None:
    thread_messages = await _gmail_thread_messages(
        emulate_url, SEEDED_GMAIL_THREAD_ID
    )
    assert len(thread_messages) == 2, thread_messages
    assert any(
        message["id"] == "msg_emulate_unread" for message in thread_messages
    ), thread_messages
    replies = [
        message for message in thread_messages if "SENT" in message["labelIds"]
    ]
    assert len(replies) == 1, thread_messages
    reply = replies[0]
    assert reply["threadId"] == SEEDED_GMAIL_THREAD_ID, reply
    assert gmail_header(reply, "To") == "qa-sender@example.com", reply
    assert gmail_header(reply, "Subject") == SEEDED_GMAIL_SUBJECT, reply
    assert gmail_header(reply, "In-Reply-To") == SEEDED_GMAIL_MESSAGE_ID, reply
    assert gmail_header(reply, "References") == SEEDED_GMAIL_MESSAGE_ID, reply
    assert GMAIL_REPLY_MARKER in reply["snippet"], reply
    assert GMAIL_REPLY_MARKER in json.dumps(preview), preview


async def _assert_gmail_trash_baseline(emulate_url: str) -> None:
    message = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert "TRASH" not in message["labelIds"], message


async def _assert_gmail_trash_outcome(emulate_url: str, preview: dict) -> None:
    message = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert "TRASH" in message["labelIds"], message
    assert "msg_emulate_unread" in json.dumps(preview), preview


GMAIL_PROVIDER_OPERATION_CASES = (
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
                    subject=SEEDED_GMAIL_SUBJECT,
                    body=(
                        f"{GMAIL_REPLY_MARKER}: Reply sent through the reusable "
                        "provider operation runner."
                    ),
                    in_reply_to=SEEDED_GMAIL_MESSAGE_ID,
                    references=SEEDED_GMAIL_MESSAGE_ID,
                ),
                "threadId": SEEDED_GMAIL_THREAD_ID,
            }
        },
        assert_baseline=_assert_gmail_reply_baseline,
        assert_outcome=_assert_gmail_reply_outcome,
    ),
)
