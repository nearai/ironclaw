"""Legacy Skills settings lifecycle coverage ported to Reborn WebChat v2."""

import asyncio
import json
from urllib.parse import unquote, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


MOCK_INSTALLED_SKILL = {
    "name": "markdown-helper",
    "description": "Deterministic E2E skill for markdown workflows.",
    "version": "1.0.0",
    "trust": "Installed",
    "source_kind": "installed",
    "keywords": ["markdown", "e2e"],
    "usage_hint": "Type `/markdown-helper` in chat to force-activate this skill.",
    "has_requirements": False,
    "has_scripts": False,
    "can_edit": True,
    "can_delete": True,
    "auto_activate": True,
}

MOCK_SYSTEM_SKILL = {
    "name": "system-helper",
    "description": "Read-only system helper.",
    "version": "1.0.0",
    "trust": "Trusted",
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


async def _open_mocked_skills_page(reborn_v2_server, reborn_v2_browser, *, initial_skills=None):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    installed = [dict(skill) for skill in (initial_skills or [])]
    install_requests: list[dict] = []
    update_requests: list[dict] = []
    delete_requests: list[str] = []

    async def fulfill_json(route, payload, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(payload),
            headers={"Cache-Control": "no-store"},
        )

    async def handle_skills(route):
        nonlocal installed
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/skills" and request.method == "GET":
            await fulfill_json(
                route,
                {
                    "skills": installed,
                    "count": len(installed),
                    "auto_activate_learned": True,
                },
            )
            return

        if path == "/api/webchat/v2/skills/install" and request.method == "POST":
            payload = json.loads(request.post_data or "{}")
            install_requests.append({"headers": request.headers, "body": payload})
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
            await fulfill_json(route, {"success": True, "message": f"Skill '{name}' updated"})
            return

        if path.startswith("/api/webchat/v2/skills/") and request.method == "DELETE":
            name = unquote(path.removeprefix("/api/webchat/v2/skills/"))
            delete_requests.append(name)
            installed = [skill for skill in installed if skill["name"] != name]
            await fulfill_json(route, {"success": True, "message": f"Skill '{name}' removed"})
            return

        await route.continue_()

    await page.route("**/api/webchat/v2/skills**", handle_skills)
    await page.goto(f"{reborn_v2_server}/v2/settings/skills?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.get_by_text("Add skill")).to_be_visible(timeout=15000)

    return {
        "context": context,
        "page": page,
        "install_requests": install_requests,
        "update_requests": update_requests,
        "delete_requests": delete_requests,
    }


async def _add_mock_skill(page):
    await page.get_by_placeholder(SEL_V2["skill_name_placeholder"]).fill(
        "markdown-helper"
    )
    await page.get_by_placeholder(SEL_V2["skill_content_placeholder"]).fill(
        MOCK_SKILL_CONTENT
    )
    await page.get_by_role("button", name="Add").click()
    card = page.locator(SEL_V2["skills_card"]).filter(has_text="markdown-helper")
    await expect(card).to_be_visible(timeout=5000)
    return card


async def test_reborn_legacy_skills_tab_visible(reborn_v2_server, reborn_v2_browser):
    harness = await _open_mocked_skills_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        await expect(page.get_by_text("Add skill")).to_be_visible()
        await expect(
            page.get_by_placeholder(SEL_V2["skill_name_placeholder"])
        ).to_be_visible()
        await expect(
            page.get_by_placeholder(SEL_V2["skill_content_placeholder"])
        ).to_be_visible()
        await expect(page.get_by_role("button", name="Default: On")).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_skills_add_edit_delete(reborn_v2_server, reborn_v2_browser):
    harness = await _open_mocked_skills_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        card = await _add_mock_skill(page)

        assert len(harness["install_requests"]) == 1
        assert harness["install_requests"][0]["headers"].get("x-confirm-action") == "true"
        assert harness["install_requests"][0]["body"] == {
            "name": "markdown-helper",
            "content": MOCK_SKILL_CONTENT.strip(),
        }

        await card.get_by_role("button", name="Edit").click()
        editor = card.locator("textarea")
        await expect(editor).to_be_visible(timeout=5000)
        await editor.fill(
            "---\nname: markdown-helper\ndescription: Updated E2E skill\n---\n\n# Updated\n"
        )
        await card.get_by_role("button", name="Save").click()
        await expect(editor).to_be_hidden(timeout=5000)

        assert len(harness["update_requests"]) == 1
        update = harness["update_requests"][0]
        assert update["name"] == "markdown-helper"
        assert update["headers"].get("x-confirm-action") == "true"
        assert "Updated E2E skill" in update["body"]["content"]

        loop = asyncio.get_running_loop()
        dialog_future = loop.create_future()

        async def handle_dialog(dialog):
            if not dialog_future.done():
                dialog_future.set_result(
                    {"type": dialog.type, "message": dialog.message}
                )
            await dialog.accept()

        page.once("dialog", handle_dialog)
        await card.get_by_role("button", name="Delete").click()
        dialog = await asyncio.wait_for(dialog_future, timeout=5)
        assert dialog["type"] == "confirm"
        assert "markdown-helper" in dialog["message"]

        await expect(
            page.locator(SEL_V2["skills_card"]).filter(has_text="markdown-helper")
        ).to_have_count(0, timeout=5000)
        assert harness["delete_requests"] == ["markdown-helper"]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_skills_read_only_sources_hide_edit_and_delete(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_skills_page(
        reborn_v2_server,
        reborn_v2_browser,
        initial_skills=[MOCK_SYSTEM_SKILL, MOCK_WORKSPACE_SKILL],
    )
    try:
        page = harness["page"]
        system_card = page.locator(SEL_V2["skills_card"]).filter(
            has_text="system-helper"
        )
        workspace_card = page.locator(SEL_V2["skills_card"]).filter(
            has_text="workspace-helper"
        )
        await expect(system_card).to_be_visible(timeout=5000)
        await expect(workspace_card).to_be_visible(timeout=5000)

        for card in (system_card, workspace_card):
            await expect(card.get_by_role("button", name="Edit")).to_have_count(0)
            await expect(card.get_by_role("button", name="Delete")).to_have_count(0)
            await expect(
                card.get_by_role("button", name="Auto-activate: On")
            ).to_have_count(0)
    finally:
        await harness["context"].close()
