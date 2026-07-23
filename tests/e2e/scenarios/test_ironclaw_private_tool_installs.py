"""Private tool installs E2E (#5459 P1): import, install/activate, per-user
membership visibility, and real tool dispatch — driven through the real
``ironclaw serve`` binary.

Pure httpx/API surface, like ``test_admin_api.py`` and
``test_ironclaw_webui_v2_extensions_api.py``: the acceptance criteria here are
server-side (capability dispatch, list visibility), not DOM rendering, so no
Playwright ``page`` fixture is used.

Scenario:

1. Operator imports the three ``test-tools/`` fixture bundles
   (``test-tools/README.md``).
2. Operator installs + activates ``ascii-renderer`` tenant-wide (available to
   everyone).
3. Operator creates two member users, alice and bob, through the admin API.
4. Alice privately installs + activates ``hacker-news``.
5. Alice prompts for ``ascii-renderer`` (shared) and ``hacker-news`` (her
   private install); both dispatch.
6. Bob privately installs + activates ``market-data``.
7. ``hacker-news`` (alice's private install) is absent from bob's extension
   list — membership-scoped visibility, checked without prompting.
8. Bob prompts for ``ascii-renderer`` (shared) and ``market-data`` (his
   private install) in one turn; both dispatch.
"""

import uuid

import httpx

from ironclaw_webui_harness import (
    create_thread,
    enable_ironclaw_global_auto_approve,
    ironclaw_bearer_headers,
    ironclaw_v2_private_installs_yolo_server,  # noqa: F401 - imported fixture
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


async def _install_and_activate(
    client: httpx.AsyncClient, base_url: str, tool_id: str
) -> None:
    install = await client.post(
        f"{base_url}{EXTENSIONS_BASE}/install",
        json={"package_ref": _package_ref(tool_id)},
        timeout=15,
    )
    assert install.status_code == 200, install.text
    assert install.json()["success"] is True

    activate = await client.post(
        f"{base_url}{EXTENSIONS_BASE}/{tool_id}/activate",
        timeout=15,
    )
    assert activate.status_code == 200, activate.text
    assert activate.json()["success"] is True


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
    ironclaw_v2_private_installs_yolo_server, test_tool_zips
):
    base_url = ironclaw_v2_private_installs_yolo_server
    alice = None
    bob = None
    try:
        async with httpx.AsyncClient(
            base_url=base_url, headers=ironclaw_bearer_headers(), timeout=15
        ) as operator:
            # 1. Operator imports the three test-tools/ fixture bundles.
            for tool_id in ("ascii-renderer", "hacker-news", "market-data"):
                await _import_tool(operator, base_url, test_tool_zips[tool_id])

            # 2. Operator installs + activates ascii-renderer tenant-wide.
            await _install_and_activate(operator, base_url, "ascii-renderer")

            # 3. Operator creates alice and bob.
            alice = await _create_member_user(operator, base_url, display_name="Alice")
            bob = await _create_member_user(operator, base_url, display_name="Bob")

            # Auto-approve is caller-scoped, so the fixture's operator setting does
            # not grant it to newly-created members.
            await enable_ironclaw_global_auto_approve(base_url, token=alice["token"])
            await enable_ironclaw_global_auto_approve(base_url, token=bob["token"])

        async with _user_client(base_url, alice["token"]) as alice_client:
            # 4. Alice privately installs + activates hacker-news.
            await _install_and_activate(alice_client, base_url, "hacker-news")

            # 5. Alice's ascii-renderer (shared) and hacker-news (her private
            #    install) prompts both dispatch.
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
            # 6. Bob privately installs + activates market-data.
            await _install_and_activate(bob_client, base_url, "market-data")

            # 7. Alice's private hacker-news install is invisible to bob —
            #    a pure visibility check, no prompting needed.
            bob_extensions = await bob_client.get(f"{base_url}{EXTENSIONS_BASE}", timeout=15)
            assert bob_extensions.status_code == 200, bob_extensions.text
            bob_ids = _extension_ids(bob_extensions.json()["extensions"])
            assert "hacker-news" not in bob_ids
            # The shared ascii-renderer and his own market-data ARE visible.
            assert "ascii-renderer" in bob_ids
            assert "market-data" in bob_ids

            # 8. Bob's combined prompt dispatches both the shared
            #    ascii-renderer and his private market-data install in the
            #    same turn.
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
            base_url=base_url, headers=ironclaw_bearer_headers(), timeout=15
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
