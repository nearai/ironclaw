"""Self-tests for the reusable provider fault proxy."""

from __future__ import annotations

import asyncio
import gzip

import httpx
import pytest
from aiohttp import web

from helpers import AUTH_TOKEN
from provider_fault_proxy import (
    PROVIDER_FAULT_PROFILES,
    ProviderFaultProfile,
    ProviderFaultProxy,
    ProviderFaultProxyWorld,
)

_RESPONSE_PROFILE_NAMES = [
    name
    for name, profile in PROVIDER_FAULT_PROFILES.items()
    if profile.action == "respond"
]


async def _start_upstream() -> tuple[str, list[dict], web.AppRunner]:
    requests = []

    async def handle(request: web.Request) -> web.Response:
        body = await request.text()
        requests.append(
            {
                "method": request.method,
                "path": request.path,
                "query": request.query_string,
                "body": body,
            }
        )
        response_body = {
            "id": "provider-object",
            "method": request.method,
        }
        if request.path == "/compressed":
            return web.Response(
                body=gzip.compress(
                    b'{"id":"provider-object","method":"GET"}'
                ),
                headers={
                    "Content-Encoding": "gzip",
                    "Content-Type": "application/json",
                    "X-Upstream": "emulate",
                },
            )
        if request.path == "/oauth/token":
            return web.json_response(
                {"access_token": "provider-issued-secret-token"}
            )
        if request.path == "/oauth/user-token":
            return web.json_response(
                {
                    "access_token": "provider-issued-bot-token",
                    "authed_user": {
                        "access_token": "provider-issued-user-token"
                    },
                }
            )
        return web.json_response(
            response_body,
            status=201 if request.method == "POST" else 200,
            headers={"X-Upstream": "emulate"},
        )

    app = web.Application()
    app.router.add_route("*", "/{path:.*}", handle)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    server = site._server
    assert server is not None and server.sockets
    url = f"http://127.0.0.1:{server.sockets[0].getsockname()[1]}"
    return url, requests, runner


@pytest.fixture
async def fault_proxy():
    upstream_url, upstream_requests, upstream_runner = await _start_upstream()
    proxy = ProviderFaultProxy(upstream_url, service="test")
    await proxy.start()
    try:
        yield proxy, upstream_requests
    finally:
        await proxy.close()
        await upstream_runner.cleanup()


async def test_provider_fault_proxy_is_transparent_and_redacts_credentials(
    fault_proxy,
):
    proxy, upstream_requests = fault_proxy
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{proxy.url}/objects?account=primary",
            headers={"Authorization": "Bearer never-record-this-token"},
            json={"name": "created"},
        )

    assert response.status_code == 201
    assert response.json()["id"] == "provider-object"
    assert response.headers["X-Upstream"] == "emulate"
    assert upstream_requests == [
        {
            "method": "POST",
            "path": "/objects",
            "query": "account=primary",
            "body": '{"name":"created"}',
        }
    ]
    request = proxy.state["requests"][0]
    assert request["service"] == "test"
    assert request["forwarded"] is True
    assert request["responded"] is True
    assert request["credential_fingerprint"]
    assert "never-record-this-token" not in str(proxy.state)


async def test_provider_fault_proxy_fingerprints_issued_token_without_recording_it(
    fault_proxy,
):
    proxy, _ = fault_proxy
    async with httpx.AsyncClient() as client:
        response = await client.post(f"{proxy.url}/oauth/token")

    response.raise_for_status()
    request = proxy.state["requests"][0]
    assert request["issued_credential_fingerprint"] == "c3da4738d8e1"
    assert "provider-issued-secret-token" not in str(proxy.state)


async def test_provider_fault_proxy_prefers_issued_user_account_token(
    fault_proxy,
):
    proxy, _ = fault_proxy
    async with httpx.AsyncClient() as client:
        response = await client.post(f"{proxy.url}/oauth/user-token")

    response.raise_for_status()
    request = proxy.state["requests"][0]
    assert request["issued_credential_fingerprint"] == "5309d6d55911"
    assert "provider-issued-user-token" not in str(proxy.state)
    assert "provider-issued-bot-token" not in str(proxy.state)


async def test_provider_fault_proxy_preserves_compressed_responses(fault_proxy):
    proxy, upstream_requests = fault_proxy

    async with httpx.AsyncClient() as client:
        response = await client.get(f"{proxy.url}/compressed")

    assert response.json() == {"id": "provider-object", "method": "GET"}
    assert response.headers["Content-Encoding"] == "gzip"
    assert len(upstream_requests) == 1


@pytest.mark.parametrize(
    "profile_name",
    _RESPONSE_PROFILE_NAMES,
)
async def test_response_fault_profiles_do_not_reach_provider(
    fault_proxy,
    profile_name,
):
    proxy, upstream_requests = fault_proxy
    profile = PROVIDER_FAULT_PROFILES[profile_name]
    proxy.arm(profile, method="GET", path="/objects/1")

    async with httpx.AsyncClient() as client:
        response = await client.get(f"{proxy.url}/objects/1")

    assert response.status_code == profile.status
    assert response.text == profile.body
    assert upstream_requests == []
    assert proxy.state["requests"][0]["forwarded"] is False


@pytest.mark.parametrize(
    ("profile_name", "status", "challenge_contains"),
    [
        ("expired_credential", 401, 'error="invalid_token"'),
        ("wrong_scope", 403, 'error="insufficient_scope"'),
    ],
)
def test_credential_lifecycle_profiles_state_their_condition(
    profile_name,
    status,
    challenge_contains,
):
    """Each credential-lifecycle fault names its condition in the challenge.

    The point of these profiles is that they are *not* interchangeable with a
    bare `http_401`/`http_403`. A provider distinguishes "your token expired"
    from "your token lacks the scope" in the RFC 6750 `WWW-Authenticate`
    challenge, and that is the only signal a client can key
    credential-lifecycle handling on. A profile that dropped the challenge
    would still have the same status and would still look like a passing fault
    case, so assert the discriminator itself.
    """
    profile = PROVIDER_FAULT_PROFILES[profile_name]
    challenge = profile.headers.get("WWW-Authenticate", "")

    assert profile.status == status
    assert challenge_contains in challenge, challenge


def test_credential_lifecycle_profiles_are_not_aliases_of_generic_faults():
    """The generic status profiles carry no challenge, so the pair differ."""
    for generic_name in ("http_401", "http_403"):
        assert "WWW-Authenticate" not in PROVIDER_FAULT_PROFILES[generic_name].headers

    challenges = {
        name: PROVIDER_FAULT_PROFILES[name].headers["WWW-Authenticate"]
        for name in ("expired_credential", "wrong_scope")
    }
    assert len(set(challenges.values())) == 2, challenges


async def test_credential_lifecycle_faults_never_reach_the_provider(fault_proxy):
    """A rejected credential must not let the request through.

    The caller is authenticated but not for this operation, so forwarding the
    rejected request would perform the very side effect the scope was meant to
    prevent.
    """
    proxy, upstream_requests = fault_proxy
    proxy.arm(
        PROVIDER_FAULT_PROFILES["wrong_scope"],
        method="POST",
        path="/objects",
    )

    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{proxy.url}/objects",
            headers={"Authorization": "Bearer scoped-too-narrowly"},
            json={"name": "never-created"},
        )

    assert response.status_code == 403
    assert 'error="insufficient_scope"' in response.headers["WWW-Authenticate"]
    assert upstream_requests == []
    requests = proxy.state["requests"]
    assert len(requests) == 1
    request = requests[0]
    assert request["fault"] == "wrong_scope"
    assert request["forwarded"] is False
    assert "scoped-too-narrowly" not in str(proxy.state)


async def test_timeout_profile_never_forwards_after_caller_times_out(fault_proxy):
    proxy, upstream_requests = fault_proxy
    proxy.arm(
        ProviderFaultProfile(
            name="short_timeout",
            action="delay_before_disconnect",
            delay_seconds=0.05,
        ),
        method="POST",
        path="/objects",
    )

    async with httpx.AsyncClient(timeout=0.01) as client:
        with pytest.raises(httpx.ReadTimeout):
            await client.post(f"{proxy.url}/objects", json={"name": "never-created"})

    await asyncio.sleep(0.06)
    assert upstream_requests == []
    request = proxy.state["requests"][0]
    assert request["fault"] == "short_timeout"
    assert request["forwarded"] is False
    assert request["responded"] is False


async def test_truncated_response_aborts_before_declared_body_length(fault_proxy):
    proxy, upstream_requests = fault_proxy
    profile = PROVIDER_FAULT_PROFILES["truncated_response"]
    proxy.arm(profile, method="GET", path="/objects/1")

    async with httpx.AsyncClient() as client:
        with pytest.raises(httpx.RemoteProtocolError):
            await client.get(f"{proxy.url}/objects/1")

    assert upstream_requests == []
    request = proxy.state["requests"][0]
    assert request["fault"] == "truncated_response"
    assert request["forwarded"] is False
    assert request["responded"] is False


async def test_connection_reset_before_forward_never_reaches_provider(fault_proxy):
    proxy, upstream_requests = fault_proxy
    proxy.arm(
        PROVIDER_FAULT_PROFILES["connection_reset"],
        method="POST",
        path="/objects",
    )

    async with httpx.AsyncClient() as client:
        with pytest.raises(httpx.RemoteProtocolError):
            await client.post(f"{proxy.url}/objects", json={"name": "not-created"})

    assert upstream_requests == []
    assert proxy.state["requests"][0]["forwarded"] is False


async def test_lost_acknowledgement_commits_once_then_disconnects(fault_proxy):
    proxy, upstream_requests = fault_proxy
    proxy.arm(
        PROVIDER_FAULT_PROFILES["lost_acknowledgement"],
        method="POST",
        path="/objects",
    )

    async with httpx.AsyncClient() as client:
        with pytest.raises(httpx.RemoteProtocolError):
            await client.post(f"{proxy.url}/objects", json={"name": "created-once"})

    await asyncio.sleep(0)
    assert len(upstream_requests) == 1
    request = proxy.state["requests"][0]
    assert request["forwarded"] is True
    assert request["upstream_status"] == 201
    assert request["responded"] is False


async def test_counted_fifo_rules_fire_then_restore_transparency(fault_proxy):
    proxy, upstream_requests = fault_proxy
    proxy.arm(
        PROVIDER_FAULT_PROFILES["http_400"],
        method="GET",
        path="/objects/1",
        count=2,
    )
    proxy.arm(
        PROVIDER_FAULT_PROFILES["http_503"],
        method="GET",
        path="/objects/1",
    )

    async with httpx.AsyncClient() as client:
        responses = [
            await client.get(f"{proxy.url}/objects/1")
            for _ in range(4)
        ]

    assert [response.status_code for response in responses] == [400, 400, 503, 200]
    assert [request["fault"] for request in proxy.state["requests"]] == [
        "http_400",
        "http_400",
        "http_503",
        None,
    ]
    assert proxy.state["rules"] == []
    assert len(upstream_requests) == 1


# Response profiles are exercised automatically by the parametrized test
# above. Non-response profiles have distinct behavior and therefore point to
# the dedicated tests pytest collects for each one.
_NON_RESPONSE_PROFILE_TESTS = {
    "timeout": test_timeout_profile_never_forwards_after_caller_times_out,
    "connection_reset": test_connection_reset_before_forward_never_reaches_provider,
    "truncated_response": test_truncated_response_aborts_before_declared_body_length,
    "lost_acknowledgement": test_lost_acknowledgement_commits_once_then_disconnects,
}


def test_every_published_profile_has_a_self_test():
    """Every published profile is exercised by a collected test."""
    dedicated_tests = tuple(_NON_RESPONSE_PROFILE_TESTS.values())
    assert all(
        test.__module__ == __name__ and test.__name__.startswith("test_")
        for test in dedicated_tests
    ), dedicated_tests

    covered = set(_RESPONSE_PROFILE_NAMES) | set(_NON_RESPONSE_PROFILE_TESTS)
    assert covered == set(PROVIDER_FAULT_PROFILES), {
        "untested": sorted(set(PROVIDER_FAULT_PROFILES) - covered),
        "stale": sorted(covered - set(PROVIDER_FAULT_PROFILES)),
    }


# --- fault-state isolation between cases (#6524 workstream 3) -----------------
#
# `reset()` is the only thing standing between one case's fault rules and the
# next case's requests, and it had no test. A leak here is nasty to diagnose
# from the far end: the next journey fails against a fault it never armed, and
# nothing in that journey's own code explains why.


async def test_reset_disarms_rules_the_previous_case_left_behind(fault_proxy):
    """An armed-but-unfired rule must not fire for whoever runs next."""
    proxy, upstream_requests = fault_proxy
    proxy.arm(PROVIDER_FAULT_PROFILES["http_500"], method="GET", path="/objects/1")

    proxy.reset()

    async with httpx.AsyncClient() as client:
        response = await client.get(f"{proxy.url}/objects/1")

    assert response.status_code == 200
    assert len(upstream_requests) == 1
    assert proxy.state["rules"] == []


async def test_reset_clears_recorded_evidence_from_the_previous_case(fault_proxy):
    """Request evidence is per-case: counts and fingerprints must not carry."""
    proxy, _ = fault_proxy
    async with httpx.AsyncClient() as client:
        await client.get(
            f"{proxy.url}/objects/1",
            headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
        )
    assert proxy.state["requests"]

    proxy.reset()

    assert proxy.state["requests"] == []
    # An assertion like `assert_network_egress_count(1)` in the next case would
    # silently pass on a leftover request, so the fingerprint must go too.
    assert AUTH_TOKEN not in str(proxy.state)


async def test_reset_leaves_no_delayed_fault_task_pending(fault_proxy):
    """A parked delay must not outlive the case that armed it.

    Not about the next request: the parked task holds its own request object,
    so leaving it running cannot abort anyone else's call — I asserted that
    first and sabotage proved the assertion blind. What it does leak is the
    task itself, which stays asleep past the case, keeps the proxy alive in
    the event loop, and fires its abort during whatever runs next.
    """
    proxy, _ = fault_proxy
    proxy.arm(
        ProviderFaultProfile(
            name="slow_fault",
            action="delay_before_disconnect",
            delay_seconds=30.0,
        ),
        method="GET",
        path="/objects/1",
    )
    async with httpx.AsyncClient(timeout=0.01) as client:
        with pytest.raises(httpx.ReadTimeout):
            await client.get(f"{proxy.url}/objects/1")
    await asyncio.sleep(0)
    assert proxy.state["pending_faults"] == 1, proxy.state

    proxy.reset()
    await asyncio.sleep(0)

    assert proxy.state["pending_faults"] == 0, proxy.state


async def test_world_reset_covers_every_provider_not_just_the_first():
    """One un-reset provider is enough to leak, so the loop must cover all."""
    upstreams = []
    proxies = {}
    try:
        for service in ("google", "slack", "github"):
            url, _requests, runner = await _start_upstream()
            upstreams.append(runner)
            proxies[service] = url
        world = ProviderFaultProxyWorld(proxies)
        await world.start()
        try:
            for proxy in world.proxies.values():
                proxy.arm(
                    PROVIDER_FAULT_PROFILES["http_500"],
                    method="GET",
                    path="/objects/1",
                )
            world.reset()
            assert [proxy.state["rules"] for proxy in world.proxies.values()] == [
                [],
                [],
                [],
            ]
        finally:
            await world.close()
    finally:
        for runner in upstreams:
            await runner.cleanup()
