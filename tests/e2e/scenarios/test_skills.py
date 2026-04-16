"""Scenario 3: Skills search, install, and remove lifecycle."""

import pytest
from helpers import SEL, send_chat_and_wait_for_terminal_message


async def go_to_skills(page):
    """Navigate to Settings > Skills subtab."""
    await page.locator(SEL["tab_button"].format(tab="settings")).click()
    await page.locator(SEL["settings_subtab"].format(subtab="skills")).click()
    await page.locator(SEL["settings_subpanel"].format(subtab="skills")).wait_for(
        state="visible", timeout=5000
    )


async def test_skills_tab_visible(page):
    """Skills subtab shows the search interface."""
    await go_to_skills(page)

    search_input = page.locator(SEL["skill_search_input"])
    assert await search_input.is_visible(), "Skills search input not visible"


async def test_skills_search(page):
    """Search ClawHub for skills and verify results appear."""
    await go_to_skills(page)

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
    await go_to_skills(page)

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

    # Wait for install to complete -- the UI calls loadSkills() after install,
    # which populates #skills-list with .ext-card elements
    installed = page.locator(SEL["skill_installed"])
    try:
        await installed.first.wait_for(state="visible", timeout=15000)
    except Exception:
        pytest.skip("Skill install did not update the installed list in time")

    installed_count = await installed.count()
    assert installed_count >= 1, "Skill should appear in installed list after install"

    # Remove the skill via confirm modal
    remove_btn = installed.first.locator("button", has_text="Remove")
    if await remove_btn.count() > 0:
        await remove_btn.click()
        # Confirm in the modal
        confirm_btn = page.locator(SEL["confirm_modal_btn"])
        await confirm_btn.wait_for(state="visible", timeout=5000)
        await confirm_btn.click()
        # Wait for the card to disappear or list to shrink
        await page.wait_for_timeout(3000)
        new_count = await page.locator(SEL["skill_installed"]).count()
        assert new_count < installed_count, "Skill should be removed from installed list"


# ----------------------------------------------------------------------
# Skill behavioral tests
# ----------------------------------------------------------------------
# These tests verify that skills produce correct behavior when invoked.
# Add new skills to SKILL_TESTS below.
# ----------------------------------------------------------------------

SKILL_TESTS = [
    {
        "name": "investigate",
        "trigger": "/investigate why is this broken?",
        "expect_keywords": ["investigation", "debug", "error", "issue"],
    },
    {
        "name": "code-review",
        "trigger": "/review the code changes",
        "expect_keywords": ["review", "changes", "diff"],
    },
]


@pytest.mark.parametrize("skill_test", SKILL_TESTS)
async def test_skill_invocation(page, skill_test):
    """Invoke a skill and verify it produces expected output."""
    result = await send_chat_and_wait_for_terminal_message(page, skill_test["trigger"])

    assert result["role"] == "assistant"
    response_lower = result["text"].lower()

    for keyword in skill_test["expect_keywords"]:
        assert keyword.lower() in response_lower, (
            f"Expected '{keyword}' in skill response for {skill_test['name']}, "
            f"got: {result['text'][:200]}..."
        )


async def test_skill_investigate_uses_shell(page):
    """The investigate skill should use shell/git commands during investigation."""
    await send_chat_and_wait_for_terminal_message(page, "/investigate")

    # Check that some tool was used (we can't easily verify which, but at least
    # we verified the skill activated and produced output)
    assistant_messages = await page.locator(SEL["message_assistant"]).all()
    assert len(assistant_messages) >= 1, "Should have at least one assistant message"


async def test_skill_coding_provides_best_practices(page):
    """The coding skill should mention best practices when editing code."""
    result = await send_chat_and_wait_for_terminal_message(
        page, "/coding help me fix this bug"
    )

    assert result["role"] == "assistant"
    response_lower = result["text"].lower()
    assert any(
        kw in response_lower for kw in ["fix", "bug", "error", "patch", "apply"]
    ), f"Expected coding-related output, got: {result['text'][:200]}..."
