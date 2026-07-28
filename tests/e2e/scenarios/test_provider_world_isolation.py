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
from emulate_provider import github_headers, github_json, slack_post
from helpers import EMULATE_GITHUB_SECONDARY_BEARER

ISSUES_PATH = "/repos/nearai/ironclaw/issues"
GITHUB_TEAM_SLUG = "provider-contract"
GITHUB_TEAM_MEMBERS_PATH = f"/orgs/nearai/teams/{GITHUB_TEAM_SLUG}/members"


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


@pytest.fixture
async def slack_world():
    world = ResettableEmulateProviderWorld()
    await world.start({"slack"})
    try:
        yield world
    finally:
        await world.close()

async def _team_member_logins(base_url: str) -> list[str]:
    async with httpx.AsyncClient(timeout=15) as client:
        members = await github_json(client, base_url, "GET", GITHUB_TEAM_MEMBERS_PATH)
    assert isinstance(members, list), members
    return [member["login"] for member in members]

async def _remaining_rate_limit(client: httpx.AsyncClient, base_url: str) -> int:
    """Read x-ratelimit-remaining off a real request, not the /rate_limit body.

    Emulate's `/rate_limit` endpoint reports a value computed independently
    of the header counter that guards every other route, so it does not
    reflect requests already spent. The header is what a Reborn caller
    actually sees hit its budget.
    """
    response = await client.get(f"{base_url}/user", headers=github_headers())
    assert response.status_code == 200, response.text
    return int(response.headers["x-ratelimit-remaining"])

async def test_reset_restores_rate_limit_headroom_after_a_journey_burns_requests(
    github_world,
):
    """A journey that eats into the 5000/hr budget must not hand the deficit on.

    `x-ratelimit-remaining` decrements on every authenticated call. If reset
    only wiped fixture data and left the counter running, a case scheduled
    after a request-heavy journey would inherit its depleted budget and could
    fail on a rate-limit error that has nothing to do with what it did.
    """
    base_url = github_world.servers["github"]["url"]
    async with httpx.AsyncClient(timeout=15) as client:
        readings = [await _remaining_rate_limit(client, base_url) for _ in range(5)]
        assert readings == sorted(readings, reverse=True), readings
        assert readings[-1] < readings[0], readings

    await github_world.reset({"github"})

    async with httpx.AsyncClient(timeout=15) as client:
        after_reset = await _remaining_rate_limit(client, base_url)

    # A fresh process starts back near the 5000 ceiling; a leaked counter
    # would instead continue downward from `readings[-1]`.
    assert after_reset > readings[-1], (after_reset, readings)

async def test_reset_invalidates_a_pending_oauth_authorization_code(github_world):
    """A code minted by one journey's OAuth dance must not redeem in the next.

    Emulate's `/login/oauth/authorize` -> `/login/oauth/callback` exchange
    hands back a single-use `code`; `/login/oauth/access_token` looks it up
    in an in-memory pending-codes table. If that table survived a reset, a
    code leaked out of one journey's logs or fixtures could mint a live
    token for a case that never ran the OAuth flow itself.
    """
    base_url = github_world.servers["github"]["url"]
    form = {
        "login": "reborn-dev",
        "redirect_uri": "http://localhost/cb",
        "scope": "",
        "state": "isolation-check",
        "client_id": "provider-world-isolation-test",
    }
    async with httpx.AsyncClient(timeout=15) as client:
        control_callback = await client.post(
            f"{base_url}/login/oauth/callback", data=form
        )
        assert control_callback.status_code == 302, control_callback.text
        control_code = httpx.URL(control_callback.headers["location"]).params[
            "code"
        ]
        control_exchange = await client.post(
            f"{base_url}/login/oauth/access_token",
            data={
                "client_id": form["client_id"],
                "client_secret": "unused-in-emulate",
                "code": control_code,
                "redirect_uri": form["redirect_uri"],
            },
            headers={"Accept": "application/json"},
        )
        assert control_exchange.status_code == 200, control_exchange.text
        assert control_exchange.json().get("access_token"), control_exchange.json()

        pending_callback = await client.post(
            f"{base_url}/login/oauth/callback", data=form
        )
        assert pending_callback.status_code == 302, pending_callback.text
        pending_code = httpx.URL(pending_callback.headers["location"]).params[
            "code"
        ]

    await github_world.reset({"github"})

    async with httpx.AsyncClient(timeout=15) as client:
        exchange = await client.post(
            f"{base_url}/login/oauth/access_token",
            data={
                "client_id": form["client_id"],
                "client_secret": "unused-in-emulate",
                "code": pending_code,
                "redirect_uri": form["redirect_uri"],
            },
            headers={"Accept": "application/json"},
        )
    assert exchange.status_code == 200, exchange.text
    body = exchange.json()
    assert "access_token" not in body, body
    assert body.get("error") == "bad_verification_code", body

async def test_reset_of_team_membership_drops_grants_a_journey_added(github_world):
    """Org/team membership a journey grants must not carry into the next case.

    Every world start bootstraps `reborn-dev` onto the `Provider Contract`
    team so the primary fixture actor has push access. A journey that grants
    a second collaborator (e.g. to test review-assignment flows) must not
    leave that grant in place for a case that never asked for it — that grant
    is itself a capability the next case's assertions could be silently
    depending on, or silently defeated by.
    """
    base_url = github_world.servers["github"]["url"]
    async with httpx.AsyncClient(timeout=15) as client:
        await github_json(
            client,
            base_url,
            "PUT",
            f"/orgs/nearai/teams/{GITHUB_TEAM_SLUG}/memberships/reborn-reviewer",
            payload={"role": "member"},
            token=EMULATE_GITHUB_SECONDARY_BEARER,
        )
    assert sorted(await _team_member_logins(base_url)) == [
        "reborn-dev",
        "reborn-reviewer",
    ]

    await github_world.reset({"github"})

    # Reset re-bootstraps only the primary actor; the second grant is gone.
    assert await _team_member_logins(base_url) == ["reborn-dev"]

async def test_reset_clears_slack_messages_a_journey_posted(slack_world):
    """A message a journey posts to a seeded channel must not outlive it.

    Nothing in the Slack seed pre-populates `reborn-alerts` with messages, so
    a message showing up in the next case's `conversations.history` can only
    have leaked from this journey — the classic false-positive-attributed-
    to-the-wrong-case failure this whole box exists to rule out.
    """
    base_url = slack_world.servers["slack"]["url"]
    async with httpx.AsyncClient(timeout=15) as client:
        channels = await slack_post(client, base_url, "conversations.list")
        channel_id = next(
            c["id"] for c in channels["channels"] if c["name"] == "reborn-alerts"
        )
        await slack_post(
            client,
            base_url,
            "chat.postMessage",
            {"channel": channel_id, "text": "left-behind-by-a-previous-journey"},
        )
        history = await slack_post(
            client, base_url, "conversations.history", {"channel": channel_id}
        )
        assert [m["text"] for m in history["messages"]] == [
            "left-behind-by-a-previous-journey"
        ]

    await slack_world.reset({"slack"})

    async with httpx.AsyncClient(timeout=15) as client:
        # The restarted process assigns fresh channel ids from the same seed,
        # so look the channel back up by name rather than reusing channel_id.
        channels_after = await slack_post(client, base_url, "conversations.list")
        channel_id_after = next(
            c["id"] for c in channels_after["channels"] if c["name"] == "reborn-alerts"
        )
        history_after = await slack_post(
            client, base_url, "conversations.history", {"channel": channel_id_after}
        )
    assert history_after["messages"] == []

async def test_reset_clears_slack_channel_membership_a_journey_joined(slack_world):
    """Joining a channel that isn't seeded-joined must not stick across a reset.

    `general` seeds with only its creator as a member. A journey that joins
    it (e.g. to test channel-discovery flows) changes both `is_member` and
    `num_members` for that actor; a reset that only replayed fixture data
    without restarting the process would leave that join in place.
    """
    base_url = slack_world.servers["slack"]["url"]
    async with httpx.AsyncClient(timeout=15) as client:
        channels = await slack_post(client, base_url, "conversations.list")
        general = next(c for c in channels["channels"] if c["name"] == "general")
        assert general["is_member"] is False
        seeded_member_count = general["num_members"]

        joined = await slack_post(
            client, base_url, "conversations.join", {"channel": general["id"]}
        )
        assert joined["channel"]["is_member"] is True
        assert joined["channel"]["num_members"] == seeded_member_count + 1

    await slack_world.reset({"slack"})

    async with httpx.AsyncClient(timeout=15) as client:
        channels_after = await slack_post(client, base_url, "conversations.list")
        general_after = next(
            c for c in channels_after["channels"] if c["name"] == "general"
        )
    assert general_after["is_member"] is False
    assert general_after["num_members"] == seeded_member_count

@pytest.fixture
async def google_world():
    world = ResettableEmulateProviderWorld()
    await world.start({"google"})
    try:
        yield world
    finally:
        await world.close()

async def test_reset_invalidates_a_google_refresh_token(google_world):
    """A refresh token one journey obtained must not still work for the next.

    This is the Google counterpart to the GitHub authorization-code case, and
    it is the longer-lived half: an authorization code is single-use and
    short-lived, while a refresh token is exactly the credential a journey
    would keep using. If it survived a reset, a later journey could silently
    mint access tokens against a grant it never performed — and would keep
    passing while doing so.
    """
    base_url = google_world.servers["google"]["url"]
    redirect_uri = "http://127.0.0.1:1/callback"
    async with httpx.AsyncClient(timeout=15) as client:
        callback = await client.post(
            f"{base_url}/o/oauth2/v2/auth/callback",
            data={
                "email": "e2e.google@example.com",
                "redirect_uri": redirect_uri,
                "scope": "openid email profile",
                "client_id": "",
            },
        )
        assert callback.status_code == 302, callback.text
        code = httpx.URL(callback.headers["location"]).params["code"]

        exchanged = await client.post(
            f"{base_url}/oauth2/token",
            data={
                "grant_type": "authorization_code",
                "code": code,
                "redirect_uri": redirect_uri,
            },
        )
        assert exchanged.status_code == 200, exchanged.text
        refresh_token = exchanged.json().get("refresh_token")
        assert refresh_token, exchanged.json()

        # Redeemable before the reset, so the assertion after it is meaningful
        # rather than passing because the token never worked at all.
        before = await client.post(
            f"{base_url}/oauth2/token",
            data={"grant_type": "refresh_token", "refresh_token": refresh_token},
        )
        assert before.status_code == 200, before.text
        assert before.json()["access_token"]

    await google_world.reset({"google"})

    async with httpx.AsyncClient(timeout=15) as client:
        after = await client.post(
            f"{base_url}/oauth2/token",
            data={"grant_type": "refresh_token", "refresh_token": refresh_token},
        )
    assert after.status_code == 400, after.text
    assert after.json()["error"] == "invalid_grant", after.json()
