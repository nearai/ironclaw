"""Workspace entities: CRUD, membership, scoped access, and isolation."""

import httpx
from helpers import AUTH_TOKEN, auth_headers, api_get, api_post


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

async def api_put(base_url: str, path: str, *, token: str = AUTH_TOKEN, **kwargs) -> httpx.Response:
    async with httpx.AsyncClient() as client:
        return await client.put(
            f"{base_url}{path}",
            headers=auth_headers(token),
            timeout=kwargs.pop("timeout", 10),
            **kwargs,
        )


async def api_delete(base_url: str, path: str, *, token: str = AUTH_TOKEN, **kwargs) -> httpx.Response:
    async with httpx.AsyncClient() as client:
        return await client.delete(
            f"{base_url}{path}",
            headers=auth_headers(token),
            timeout=kwargs.pop("timeout", 10),
            **kwargs,
        )


# ---------------------------------------------------------------------------
# Workspace CRUD
# ---------------------------------------------------------------------------

async def test_workspace_lifecycle(ironclaw_server):
    """Create, list, update, archive a workspace."""
    base = ironclaw_server

    # Create
    r = await api_post(base, "/api/workspaces", json={
        "name": "Test Team",
        "slug": "test-team",
        "description": "E2E workspace test",
        "settings": {},
    })
    assert r.status_code == 200, f"create failed: {r.status_code} {r.text}"
    ws = r.json()
    assert ws["slug"] == "test-team"
    assert ws["role"] == "owner"

    # List
    r = await api_get(base, "/api/workspaces")
    assert r.status_code == 200
    slugs = [w["slug"] for w in r.json()["workspaces"]]
    assert "test-team" in slugs

    # Detail
    r = await api_get(base, "/api/workspaces/test-team")
    assert r.status_code == 200
    assert r.json()["name"] == "Test Team"

    # Update
    r = await api_put(base, "/api/workspaces/test-team", json={
        "name": "Updated Team",
        "description": "Updated description",
        "settings": {"custom": True},
    })
    assert r.status_code == 200
    assert r.json()["name"] == "Updated Team"

    # Archive
    r = await api_post(base, "/api/workspaces/test-team/archive")
    assert r.status_code == 204

    # Archived workspace returns 410
    r = await api_get(base, "/api/workspaces/test-team")
    assert r.status_code == 410


# ---------------------------------------------------------------------------
# Membership and access control
# ---------------------------------------------------------------------------

async def test_workspace_membership(ironclaw_server):
    """Add members, enforce roles, prevent last-owner removal."""
    base = ironclaw_server

    # Create workspace
    r = await api_post(base, "/api/workspaces", json={
        "name": "Members Test",
        "slug": "members-test",
        "description": "",
        "settings": {},
    })
    assert r.status_code == 200

    # Create a second user via admin API
    r = await api_post(base, "/api/admin/users", json={
        "display_name": "Bob",
        "email": "bob@test.local",
        "role": "member",
    })
    assert r.status_code in (200, 201), f"create user failed: {r.status_code} {r.text}"
    bob_id = r.json()["id"]

    # Add bob as member
    r = await api_put(base, f"/api/workspaces/members-test/members/{bob_id}", json={
        "role": "member",
    })
    assert r.status_code == 204

    # List members
    r = await api_get(base, "/api/workspaces/members-test/members")
    assert r.status_code == 200
    members = r.json()["members"]
    member_ids = [m["user_id"] for m in members]
    assert bob_id in member_ids

    # Find our own user ID (the owner) from the members list
    owner_id = next(m["user_id"] for m in members if m["role"] == "owner")

    # Cannot remove the last owner (ourselves)
    r = await api_delete(base, f"/api/workspaces/members-test/members/{owner_id}")
    assert r.status_code == 409, f"expected 409 for last owner removal, got {r.status_code}"

    # Remove bob (non-owner removal is fine)
    r = await api_delete(base, f"/api/workspaces/members-test/members/{bob_id}")
    assert r.status_code == 204


# ---------------------------------------------------------------------------
# Workspace-scoped chat isolation
# ---------------------------------------------------------------------------

async def test_workspace_chat_isolation(ironclaw_server):
    """Messages sent in workspace scope don't appear in personal threads."""
    base = ironclaw_server

    # Create workspace
    r = await api_post(base, "/api/workspaces", json={
        "name": "Chat Isolation",
        "slug": "chat-isolation",
        "description": "",
        "settings": {},
    })
    assert r.status_code == 200

    # Create a thread in workspace scope
    r = await api_post(base, "/api/chat/thread/new", params={"workspace": "chat-isolation"})
    assert r.status_code == 200, f"thread create failed: {r.status_code} {r.text}"
    ws_thread_id = r.json()["id"]

    # Create a personal thread
    r = await api_post(base, "/api/chat/thread/new")
    assert r.status_code == 200
    personal_thread_id = r.json()["id"]

    # List personal threads -- workspace thread should NOT appear
    r = await api_get(base, "/api/chat/threads")
    assert r.status_code == 200
    personal_thread_ids = [t["id"] for t in r.json().get("threads", [])]
    assert ws_thread_id not in personal_thread_ids, "workspace thread leaked into personal list"
    assert personal_thread_id in personal_thread_ids, "personal thread missing from personal list"


# ---------------------------------------------------------------------------
# Slug validation
# ---------------------------------------------------------------------------

async def test_workspace_slug_validation(ironclaw_server):
    """Invalid slugs are rejected."""
    base = ironclaw_server

    for bad_slug in ["ab", "AB", "-bad", "bad-", "has spaces", "a" * 65]:
        r = await api_post(base, "/api/workspaces", json={
            "name": "Bad Slug Test",
            "slug": bad_slug,
            "description": "",
            "settings": {},
        })
        assert r.status_code == 400, f"slug '{bad_slug}' should be rejected, got {r.status_code}"


# ---------------------------------------------------------------------------
# Role validation
# ---------------------------------------------------------------------------

async def test_workspace_role_validation(ironclaw_server):
    """Invalid roles are rejected on member upsert."""
    base = ironclaw_server

    r = await api_post(base, "/api/workspaces", json={
        "name": "Role Test",
        "slug": "role-test",
        "description": "",
        "settings": {},
    })
    assert r.status_code == 200

    r = await api_put(base, "/api/workspaces/role-test/members/someone", json={
        "role": "superadmin",
    })
    assert r.status_code == 400, f"invalid role should be rejected, got {r.status_code}"
