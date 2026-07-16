"""Admin user-management E2E against the real ``ironclaw serve`` binary.

Drives the WebChat v2 admin surface (`/api/webchat/v2/admin/*`, backed by
`ironclaw_product_workflow::AdminUserService`) over HTTP against the standalone
Reborn binary — so unlike the crate-tier `admin_api_e2e.rs` (which composes the
router in-process), this exercises serve.rs's real wiring: the operator
env-bearer authenticator, and the signed-session-store token minter that must
share its signing secret with the SSO login surface.

The flagship proof is `test_created_user_token_authenticates_as_that_user`: an
admin creates a user, receives the one-time `api_token`, and that token then
authenticates a follow-up `/session` request AS the new user — end-to-end
through the real minter + session-store wiring in the binary.

Authorization: the operator env-bearer (`IRONCLAW_REBORN_WEBUI_TOKEN`) is an
implicit owner, so it clears the admin boundary. Last-admin protection (409) is
covered at the crate tier; here every lifecycle/delete user stays a `member`,
which can never strand the tenant's admins.
"""

import uuid

import httpx
import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    reborn_bearer_headers,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)

ADMIN_BASE = "/api/webchat/v2/admin"


@pytest.fixture()
async def admin_client(reborn_v2_server):
    """Async HTTP client bearing the operator (implicit-owner) token."""
    async with httpx.AsyncClient(
        base_url=reborn_v2_server,
        headers={**reborn_bearer_headers(), "Content-Type": "application/json"},
        timeout=15,
    ) as client:
        yield client


@pytest.fixture()
async def test_user(admin_client):
    """Create a member user, yield its record + one-time token, delete after."""
    suffix = uuid.uuid4().hex[:8]
    email = f"test-{suffix}@example.com"
    display_name = f"E2E Test User {suffix}"
    r = await admin_client.post(
        f"{ADMIN_BASE}/users",
        json={"display_name": display_name, "email": email, "role": "member"},
    )
    assert r.status_code == 200, r.text
    body = r.json()
    user = body["user"]
    yield {
        "id": user["user_id"],
        "email": email,
        "display_name": display_name,
        "token": body["api_token"],
    }
    await admin_client.delete(f"{ADMIN_BASE}/users/{user['user_id']}")


# ---------------------------------------------------------------
# Create + one-time token
# ---------------------------------------------------------------


async def test_create_user_returns_record_and_one_time_token(admin_client):
    email = f"test-{uuid.uuid4().hex[:8]}@example.com"
    r = await admin_client.post(
        f"{ADMIN_BASE}/users",
        json={"display_name": "Create Test", "email": email, "role": "member"},
    )
    assert r.status_code == 200, r.text
    body = r.json()
    # v2 shape: the record is nested under `user`; the token is a sibling
    # exposed exactly once here.
    assert body["user"]["user_id"]
    assert body["user"]["status"] == "active"
    assert body["user"]["role"] == "member"
    assert body["api_token"]
    await admin_client.delete(f"{ADMIN_BASE}/users/{body['user']['user_id']}")


async def test_created_user_token_authenticates_as_that_user(admin_client, reborn_v2_server):
    """Flagship: the one-time api_token logs in AS the new user.

    Exercises the whole mint -> return -> validate chain through the real serve
    binary: the admin minter's signed session store must share its signing
    secret with the SSO login surface's store, or this bearer would not
    validate at `/session`.
    """
    email = f"test-{uuid.uuid4().hex[:8]}@example.com"
    r = await admin_client.post(
        f"{ADMIN_BASE}/users",
        json={"display_name": "Token Login", "email": email, "role": "member"},
    )
    assert r.status_code == 200, r.text
    created = r.json()
    new_user_id = created["user"]["user_id"]
    api_token = created["api_token"]

    # The minted token is distinct from the operator bearer that created the user.
    assert api_token != REBORN_V2_AUTH_TOKEN

    async with httpx.AsyncClient(base_url=reborn_v2_server, timeout=15) as user_client:
        session = await user_client.get(
            "/api/webchat/v2/session",
            headers={"Authorization": f"Bearer {api_token}"},
        )
    assert session.status_code == 200, session.text
    assert session.json()["user_id"] == new_user_id

    await admin_client.delete(f"{ADMIN_BASE}/users/{new_user_id}")


# ---------------------------------------------------------------
# Read + update
# ---------------------------------------------------------------


async def test_list_users_contains_new_user(admin_client, test_user):
    r = await admin_client.get(f"{ADMIN_BASE}/users")
    assert r.status_code == 200, r.text
    ids = [u["user_id"] for u in r.json()["users"]]
    assert test_user["id"] in ids


async def test_get_user_detail(admin_client, test_user):
    r = await admin_client.get(f"{ADMIN_BASE}/users/{test_user['id']}")
    assert r.status_code == 200, r.text
    user = r.json()["user"]
    assert user["user_id"] == test_user["id"]
    assert user["display_name"] == test_user["display_name"]


async def test_admin_token_visibility_matches_user_creation_lifecycle(
    admin_client, reborn_v2_page, reborn_v2_server, test_user
):
    """Creation shows the one-time token; existing-user details do not."""
    await reborn_v2_page.goto(
        f"{reborn_v2_server}/v2/admin/users?token={REBORN_V2_AUTH_TOKEN}"
    )
    created_user_id = None
    try:
        display_name = f"UI Token User {uuid.uuid4().hex[:8]}"
        email = f"ui-token-{uuid.uuid4().hex[:8]}@example.com"
        await reborn_v2_page.get_by_role(
            "button", name=SEL_V2["admin_new_user_button_name"], exact=True
        ).click()
        create_form = reborn_v2_page.locator(SEL_V2["admin_create_form"])
        await create_form.locator(SEL_V2["admin_display_name_input"]).fill(display_name)
        await create_form.locator(SEL_V2["admin_email_input"]).fill(email)

        async with reborn_v2_page.expect_response(
            lambda response: response.request.method == "POST"
            and response.url.endswith(f"{ADMIN_BASE}/users")
        ) as response_info:
            await create_form.get_by_role(
                "button", name=SEL_V2["admin_create_user_button_name"], exact=True
            ).click()
        create_response = await response_info.value
        assert create_response.status == 200
        created = await create_response.json()
        created_user_id = created["user"]["user_id"]
        one_time_token = created["api_token"]

        await expect(
            reborn_v2_page.get_by_text(
                SEL_V2["admin_token_created_text"], exact=True
            )
        ).to_be_visible(timeout=15000)
        await expect(
            reborn_v2_page.locator(SEL_V2["admin_token_value"]).filter(
                has_text=one_time_token
            )
        ).to_be_visible()
        await expect(
            reborn_v2_page.get_by_text(
                SEL_V2["admin_token_description_text"], exact=True
            )
        ).to_be_visible()

        await reborn_v2_page.get_by_role(
            "button", name=test_user["display_name"], exact=True
        ).click()
        await expect(
            reborn_v2_page.get_by_role(
                "heading", name=test_user["display_name"], exact=True
            )
        ).to_be_visible(timeout=15000)
        await expect(
            reborn_v2_page.get_by_role(
                "button",
                name=SEL_V2["admin_create_token_button_name"],
                exact=True,
            )
        ).to_have_count(0)
        await expect(
            reborn_v2_page.get_by_text(
                SEL_V2["admin_token_description_text"], exact=True
            )
        ).to_have_count(0)
    finally:
        if created_user_id is not None:
            cleanup = await admin_client.delete(
                f"{ADMIN_BASE}/users/{created_user_id}"
            )
            assert cleanup.status_code == 200, cleanup.text


async def test_update_user_profile(admin_client, test_user):
    r = await admin_client.patch(
        f"{ADMIN_BASE}/users/{test_user['id']}",
        json={"display_name": "Updated Name", "metadata": {"ref": "abound-123"}},
    )
    assert r.status_code == 200, r.text
    user = r.json()["user"]
    assert user["display_name"] == "Updated Name"
    assert user["metadata"]["ref"] == "abound-123"


async def test_set_role_endpoint(admin_client):
    """The dedicated `/role` route promotes a member to admin (v2 shape:
    `POST .../role` with a body, not a bare `/promote`). The user is left in
    place — deleting a sole admin would trip last-admin protection, which is a
    crate-tier concern; this asserts only that the route mutates the role."""
    email = f"test-{uuid.uuid4().hex[:8]}@example.com"
    created = (
        await admin_client.post(
            f"{ADMIN_BASE}/users",
            json={"display_name": "Role Target", "email": email, "role": "member"},
        )
    ).json()
    uid = created["user"]["user_id"]

    r = await admin_client.post(f"{ADMIN_BASE}/users/{uid}/role", json={"role": "admin"})
    assert r.status_code == 200, r.text
    assert r.json()["user"]["role"] == "admin"

    # Confirmed durable on read-back.
    got = await admin_client.get(f"{ADMIN_BASE}/users/{uid}")
    assert got.json()["user"]["role"] == "admin"


# ---------------------------------------------------------------
# Status (member -> suspended -> active; never strands admins)
# ---------------------------------------------------------------


async def test_suspend_and_activate(admin_client, test_user):
    uid = test_user["id"]

    r = await admin_client.post(f"{ADMIN_BASE}/users/{uid}/status", json={"status": "suspended"})
    assert r.status_code == 200, r.text
    assert r.json()["user"]["status"] == "suspended"

    r = await admin_client.post(f"{ADMIN_BASE}/users/{uid}/status", json={"status": "active"})
    assert r.status_code == 200, r.text
    assert r.json()["user"]["status"] == "active"


# ---------------------------------------------------------------
# Per-user secrets
# ---------------------------------------------------------------


async def test_secret_lifecycle(admin_client, test_user):
    uid = test_user["id"]

    r = await admin_client.put(
        f"{ADMIN_BASE}/users/{uid}/secrets/abound_token",
        json={"value": "secret-value"},
    )
    assert r.status_code == 200, r.text
    assert r.json()["secret"]["handle"] == "abound_token"

    r = await admin_client.get(f"{ADMIN_BASE}/users/{uid}/secrets")
    assert r.status_code == 200, r.text
    handles = [s["handle"] for s in r.json()["secrets"]]
    assert "abound_token" in handles
    # The material is never echoed back through the list.
    assert "secret-value" not in r.text

    r = await admin_client.delete(f"{ADMIN_BASE}/users/{uid}/secrets/abound_token")
    assert r.status_code == 200, r.text
    assert r.json()["deleted"] is True


# ---------------------------------------------------------------
# Delete (member -> gone)
# ---------------------------------------------------------------


async def test_delete_user_and_verify_gone(admin_client):
    email = f"test-{uuid.uuid4().hex[:8]}@example.com"
    created = (
        await admin_client.post(
            f"{ADMIN_BASE}/users",
            json={"display_name": "Delete Me", "email": email, "role": "member"},
        )
    ).json()
    uid = created["user"]["user_id"]

    r = await admin_client.delete(f"{ADMIN_BASE}/users/{uid}")
    assert r.status_code == 200, r.text
    assert r.json()["deleted"] is True

    r = await admin_client.get(f"{ADMIN_BASE}/users/{uid}")
    assert r.status_code == 404


# ---------------------------------------------------------------
# Authorization boundary
# ---------------------------------------------------------------


async def test_admin_routes_require_auth(reborn_v2_server):
    """No bearer -> the admin surface rejects before any facade work."""
    async with httpx.AsyncClient(base_url=reborn_v2_server, timeout=15) as anon:
        r = await anon.get(f"{ADMIN_BASE}/users")
    assert r.status_code == 401
