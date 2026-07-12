"""Shared Emulate provider helpers for E2E tests."""

import base64

import httpx

from helpers import (
    EMULATE_GITHUB_BEARER,
    EMULATE_GOOGLE_BEARER,
    EMULATE_SLACK_BEARER,
)


def google_headers(token: str = EMULATE_GOOGLE_BEARER) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


def slack_headers(token: str = EMULATE_SLACK_BEARER) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


def github_headers(token: str = EMULATE_GITHUB_BEARER) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


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
    *,
    token: str = EMULATE_SLACK_BEARER,
    expect_ok: bool = True,
) -> dict:
    response = await client.post(
        f"{base_url}/api/{method}",
        headers=slack_headers(token),
        json=payload or {},
    )
    response.raise_for_status()
    body = response.json()
    if expect_ok:
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
    token: str = EMULATE_GITHUB_BEARER,
) -> dict | list:
    response = await client.request(
        method,
        f"{base_url}{path}",
        headers=github_headers(token),
        json=payload,
        params=params,
    )
    assert response.status_code == expected_status, (
        f"GitHub {method} {path} returned {response.status_code}: {response.text}"
    )
    if not response.content:
        return {}
    return response.json()
