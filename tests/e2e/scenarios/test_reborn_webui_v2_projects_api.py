"""Project lifecycle and membership E2E against the shipping binary."""

import uuid

import httpx

from reborn_webui_harness import reborn_bearer_headers

pytest_plugins = ["reborn_webui_harness"]


ADMIN_USERS = "/api/webchat/v2/admin/users"
PROJECTS = "/api/webchat/v2/projects"


async def _create_user(client: httpx.AsyncClient, suffix: str, label: str) -> dict:
    response = await client.post(
        ADMIN_USERS,
        json={
            "display_name": f"Project E2E {label} {suffix}",
            "email": f"project-e2e-{label}-{suffix}@example.com",
            "role": "member",
        },
    )
    assert response.status_code == 200, response.text
    body = response.json()
    return {
        "id": body["user"]["user_id"],
        "token": body["api_token"],
    }


def _project_ids(response: httpx.Response) -> set[str]:
    assert response.status_code == 200, response.text
    return {project["project_id"] for project in response.json()["projects"]}


async def test_project_lifecycle_membership_and_restart_persistence_served(
    reborn_v2_restartable_server,
):
    """Exercise CRUD, role enforcement, revocation, and durable readback."""
    state, start, stop = reborn_v2_restartable_server
    suffix = uuid.uuid4().hex[:8]
    project_name = f"Project lifecycle {suffix}"

    async with httpx.AsyncClient(
        base_url=state["base_url"],
        headers=reborn_bearer_headers(),
        timeout=15,
    ) as operator:
        owner_user = await _create_user(operator, suffix, "owner")
        member = await _create_user(operator, suffix, "member")
        non_member = await _create_user(operator, suffix, "outsider")

    async with httpx.AsyncClient(
        base_url=state["base_url"],
        headers=reborn_bearer_headers(owner_user["token"]),
        timeout=15,
    ) as owner:
        owner_session = await owner.get("/api/webchat/v2/session")
        assert owner_session.status_code == 200, owner_session.text
        assert owner_session.json()["user_id"] == owner_user["id"]

        created = await owner.post(
            PROJECTS,
            json={
                "name": project_name,
                "description": "Created by the project lifecycle E2E",
                "icon": "folder",
                "color": "blue",
                "metadata": {"source": "e2e", "version": 1},
            },
        )
        assert created.status_code == 200, created.text
        project = created.json()["project"]
        project_id = project["project_id"]
        project_path = f"{PROJECTS}/{project_id}"
        members_path = f"{project_path}/members"
        assert project["name"] == project_name
        assert project["role"] == "owner"
        assert project["state"] == "active"

        updated = await owner.post(
            project_path,
            json={
                "name": f"{project_name} updated",
                "description": "Updated by the owner",
                "metadata": {"source": "e2e", "version": 2},
            },
        )
        assert updated.status_code == 200, updated.text
        assert updated.json()["project"]["description"] == "Updated by the owner"

        readback = await owner.get(project_path)
        assert readback.status_code == 200, readback.text
        assert readback.json()["project"]["metadata"]["version"] == 2
        assert project_id in _project_ids(await owner.get(PROJECTS))

        granted = await owner.post(
            members_path,
            json={"user_id": member["id"], "role": "viewer"},
        )
        assert granted.status_code == 200, granted.text
        assert granted.json()["role"] == "viewer"

        members = await owner.get(members_path)
        assert members.status_code == 200, members.text
        active_roles = {
            item["user_id"]: item["role"]
            for item in members.json()["members"]
            if item["status"] == "active"
        }
        assert active_roles == {member["id"]: "viewer"}

    async with httpx.AsyncClient(
        base_url=state["base_url"],
        headers=reborn_bearer_headers(member["token"]),
        timeout=15,
    ) as viewer:
        visible = await viewer.get(project_path)
        assert visible.status_code == 200, visible.text
        assert visible.json()["project"]["role"] == "viewer"
        denied_update = await viewer.post(project_path, json={"name": "not allowed"})
        assert denied_update.status_code == 403, denied_update.text

    async with httpx.AsyncClient(
        base_url=state["base_url"],
        headers=reborn_bearer_headers(non_member["token"]),
        timeout=15,
    ) as outsider:
        hidden = await outsider.get(project_path)
        assert hidden.status_code == 404, hidden.text
        assert hidden.json()["error"] == "not_found"
        assert project_id not in _project_ids(await outsider.get(PROJECTS))

    async with httpx.AsyncClient(
        base_url=state["base_url"],
        headers=reborn_bearer_headers(owner_user["token"]),
        timeout=15,
    ) as owner:
        promoted = await owner.post(
            f"{members_path}/{member['id']}",
            json={"role": "editor"},
        )
        assert promoted.status_code == 200, promoted.text
        assert promoted.json()["role"] == "editor"

    await stop()
    restarted_url = await start()

    async with httpx.AsyncClient(
        base_url=restarted_url,
        headers=reborn_bearer_headers(member["token"]),
        timeout=15,
    ) as editor:
        session = await editor.get("/api/webchat/v2/session")
        assert session.status_code == 200, session.text
        assert session.json()["user_id"] == member["id"]

        persisted = await editor.get(project_path)
        assert persisted.status_code == 200, persisted.text
        assert persisted.json()["project"]["role"] == "editor"
        assert persisted.json()["project"]["metadata"]["version"] == 2

        editor_update = await editor.post(
            project_path,
            json={"description": "Updated by the editor after restart"},
        )
        assert editor_update.status_code == 200, editor_update.text
        assert (
            editor_update.json()["project"]["description"]
            == "Updated by the editor after restart"
        )

    async with httpx.AsyncClient(
        base_url=restarted_url,
        headers=reborn_bearer_headers(owner_user["token"]),
        timeout=15,
    ) as owner:
        persisted_members = await owner.get(members_path)
        assert persisted_members.status_code == 200, persisted_members.text
        member_record = next(
            item
            for item in persisted_members.json()["members"]
            if item["user_id"] == member["id"]
        )
        assert member_record["role"] == "editor"
        assert member_record["status"] == "active"

        revoked = await owner.delete(f"{members_path}/{member['id']}")
        assert revoked.status_code == 204, revoked.text

    async with httpx.AsyncClient(
        base_url=restarted_url,
        headers=reborn_bearer_headers(member["token"]),
        timeout=15,
    ) as former_member:
        hidden_after_revoke = await former_member.get(project_path)
        assert hidden_after_revoke.status_code == 404, hidden_after_revoke.text
        assert project_id not in _project_ids(await former_member.get(PROJECTS))

    async with httpx.AsyncClient(
        base_url=restarted_url,
        headers=reborn_bearer_headers(owner_user["token"]),
        timeout=15,
    ) as owner:
        deleted = await owner.delete(project_path)
        assert deleted.status_code == 204, deleted.text

        missing = await owner.get(project_path)
        assert missing.status_code == 404, missing.text
        assert project_id not in _project_ids(await owner.get(PROJECTS))

    async with httpx.AsyncClient(
        base_url=restarted_url,
        headers=reborn_bearer_headers(),
        timeout=15,
    ) as operator:
        for user_id in (owner_user["id"], member["id"], non_member["id"]):
            removed_user = await operator.delete(f"{ADMIN_USERS}/{user_id}")
            assert removed_user.status_code == 200, removed_user.text
            assert removed_user.json() == {"user_id": user_id, "deleted": True}
