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
from emulate_provider import github_json

ISSUES_PATH = "/repos/nearai/ironclaw/issues"


@pytest.fixture
async def github_world():
    world = ResettableEmulateProviderWorld()
    await world.start({"github"})
    try:
        yield world
    finally:
        await world.close()


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


async def test_reset_of_one_service_is_scoped_to_that_service(github_world):
    """Resetting one provider must not disturb another's process.

    The journey fixtures reset only the worlds a case actually mutated, so a
    reset that reached further would silently discard state a concurrent or
    later assertion still depends on.
    """
    base_url = github_world.servers["github"]["url"]
    async with httpx.AsyncClient(timeout=15) as client:
        await github_json(
            client,
            base_url,
            "POST",
            ISSUES_PATH,
            payload={"title": "survives-an-unrelated-reset"},
            expected_status=201,
        )

    # Nothing else is running in this world, so the observable contract is that
    # resetting an empty selection leaves github's process untouched: same URL,
    # still serving, and its data NOT discarded out from under a caller.
    await github_world.reset(set())

    assert github_world.servers["github"]["url"] == base_url
    assert "survives-an-unrelated-reset" in await _issue_titles(base_url)
