"""Standalone Reborn WebUI v2 SSO and multi-user isolation smoke (#4636).

This scenario starts the shipping ``ironclaw serve`` process in session-auth
mode, drives the full Google-shaped OAuth redirect/callback/ticket exchange
against a local mock provider, and proves two admitted users retain distinct
thread and timeline scopes in one tenant.
"""

import json
from urllib.parse import parse_qs, urljoin, urlparse

import httpx

from helpers import sse_stream, wait_for_sse_line
from reborn_webui_harness import create_thread, reborn_bearer_headers

pytest_plugins = ["reborn_webui_harness"]


async def _login_with_next_mock_user(client: httpx.AsyncClient, base_url: str) -> str:
    login = await client.get(
        f"{base_url}/auth/login/google",
        params={"redirect_after": "/"},
        timeout=15,
    )
    assert login.status_code == 307, login.text

    authorize = await client.get(
        urljoin(base_url, login.headers["location"]),
        timeout=15,
    )
    assert authorize.status_code == 302, authorize.text

    callback = await client.get(
        urljoin(base_url, authorize.headers["location"]),
        timeout=15,
    )
    assert callback.status_code == 303, callback.text
    landing = urljoin(base_url, callback.headers["location"])
    ticket = parse_qs(urlparse(landing).query).get("login_ticket", [""])[0]
    assert ticket, callback.headers["location"]

    exchange = await client.post(
        f"{base_url}/auth/session/exchange",
        json={"ticket": ticket},
        timeout=15,
    )
    exchange.raise_for_status()
    token = exchange.json()["token"]
    assert token
    return token


async def _session(base_url: str, token: str) -> httpx.Response:
    async with httpx.AsyncClient(
        headers=reborn_bearer_headers(token),
        trust_env=False,
    ) as client:
        return await client.get(
            f"{base_url}/api/webchat/v2/session",
            timeout=15,
        )


async def _assert_cross_user_stream_denied(
    base_url: str,
    thread_id: str,
    token: str,
) -> None:
    # EventSource requires a successful HTTP open. Authorization failures are
    # therefore delivered as one redacted stream_error frame before close.
    async with sse_stream(
        base_url,
        path=f"/api/webchat/v2/threads/{thread_id}/events",
        token=token,
        timeout=15,
    ) as response:
        assert response.status == 200
        assert response.headers["content-type"].startswith("text/event-stream")
        event_line = await wait_for_sse_line(
            response,
            predicate=lambda line: line.startswith("event:"),
            timeout=10,
        )
        data_line = await wait_for_sse_line(
            response,
            predicate=lambda line: line.startswith("data:"),
            timeout=10,
        )

    assert event_line.partition(":")[2].strip() == "stream_error"
    assert json.loads(data_line.partition(":")[2].strip()) == {
        "error": "not_found",
        "kind": "not_found",
        "retryable": False,
    }


async def test_reborn_v2_sso_login_logout_and_multi_user_scope_isolation(
    reborn_v2_sso_server,
):
    base_url = reborn_v2_sso_server["base_url"]
    provider = reborn_v2_sso_server["provider"]

    async with httpx.AsyncClient(follow_redirects=False, trust_env=False) as public:
        providers = await public.get(f"{base_url}/auth/providers", timeout=15)
        providers.raise_for_status()
        assert providers.json() == {"providers": ["google"]}

        alice_token = await _login_with_next_mock_user(public, base_url)
        bob_token = await _login_with_next_mock_user(public, base_url)

    assert alice_token != bob_token
    assert len(provider.received_codes) == 2

    alice_session = await _session(base_url, alice_token)
    bob_session = await _session(base_url, bob_token)
    alice_session.raise_for_status()
    bob_session.raise_for_status()
    alice_identity = alice_session.json()
    bob_identity = bob_session.json()
    assert alice_identity["tenant_id"] == bob_identity["tenant_id"] == "reborn-v2-e2e"
    assert alice_identity["user_id"] != bob_identity["user_id"]
    assert alice_identity["capabilities"]["operator_webui_config"] is False
    assert bob_identity["capabilities"]["operator_webui_config"] is False

    async with (
        httpx.AsyncClient(
            headers=reborn_bearer_headers(alice_token),
            trust_env=False,
        ) as alice,
        httpx.AsyncClient(
            headers=reborn_bearer_headers(bob_token),
            trust_env=False,
        ) as bob,
    ):
        alice_thread = await create_thread(alice, base_url)
        bob_thread = await create_thread(bob, base_url)
        assert alice_thread != bob_thread

        alice_threads = await alice.get(
            f"{base_url}/api/webchat/v2/threads",
            timeout=15,
        )
        bob_threads = await bob.get(
            f"{base_url}/api/webchat/v2/threads",
            timeout=15,
        )
        alice_threads.raise_for_status()
        bob_threads.raise_for_status()
        alice_thread_ids = {
            thread["thread_id"] for thread in alice_threads.json()["threads"]
        }
        bob_thread_ids = {
            thread["thread_id"] for thread in bob_threads.json()["threads"]
        }
        assert alice_thread in alice_thread_ids
        assert bob_thread not in alice_thread_ids
        assert bob_thread in bob_thread_ids
        assert alice_thread not in bob_thread_ids

        alice_own_timeline = await alice.get(
            f"{base_url}/api/webchat/v2/threads/{alice_thread}/timeline",
            timeout=15,
        )
        bob_own_timeline = await bob.get(
            f"{base_url}/api/webchat/v2/threads/{bob_thread}/timeline",
            timeout=15,
        )
        alice_own_timeline.raise_for_status()
        bob_own_timeline.raise_for_status()
        assert alice_own_timeline.json()["messages"] == []
        assert bob_own_timeline.json()["messages"] == []

        alice_reads_bob = await alice.get(
            f"{base_url}/api/webchat/v2/threads/{bob_thread}/timeline",
            timeout=15,
        )
        bob_reads_alice = await bob.get(
            f"{base_url}/api/webchat/v2/threads/{alice_thread}/timeline",
            timeout=15,
        )
        assert alice_reads_bob.status_code == 404
        assert bob_reads_alice.status_code == 404

        await _assert_cross_user_stream_denied(
            base_url,
            bob_thread,
            alice_token,
        )
        await _assert_cross_user_stream_denied(
            base_url,
            alice_thread,
            bob_token,
        )

        logout = await alice.post(f"{base_url}/auth/logout", timeout=15)
        assert logout.status_code == 204

    assert (await _session(base_url, alice_token)).status_code == 401
    bob_still_authenticated = await _session(base_url, bob_token)
    bob_still_authenticated.raise_for_status()
    assert bob_still_authenticated.json()["user_id"] == bob_identity["user_id"]
