"""Gmail full-path provider operation cases."""

import json

import httpx
from emulate_provider import gmail_header, google_headers, raw_mime
from provider_operation_types import (
    ProviderOperationCase,
    exact_provider_http_output,
    provider_http_body,
    static_provider_json_response,
)

GMAIL_REPLY_MARKER = "REBORN_PROVIDER_CASE_REPLY"
GMAIL_SEND_MARKER = "REBORN_PROVIDER_CASE_SEND"
GMAIL_SEND_SUBJECT = "Reborn provider operation send contract"
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


async def _gmail_messages(
    emulate_url: str,
    *,
    query: str | None = None,
) -> list[dict]:
    params: dict[str, str | int] = {
        "includeSpamTrash": "true",
        "maxResults": 100,
    }
    if query is not None:
        params["q"] = query
    response = await _get(
        emulate_url,
        "/gmail/v1/users/me/messages",
        params=params,
    )
    return response.json().get("messages", [])


async def _gmail_thread_messages(emulate_url: str, thread_id: str) -> list[dict]:
    messages = [
        message
        for message in await _gmail_messages(emulate_url)
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


async def _assert_gmail_get_message_outcome(
    emulate_url: str, preview: dict
) -> None:
    message = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert gmail_header(message, "Subject") == SEEDED_GMAIL_SUBJECT, message
    rendered = json.dumps(preview)
    assert "msg_emulate_unread" in rendered, preview
    assert SEEDED_GMAIL_SUBJECT in rendered, preview


async def _assert_gmail_list_messages_outcome(
    emulate_url: str, preview: dict
) -> None:
    messages = await _gmail_messages(emulate_url)
    assert any(message["id"] == "msg_emulate_unread" for message in messages)
    assert "msg_emulate_unread" in json.dumps(preview), preview


EMPTY_GMAIL_QUERY = "subject:REBORN_PROVIDER_CASE_NO_SUCH_MESSAGE"


async def _assert_gmail_list_messages_empty(
    emulate_url: str, preview: dict
) -> None:
    assert await _gmail_messages(emulate_url, query=EMPTY_GMAIL_QUERY) == []
    body = provider_http_body(preview)
    assert body.get("messages", []) == [], body
    assert body["resultSizeEstimate"] == 0, body


async def _assert_gmail_send_baseline(emulate_url: str) -> None:
    assert not await _gmail_messages(
        emulate_url, query=f"subject:{GMAIL_SEND_SUBJECT}"
    ), "provider world already contains the send-message contract email"


async def _assert_gmail_send_outcome(emulate_url: str, preview: dict) -> None:
    messages = await _gmail_messages(
        emulate_url, query=f"subject:{GMAIL_SEND_SUBJECT}"
    )
    assert len(messages) == 1, messages
    message = await _gmail_message(emulate_url, messages[0]["id"])
    assert "SENT" in message["labelIds"], message
    assert gmail_header(message, "To") == "contract-recipient@example.com", message
    assert gmail_header(message, "Subject") == GMAIL_SEND_SUBJECT, message
    assert GMAIL_SEND_MARKER in message["snippet"], message
    body = provider_http_body(preview)
    assert body["id"] == message["id"], (body, message)
    assert body["threadId"] == message["threadId"], (body, message)


GMAIL_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="gmail_get_message",
        provider_service="google",
        capability_id="gmail.get_message",
        arguments={"message_id": "msg_emulate_unread"},
        assert_baseline=_assert_gmail_reply_baseline,
        assert_outcome=_assert_gmail_get_message_outcome,
    ),
    ProviderOperationCase(
        case_id="gmail_get_message_empty",
        provider_service="google",
        capability_id="gmail.get_message",
        arguments={"message_id": "msg_provider_contract_empty"},
        assert_baseline=_assert_gmail_reply_baseline,
        assert_outcome=exact_provider_http_output({}),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/gmail/v1/users/me/messages/msg_provider_contract_empty",
            payload={},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="gmail_list_messages",
        provider_service="google",
        capability_id="gmail.list_messages",
        arguments={"max_results": 100},
        assert_baseline=_assert_gmail_reply_baseline,
        assert_outcome=_assert_gmail_list_messages_outcome,
    ),
    ProviderOperationCase(
        case_id="gmail_list_messages_empty",
        provider_service="google",
        capability_id="gmail.list_messages",
        arguments={"query": EMPTY_GMAIL_QUERY, "max_results": 100},
        assert_baseline=_assert_gmail_reply_baseline,
        assert_outcome=_assert_gmail_list_messages_empty,
        outcome_class="empty",
    ),
    ProviderOperationCase(
        case_id="gmail_send_message",
        provider_service="google",
        capability_id="gmail.send_message",
        arguments={
            "message": {
                "raw": raw_mime(
                    to="contract-recipient@example.com",
                    subject=GMAIL_SEND_SUBJECT,
                    body=(
                        f"{GMAIL_SEND_MARKER}: sent through the typed provider "
                        "operation contract."
                    ),
                )
            }
        },
        assert_baseline=_assert_gmail_send_baseline,
        assert_outcome=_assert_gmail_send_outcome,
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
