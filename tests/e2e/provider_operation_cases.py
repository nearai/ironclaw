"""Typed full-path provider operation cases and provider-side oracles."""

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
import json
from typing import Literal

import httpx

from emulate_provider import gmail_header, google_headers, raw_mime

BaselineAssertion = Callable[[str], Awaitable[None]]
OutcomeAssertion = Callable[[str, dict], Awaitable[None]]
ProviderService = Literal["google", "github", "slack"]


@dataclass(frozen=True)
class ProviderOperationCase:
    """One capability invocation with provider-observable proof."""

    case_id: str
    provider_service: ProviderService
    capability_id: str
    arguments: dict
    assert_baseline: BaselineAssertion
    assert_outcome: OutcomeAssertion


async def _drive_file(emulate_url: str, file_id: str) -> dict:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(f"{emulate_url}/drive/v3/files/{file_id}")
    response.raise_for_status()
    return response.json()


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


async def _assert_gmail_trash_baseline(emulate_url: str) -> None:
    message = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert "TRASH" not in message["labelIds"], message


async def _assert_gmail_trash_outcome(emulate_url: str, preview: dict) -> None:
    message = await _gmail_message(emulate_url, "msg_emulate_unread")
    assert "TRASH" in message["labelIds"], message
    assert "msg_emulate_unread" in json.dumps(preview), preview


PROVIDER_OPERATION_CASES = (
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
)
