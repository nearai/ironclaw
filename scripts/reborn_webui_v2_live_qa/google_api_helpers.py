"""Google API polling helpers for Reborn WebUI v2 live QA."""

from __future__ import annotations

import asyncio
import re
import time
import urllib.parse

from scripts.reborn_webui_v2_live_qa.env_helpers import _first_env_value

_SPREADSHEET_ID_PATTERNS = (
    re.compile(r"https://docs\.google\.com/spreadsheets/d/([A-Za-z0-9_-]+)", re.IGNORECASE),
    re.compile(r"\bspreadsheet(?:\s+id)?\s*[:=]\s*([A-Za-z0-9_-]{20,})", re.IGNORECASE),
)

_DOCUMENT_ID_PATTERNS = (
    re.compile(r"https://docs\.google\.com/document/d/([A-Za-z0-9_-]+)", re.IGNORECASE),
    re.compile(
        r"\b(?:google\s+)?(?:docs?\s+)?document\s+id\s*[:=]\s*([A-Za-z0-9_-]{20,})",
        re.IGNORECASE,
    ),
    re.compile(r"\bdoc(?:ument)?\s+id\s*[:=]\s*([A-Za-z0-9_-]{20,})", re.IGNORECASE),
    re.compile(r"\(ID:\s*([A-Za-z0-9_-]{20,})\)", re.IGNORECASE),
)


def _latest_google_id_match(
    text: str,
    patterns: tuple[re.Pattern[str], ...],
) -> str | None:
    candidates: list[tuple[int, str]] = []
    for pattern in patterns:
        for match in pattern.finditer(text):
            candidate = match.group(1)
            if not candidate.startswith("REBORN_QA_"):
                candidates.append((match.start(), candidate))
    if not candidates:
        return None
    return max(candidates, key=lambda item: item[0])[1]


def _extract_google_spreadsheet_id(text: str) -> str | None:
    return _latest_google_id_match(text, _SPREADSHEET_ID_PATTERNS)


def _extract_google_document_id(text: str) -> str | None:
    return _latest_google_id_match(text, _DOCUMENT_ID_PATTERNS)


def _google_drive_query_literal(value: str) -> str:
    return value.replace("\\", "\\\\").replace("'", "\\'")


async def _google_drive_file_id_by_name(
    *,
    access_token: str,
    name: str,
    mime_type: str,
) -> str | None:
    import httpx

    query = (
        f"name = '{_google_drive_query_literal(name)}' "
        f"and mimeType = '{_google_drive_query_literal(mime_type)}' "
        "and trashed = false"
    )
    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.get(
            "https://www.googleapis.com/drive/v3/files",
            headers={"Authorization": f"Bearer {access_token}"},
            params={
                "q": query,
                "fields": "files(id,name,mimeType,modifiedTime)",
                "orderBy": "modifiedTime desc",
                "pageSize": "10",
                "supportsAllDrives": "true",
                "includeItemsFromAllDrives": "true",
            },
        )
    try:
        payload: object = response.json()
    except ValueError:
        payload = {}
    if response.status_code < 200 or response.status_code >= 300:
        error = payload.get("error") if isinstance(payload, dict) else None
        message = error.get("message") if isinstance(error, dict) else str(payload)[:300]
        raise AssertionError(
            f"Google Drive file lookup returned HTTP {response.status_code}: {message}"
        )
    files = payload.get("files") if isinstance(payload, dict) else None
    if not isinstance(files, list):
        return None
    for item in files:
        if not isinstance(item, dict):
            continue
        if item.get("name") == name and item.get("mimeType") == mime_type:
            file_id = str(item.get("id") or "").strip()
            if file_id:
                return file_id
    return None


async def _create_google_spreadsheet_fixture(
    *,
    access_token: str,
    title: str,
    values: list[list[str]],
    sheet_name: str = "Sheet1",
) -> dict[str, object]:
    import httpx

    async with httpx.AsyncClient(timeout=30.0) as client:
        create_response = await client.post(
            "https://sheets.googleapis.com/v4/spreadsheets",
            headers={"Authorization": f"Bearer {access_token}"},
            params={"fields": "spreadsheetId,spreadsheetUrl"},
            json={
                "properties": {"title": title},
                "sheets": [{"properties": {"title": sheet_name}}],
            },
        )
        try:
            create_payload: object = create_response.json()
        except ValueError:
            create_payload = {}
        if create_response.status_code < 200 or create_response.status_code >= 300:
            error = create_payload.get("error") if isinstance(create_payload, dict) else None
            message = (
                error.get("message") if isinstance(error, dict) else str(create_payload)[:300]
            )
            raise AssertionError(
                "Google Sheets fixture create returned HTTP "
                f"{create_response.status_code}: {message}"
            )
        if not isinstance(create_payload, dict):
            raise AssertionError(f"Google Sheets fixture create returned {create_payload!r}")
        spreadsheet_id = str(create_payload.get("spreadsheetId") or "").strip()
        if not spreadsheet_id:
            raise AssertionError(
                f"Google Sheets fixture create omitted spreadsheetId: {create_payload!r}"
            )

        values_written = False
        if values:
            update_response = await client.put(
                "https://sheets.googleapis.com/v4/spreadsheets/"
                f"{spreadsheet_id}/values/"
                f"{urllib.parse.quote(f'{sheet_name}!A1', safe='!:$')}",
                headers={"Authorization": f"Bearer {access_token}"},
                params={"valueInputOption": "RAW"},
                json={"majorDimension": "ROWS", "values": values},
            )
            try:
                update_payload: object = update_response.json()
            except ValueError:
                update_payload = {}
            if update_response.status_code < 200 or update_response.status_code >= 300:
                error = update_payload.get("error") if isinstance(update_payload, dict) else None
                message = (
                    error.get("message")
                    if isinstance(error, dict)
                    else str(update_payload)[:300]
                )
                raise AssertionError(
                    "Google Sheets fixture value update returned HTTP "
                    f"{update_response.status_code}: {message}"
                )
            values_written = True

    return {
        "spreadsheet_id": spreadsheet_id,
        "spreadsheet_url": str(create_payload.get("spreadsheetUrl") or ""),
        "title": title,
        "sheet_name": sheet_name,
        "values_written": values_written,
    }


async def _google_sheet_contains_marker(
    *,
    access_token: str,
    spreadsheet_id: str,
    marker: str,
    range_name: str = "A1:Z1000",
) -> dict[str, object]:
    import httpx

    url = (
        "https://sheets.googleapis.com/v4/spreadsheets/"
        f"{spreadsheet_id}/values/{urllib.parse.quote(range_name, safe='!:$')}"
    )
    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.get(
            url,
            headers={"Authorization": f"Bearer {access_token}"},
            params={"majorDimension": "ROWS"},
        )
    try:
        payload: object = response.json()
    except ValueError:
        payload = {}
    if response.status_code < 200 or response.status_code >= 300:
        error = payload.get("error") if isinstance(payload, dict) else None
        message = error.get("message") if isinstance(error, dict) else str(payload)[:300]
        raise AssertionError(
            f"Google Sheets read returned HTTP {response.status_code}: {message}"
        )
    values = payload.get("values") if isinstance(payload, dict) else None
    if not isinstance(values, list):
        values = []
    marker_lower = marker.lower()
    for row_index, row in enumerate(values, start=1):
        if not isinstance(row, list):
            continue
        for column_index, cell in enumerate(row, start=1):
            if marker_lower in str(cell).lower():
                return {
                    "found": True,
                    "row_index": row_index,
                    "column_index": column_index,
                    "row_count": len(values),
                }
    return {"found": False, "row_count": len(values)}


async def _wait_for_google_sheet_marker(
    *,
    access_token: str,
    spreadsheet_id: str,
    marker: str,
    timeout: float = 240.0,
    range_name: str = "A1:Z1000",
) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    last_check: dict[str, object] | None = None
    while time.monotonic() < deadline:
        last_check = await _google_sheet_contains_marker(
            access_token=access_token,
            spreadsheet_id=spreadsheet_id,
            marker=marker,
            range_name=range_name,
        )
        if last_check.get("found"):
            return last_check
        await asyncio.sleep(2.0)
    raise AssertionError(
        "Google Sheet marker was not observed before timeout. "
        f"spreadsheet_id_present={bool(spreadsheet_id)} marker={marker!r} "
        f"last_check={last_check!r}"
    )


async def _gmail_delivery_target_email(
    *,
    access_token: str,
    extra_env: dict[str, str] | None = None,
) -> str:
    configured = _first_env_value(
        [
            "REBORN_WEBUI_V2_LIVE_QA_EMAIL_TARGET",
            "LIVE_CANARY_EMAIL_TARGET",
            "AUTH_LIVE_GOOGLE_EMAIL",
            "GOOGLE_TEST_EMAIL",
        ],
        extra_env,
    )
    if configured:
        return configured[1]

    return await _gmail_profile_email(access_token=access_token)


async def _gmail_profile_email(
    *,
    access_token: str,
) -> str:
    import httpx

    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.get(
            "https://gmail.googleapis.com/gmail/v1/users/me/profile",
            headers={"Authorization": f"Bearer {access_token}"},
        )
    try:
        payload: object = response.json()
    except ValueError:
        payload = {}
    if response.status_code < 200 or response.status_code >= 300:
        error = payload.get("error") if isinstance(payload, dict) else None
        message = error.get("message") if isinstance(error, dict) else str(payload)[:300]
        raise AssertionError(
            f"Gmail profile read returned HTTP {response.status_code}: {message}"
        )
    email = str(payload.get("emailAddress") if isinstance(payload, dict) else "").strip()
    if not email:
        raise AssertionError("Gmail profile did not include an emailAddress")
    return email


async def _gmail_message_contains_marker(
    *,
    access_token: str,
    marker: str,
) -> dict[str, object]:
    import httpx

    query = f'"{marker}" newer_than:1d'
    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.get(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages",
            headers={"Authorization": f"Bearer {access_token}"},
            params={"q": query, "maxResults": 10},
        )
    try:
        payload: object = response.json()
    except ValueError:
        payload = {}
    if response.status_code < 200 or response.status_code >= 300:
        error = payload.get("error") if isinstance(payload, dict) else None
        message = error.get("message") if isinstance(error, dict) else str(payload)[:300]
        raise AssertionError(
            f"Gmail message search returned HTTP {response.status_code}: {message}"
        )
    messages = payload.get("messages") if isinstance(payload, dict) else None
    if not isinstance(messages, list):
        messages = []
    return {
        "found": bool(messages),
        "message_count": len(messages),
        "result_size_estimate": (
            payload.get("resultSizeEstimate") if isinstance(payload, dict) else None
        ),
    }


async def _wait_for_gmail_marker(
    *,
    access_token: str,
    marker: str,
    timeout: float = 240.0,
) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    last_check: dict[str, object] | None = None
    while time.monotonic() < deadline:
        last_check = await _gmail_message_contains_marker(
            access_token=access_token,
            marker=marker,
        )
        if last_check.get("found"):
            return last_check
        await asyncio.sleep(3.0)
    raise AssertionError(
        "Gmail marker was not observed before timeout. "
        f"marker={marker!r} last_check={last_check!r}"
    )
