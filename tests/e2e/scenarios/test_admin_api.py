"""Admin API integration tests — user CRUD, secrets, suspend/activate."""

import uuid

import httpx
import pytest

from helpers import AUTH_TOKEN


@pytest.fixture()
async def admin_client(ironclaw_server):
    """Async HTTP client with admin auth headers."""
    async with httpx.AsyncClient(
        base_url=ironclaw_server,
        headers={
            "Authorization": f"Bearer {AUTH_TOKEN}",
            "Content-Type": "application/json",
        },
        timeout=10,
    ) as client:
        yield client


@pytest.fixture()
async def test_user(admin_client):
    """Create a test user and clean up after the test."""
    email = f"test-{uuid.uuid4().hex[:8]}@example.com"
    r = await admin_client.post("/api/admin/users", json={
        "display_name": "E2E Test User",
        "email": email,
        "role": "member",
    })
    assert r.status_code == 200
    data = r.json()
    yield data
    # Cleanup
    await admin_client.delete(f"/api/admin/users/{data['id']}")


# ---------------------------------------------------------------
# User CRUD
# ---------------------------------------------------------------


async def test_create_user(admin_client):
    email = f"test-{uuid.uuid4().hex[:8]}@example.com"
    r = await admin_client.post("/api/admin/users", json={
        "display_name": "Create Test",
        "email": email,
        "role": "member",
    })
    assert r.status_code == 200
    data = r.json()
    assert "id" in data
    assert "token" in data
    assert data["status"] == "active"
    assert data["role"] == "member"
    # Cleanup
    await admin_client.delete(f"/api/admin/users/{data['id']}")


async def test_list_users_contains_new_user(admin_client, test_user):
    r = await admin_client.get("/api/admin/users")
    assert r.status_code == 200
    ids = [u["id"] for u in r.json()["users"]]
    assert test_user["id"] in ids


async def test_get_user_detail(admin_client, test_user):
    r = await admin_client.get(f"/api/admin/users/{test_user['id']}")
    assert r.status_code == 200
    data = r.json()
    assert data["display_name"] == "E2E Test User"
    assert data["id"] == test_user["id"]


async def test_update_user(admin_client, test_user):
    r = await admin_client.patch(f"/api/admin/users/{test_user['id']}", json={
        "display_name": "Updated Name",
        "metadata": {"ref": "abound-123"},
    })
    assert r.status_code == 200
    data = r.json()
    assert data["display_name"] == "Updated Name"
    assert data["metadata"]["ref"] == "abound-123"


# ---------------------------------------------------------------
# Suspend / Activate
# ---------------------------------------------------------------


async def test_suspend_and_activate(admin_client, test_user):
    uid = test_user["id"]

    r = await admin_client.post(f"/api/admin/users/{uid}/suspend")
    assert r.status_code == 200
    assert r.json()["status"] == "suspended"

    r = await admin_client.post(f"/api/admin/users/{uid}/activate")
    assert r.status_code == 200
    assert r.json()["status"] == "active"


# ---------------------------------------------------------------
# Secrets
# ---------------------------------------------------------------


async def test_secret_lifecycle(admin_client, test_user):
    uid = test_user["id"]

    # Create
    r = await admin_client.put(
        f"/api/admin/users/{uid}/secrets/abound_token",
        json={"value": "secret-value", "provider": "abound"},
    )
    assert r.status_code == 200
    assert r.json()["name"] == "abound_token"

    # List
    r = await admin_client.get(f"/api/admin/users/{uid}/secrets")
    assert r.status_code == 200
    names = [s["name"] for s in r.json()["secrets"]]
    assert "abound_token" in names

    # Delete
    r = await admin_client.delete(f"/api/admin/users/{uid}/secrets/abound_token")
    assert r.status_code == 200
    assert r.json()["deleted"] is True


# ---------------------------------------------------------------
# Delete
# ---------------------------------------------------------------


async def test_delete_user_and_verify_gone(admin_client):
    email = f"test-{uuid.uuid4().hex[:8]}@example.com"
    r = await admin_client.post("/api/admin/users", json={
        "display_name": "Delete Me",
        "email": email,
        "role": "member",
    })
    uid = r.json()["id"]

    r = await admin_client.delete(f"/api/admin/users/{uid}")
    assert r.status_code == 200
    assert r.json()["deleted"] is True

    r = await admin_client.get(f"/api/admin/users/{uid}")
    assert r.status_code == 404
