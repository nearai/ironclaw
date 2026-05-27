"""Scenario 3: Skills search, install, and remove lifecycle."""

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
    "keywords": ["markdown", "e2e"],
    "usage_hint": "Type `/markdown-helper` in chat to force-activate this skill.",
    "has_requirements": False,
    "has_scripts": False,
}


async def go_to_skills(page):
    """Navigate to Settings > Skills subtab."""
    await page.locator(SEL["tab_button"].format(tab="settings")).click()
    await page.locator(SEL["settings_subtab"].format(subtab="skills")).click()
    await page.locator(SEL["settings_subpanel"].format(subtab="skills")).wait_for(
        state="visible", timeout=5000
    )


async def mock_skills_api(page):
    """Mock skills API endpoints used by the browser lifecycle tests.

    These tests validate the Settings > Skills UI contract, not live ClawHub
    availability. Keeping the API local avoids skip-on-network behavior while
    still exercising the real browser code paths.
    """
    installed = []
    install_requests = []

    async def fulfill_json(route, payload):
        await route.fulfill(
            json=payload,
            headers={"Cache-Control": "no-store"},
        )

    async def handle(route):
        nonlocal installed
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/skills" and request.method == "GET":
            await fulfill_json(route, {"skills": installed, "count": len(installed)})
            return

        if path == "/api/skills/search" and request.method == "POST":
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

        if path == "/api/skills/install" and request.method == "POST":
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

        if path.startswith("/api/skills/") and request.method == "DELETE":
            name = unquote(path.removeprefix("/api/skills/"))
            installed = [skill for skill in installed if skill["name"] != name]
            await fulfill_json(
                route,
                {"success": True, "message": f"Skill '{name}' removed"},
            )
            return

        await route.continue_()

    await page.route("**/api/skills**", handle)
    return {"install_requests": install_requests}


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


async def test_skills_install_and_remove(page):
    """Install a mocked catalog skill from search results, then remove it."""
    mock_api = await mock_skills_api(page)
    await go_to_skills(page)

    search_input = page.locator(SEL["skill_search_input"])
    await search_input.fill("markdown")
    await search_input.press("Enter")

    results = page.locator(SEL["skill_search_result"])
    await results.first.wait_for(state="visible", timeout=5000)

    install_btn = results.first.locator("button", has_text="Install")
    assert await install_btn.count() == 1, "Expected mocked catalog skill to be installable"
    async with page.expect_response(lambda r: "/api/skills/install" in r.url) as install_response:
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

    remove_btn = installed.first.locator("button", has_text="Remove")
    assert await remove_btn.count() == 1, "Installed mocked skill should be removable"
    await remove_btn.click()

    confirm_btn = page.locator(SEL["confirm_modal_btn"])
    await confirm_btn.wait_for(state="visible", timeout=5000)
    await confirm_btn.click()

    await page.wait_for_function(
        """(selector) => document.querySelectorAll(selector).length === 0""",
        arg=SEL["skill_installed"],
        timeout=5000,
    )
