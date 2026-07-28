"""Served Reborn WebUI v2 automation, trace, outbound, and channel API tests.

These scenarios exercise the browser-facing `/api/webchat/v2/*` route family
through a real `ironclaw-reborn serve` process. They intentionally cover the
served HTTP boundary rather than the Rust contract/substrate tests that normal
CI already owns.
"""

import uuid

import httpx

from reborn_webui_harness import reborn_bearer_headers

pytest_plugins = ["reborn_webui_harness"]


READ_PATHS = [
    "/api/webchat/v2/automations",
    "/api/webchat/v2/traces/credit",
    "/api/webchat/v2/outbound/preferences",
    "/api/webchat/v2/outbound/targets",
]


def _assert_secret_free(body: object, *secrets: str) -> None:
    rendered = str(body)
    for secret in secrets:
        assert secret not in rendered


async def test_reborn_v2_automation_trace_outbound_routes_require_auth_served(
    reborn_v2_server,
):
    async with httpx.AsyncClient() as anonymous:
        for path in READ_PATHS:
            response = await anonymous.get(f"{reborn_v2_server}{path}", timeout=15)
            assert response.status_code == 401, (path, response.text)

        for method, path in [
            ("post", "/api/webchat/v2/automations/missing-automation/pause"),
            ("post", "/api/webchat/v2/automations/missing-automation/resume"),
            ("delete", "/api/webchat/v2/automations/missing-automation"),
            (
                "post",
                "/api/webchat/v2/traces/holds/"
                f"{uuid.uuid4()}/authorize",
            ),
            ("post", "/api/webchat/v2/outbound/preferences"),
        ]:
            kwargs = {"timeout": 15}
            if method == "post" and path.endswith("preferences"):
                kwargs["json"] = {}
            response = await getattr(anonymous, method)(f"{reborn_v2_server}{path}", **kwargs)
            assert response.status_code == 401, (method, path, response.text)


async def test_reborn_v2_automations_empty_state_and_validation_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()

    async with httpx.AsyncClient(headers=headers) as client:
        listed = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/automations",
            params={"limit": 3, "run_limit": 2, "include_completed": "true"},
            timeout=15,
        )
        listed.raise_for_status()
        listed_body = listed.json()
        assert isinstance(listed_body["automations"], list)
        assert isinstance(listed_body["scheduler_enabled"], bool)

        malformed_bool = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/automations",
            params={"include_completed": "not-a-bool"},
            timeout=15,
        )
        assert malformed_bool.status_code == 400

        malformed_limit = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/automations",
            params={"limit": "not-a-number"},
            timeout=15,
        )
        assert malformed_limit.status_code == 400

        for method, suffix in [
            ("post", "pause"),
            ("post", "resume"),
            ("delete", ""),
        ]:
            path = "/api/webchat/v2/automations/not-a-trigger-id"
            if suffix:
                path = f"{path}/{suffix}"
            response = await getattr(client, method)(
                f"{reborn_v2_server}{path}",
                timeout=15,
            )
            assert response.status_code == 400, (method, path, response.text)


async def test_reborn_v2_trace_credits_and_hold_authorize_served(reborn_v2_server):
    headers = reborn_bearer_headers()
    submission_id = str(uuid.uuid4())

    async with httpx.AsyncClient(headers=headers) as client:
        credits = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/traces/credit",
            timeout=15,
        )
        credits.raise_for_status()
        credits_body = credits.json()
        assert credits_body["enrolled"] is False
        assert credits_body["pending_credit"] == 0
        assert credits_body["final_credit"] == 0
        assert credits_body["submissions_total"] == 0
        assert credits_body["manual_review_hold_count"] == 0
        assert credits_body.get("holds", []) == []
        assert "authoritative ledger" in credits_body["note"]
        _assert_secret_free(credits_body, "access_token", "refresh_token")

        authorized = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/traces/holds/"
            f"{submission_id}/authorize",
            timeout=15,
        )
        authorized.raise_for_status()
        assert authorized.json() == {"authorized": False}

        malformed_submission = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/traces/holds/not-a-uuid/authorize",
            timeout=15,
        )
        assert malformed_submission.status_code == 400


async def test_reborn_v2_outbound_preferences_targets_and_channels_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()

    async with httpx.AsyncClient(headers=headers) as client:
        preferences = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/outbound/preferences",
            timeout=15,
        )
        preferences.raise_for_status()
        preferences_body = preferences.json()
        assert preferences_body.get("final_reply_target") is None
        assert preferences_body["final_reply_target_status"] == "none_configured"
        assert preferences_body["default_modality"] == "text"

        clear = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/outbound/preferences",
            json={"final_reply_target_id": None},
            timeout=15,
        )
        clear.raise_for_status()
        assert clear.json()["final_reply_target_status"] == "none_configured"

        unknown_target = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/outbound/preferences",
            json={"final_reply_target_id": "missing-target"},
            timeout=15,
        )
        assert unknown_target.status_code in {400, 404}, unknown_target.text

        targets = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/outbound/targets",
            timeout=15,
        )
        targets.raise_for_status()
        targets_body = targets.json()
        assert isinstance(targets_body["targets"], list)
        assert targets_body.get("next_cursor") in {None, ""}
