"""DM pairing flow E2E tests.

Tests the pairing security model for HTTP endpoints:
- pending pairing requests are admin-only
- member users redeem codes for themselves but cannot enumerate pending codes
- invalid submissions fail cleanly without server errors
"""

import httpx

from helpers import AUTH_TOKEN, api_post, auth_headers, create_member_user


def _assert_action_failure(response: httpx.Response) -> None:
    """Accept either action-response failure or an explicit client/server error."""
    assert response.status_code != 500, response.text[:200]
    if response.status_code == 200:
        data = response.json()
        assert data.get("success") is False, data
    else:
        assert response.status_code >= 400, (
            f"Expected failure status or action payload, got {response.status_code}"
        )


async def test_pairing_list_requires_auth(ironclaw_server):
    """GET /api/pairing/{channel} rejects unauthenticated requests."""
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{ironclaw_server}/api/pairing/ownership-test-channel",
            timeout=10,
        )
    assert response.status_code in (401, 403)


async def test_admin_can_access_pairing_list_endpoint(ironclaw_server):
    """Admins can access the pairing list endpoint even when the channel has no requests."""
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{ironclaw_server}/api/pairing/nonexistent-channel",
            headers=auth_headers(AUTH_TOKEN),
            timeout=10,
        )

    assert response.status_code in (200, 404), (
        f"Expected admin access, got {response.status_code}: {response.text[:200]}"
    )


async def test_member_cannot_access_pairing_list_endpoint(ironclaw_server):
    """Members receive 403 when attempting to enumerate pending pairing codes."""
    member = await create_member_user(ironclaw_server)

    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{ironclaw_server}/api/pairing/nonexistent-channel",
            headers=auth_headers(member["token"]),
            timeout=10,
        )

    assert response.status_code == 403, (
        f"Expected forbidden for member list access, got {response.status_code}: {response.text[:200]}"
    )


async def test_pairing_approve_requires_auth(ironclaw_server):
    """POST /api/pairing/{channel}/approve rejects unauthenticated requests."""
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{ironclaw_server}/api/pairing/test-channel/approve",
            json={"code": "ABCD1234"},
            timeout=10,
        )
    assert response.status_code in (401, 403)


async def test_member_invalid_code_returns_expected_failure_shape(ironclaw_server):
    """Members can attempt self-claim pairing, but invalid codes fail cleanly."""
    member = await create_member_user(ironclaw_server)
    response = await api_post(
        ironclaw_server,
        "/api/pairing/test-channel/approve",
        token=member["token"],
        json={"code": "INVALID0"},
        timeout=10,
    )

    _assert_action_failure(response)


async def test_member_blank_code_rejected_cleanly(ironclaw_server):
    """Blank and whitespace-only pairing codes return a clean failure and never 500."""
    member = await create_member_user(ironclaw_server)

    for code in ("", "   \t   "):
        response = await api_post(
            ironclaw_server,
            "/api/pairing/test-channel/approve",
            token=member["token"],
            json={"code": code},
            timeout=10,
        )
        _assert_action_failure(response)


async def test_member_lowercase_invalid_code_does_not_500(ironclaw_server):
    """Lowercase invalid codes do not cause a server error."""
    member = await create_member_user(ironclaw_server)
    response = await api_post(
        ironclaw_server,
        "/api/pairing/db-test-channel/approve",
        token=member["token"],
        json={"code": "abcd1234"},
        timeout=10,
    )

    _assert_action_failure(response)
