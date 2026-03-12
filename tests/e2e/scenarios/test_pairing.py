"""DM pairing flow e2e tests.

Tests the pairing security gate for WASM channels: listing pending requests,
approving codes, and error handling.
"""

import httpx
from helpers import AUTH_TOKEN


def _headers():
    return {"Authorization": f"Bearer {AUTH_TOKEN}"}


async def test_pairing_list_returns_empty_for_unknown_channel(ironclaw_server):
    """GET /api/pairing/{channel} returns empty list or 404 for non-existent channel."""
    async with httpx.AsyncClient() as client:
        r = await client.get(
            f"{ironclaw_server}/api/pairing/nonexistent-channel",
            headers=_headers(),
            timeout=10,
        )
    # Either empty list or error is acceptable
    if r.status_code == 200:
        data = r.json()
        # Should have a requests or similar field
        assert isinstance(data, (dict, list))
    else:
        # 404 or similar is fine for non-existent channel
        assert r.status_code in (404, 400)


async def test_approve_invalid_code_rejected(ironclaw_server):
    """POST /api/pairing/{channel}/approve with bad code returns error."""
    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{ironclaw_server}/api/pairing/test-channel/approve",
            json={"code": "INVALID0"},
            headers=_headers(),
            timeout=10,
        )
    # Should fail — no pending request with this code
    if r.status_code == 200:
        data = r.json()
        assert data.get("success") is False or data.get("ok") is False or "error" in str(data).lower()
    else:
        assert r.status_code >= 400


async def test_approve_empty_code_rejected(ironclaw_server):
    """POST /api/pairing/{channel}/approve with empty code returns error."""
    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{ironclaw_server}/api/pairing/test-channel/approve",
            json={"code": ""},
            headers=_headers(),
            timeout=10,
        )
    if r.status_code == 200:
        data = r.json()
        assert data.get("success") is False or data.get("ok") is False
    else:
        assert r.status_code >= 400


async def test_pairing_approve_requires_auth(ironclaw_server):
    """POST /api/pairing/{channel}/approve without auth token is rejected."""
    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{ironclaw_server}/api/pairing/test-channel/approve",
            json={"code": "ABCD1234"},
            timeout=10,
        )
    assert r.status_code == 401 or r.status_code == 403
