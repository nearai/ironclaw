"""Private tool installs E2E (#5459 P1): import, install, per-user
membership visibility, and real tool dispatch — driven through the real
``ironclaw-reborn serve`` binary.

Pure httpx/API surface, like ``test_admin_api.py`` and
``test_reborn_webui_v2_extensions_api.py``: the acceptance criteria here are
server-side (capability dispatch, list visibility), not DOM rendering, so no
Playwright ``page`` fixture is used.

Scenario:

1. Operator imports the three ``test-tools/`` fixture bundles
   (``test-tools/README.md``).
2. Operator installs ``ascii-renderer`` for their own user; administrator
   privilege does not make that membership tenant-wide.
3. Operator creates two member users, alice and bob, through the admin API.
4. Alice privately installs ``ascii-renderer`` and ``hacker-news``; both
   dispatch.
5. Bob cannot see either of Alice's memberships, then privately installs
   ``ascii-renderer`` and ``market-data``; both dispatch.
"""

import uuid

import httpx

from reborn_webui_harness import (
    create_thread,
    enable_reborn_global_auto_approve,
    reborn_bearer_headers,
    reborn_v2_private_installs_yolo_server,  # noqa: F401 - imported fixture
    send_and_settle,
    wait_for_capability_preview,
)

EXTENSIONS_BASE = "/api/webchat/v2/extensions"
ADMIN_BASE = "/api/webchat/v2/admin"


def _package_ref(tool_id: str) -> dict:
    return {"kind": "extension", "id": tool_id}


def _extension_ids(extensions: list[dict]) -> set[str]:
    return {
        extension["package_ref"]["id"]
        for extension in extensions
        if extension.get("package_ref", {}).get("id")
    }


async def _import_tool(client: httpx.AsyncClient, base_url: str, zip_path) -> None:
    response = await client.post(
        f"{base_url}{EXTENSIONS_BASE}/import",
        content=zip_path.read_bytes(),
        timeout=15,
    )
    assert response.status_code == 200, response.text
    assert response.json()["success"] is True


async def _install(client: httpx.AsyncClient, base_url: str, tool_id: str) -> None:
    install = await client.post(
        f"{base_url}{EXTENSIONS_BASE}/install",
        json={"package_ref": _package_ref(tool_id)},
        timeout=15,
    )
    assert install.status_code == 200, install.text
    assert install.json()["success"] is True


async def _create_member_user(
    operator_client: httpx.AsyncClient, base_url: str, *, display_name: str
) -> dict:
    email = f"{display_name.lower()}-{uuid.uuid4().hex[:8]}@example.com"
    response = await operator_client.post(
        f"{base_url}{ADMIN_BASE}/users",
        json={"display_name": display_name, "email": email, "role": "member"},
        timeout=15,
    )
    assert response.status_code == 200, response.text
    body = response.json()
    return {"user_id": body["user"]["user_id"], "token": body["api_token"]}


def _user_client(base_url: str, token: str) -> httpx.AsyncClient:
    return httpx.AsyncClient(
        base_url=base_url,
        headers={"Authorization": f"Bearer {token}"},
        timeout=15,
    )


async def test_private_tool_installs_full_path(
    reborn_v2_private_installs_yolo_server, test_tool_zips
):
    base_url = reborn_v2_private_installs_yolo_server
    alice = None
    bob = None
    try:
        async with httpx.AsyncClient(
            base_url=base_url, headers=reborn_bearer_headers(), timeout=15
        ) as operator:
            # 1. Operator imports the three test-tools/ fixture bundles.
            for tool_id in ("ascii-renderer", "hacker-news", "market-data"):
                await _import_tool(operator, base_url, test_tool_zips[tool_id])

            # 2. Operator installs ascii-renderer only for their own user.
            await _install(operator, base_url, "ascii-renderer")

            # 3. Operator creates alice and bob.
            alice = await _create_member_user(operator, base_url, display_name="Alice")
            bob = await _create_member_user(operator, base_url, display_name="Bob")

            # Auto-approve is caller-scoped, so the fixture's operator setting does
            # not grant it to newly-created members.
            await enable_reborn_global_auto_approve(base_url, token=alice["token"])
            await enable_reborn_global_auto_approve(base_url, token=bob["token"])

        async with _user_client(base_url, alice["token"]) as alice_client:
            # 4. Alice's memberships are independent from the operator's.
            alice_before = await alice_client.get(
                f"{base_url}{EXTENSIONS_BASE}", timeout=15
            )
            assert alice_before.status_code == 200, alice_before.text
            assert "ascii-renderer" not in _extension_ids(
                alice_before.json()["extensions"]
            )
            await _install(alice_client, base_url, "ascii-renderer")
            await _install(alice_client, base_url, "hacker-news")

            # Both of Alice's personal installs dispatch.
            thread_id = await create_thread(alice_client, base_url)
            await send_and_settle(
                alice_client,
                base_url,
                thread_id,
                "let's use the ascii renderer to draw a cat",
                expected=1,
            )
            ascii_preview = await wait_for_capability_preview(
                alice_client,
                base_url,
                thread_id,
                "ascii-renderer.draw",
                output_fragment="cat",
            )
            assert ascii_preview["status"] == "completed", ascii_preview

            await send_and_settle(
                alice_client,
                base_url,
                thread_id,
                "let's use the hacker news tool",
                expected=2,
            )
            hn_preview = await wait_for_capability_preview(
                alice_client,
                base_url,
                thread_id,
                "hacker-news.top_stories",
                output_fragment="canned fixture data",
            )
            assert hn_preview["status"] == "completed", hn_preview

        async with _user_client(base_url, bob["token"]) as bob_client:
            # 5. Alice's memberships are invisible to Bob.
            bob_extensions = await bob_client.get(f"{base_url}{EXTENSIONS_BASE}", timeout=15)
            assert bob_extensions.status_code == 200, bob_extensions.text
            bob_ids = _extension_ids(bob_extensions.json()["extensions"])
            assert "hacker-news" not in bob_ids
            assert "ascii-renderer" not in bob_ids
            assert "market-data" not in bob_ids

            await _install(bob_client, base_url, "ascii-renderer")
            await _install(bob_client, base_url, "market-data")

            # Bob's own installs dispatch in the same turn.
            thread_id = await create_thread(bob_client, base_url)
            await send_and_settle(
                bob_client,
                base_url,
                thread_id,
                "let's use ascii renderer and market data",
                expected=1,
            )
            ascii_preview = await wait_for_capability_preview(
                bob_client,
                base_url,
                thread_id,
                "ascii-renderer.draw",
                output_fragment="robot",
            )
            market_preview = await wait_for_capability_preview(
                bob_client,
                base_url,
                thread_id,
                "market-data.snp500",
                output_fragment="SPX",
            )
            assert ascii_preview["status"] == "completed", ascii_preview
            assert market_preview["status"] == "completed", market_preview
    finally:
        async with httpx.AsyncClient(
            base_url=base_url, headers=reborn_bearer_headers(), timeout=15
        ) as operator:
            if alice is not None:
                alice_delete = await operator.delete(
                    f"{base_url}{ADMIN_BASE}/users/{alice['user_id']}"
                )
                assert alice_delete.status_code == 200, alice_delete.text
            if bob is not None:
                bob_delete = await operator.delete(
                    f"{base_url}{ADMIN_BASE}/users/{bob['user_id']}"
                )
                assert bob_delete.status_code == 200, bob_delete.text
