"""Served Reborn WebUI v2 product-auth API tests.

These scenarios exercise `/api/reborn/product-auth/*` through a real
`ironclaw-reborn serve` process. They cover caller-facing auth, validation,
redaction, and empty-state behavior without re-running Rust contract suites.
"""

import uuid

import httpx

from reborn_webui_harness import reborn_bearer_headers

pytest_plugins = ["reborn_webui_harness"]


PRODUCT_AUTH_PATHS = [
    "/api/reborn/product-auth/manual-token/setup",
    "/api/reborn/product-auth/manual-token/secret-submit",
    "/api/reborn/product-auth/accounts/list",
    "/api/reborn/product-auth/accounts/select",
    "/api/reborn/product-auth/accounts/recovery",
    "/api/reborn/product-auth/accounts/refresh",
    "/api/reborn/product-auth/lifecycle/cleanup",
    "/api/reborn/product-auth/oauth/start",
    "/api/reborn/product-auth/oauth/google/start",
]


def _invocation_id() -> str:
    return str(uuid.uuid4())


def _assert_secret_free(body: object, *secrets: str) -> None:
    rendered = str(body)
    for secret in secrets:
        assert secret not in rendered


async def test_reborn_v2_product_auth_routes_require_auth_served(reborn_v2_server):
    async with httpx.AsyncClient() as anonymous:
        for path in PRODUCT_AUTH_PATHS:
            response = await anonymous.post(
                f"{reborn_v2_server}{path}",
                json={},
                timeout=15,
            )
            assert response.status_code == 401, (path, response.text)


async def test_reborn_v2_product_auth_manual_token_setup_redacts_submit_error_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()
    raw_token = "qa-fake-manual-token-must-not-echo"
    run_id = _invocation_id()

    async with httpx.AsyncClient(headers=headers) as client:
        setup = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/manual-token/setup",
            json={
                "provider": "github",
                "account_label": "served github account",
                "run_id": run_id,
                "gate_ref": "gate:served-product-auth",
                "thread_id": "thread-served-product-auth",
            },
            timeout=15,
        )
        setup.raise_for_status()
        setup_body = setup.json()
        interaction_id = setup_body["interaction_id"]
        invocation_id = setup_body["invocation_id"]
        assert setup_body["provider"] == "github"
        _assert_secret_free(setup_body, raw_token)

        submit = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/manual-token/secret-submit",
            json={
                "interaction_id": interaction_id,
                "token": raw_token,
                "thread_id": "thread-served-product-auth",
                "invocation_id": invocation_id,
            },
            timeout=15,
        )
        assert submit.status_code in {400, 403, 404}, submit.text
        submit_body = submit.json()
        error_code = submit_body.get("error", {}).get("code") or submit_body.get("code")
        assert error_code in {
            "forbidden",
            "invalid_request",
            "not_found",
            "unknown_or_expired_flow",
        }
        _assert_secret_free(submit_body, raw_token)


async def test_reborn_v2_product_auth_accounts_and_recovery_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()
    invocation_id = _invocation_id()

    async with httpx.AsyncClient(headers=headers) as client:
        listed = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/accounts/list",
            json={"provider": "github", "invocation_id": invocation_id},
            timeout=15,
        )
        listed.raise_for_status()
        listed_body = listed.json()
        assert isinstance(listed_body["accounts"], list)

        recovery = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/accounts/recovery",
            json={"provider": "github", "invocation_id": invocation_id},
            timeout=15,
        )
        recovery.raise_for_status()
        recovery_body = recovery.json()
        assert recovery_body["provider"] == "github"
        assert recovery_body["kind"] in {"configured", "setup_required", "unavailable"}
        _assert_secret_free(recovery_body, "ghp_", "access_token", "refresh_token")


async def test_reborn_v2_product_auth_routes_reject_invalid_input_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()

    async with httpx.AsyncClient(headers=headers) as client:
        missing_invocation = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/accounts/list",
            json={"provider": "github"},
            timeout=15,
        )
        assert missing_invocation.status_code == 400

        malformed_invocation = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/accounts/select",
            json={
                "provider": "github",
                "account_id": _invocation_id(),
                "invocation_id": "not-a-uuid",
            },
            timeout=15,
        )
        assert malformed_invocation.status_code == 400

        malformed_account = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/accounts/refresh",
            json={
                "provider": "github",
                "account_id": "not-a-uuid",
                "invocation_id": _invocation_id(),
            },
            timeout=15,
        )
        assert malformed_account.status_code == 400

        empty_manual_provider = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/manual-token/setup",
            json={"provider": "", "account_label": "served account"},
            timeout=15,
        )
        assert empty_manual_provider.status_code == 400

        invalid_extension_id = await client.post(
            f"{reborn_v2_server}/api/reborn/product-auth/lifecycle/cleanup",
            json={
                "extension_id": "../bad",
                "action": "deactivate",
                "invocation_id": _invocation_id(),
            },
            timeout=15,
        )
        assert invalid_extension_id.status_code == 400
