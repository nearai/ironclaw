"""Self-tests for the reusable provider fault proxy."""

from __future__ import annotations

import asyncio
import gzip

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


async def test_provider_fault_proxy_preserves_compressed_responses(fault_proxy):
    proxy, upstream_requests = fault_proxy

    async with httpx.AsyncClient() as client:
        response = await client.get(f"{proxy.url}/compressed")

    assert response.json() == {"id": "provider-object", "method": "GET"}
    assert response.headers["Content-Encoding"] == "gzip"
    assert len(upstream_requests) == 1


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
