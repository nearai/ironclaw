"""Reborn WebUI v2 tier-3 coverage: management surfaces beyond chat.

Exercises the operator/management routes the SPA drives outside the chat
timeline: extension lifecycle (list / registry / setup projection /
install+activate+remove), skills (list / details / search / install round-trip),
LLM provider config (snapshot / upsert / set-active / test-connection /
list-models / delete against the mock provider), and the automations +
connectable-channels read projections.

All routes authenticate with the same env-bearer caller the smoke suite uses;
the session reports `operator_webui_config: true`, which is what mounts the
`/llm/*` operator routes in a trusted single-operator deployment. The mock LLM
speaks the `open_ai_completions` adapter wire shape, so connection probes resolve
deterministically without a real provider.

Tracks nearai/ironclaw#4635.
"""

import httpx

from helpers import REBORN_V2_AUTH_TOKEN

_BEARER = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}

# The mock LLM serves OpenAI `/v1/chat/completions` + `/v1/models`; the adapter
# wire name that speaks that protocol is `open_ai_completions`.
_OPENAI_ADAPTER = "open_ai_completions"


def _client(base_url: str) -> httpx.AsyncClient:
    return httpx.AsyncClient(base_url=base_url, headers=_BEARER, timeout=30)


async def test_reborn_v2_session_reports_operator_capability(reborn_v2_server):
    """The env-bearer caller is an operator, which is what mounts the /llm/* routes."""
    async with _client(reborn_v2_server) as client:
        response = await client.get("/api/webchat/v2/session")
        response.raise_for_status()
        body = response.json()
        assert body["tenant_id"] == "reborn-v2-e2e", body
        assert body["capabilities"]["operator_webui_config"] is True, body


async def test_reborn_v2_extensions_list_and_registry(reborn_v2_server):
    """The installed-extension list and the registry catalog both project cleanly."""
    async with _client(reborn_v2_server) as client:
        installed = (await client.get("/api/webchat/v2/extensions")).json()
        assert isinstance(installed.get("extensions"), list), installed

        registry = (await client.get("/api/webchat/v2/extensions/registry")).json()
        entries = registry.get("entries")
        assert entries, "the embedded registry catalog must expose entries"
        by_id = {e["package_ref"]["id"]: e for e in entries}
        # The first-party / bundled catalog ships these reference extensions.
        assert "web-access" in by_id, by_id.keys()
        sample = by_id["web-access"]
        for field in ("display_name", "kind", "description", "installed", "version"):
            assert field in sample, (field, sample)
        assert sample["installed"] is False


async def test_reborn_v2_extension_setup_projection(reborn_v2_server):
    """The per-extension setup route projects a lifecycle phase and payload."""
    async with _client(reborn_v2_server) as client:
        response = await client.get("/api/webchat/v2/extensions/github/setup")
        response.raise_for_status()
        body = response.json()
        assert body["package_ref"] == {"kind": "extension", "id": "github"}, body
        assert body.get("phase"), f"setup projection must carry a lifecycle phase: {body}"
        assert "payload" in body, body


async def test_reborn_v2_extension_install_activate_remove(reborn_v2_server):
    """Install -> list -> activate -> remove lifecycle for a first-party extension.

    `web-access` is first-party (no artifact download, no credentials), so the
    lifecycle is deterministic offline.
    """
    package_ref = {"kind": "extension", "id": "web-access"}
    async with _client(reborn_v2_server) as client:
        try:
            installed = await client.post(
                "/api/webchat/v2/extensions/install", json={"package_ref": package_ref}
            )
            assert installed.status_code == 200, installed.text
            assert installed.json().get("success") is True, installed.text

            listed = (await client.get("/api/webchat/v2/extensions")).json()
            ids = {e["package_ref"]["id"] for e in listed["extensions"]}
            assert "web-access" in ids, f"installed extension must appear in the list: {ids}"

            activated = await client.post(
                "/api/webchat/v2/extensions/web-access/activate"
            )
            assert activated.status_code == 200, activated.text
            assert activated.json().get("success") is True, activated.text
        finally:
            removed = await client.post("/api/webchat/v2/extensions/web-access/remove")
            assert removed.status_code == 200, removed.text

        after = (await client.get("/api/webchat/v2/extensions")).json()
        remaining = {e["package_ref"]["id"] for e in after["extensions"]}
        assert "web-access" not in remaining, "removed extension must drop out of the list"


async def test_reborn_v2_skills_list_projects_metadata(reborn_v2_server):
    """The skills list projects bundled skills with their selection metadata.

    (Fetching a *system* skill's content by name is intentionally not exposed —
    GET /skills/{name} serves installed/workspace skills; that path is covered by
    `test_reborn_v2_skill_install_get_remove`.)
    """
    async with _client(reborn_v2_server) as client:
        listing = (await client.get("/api/webchat/v2/skills")).json()
        skills = listing.get("skills")
        assert skills, "bundled skills must be listed"
        for field in ("name", "description", "version", "trust", "source"):
            assert field in skills[0], (field, skills[0])


async def test_reborn_v2_skills_search_projection(reborn_v2_server):
    """The skill search route returns the catalog/installed/registry projection."""
    async with _client(reborn_v2_server) as client:
        response = await client.post(
            "/api/webchat/v2/skills/search", json={"query": "research"}
        )
        response.raise_for_status()
        body = response.json()
        # ClawHub may be unconfigured in local-dev (empty catalog); the shape
        # must still be present and well-formed.
        assert isinstance(body.get("catalog"), list), body
        assert isinstance(body.get("installed"), list), body
        assert "registry_url" in body, body


async def test_reborn_v2_skill_install_get_remove(reborn_v2_server):
    """A skill install -> get -> remove round-trip persists and clears local content."""
    name = "e2e-management-probe-skill"
    async with _client(reborn_v2_server) as client:
        try:
            installed = await client.post(
                "/api/webchat/v2/skills/install",
                json={"name": name, "content": "# Probe\n\nManagement-surface E2E probe.\n"},
            )
            assert installed.status_code == 200, installed.text
            assert installed.json().get("success") is True, installed.text

            fetched = await client.get(f"/api/webchat/v2/skills/{name}")
            assert fetched.status_code == 200, fetched.text
            assert fetched.json()["name"] == name, fetched.text
        finally:
            removed = await client.request("DELETE", f"/api/webchat/v2/skills/{name}")
            assert removed.status_code == 200, removed.text

        gone = await client.get(f"/api/webchat/v2/skills/{name}")
        assert gone.status_code == 404, gone.text


async def test_reborn_v2_llm_provider_config_round_trip(reborn_v2_server, mock_llm_server):
    """Upsert a mock provider, set it active, probe it, list its models, delete it."""
    mock_base = f"{mock_llm_server}/v1"
    async with _client(reborn_v2_server) as client:
        # The bundled provider catalog ships the builtins and the config-selected
        # provider is active.
        snapshot = (await client.get("/api/webchat/v2/llm/providers")).json()
        provider_ids = {p["id"] for p in snapshot["providers"]}
        assert {"openai", "anthropic", "nearai"} <= provider_ids, provider_ids
        assert snapshot["active"]["provider_id"], snapshot["active"]

        try:
            upsert = await client.post(
                "/api/webchat/v2/llm/providers",
                json={
                    "id": "e2e-mock-provider",
                    "name": "E2E Mock",
                    "adapter": _OPENAI_ADAPTER,
                    "base_url": mock_base,
                    "default_model": "mock-model",
                    "api_key": "mock-api-key",
                    "set_active": True,
                    "model": "mock-model",
                },
            )
            assert upsert.status_code == 200, upsert.text
            after_upsert = upsert.json()
            assert after_upsert["active"]["provider_id"] == "e2e-mock-provider", after_upsert
            assert "e2e-mock-provider" in {p["id"] for p in after_upsert["providers"]}

            probe = await client.post(
                "/api/webchat/v2/llm/test-connection",
                json={
                    "adapter": _OPENAI_ADAPTER,
                    "base_url": mock_base,
                    "provider_id": "e2e-mock-provider",
                    "model": "mock-model",
                },
            )
            assert probe.status_code == 200, probe.text
            assert probe.json()["ok"] is True, probe.text

            models = await client.post(
                "/api/webchat/v2/llm/list-models",
                json={
                    "adapter": _OPENAI_ADAPTER,
                    "base_url": mock_base,
                    "provider_id": "e2e-mock-provider",
                    "model": "mock-model",
                },
            )
            assert models.status_code == 200, models.text
            models_body = models.json()
            assert models_body["ok"] is True, models_body
            assert "mock-model" in models_body["models"], models_body

            reactivate = await client.post(
                "/api/webchat/v2/llm/active",
                json={"provider_id": "openai", "model": "mock-model"},
            )
            assert reactivate.status_code == 200, reactivate.text
            assert reactivate.json()["active"]["provider_id"] == "openai", reactivate.text
        finally:
            deleted = await client.post(
                "/api/webchat/v2/llm/providers/e2e-mock-provider/delete"
            )
            assert deleted.status_code == 200, deleted.text

        final = (await client.get("/api/webchat/v2/llm/providers")).json()
        assert "e2e-mock-provider" not in {p["id"] for p in final["providers"]}, final


async def test_reborn_v2_automations_and_connectable_channels(reborn_v2_server):
    """The automations and connectable-channels read projections return their shapes."""
    async with _client(reborn_v2_server) as client:
        automations = (await client.get("/api/webchat/v2/automations")).json()
        assert isinstance(automations.get("automations"), list), automations

        channels = (await client.get("/api/webchat/v2/channels/connectable")).json()
        assert isinstance(channels.get("channels"), list), channels
