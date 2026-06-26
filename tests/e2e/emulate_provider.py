"""Shared Emulate provider helpers for E2E tests."""

import base64

import httpx

from helpers import (
    EMULATE_GITHUB_BEARER,
    EMULATE_GOOGLE_BEARER,
    EMULATE_SLACK_BEARER,
)


def google_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {EMULATE_GOOGLE_BEARER}"}


def slack_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {EMULATE_SLACK_BEARER}"}


def github_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {EMULATE_GITHUB_BEARER}"}


def gmail_header(message: dict, name: str) -> str | None:
    for header in message.get("payload", {}).get("headers", []):
        if header.get("name", "").lower() == name.lower():
            return header.get("value")
    return None


def raw_mime(*, to: str, subject: str, body: str) -> str:
    message = (
        f"To: {to}\r\n"
        f"Subject: {subject}\r\n"
        "Content-Type: text/plain; charset=utf-8\r\n"
        "\r\n"
        f"{body}"
    )
    return base64.urlsafe_b64encode(message.encode("utf-8")).decode("ascii").rstrip("=")


async def slack_post(
    client: httpx.AsyncClient,
    base_url: str,
    method: str,
    payload: dict | None = None,
) -> dict:
    response = await client.post(
        f"{base_url}/api/{method}",
        headers=slack_headers(),
        json=payload or {},
    )
    response.raise_for_status()
    body = response.json()
    assert body.get("ok") is True, f"Slack {method} failed: {body}"
    return body


async def github_json(
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
        headers=github_headers(),
        json=payload,
        params=params,
    )
    assert response.status_code == expected_status, (
        f"GitHub {method} {path} returned {response.status_code}: {response.text}"
    )
    if not response.content:
        return {}
    return response.json()
