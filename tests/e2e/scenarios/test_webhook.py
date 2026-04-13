"""HTTP webhook authentication tests with HMAC-SHA256 signatures."""

import hashlib
import hmac
import json

import httpx
from helpers import HTTP_WEBHOOK_SECRET


def compute_signature(secret: str, body: bytes) -> str:
    """Compute X-Hub-Signature-256 HMAC-SHA256 signature."""
    mac = hmac.new(secret.encode(), body, hashlib.sha256)
    return f"sha256={mac.hexdigest()}"


async def _post_webhook(
    base_url: str,
    body_data: dict,
    *,
    signature: str | None = None,
    content_type: str = "application/json",
) -> httpx.Response:
    """Send a raw webhook request with optional signature."""
    body_bytes = json.dumps(body_data).encode()
    headers = {"Content-Type": content_type}
    if signature is not None:
        headers["X-Hub-Signature-256"] = signature

    async with httpx.AsyncClient() as client:
        return await client.post(
            f"{base_url}/webhook",
            content=body_bytes,
            headers=headers,
        )


async def test_webhook_requires_http_webhook_secret_configured(
    http_channel_server_without_secret,
):
    """Webhook fails closed when no secret is configured."""
    response = await _post_webhook(
        http_channel_server_without_secret,
        {"content": "test message"},
    )

    assert response.status_code == 503
    data = response.json()
    assert data["status"] == "error"
    assert "Webhook authentication not configured" in data.get("response", "")


async def test_webhook_hmac_signature_valid(http_channel_server):
    """Valid X-Hub-Signature-256 HMAC signature is accepted."""
    body = {"content": "hello from webhook"}
    signature = compute_signature(HTTP_WEBHOOK_SECRET, json.dumps(body).encode())

    response = await _post_webhook(http_channel_server, body, signature=signature)

    assert response.status_code == 200, (
        f"Expected 200, got {response.status_code}: {response.text}"
    )
    data = response.json()
    assert data["status"] == "accepted"


async def test_webhook_invalid_hmac_signature_rejected(http_channel_server):
    """Invalid X-Hub-Signature-256 signature is rejected with 401."""
    response = await _post_webhook(
        http_channel_server,
        {"content": "hello"},
        signature="sha256=0000000000000000000000000000000000000000000000000000000000000000",
    )

    assert response.status_code == 401
    data = response.json()
    assert data["status"] == "error"
    assert "Invalid webhook signature" in data.get("response", "")


async def test_webhook_wrong_secret_rejected(http_channel_server):
    """Signature computed with wrong secret is rejected."""
    body = {"content": "hello"}
    signature = compute_signature("wrong-secret", json.dumps(body).encode())

    response = await _post_webhook(http_channel_server, body, signature=signature)

    assert response.status_code == 401
    assert response.json()["status"] == "error"


async def test_webhook_missing_signature_header_rejected(http_channel_server):
    """Missing X-Hub-Signature-256 header is rejected when no body secret is provided."""
    response = await _post_webhook(http_channel_server, {"content": "hello"})

    assert response.status_code == 401
    data = response.json()
    assert "Webhook authentication required" in data.get("response", "")
    assert "X-Hub-Signature-256" in data.get("response", "")


async def test_webhook_deprecated_body_secret_still_works(http_channel_server):
    """Deprecated body secret support still accepts old clients."""
    response = await _post_webhook(
        http_channel_server,
        {"content": "hello", "secret": HTTP_WEBHOOK_SECRET},
    )

    assert response.status_code == 200, (
        f"Expected 200, got {response.status_code}: {response.text}"
    )
    assert response.json()["status"] == "accepted"


async def test_webhook_header_takes_precedence_over_body_secret(http_channel_server):
    """Header signature wins when both header and body secret are provided."""
    body = {"content": "hello", "secret": "wrong-secret-in-body"}
    signature = compute_signature(HTTP_WEBHOOK_SECRET, json.dumps(body).encode())

    response = await _post_webhook(http_channel_server, body, signature=signature)

    assert response.status_code == 200
    assert response.json()["status"] == "accepted"


async def test_webhook_case_insensitive_header_lookup(http_channel_server):
    """HTTP headers are treated case-insensitively."""
    body = {"content": "hello"}
    body_bytes = json.dumps(body).encode()
    signature = compute_signature(HTTP_WEBHOOK_SECRET, body_bytes)

    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{http_channel_server}/webhook",
            content=body_bytes,
            headers={
                "Content-Type": "application/json",
                "x-hub-signature-256": signature,
            },
        )

    assert response.status_code == 200


async def test_webhook_wrong_content_type_rejected(http_channel_server):
    """Webhook only accepts application/json Content-Type."""
    body = {"content": "hello"}
    signature = compute_signature(HTTP_WEBHOOK_SECRET, json.dumps(body).encode())

    response = await _post_webhook(
        http_channel_server,
        body,
        signature=signature,
        content_type="text/plain",
    )

    assert response.status_code == 415
    assert "application/json" in response.json().get("response", "")


async def test_webhook_invalid_json_rejected(http_channel_server):
    """Invalid JSON in body is rejected."""
    body_bytes = b"not valid json"
    signature = compute_signature(HTTP_WEBHOOK_SECRET, body_bytes)

    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{http_channel_server}/webhook",
            content=body_bytes,
            headers={
                "Content-Type": "application/json",
                "X-Hub-Signature-256": signature,
            },
        )

    assert response.status_code in (400, 401)


async def test_webhook_message_queued_for_processing(http_channel_server):
    """Accepted webhook requests return a real message id."""
    body = {"content": "webhook test message 12345"}
    signature = compute_signature(HTTP_WEBHOOK_SECRET, json.dumps(body).encode())

    response = await _post_webhook(http_channel_server, body, signature=signature)

    assert response.status_code == 200
    data = response.json()
    assert data["status"] == "accepted"
    assert "message_id" in data
    assert data["message_id"] != "00000000-0000-0000-0000-000000000000"
