"""GitHub account setup and provider compatibility probes for journeys."""

import httpx
from emulate_provider import github_headers
from helpers import EMULATE_GITHUB_BEARER
from reborn_webui_harness import client_action_id, reborn_bearer_headers

GITHUB_RELEASE_WRITE_UNAVAILABLE = {403, 404}
GITHUB_RELEASE_WRITE_PROBE_PAYLOAD = {"tag_name": ""}


async def seed_github_account(base_url: str) -> None:
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        response = await client.post(
            f"{base_url}/api/webchat/v2/extensions/github/setup",
            json={
                "action": "submit",
                "client_action_id": client_action_id(),
                "payload": {
                    "secrets": {"github_runtime_token": EMULATE_GITHUB_BEARER},
                    "fields": {},
                },
            },
            timeout=30,
        )
        response.raise_for_status()
        secret = next(
            item
            for item in response.json()["secrets"]
            if item["name"] == "github_runtime_token"
        )
        assert secret["provided"] is True, response.text


async def emulate_github_supports_release_writes(emulate_url: str) -> bool:
    async with httpx.AsyncClient(headers=github_headers(), timeout=15) as client:
        response = await client.post(
            f"{emulate_url}/repos/nearai/ironclaw/releases",
            json=GITHUB_RELEASE_WRITE_PROBE_PAYLOAD,
        )
    if response.status_code in GITHUB_RELEASE_WRITE_UNAVAILABLE:
        return False
    if response.status_code != 422:
        raise AssertionError(
            "GitHub release write probe must reject the invalid payload "
            f"without mutation; got {response.status_code}: {response.text}"
        )
    return True
