"""Scenario 3: Skills search, install, edit, and delete lifecycle."""

import json
from urllib.parse import unquote, urlparse

from helpers import SEL


MOCK_CATALOG_SKILL = {
    "slug": "e2e/markdown-helper",
    "name": "Markdown Helper",
    "description": "Deterministic E2E skill for markdown workflows.",
    "version": "1.0.0",
    "score": 1.0,
    "updatedAt": 1778000000000,
    "stars": 12,
    "downloads": 3456,
    "owner": "e2e",
    "installed": False,
}

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


async def go_to_skills(page):
    """Navigate to Settings > Skills subtab."""
    await page.locator(SEL["tab_button"].format(tab="settings")).click()
    await page.locator(SEL["settings_subtab"].format(subtab="skills")).click()
    await page.locator(SEL["settings_subpanel"].format(subtab="skills")).wait_for(
        state="visible", timeout=5000
    )


async def mock_skills_api(page, initial_skills=None):
    """Mock skills API endpoints used by the browser lifecycle tests.

    These tests validate the Settings > Skills UI contract, not live ClawHub
    availability. Keeping the API local avoids skip-on-network behavior while
    still exercising the real browser code paths.
    """
    installed = [dict(skill) for skill in (initial_skills or [])]
    install_requests = []
    update_requests = []

    async def fulfill_json(route, payload):
        await route.fulfill(
            json=payload,
            headers={"Cache-Control": "no-store"},
        )

    async def handle(route):
        nonlocal installed
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/skills" and request.method == "GET":
            await fulfill_json(route, {"skills": installed, "count": len(installed)})
            return

        if path == "/api/webchat/v2/skills/search" and request.method == "POST":
            catalog_skill = dict(MOCK_CATALOG_SKILL)
            catalog_skill["installed"] = any(
                skill["name"] == MOCK_INSTALLED_SKILL["name"] for skill in installed
            )
            await fulfill_json(
                route,
                {
                    "catalog": [catalog_skill],
                    "installed": installed,
                    "registry_url": "https://clawhub.example.test",
                },
            )
            return

        if path == "/api/webchat/v2/skills/install" and request.method == "POST":
            install_requests.append(json.loads(request.post_data or "{}"))
            if not any(skill["name"] == MOCK_INSTALLED_SKILL["name"] for skill in installed):
                installed = [dict(MOCK_INSTALLED_SKILL)]
            await fulfill_json(
                route,
                {
                    "success": True,
                    "message": "Skill 'markdown-helper' installed",
                },
            )
            return

        if path.startswith("/api/webchat/v2/skills/") and request.method == "GET":
            name = unquote(path.removeprefix("/api/webchat/v2/skills/"))
            await fulfill_json(
                route,
                {
                    "name": name,
                    "content": (
                        f"---\nname: {name}\ndescription: Deterministic E2E skill for "
                        "markdown workflows.\n---\n\n# Markdown Helper\n"
                    ),
                },
            )
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


async def test_skills_tab_visible(page):
    """Skills subtab shows the search interface."""
    await go_to_skills(page)

    search_input = page.locator(SEL["skill_search_input"])
    assert await search_input.is_visible(), "Skills search input not visible"


async def test_skills_search(page):
    """Search renders deterministic catalog results without live ClawHub."""
    await mock_skills_api(page)
    await go_to_skills(page)

    search_input = page.locator(SEL["skill_search_input"])
    await search_input.fill("markdown")
    await search_input.press("Enter")

    results = page.locator(SEL["skill_search_result"])
    await results.first.wait_for(state="visible", timeout=5000)

    count = await results.count()
    assert count >= 1, "Expected at least 1 search result"
    assert "Markdown Helper" in await results.first.inner_text()


async def test_skills_install_and_delete(page):
    """Install a mocked catalog skill from search results, then delete it."""
    mock_api = await mock_skills_api(page)
    await go_to_skills(page)

    search_input = page.locator(SEL["skill_search_input"])
    await search_input.fill("markdown")
    await search_input.press("Enter")

    results = page.locator(SEL["skill_search_result"])
    await results.first.wait_for(state="visible", timeout=5000)

    install_btn = results.first.locator("button", has_text="Install")
    assert await install_btn.count() == 1, "Expected mocked catalog skill to be installable"
    async with page.expect_response(lambda r: "/api/webchat/v2/skills/install" in r.url) as install_response:
        await install_btn.click()
    response = await install_response.value
    assert response.ok
    assert mock_api["install_requests"] == [
        {"name": MOCK_CATALOG_SKILL["name"], "slug": MOCK_CATALOG_SKILL["slug"]}
    ], "Install request should use the catalog skill name and slug"

    # The app refreshes the installed-skills list after a successful install;
    # waiting on the DOM keeps this as a black-box UI contract.
    installed = page.locator(SEL["skill_installed"])
    await installed.first.wait_for(state="visible", timeout=5000)

    installed_count = await installed.count()
    assert installed_count >= 1, "Skill should appear in installed list after install"
    assert "markdown-helper" in await installed.first.inner_text()

    delete_btn = installed.first.locator("button", has_text="Delete")
    assert await delete_btn.count() == 1, "Installed mocked skill should be deletable"
    await delete_btn.click()

    confirm_btn = page.locator(SEL["confirm_modal_btn"])
    await confirm_btn.wait_for(state="visible", timeout=5000)
    await confirm_btn.click()

    await page.wait_for_function(
        """(selector) => document.querySelectorAll(selector).length === 0""",
        arg=SEL["skill_installed"],
        timeout=5000,
    )


async def test_skills_edit_user_managed_skill(page):
    """Edit a mocked user-managed skill through the real Settings UI flow."""
    mock_api = await mock_skills_api(page)
    await go_to_skills(page)

    search_input = page.locator(SEL["skill_search_input"])
    await search_input.fill("markdown")
    await search_input.press("Enter")

    results = page.locator(SEL["skill_search_result"])
    await results.first.wait_for(state="visible", timeout=5000)
    await results.first.locator("button", has_text="Install").click()

    installed = page.locator(SEL["skill_installed"])
    await installed.first.wait_for(state="visible", timeout=5000)

    async with page.expect_response(
        lambda r: "/api/webchat/v2/skills/markdown-helper" in r.url and r.request.method == "GET"
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
        lambda r: "/api/webchat/v2/skills/markdown-helper" in r.url and r.request.method == "PUT"
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
