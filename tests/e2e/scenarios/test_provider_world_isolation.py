"""Provider-world isolation between journeys (#6524 workstream 3).

Every mutating journey depends on `ResettableEmulateProviderWorld.reset()` to
hand the next journey a seed-state provider. That method is used by fixtures
throughout the suite and asserted by nothing: if it stopped restoring state,
the symptom would surface as an unrelated journey failing against data it
never created, which is the hardest kind of failure to attribute.

These tests exercise the world directly — no Reborn binary, no model replay —
so they stay fast and fail for exactly one reason.
"""

from __future__ import annotations

import httpx
import pytest
from conftest import ResettableEmulateProviderWorld
from emulate_provider import github_json, slack_post

ISSUES_PATH = "/repos/nearai/ironclaw/issues"


@pytest.fixture
async def github_world():
    world = ResettableEmulateProviderWorld()
    await world.start({"github"})
    try:
        yield world
    finally:
        await world.close()


@pytest.fixture
async def github_slack_world(github_world):
    await github_world.start({"slack"})
    return github_world


async def _issue_titles(base_url: str) -> list[str]:
    async with httpx.AsyncClient(timeout=15) as client:
        issues = await github_json(client, base_url, "GET", ISSUES_PATH)
    assert isinstance(issues, list), issues
    return [issue["title"] for issue in issues]


async def test_reset_restores_seed_state_after_a_journey_creates_data(github_world):
    """A resource one journey creates must not exist for the next one."""
    base_url = github_world.servers["github"]["url"]
    seeded = await _issue_titles(base_url)

    async with httpx.AsyncClient(timeout=15) as client:
        await github_json(
            client,
            base_url,
            "POST",
            ISSUES_PATH,
            payload={"title": "left-behind-by-a-previous-journey"},
            expected_status=201,
        )
    assert "left-behind-by-a-previous-journey" in await _issue_titles(base_url)

    await github_world.reset({"github"})

    assert await _issue_titles(base_url) == seeded


async def test_reset_keeps_the_url_stable_so_a_running_reborn_still_reaches_it(
    github_world,
):
    """Reborn is started once with these URLs and outlives every reset.

    Restarting the provider on a fresh port would leave the running process
    pointing at a dead socket, and every journey after the first reset would
    fail for a reason that has nothing to do with the journey.
    """
    before = github_world.servers["github"]["url"]
    seeded = await _issue_titles(before)

    await github_world.reset({"github"})

    assert github_world.servers["github"]["url"] == before
    # Still answering on the same socket after the restart.
    assert await _issue_titles(before) == seeded


async def test_reset_of_one_service_is_scoped_to_that_service(github_slack_world):
    """Resetting one provider must not disturb another's process.

    The journey fixtures reset only the worlds a case actually mutated, so a
    reset that reached further would silently discard state a concurrent or
    later assertion still depends on.
    """
    github_url = github_slack_world.servers["github"]["url"]
    slack_url = github_slack_world.servers["slack"]["url"]
    github_marker = "discarded-by-the-selected-reset"
    slack_marker = "survives-an-unrelated-reset"

    async with httpx.AsyncClient(timeout=15) as client:
        await github_json(
            client,
            github_url,
            "POST",
            ISSUES_PATH,
            payload={"title": github_marker},
            expected_status=201,
        )

        channels = await slack_post(
            client,
            slack_url,
            "conversations.list",
            {"types": "public_channel"},
        )
        channel_id = next(
            channel["id"]
            for channel in channels["channels"]
            if channel["name"] == "reborn-alerts"
        )
        await slack_post(
            client,
            slack_url,
            "chat.postMessage",
            {"channel": channel_id, "text": slack_marker},
        )

        await github_slack_world.reset({"github"})

        history = await slack_post(
            client,
            slack_url,
            "conversations.history",
            {"channel": channel_id},
        )

    assert github_marker not in await _issue_titles(github_url)
    assert github_slack_world.servers["slack"]["url"] == slack_url
    assert any(message["text"] == slack_marker for message in history["messages"])
