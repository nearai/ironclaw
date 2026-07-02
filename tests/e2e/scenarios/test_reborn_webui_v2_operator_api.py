"""Served Reborn WebUI v2 operator and LLM configuration API tests.

These scenarios convert REBCLI-048 QA-matrix rows from Rust contract proxies
to caller-facing coverage through a real `ironclaw-reborn serve` process.
They intentionally avoid provider-login browser flows, which are covered by
the dedicated provider-login scenarios.
"""

import httpx

from reborn_webui_harness import reborn_bearer_headers

pytest_plugins = ["reborn_webui_harness"]


async def test_reborn_v2_operator_and_llm_routes_require_bearer_served(reborn_v2_server):
    async with httpx.AsyncClient() as client:
        for method, path in [
            ("GET", "/api/webchat/v2/llm/providers"),
            ("POST", "/api/webchat/v2/llm/active"),
            ("GET", "/api/webchat/v2/operator/config"),
            ("GET", "/api/webchat/v2/operator/status"),
            ("GET", "/api/webchat/v2/operator/logs?limit=1"),
        ]:
            response = await client.request(
                method,
                f"{reborn_v2_server}{path}",
                json={"provider_id": "openai"} if method == "POST" else None,
                timeout=15,
            )
            assert response.status_code == 401, (method, path, response.text)


async def test_reborn_v2_llm_provider_config_round_trip_served(
    reborn_v2_server, mock_llm_server
):
    headers = reborn_bearer_headers()
    provider_payload = {
        "id": "qa-provider",
        "name": "QA Provider",
        "adapter": "open_ai_completions",
        "base_url": f"{mock_llm_server}/v1",
        "default_model": "mock-model",
        "api_key": "qa-provider-secret",
        "set_active": True,
        "model": "mock-model",
    }
    probe_payload = {
        "provider_id": "qa-provider",
        "adapter": "open_ai_completions",
        "base_url": f"{mock_llm_server}/v1",
        "model": "mock-model",
        "api_key": "qa-provider-secret",
    }

    async with httpx.AsyncClient(headers=headers) as client:
        initial = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/llm/providers",
            timeout=15,
        )
        initial.raise_for_status()
        initial_body = initial.json()
        assert isinstance(initial_body.get("providers"), list)
        assert initial_body.get("active", {}).get("provider_id") == "openai"

        upsert = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/llm/providers",
            json=provider_payload,
            timeout=15,
        )
        upsert.raise_for_status()
        providers = {provider["id"]: provider for provider in upsert.json()["providers"]}
        assert providers["qa-provider"]["active"] is True
        assert providers["qa-provider"]["api_key_set"] is True
        assert "qa-provider-secret" not in upsert.text

        models = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/llm/list-models",
            json=probe_payload,
            timeout=15,
        )
        models.raise_for_status()
        models_body = models.json()
        assert models_body["ok"] is True
        assert "mock-model" in models_body["models"]
        assert "qa-provider-secret" not in models.text

        probe = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/llm/test-connection",
            json=probe_payload,
            timeout=15,
        )
        probe.raise_for_status()
        assert probe.json()["ok"] is True
        assert "qa-provider-secret" not in probe.text

        active = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/llm/active",
            json={"provider_id": "openai", "model": "mock-model"},
            timeout=15,
        )
        active.raise_for_status()
        assert active.json()["active"]["provider_id"] == "openai"

        delete = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/llm/providers/qa-provider/delete",
            timeout=15,
        )
        delete.raise_for_status()
        assert "qa-provider" not in {
            provider["id"] for provider in delete.json()["providers"]
        }

        invalid = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/llm/providers",
            json={**provider_payload, "id": "Bad Provider"},
            timeout=15,
        )
        assert invalid.status_code == 400


async def test_reborn_v2_operator_config_status_and_logs_served(reborn_v2_server):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        config = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/operator/config",
            timeout=15,
        )
        config.raise_for_status()
        config_body = config.json()
        assert isinstance(config_body.get("entries"), list)

        set_config = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/operator/config/agent.auto_approve_tools",
            json={"value": True},
            timeout=15,
        )
        set_config.raise_for_status()
        assert set_config.json()["entry"]["value"] is True

        get_config = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/operator/config/agent.auto_approve_tools",
            timeout=15,
        )
        get_config.raise_for_status()
        assert get_config.json()["entry"]["value"] is True

        validate = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/operator/config/validate",
            json={"keys": ["agent.auto_approve_tools"]},
            timeout=15,
        )
        validate.raise_for_status()
        validate_body = validate.json()
        assert isinstance(validate_body["valid"], bool)
        assert isinstance(validate_body.get("diagnostics", []), list)

        reserved_static_route = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/operator/config/validate",
            timeout=15,
        )
        assert reserved_static_route.status_code == 400

        invalid_key = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/operator/config/BadKey",
            timeout=15,
        )
        assert invalid_key.status_code == 400

        status = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/operator/status",
            timeout=15,
        )
        status.raise_for_status()
        status_body = status.json()
        assert status_body["area"] == "status"
        assert status_body["status"] == "available"
        assert "operator_status" in status_body

        diagnostics = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/operator/diagnostics",
            timeout=15,
        )
        diagnostics.raise_for_status()
        diagnostics_body = diagnostics.json()
        assert diagnostics_body["area"] == "diagnostics"
        assert diagnostics_body["status"] in {"available", "unavailable"}
        assert isinstance(diagnostics_body.get("message"), str)

        logs = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/operator/logs",
            params={"limit": 1, "tail": "true"},
            timeout=15,
        )
        logs.raise_for_status()
        logs_body = logs.json()
        assert logs_body["area"] == "logs"
        assert logs_body["status"] == "available"
