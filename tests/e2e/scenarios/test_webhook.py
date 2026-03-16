"""HTTP webhook authentication tests with HMAC-SHA256 signatures."""

import hashlib
import hmac
import json

import httpx
import pytest

from helpers import AUTH_TOKEN


def compute_signature(secret: str, body: bytes) -> str:
    """Compute X-Hub-Signature-256 HMAC-SHA256 signature."""
    mac = hmac.new(secret.encode(), body, hashlib.sha256)
    return f"sha256={mac.hexdigest()}"


@pytest.mark.asyncio
async def test_webhook_requires_http_webhook_secret_configured(ironclaw_server):
    """
    Webhook endpoint rejects requests when HTTP_WEBHOOK_SECRET is not configured.
    This tests the fail-closed security posture.
    """
    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    async with httpx.AsyncClient() as client:
        # When no webhook secret is configured on the server, all requests fail
        r = await client.post(
            f"{ironclaw_server}/webhook",
            json={"content": "test message"},
            headers=headers,
        )
        # Server should reject with 503 Service Unavailable (fail closed)
        assert r.status_code in (401, 503)


@pytest.mark.asyncio
async def test_webhook_hmac_signature_valid(ironclaw_server_with_webhook_secret):
    """Valid X-Hub-Signature-256 HMAC signature is accepted."""
    secret = ironclaw_server_with_webhook_secret["secret"]
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_data = {"content": "hello from webhook"}
    body_bytes = json.dumps(body_data).encode()
    signature = compute_signature(secret, body_bytes)

    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
                "X-Hub-Signature-256": signature,
            },
        )
        assert r.status_code == 200, f"Expected 200, got {r.status_code}: {r.text}"
        resp = r.json()
        assert resp["status"] == "accepted"


@pytest.mark.asyncio
async def test_webhook_invalid_hmac_signature_rejected(
    ironclaw_server_with_webhook_secret,
):
    """Invalid X-Hub-Signature-256 signature is rejected with 401."""
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_data = {"content": "hello"}
    body_bytes = json.dumps(body_data).encode()
    invalid_signature = "sha256=0000000000000000000000000000000000000000000000000000000000000000"

    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
                "X-Hub-Signature-256": invalid_signature,
            },
        )
        assert r.status_code == 401, f"Expected 401, got {r.status_code}"
        resp = r.json()
        assert resp["status"] == "error"
        assert "Invalid webhook signature" in resp.get("response", "")


@pytest.mark.asyncio
async def test_webhook_wrong_secret_rejected(ironclaw_server_with_webhook_secret):
    """Signature computed with wrong secret is rejected."""
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_data = {"content": "hello"}
    body_bytes = json.dumps(body_data).encode()
    # Compute signature with wrong secret
    wrong_signature = compute_signature("wrong-secret", body_bytes)

    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
                "X-Hub-Signature-256": wrong_signature,
            },
        )
        assert r.status_code == 401
        resp = r.json()
        assert resp["status"] == "error"


@pytest.mark.asyncio
async def test_webhook_malformed_signature_rejected(
    ironclaw_server_with_webhook_secret,
):
    """Malformed X-Hub-Signature-256 header is rejected."""
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_data = {"content": "hello"}
    body_bytes = json.dumps(body_data).encode()

    async with httpx.AsyncClient() as client:
        # Missing sha256= prefix
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
                "X-Hub-Signature-256": "deadbeef",
            },
        )
        assert r.status_code == 401


@pytest.mark.asyncio
async def test_webhook_missing_signature_header_rejected(
    ironclaw_server_with_webhook_secret,
):
    """Missing X-Hub-Signature-256 header is rejected when no body secret provided."""
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_data = {"content": "hello"}
    body_bytes = json.dumps(body_data).encode()

    async with httpx.AsyncClient() as client:
        # No X-Hub-Signature-256 header and no body secret
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
            },
        )
        assert r.status_code == 401
        resp = r.json()
        assert "Webhook authentication required" in resp.get("response", "")
        assert "X-Hub-Signature-256" in resp.get("response", "")


@pytest.mark.asyncio
async def test_webhook_deprecated_body_secret_still_works(
    ironclaw_server_with_webhook_secret,
):
    """
    Deprecated: body 'secret' field still works for backward compatibility.
    This test ensures we don't break existing clients during the migration period.
    """
    secret = ironclaw_server_with_webhook_secret["secret"]
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    # Old-style request with secret in body
    body_data = {"content": "hello", "secret": secret}
    body_bytes = json.dumps(body_data).encode()

    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
            },
        )
        # Should succeed (backward compatibility)
        assert r.status_code == 200, f"Expected 200, got {r.status_code}: {r.text}"
        resp = r.json()
        assert resp["status"] == "accepted"


@pytest.mark.asyncio
async def test_webhook_header_takes_precedence_over_body_secret(
    ironclaw_server_with_webhook_secret,
):
    """
    When both X-Hub-Signature-256 header and body secret are provided,
    header takes precedence.
    """
    secret = ironclaw_server_with_webhook_secret["secret"]
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_data = {"content": "hello", "secret": "wrong-secret-in-body"}
    body_bytes = json.dumps(body_data).encode()
    # Compute signature with correct secret
    signature = compute_signature(secret, body_bytes)

    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
                "X-Hub-Signature-256": signature,
            },
        )
        # Should succeed because header signature is valid (takes precedence)
        assert r.status_code == 200
        resp = r.json()
        assert resp["status"] == "accepted"


@pytest.mark.asyncio
async def test_webhook_case_insensitive_header_lookup(
    ironclaw_server_with_webhook_secret,
):
    """HTTP headers are case-insensitive. Test with different cases."""
    secret = ironclaw_server_with_webhook_secret["secret"]
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_data = {"content": "hello"}
    body_bytes = json.dumps(body_data).encode()
    signature = compute_signature(secret, body_bytes)

    async with httpx.AsyncClient() as client:
        # Try with lowercase
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
                "x-hub-signature-256": signature,
            },
        )
        assert r.status_code == 200


@pytest.mark.asyncio
async def test_webhook_wrong_content_type_rejected(
    ironclaw_server_with_webhook_secret,
):
    """Webhook only accepts application/json Content-Type."""
    secret = ironclaw_server_with_webhook_secret["secret"]
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_data = {"content": "hello"}
    body_bytes = json.dumps(body_data).encode()
    signature = compute_signature(secret, body_bytes)

    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "text/plain",
                "X-Hub-Signature-256": signature,
            },
        )
        assert r.status_code == 415  # Unsupported Media Type
        resp = r.json()
        assert "application/json" in resp.get("response", "")


@pytest.mark.asyncio
async def test_webhook_invalid_json_rejected(ironclaw_server_with_webhook_secret):
    """Invalid JSON in body is rejected."""
    secret = ironclaw_server_with_webhook_secret["secret"]
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    body_bytes = b"not valid json"
    signature = compute_signature(secret, body_bytes)

    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
                "X-Hub-Signature-256": signature,
            },
        )
        assert r.status_code == 401 or r.status_code == 400


@pytest.mark.asyncio
async def test_webhook_message_queued_for_processing(
    ironclaw_server_with_webhook_secret,
):
    """Message via webhook is queued and can be retrieved."""
    secret = ironclaw_server_with_webhook_secret["secret"]
    base_url = ironclaw_server_with_webhook_secret["url"]

    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    test_message = "webhook test message 12345"
    body_data = {"content": test_message}
    body_bytes = json.dumps(body_data).encode()
    signature = compute_signature(secret, body_bytes)

    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers={
                **headers,
                "Content-Type": "application/json",
                "X-Hub-Signature-256": signature,
            },
        )
        assert r.status_code == 200
        resp = r.json()
        assert resp["status"] == "accepted"
        # Message ID should be present
        assert "message_id" in resp
        assert resp["message_id"] != "00000000-0000-0000-0000-000000000000"
