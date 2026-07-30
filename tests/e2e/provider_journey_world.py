"""Reusable setup for the provider worlds shared by journey cases."""

import httpx
from provider_journey_github import seed_github_account
from provider_journey_google import GOOGLE_EXTENSIONS, seed_google_account
from provider_journey_slack import (
    configure_slack,
    seed_slack_account,
    seed_slack_workspace,
)
from reborn_webui_harness import (
    client_action_id,
    reborn_bearer_headers,
)

ALL_EXTENSIONS = (*GOOGLE_EXTENSIONS, "github", "slack")


async def install_extensions(
    base_url: str, extension_ids: tuple[str, ...] = ALL_EXTENSIONS
) -> None:
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        for extension_id in extension_ids:
            installed = await client.post(
                f"{base_url}/api/webchat/v2/extensions/install",
                json={
                    "package_ref": {"kind": "extension", "id": extension_id},
                    "client_action_id": client_action_id(),
                },
                timeout=30,
            )
            installed.raise_for_status()


async def assert_extensions_active(
    base_url: str, extension_ids: tuple[str, ...] = ALL_EXTENSIONS
) -> None:
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        listed = await client.get(
            f"{base_url}/api/webchat/v2/extensions",
            timeout=30,
        )
        listed.raise_for_status()
        by_id = {
            extension["package_ref"]["id"]: extension
            for extension in listed.json()["extensions"]
        }
        for extension_id in extension_ids:
            extension = by_id.get(extension_id)
            assert extension is not None, f"{extension_id} disappeared after setup"
            assert extension["installation_state"] == "active", extension


async def build_provider_journey_world(
    base_url: str, emulate_slack_url: str
) -> dict[str, str]:
    """Install, configure, and authenticate every journey provider."""
    slack_state = await seed_slack_workspace(emulate_slack_url)
    await configure_slack(base_url, slack_state)
    await install_extensions(base_url)
    for extension_index, extension_id in enumerate(GOOGLE_EXTENSIONS):
        await seed_google_account(
            base_url,
            extension_id,
            allow_existing_account=extension_index > 0,
        )
    await seed_github_account(base_url)
    await seed_slack_account(base_url, emulate_slack_url, slack_state)
    await assert_extensions_active(base_url)
    return slack_state
