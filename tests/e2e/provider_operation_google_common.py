"""Shared Google provider readback helpers."""

import httpx

from emulate_provider import google_headers


async def google_json(
    emulate_url: str,
    method: str,
    path: str,
    *,
    payload: dict | None = None,
    params: dict | list[tuple[str, str | int]] | None = None,
    expected_status: int = 200,
) -> dict | list:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.request(
            method,
            f"{emulate_url}{path}",
            json=payload,
            params=params,
        )
    assert response.status_code == expected_status, (
        f"Google {method} {path} returned {response.status_code}: {response.text}"
    )
    if not response.content:
        return {}
    return response.json()
