"""Scenario 3: Skills add, edit, delete, and read-only source lifecycle."""

import json
from urllib.parse import unquote, urlparse

from helpers import SEL


MOCK_INSTALLED_SKILL = {
    "name": "markdown-helper",
    "description": "Deterministic E2E skill for markdown workflows.",
    "version": "1.0.0",
    "trust": "Installed",
    "source": "Installed",
    "source_kind": "installed",
    "keywords": ["markdown", "e2e"],
    "usage_hint": "Type `/markdown-helper` in chat to force-activate this skill.",
    "has_requirements": False,
    "has_scripts": False,
    "can_edit": True,
    "can_delete": True,
}

MOCK_SYSTEM_SKILL = {
    "name": "system-helper",
    "description": "Read-only system helper.",
    "version": "1.0.0",
    "trust": "Trusted",
    "source": "System",
    "source_kind": "system",
    "keywords": ["system"],
    "has_requirements": False,
    "has_scripts": False,
    "can_edit": False,
    "can_delete": False,
}

MOCK_WORKSPACE_SKILL = {
    "name": "workspace-helper",
    "description": "Read-only workspace helper.",
    "version": "1.0.0",
    "trust": "Trusted",
    "source": "Workspace",
    "source_kind": "workspace",
    "keywords": ["workspace"],
    "has_requirements": False,
    "has_scripts": False,
    "can_edit": False,
    "can_delete": False,
}

MOCK_SKILL_CONTENT = (
    "---\n"
    "name: markdown-helper\n"
    "description: Deterministic E2E skill for markdown workflows.\n"
    "---\n\n"
    "# Markdown Helper\n"
)


async def go_to_skills(page):
    """Navigate to Settings > Skills subtab."""
    await page.locator(SEL["tab_button"].format(tab="settings")).click()
    await page.locator(SEL["settings_subtab"].format(subtab="skills")).click()
    await page.locator(SEL["settings_subpanel"].format(subtab="skills")).wait_for(
        state="visible", timeout=5000
    )


async def mock_skills_api(page, initial_skills=None):
    """Mock skills API endpoints used by the browser lifecycle tests."""
    installed = [dict(skill) for skill in (initial_skills or [])]
    install_requests = []
    update_requests = []

    async def fulfill_json(route, payload):
        await route.fulfill(json=payload, headers={"Cache-Control": "no-store"})

    async def handle(route):
        nonlocal installed
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/skills" and request.method == "GET":
            await fulfill_json(route, {"skills": installed, "count": len(installed)})
            return

        if path == "/api/webchat/v2/skills/install" and request.method == "POST":
            payload = json.loads(request.post_data or "{}")
            install_requests.append(payload)
            if not any(skill["name"] == payload.get("name") for skill in installed):
                skill = dict(MOCK_INSTALLED_SKILL)
                skill["name"] = payload.get("name", skill["name"])
                installed = [skill]
            await fulfill_json(
                route,
                {"success": True, "message": f"Skill '{payload.get('name')}' installed"},
            )
            return

        if path.startswith("/api/webchat/v2/skills/") and request.method == "GET":
            name = unquote(path.removeprefix("/api/webchat/v2/skills/"))
            await fulfill_json(route, {"name": name, "content": MOCK_SKILL_CONTENT})
            return

        if path.startswith("/api/webchat/v2/skills/") and request.method == "PUT":
            name = unquote(path.removeprefix("/api/webchat/v2/skills/"))
            update_requests.append(
                {
                    "name": name,
                    "headers": request.headers,
                    "body": json.loads(request.post_data or "{}"),
                }
            )
            await fulfill_json(
                route,
                {"success": True, "message": f"Skill '{name}' updated"},
            )
            return

        if path.startswith("/api/webchat/v2/skills/") and request.method == "DELETE":
            name = unquote(path.removeprefix("/api/webchat/v2/skills/"))
            installed = [skill for skill in installed if skill["name"] != name]
            await fulfill_json(
                route,
                {"success": True, "message": f"Skill '{name}' removed"},
            )
            return

        await route.continue_()

    await page.route("**/api/webchat/v2/skills**", handle)
    return {"install_requests": install_requests, "update_requests": update_requests}


async def add_mock_skill(page):
    await page.get_by_label("Skill name").fill("markdown-helper")
    await page.get_by_label("SKILL.md content").fill(MOCK_SKILL_CONTENT)
    async with page.expect_response(
        lambda r: "/api/webchat/v2/skills/install" in r.url
    ) as install_response:
        await page.get_by_role("button", name="Add").click()
    response = await install_response.value
    assert response.ok


async def test_skills_tab_visible(page):
    """Skills subtab shows the mounted-skill add form."""
    await go_to_skills(page)

    assert await page.get_by_text("Add skill").is_visible()
    assert await page.get_by_label("Skill name").is_visible()
    assert await page.get_by_label("SKILL.md content").is_visible()


async def test_skills_add_and_delete(page):
    """Add a user-mounted skill through the form, then delete it."""
    mock_api = await mock_skills_api(page)
    await go_to_skills(page)

    await add_mock_skill(page)
    assert mock_api["install_requests"] == [
        {"name": "markdown-helper", "content": MOCK_SKILL_CONTENT}
    ]

    installed = page.locator(SEL["skill_installed"])
    await installed.first.wait_for(state="visible", timeout=5000)
    assert "markdown-helper" in await installed.first.inner_text()

    page.on("dialog", lambda dialog: dialog.accept())
    async with page.expect_response(
        lambda r: "/api/webchat/v2/skills/markdown-helper" in r.url
        and r.request.method == "DELETE"
    ) as delete_response:
        await installed.first.locator("button", has_text="Delete").click()
    response = await delete_response.value
    assert response.ok

    await page.wait_for_function(
        """(selector) => document.querySelectorAll(selector).length === 0""",
        arg=SEL["skill_installed"],
        timeout=5000,
    )


async def test_reborn_skills_delete_uses_native_confirm_dialog(page):
    """Reborn Skills settings delete confirms before calling the v2 endpoint."""
    await mock_skills_api(page, initial_skills=[MOCK_INSTALLED_SKILL])
    await go_to_skills(page)

    installed = page.locator(SEL["skill_installed"]).filter(has_text="markdown-helper")
    await installed.first.wait_for(state="visible", timeout=5000)

    async with page.expect_dialog() as dialog_info:
        await installed.first.locator("button", has_text="Delete").click()
    dialog = await dialog_info.value
    assert dialog.type == "confirm"
    assert "markdown-helper" in dialog.message

    async with page.expect_response(
        lambda r: "/api/webchat/v2/skills/markdown-helper" in r.url
        and r.request.method == "DELETE"
    ) as delete_response:
        await dialog.accept()
    response = await delete_response.value
    assert response.ok

    await page.wait_for_function(
        """(selector) => document.querySelectorAll(selector).length === 0""",
        arg=SEL["skill_installed"],
        timeout=5000,
    )


async def test_skills_edit_user_managed_skill(page):
    """Edit a mocked user-managed skill through the real Settings UI flow."""
    mock_api = await mock_skills_api(page)
    await go_to_skills(page)
    await add_mock_skill(page)

    installed = page.locator(SEL["skill_installed"])
    await installed.first.wait_for(state="visible", timeout=5000)

    async with page.expect_response(
        lambda r: "/api/webchat/v2/skills/markdown-helper" in r.url
        and r.request.method == "GET"
    ) as content_response:
        await installed.first.locator("button", has_text="Edit").click()
    response = await content_response.value
    assert response.ok

    editor = installed.first.locator("textarea")
    await editor.wait_for(state="visible", timeout=5000)
    await editor.fill(
        "---\nname: markdown-helper\ndescription: Updated E2E skill\n---\n\n# Updated\n"
    )

    async with page.expect_response(
        lambda r: "/api/webchat/v2/skills/markdown-helper" in r.url
        and r.request.method == "PUT"
    ) as update_response:
        await installed.first.locator("button", has_text="Save").click()
    response = await update_response.value
    assert response.ok

    assert len(mock_api["update_requests"]) == 1
    update = mock_api["update_requests"][0]
    assert update["headers"].get("x-confirm-action") == "true"
    assert "Updated E2E skill" in update["body"]["content"]


async def test_skills_read_only_sources_hide_edit_and_delete(page):
    """System and workspace skills remain visible but not editable/deletable."""
    await mock_skills_api(page, initial_skills=[MOCK_SYSTEM_SKILL, MOCK_WORKSPACE_SKILL])
    await go_to_skills(page)

    installed = page.locator(SEL["skill_installed"])
    await installed.first.wait_for(state="visible", timeout=5000)

    system_card = installed.filter(has_text="system-helper")
    workspace_card = installed.filter(has_text="workspace-helper")
    assert await system_card.count() == 1
    assert await workspace_card.count() == 1

    for card in [system_card, workspace_card]:
        assert await card.locator("button", has_text="Edit").count() == 0
        assert await card.locator("button", has_text="Delete").count() == 0
