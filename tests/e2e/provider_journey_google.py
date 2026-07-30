"""Google account setup and provider-side assertions for journey replays."""

import json
from datetime import UTC, datetime, timedelta
from urllib.parse import parse_qs, urlparse

import httpx
from emulate_provider import google_headers
from reborn_webui_harness import (
    fetch_extension_oauth_requirement,
    reborn_bearer_headers,
)

GOOGLE_EXTENSIONS = (
    "gmail",
    "google-calendar",
    "google-drive",
    "google-docs",
    "google-sheets",
    "google-slides",
)
GOOGLE_EXTENSION_SCOPES = {
    "gmail": (
        "https://www.googleapis.com/auth/gmail.readonly",
        "https://www.googleapis.com/auth/gmail.send",
        "https://www.googleapis.com/auth/gmail.modify",
    ),
    "google-calendar": (
        "https://www.googleapis.com/auth/calendar.readonly",
        "https://www.googleapis.com/auth/calendar.events",
    ),
    "google-drive": (
        "https://www.googleapis.com/auth/drive.readonly",
        "https://www.googleapis.com/auth/drive",
    ),
    "google-docs": (
        "https://www.googleapis.com/auth/documents",
        "https://www.googleapis.com/auth/documents.readonly",
    ),
    "google-sheets": (
        "https://www.googleapis.com/auth/spreadsheets",
        "https://www.googleapis.com/auth/spreadsheets.readonly",
    ),
    "google-slides": (
        "https://www.googleapis.com/auth/presentations",
        "https://www.googleapis.com/auth/presentations.readonly",
    ),
}
GOOGLE_CUMULATIVE_SCOPES = tuple(
    dict.fromkeys(
        scope
        for extension_scopes in GOOGLE_EXTENSION_SCOPES.values()
        for scope in extension_scopes
    )
)


def require_single_google_account(accounts: list[dict], response_text: str) -> dict:
    assert len(accounts) == 1, response_text
    return accounts[0]


async def seed_google_account(base_url: str, extension_id: str) -> None:
    expires_at = (datetime.now(UTC) + timedelta(minutes=5)).isoformat()
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        requirement = await fetch_extension_oauth_requirement(
            client, base_url, extension_id
        )
        started = await client.post(
            f"{base_url}/api/webchat/v2/extensions/{extension_id}/setup/oauth/start",
            json={
                "requirement": requirement["name"],
                "expires_at": expires_at,
                "invocation_id": requirement["setup"].get("invocation_id"),
            },
            timeout=15,
        )
        assert started.is_success, started.text
        started_body = started.json()
        state = parse_qs(urlparse(started_body["authorization_url"]).query)["state"][0]

        callback = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/google/callback",
            params={
                "state": state,
                "code": f"mock_auth_code_{extension_id.replace('-', '_')}",
                "scope": " ".join(GOOGLE_CUMULATIVE_SCOPES),
            },
            headers={"Accept": "application/json"},
            timeout=30,
        )
        assert callback.is_success, callback.text
        invocation_id = started_body["callback_scope"]["invocation_id"]
        flow_status = await client.get(
            f"{base_url}/api/reborn/product-auth/oauth/flow/"
            f"{started_body['flow_id']}/status",
            params={"invocation_id": invocation_id},
            timeout=30,
        )
        flow_status.raise_for_status()
        assert flow_status.json()["status"] == "completed", flow_status.text
        listed = await client.post(
            f"{base_url}/api/reborn/product-auth/accounts/list",
            json={
                "provider": "google",
                "requester_extension": extension_id,
                "invocation_id": invocation_id,
            },
            timeout=30,
        )
        listed.raise_for_status()
        accounts = listed.json()["accounts"]
        account = require_single_google_account(accounts, listed.text)
        selected = await client.post(
            f"{base_url}/api/reborn/product-auth/accounts/select",
            json={
                "provider": "google",
                "requester_extension": extension_id,
                "account_id": account["id"],
                "invocation_id": invocation_id,
            },
            timeout=30,
        )
        selected.raise_for_status()


def _created_resource_call(calls: list[dict]) -> dict | None:
    return next(
        (
            call
            for call in calls
            if call["name"]
            in {
                "google-docs__create_document",
                "google-drive__upload_file",
                "google-sheets__create_spreadsheet",
            }
        ),
        None,
    )


def _created_resource_name(call: dict) -> str:
    if call["name"] == "google-drive__upload_file":
        return call["arguments"]["name"]
    return call["arguments"]["title"]


def _sheet_readback_range(write_range: str) -> str:
    sheet, cells = write_range.split("!", 1)
    if ":" not in cells:
        return f"{sheet}!{cells}:Z100"
    start, end = cells.split(":", 1)
    if not any(character.isdigit() for character in start):
        start = f"{start}1"
    if not any(character.isdigit() for character in end):
        end = f"{end}100"
    return f"{sheet}!{start}:{end}"


async def assert_google_provider_outcome(emulate_url: str, calls: list[dict]) -> None:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        send = next(
            (call for call in calls if call["name"] == "gmail__send_message"),
            None,
        )
        if send is not None:
            subject = send["arguments"]["message"]["subject"]
            listed = await client.get(
                f"{emulate_url}/gmail/v1/users/me/messages",
                params={"q": f"subject:{subject}"},
            )
            listed.raise_for_status()
            assert listed.json().get("messages"), (
                f"sent message missing for subject {subject!r}"
            )

        create_call = _created_resource_call(calls)
        if create_call is None:
            return
        title = _created_resource_name(create_call)
        files = await client.get(
            f"{emulate_url}/drive/v3/files",
            params={"q": f"name = '{title}' and trashed = false"},
        )
        files.raise_for_status()
        matching = [item for item in files.json()["files"] if item["name"] == title]
        assert matching, f"created Google resource missing: {files.text}"
        resource_id = matching[-1]["id"]

        if create_call["name"] == "google-drive__upload_file":
            media = await client.get(
                f"{emulate_url}/drive/v3/files/{resource_id}",
                params={"alt": "media"},
            )
            media.raise_for_status()
            assert media.text == create_call["arguments"].get("content", "")
            return

        spreadsheet = await client.get(f"{emulate_url}/v4/spreadsheets/{resource_id}")
        spreadsheet.raise_for_status()
        for call in calls:
            arguments = call["arguments"]
            if not call["name"].startswith("google-sheets__"):
                continue
            if "range" not in arguments or "values" not in arguments:
                continue
            values = await client.get(
                f"{emulate_url}/v4/spreadsheets/{resource_id}/values/"
                f"{_sheet_readback_range(arguments['range'])}"
            )
            values.raise_for_status()
            for row in arguments["values"]:
                for expected in row:
                    assert json.dumps(expected).strip('"') in values.text, (
                        arguments,
                        values.text,
                    )


async def assert_google_provider_baseline(emulate_url: str, calls: list[dict]) -> None:
    """Prove this journey cannot observe mutations from an earlier journey."""
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        send = next(
            (call for call in calls if call["name"] == "gmail__send_message"),
            None,
        )
        if send is not None:
            subject = send["arguments"]["message"]["subject"]
            listed = await client.get(
                f"{emulate_url}/gmail/v1/users/me/messages",
                params={"q": f"subject:{subject}"},
            )
            listed.raise_for_status()
            assert not listed.json().get("messages"), (
                f"provider world already contains sent mail {subject!r}"
            )

        create_call = _created_resource_call(calls)
        if create_call is None:
            return
        title = _created_resource_name(create_call)
        files = await client.get(
            f"{emulate_url}/drive/v3/files",
            params={"q": f"name = '{title}' and trashed = false"},
        )
        files.raise_for_status()
        assert not [item for item in files.json()["files"] if item["name"] == title], (
            f"provider world already contains Google resource {title!r}"
        )
