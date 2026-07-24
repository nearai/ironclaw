"""Served Reborn WebUI v2 extension lifecycle API tests.

These scenarios exercise `/api/webchat/v2/extensions*` through a real
`ironclaw-reborn serve` process. They replace QA-matrix coverage that used to
be represented by Rust handler/composition contract tests, which are now owned
by normal CI.
"""

import httpx

from reborn_webui_harness import client_action_id, reborn_bearer_headers

pytest_plugins = ["reborn_webui_harness"]

WEB_ACCESS_PACKAGE_REF = {"kind": "extension", "id": "web-access"}


def _extension_ids(extensions: list[dict]) -> set[str]:
    return {
        extension["package_ref"]["id"]
        for extension in extensions
        if extension.get("package_ref", {}).get("id")
    }


async def _remove_web_access_if_present(
    client: httpx.AsyncClient, base_url: str
) -> None:
    response = await client.post(
        f"{base_url}/api/webchat/v2/extensions/web-access/remove",
        json={"client_action_id": client_action_id()},
        timeout=15,
    )
    assert response.status_code == 200 or 400 <= response.status_code < 500


async def test_reborn_v2_extension_lifecycle_served(reborn_v2_server):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        await _remove_web_access_if_present(client, reborn_v2_server)

        registry = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/extensions/registry",
            timeout=15,
        )
        registry.raise_for_status()
        entries = registry.json()["entries"]
        web_access_entry = next(
            entry for entry in entries if entry["package_ref"]["id"] == "web-access"
        )
        assert web_access_entry["package_ref"] == WEB_ACCESS_PACKAGE_REF
        assert web_access_entry["display_name"] == "Web Access"
        assert web_access_entry["installed"] is False

        listed = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/extensions",
            timeout=15,
        )
        listed.raise_for_status()
        assert "web-access" not in _extension_ids(listed.json()["extensions"])

        install = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/extensions/install",
            json={
                "package_ref": WEB_ACCESS_PACKAGE_REF,
                "client_action_id": client_action_id(),
            },
            timeout=15,
        )
        install.raise_for_status()
        install_body = install.json()
        assert install_body["success"] is True
        assert isinstance(install_body["message"], str)

        try:
            installed_list = await client.get(
                f"{reborn_v2_server}/api/webchat/v2/extensions",
                timeout=15,
            )
            installed_list.raise_for_status()
            installed = next(
                extension
                for extension in installed_list.json()["extensions"]
                if extension["package_ref"]["id"] == "web-access"
            )
            assert installed["display_name"] == "Web Access"
            # Runtime is an implementation badge (`runtime`), never taxonomy;
            # the retired `kind` wire string is gone (NEA-25).
            assert installed["runtime"] == "first_party"
            # Membership plus readiness is the complete public lifecycle.
            # A setup-free extension becomes active as part of install; there
            # is no caller-visible activation checkpoint.
            assert installed["installation_state"] == "active"
            for retired in (
                "authenticated",
                "active",
                "needs_setup",
                "has_auth",
                "onboarding_state",
            ):
                assert retired not in installed

            setup = await client.get(
                f"{reborn_v2_server}/api/webchat/v2/extensions/web-access/setup",
                timeout=15,
            )
            setup.raise_for_status()
            setup_body = setup.json()
            assert setup_body["package_ref"] == WEB_ACCESS_PACKAGE_REF
            assert setup_body["phase"] == "active"
            assert isinstance(setup_body.get("blockers", []), list)
            assert "web-access.search" in installed.get("tools", [])
        finally:
            await _remove_web_access_if_present(client, reborn_v2_server)

        final_list = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/extensions",
            timeout=15,
        )
        final_list.raise_for_status()
        assert "web-access" not in _extension_ids(final_list.json()["extensions"])


async def test_reborn_v2_public_activation_route_is_absent_served(reborn_v2_server):
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        response = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/extensions/web-access/activate",
            timeout=15,
        )

    assert response.status_code == 404


async def test_reborn_v2_extension_routes_require_auth_served(reborn_v2_server):
    async with httpx.AsyncClient() as anonymous:
        for method, path, body in [
            ("GET", "/api/webchat/v2/extensions", None),
            ("GET", "/api/webchat/v2/extensions/registry", None),
            (
                "POST",
                "/api/webchat/v2/extensions/install",
                {
                    "package_ref": WEB_ACCESS_PACKAGE_REF,
                    "client_action_id": client_action_id(),
                },
            ),
            (
                "POST",
                "/api/webchat/v2/extensions/web-access/activate",
                {"client_action_id": client_action_id()},
            ),
            (
                "POST",
                "/api/webchat/v2/extensions/web-access/remove",
                {"client_action_id": client_action_id()},
            ),
            ("POST", "/api/webchat/v2/extensions/web-access/remove", None),
            ("GET", "/api/webchat/v2/extensions/web-access/setup", None),
            (
                "POST",
                "/api/webchat/v2/extensions/web-access/setup",
                {"client_action_id": client_action_id()},
            ),
        ]:
            response = await anonymous.request(
                method,
                f"{reborn_v2_server}{path}",
                json=body,
                timeout=15,
            )
            assert response.status_code == 401, (method, path, response.text)


async def test_reborn_v2_extension_routes_reject_invalid_input_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        missing_install_body = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/extensions/install",
            json={},
            timeout=15,
        )
        assert missing_install_body.status_code == 422

        wrong_package_kind = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/extensions/install",
            json={
                "package_ref": {"kind": "skill", "id": "web-access"},
                "client_action_id": client_action_id(),
            },
            timeout=15,
        )
        assert wrong_package_kind.status_code == 400

        malformed_package_id = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/extensions/bad%20id/remove",
            timeout=15,
        )
        assert malformed_package_id.status_code == 400

        missing_package = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/extensions/not-installed-extension/setup",
            timeout=15,
        )
        assert missing_package.status_code in {400, 404}
