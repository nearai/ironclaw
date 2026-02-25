"""Scenario 3: Skills search, install, and remove lifecycle."""

import pytest
from helpers import SEL


async def test_skills_tab_visible(page):
    """Skills tab shows the search interface."""
    await page.locator(SEL["tab_button"].format(tab="skills")).click()
    panel = page.locator(SEL["tab_panel"].format(tab="skills"))
    await panel.wait_for(state="visible", timeout=5000)

    search_input = page.locator(SEL["skill_search_input"])
    assert await search_input.is_visible(), "Skills search input not visible"


async def test_skills_search(page):
    """Search ClawHub for skills and verify results appear."""
    await page.locator(SEL["tab_button"].format(tab="skills")).click()

    search_input = page.locator(SEL["skill_search_input"])
    await search_input.fill("markdown")
    await search_input.press("Enter")

    # Wait for results (ClawHub may be slow)
    try:
        results = page.locator(SEL["skill_search_result"])
        await results.first.wait_for(state="visible", timeout=20000)
    except Exception:
        pytest.skip("ClawHub registry unreachable or returned no results")

    count = await results.count()
    assert count >= 1, "Expected at least 1 search result"


async def test_skills_install_and_remove(page):
    """Install a skill from search results, then remove it."""
    await page.locator(SEL["tab_button"].format(tab="skills")).click()

    # Search
    search_input = page.locator(SEL["skill_search_input"])
    await search_input.fill("markdown")
    await search_input.press("Enter")

    try:
        results = page.locator(SEL["skill_search_result"])
        await results.first.wait_for(state="visible", timeout=20000)
    except Exception:
        pytest.skip("ClawHub registry unreachable or returned no results")

    # Auto-accept confirm dialogs
    await page.evaluate("window.confirm = () => true")

    # Install first result
    install_btn = results.first.locator("button", has_text="Install")
    if await install_btn.count() == 0:
        pytest.skip("No installable skills found in results")
    await install_btn.click()

    # Wait for install to complete (installed list updates)
    # The UI should show the skill in the installed section
    await page.wait_for_timeout(5000)

    # Check if any installed skills exist now
    installed = page.locator(SEL["skill_installed"])
    installed_count = await installed.count()
    if installed_count == 0:
        # Try scrolling or waiting longer
        await page.wait_for_timeout(5000)
        installed_count = await installed.count()

    assert installed_count >= 1, "Skill should appear in installed list after install"

    # Remove the skill
    remove_btn = installed.first.locator("button", has_text="Remove")
    if await remove_btn.count() > 0:
        await remove_btn.click()
        await page.wait_for_timeout(3000)

        # Verify removed
        new_count = await page.locator(SEL["skill_installed"]).count()
        assert new_count < installed_count, "Skill should be removed from installed list"
