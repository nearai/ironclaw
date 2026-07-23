"""Self-tests for the reusable provider fault proxy."""

from __future__ import annotations

import asyncio

import httpx
import pytest
from aiohttp import web

from provider_fault_proxy import (
    PROVIDER_FAULT_PROFILES,
    ProviderFaultProfile,
    ProviderFaultProxy,
)


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
        return web.json_response(
            {"id": "provider-object", "method": request.method},
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


@pytest.mark.parametrize(
    "profile_name",
    (
        "http_400",
        "http_401",
        "http_403",
        "http_404",
        "http_409",
        "http_429",
        "http_500",
        "http_503",
        "malformed_json",
        "truncated_response",
        "missing_field",
    ),
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


async def test_delay_profile_is_bounded_and_then_forwards(fault_proxy):
    proxy, upstream_requests = fault_proxy
    proxy.arm(
        ProviderFaultProfile(
            name="short_timeout",
            action="delay_before_forward",
            delay_seconds=0.02,
        ),
        method="GET",
        path="/objects/1",
    )

    async with httpx.AsyncClient(timeout=1) as client:
        response = await client.get(f"{proxy.url}/objects/1")

    assert response.status_code == 200
    assert len(upstream_requests) == 1
    assert proxy.state["requests"][0]["fault"] == "short_timeout"


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


async def test_control_reset_clears_faults_and_request_ledger(fault_proxy):
    proxy, _ = fault_proxy
    async with httpx.AsyncClient() as client:
        armed = await client.post(
            f"{proxy.url}/__mock/provider_faults",
            json={
                "profile": "http_503",
                "method": "GET",
                "path": "/objects",
                "count": 1,
            },
        )
        armed.raise_for_status()
        assert len(armed.json()["rules"]) == 1
        await client.get(f"{proxy.url}/unmatched")
        reset = await client.post(f"{proxy.url}/__mock/provider_faults/reset")

    reset.raise_for_status()
    assert proxy.state == {"rules": [], "requests": []}


async def test_control_rejects_invalid_profile_and_matcher(fault_proxy):
    proxy, _ = fault_proxy
    async with httpx.AsyncClient() as client:
        unknown = await client.post(
            f"{proxy.url}/__mock/provider_faults",
            json={
                "profile": "not-a-profile",
                "method": "GET",
                "path": "/objects",
            },
        )
        invalid_count = await client.post(
            f"{proxy.url}/__mock/provider_faults",
            json={
                "profile": "http_503",
                "method": "GET",
                "path": "/objects",
                "count": 0,
            },
        )

    assert unknown.status_code == 400
    assert invalid_count.status_code == 400
    assert proxy.state == {"rules": [], "requests": []}
