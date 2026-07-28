"""Legacy settings search coverage ported to Reborn WebChat v2."""

import json
from urllib.parse import urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


MOCK_TOOL_ENTRIES = [
    {
        "key": "agent.auto_approve_tools",
        "value": False,
        "mutable": True,
        "source": "default",
    },
    {
        "key": "tool.echo",
        "value": {
            "name": "echo",
            "description": "Echo text back for deterministic tests.",
            "state": "ask_each_time",
            "default_state": "ask_each_time",
            "effective_source": "default",
        },
        "mutable": True,
        "source": "default",
    },
    {
        "key": "tool.search_web",
        "value": {
            "name": "search_web",
            "description": "Search the web for current answers.",
            "state": "always_allow",
            "default_state": "ask_each_time",
            "effective_source": "override",
        },
        "mutable": True,
        "source": "override",
    },
]

MOCK_SKILLS = [
    {
        "name": "markdown-helper",
        "description": "Formats markdown for deterministic tests.",
        "version": "1.0.0",
        "trust": "Installed",
        "source_kind": "installed",
        "keywords": ["markdown"],
        "usage_hint": "Use for markdown formatting.",
        "can_edit": True,
        "can_delete": True,
        "auto_activate": True,
    },
    {
        "name": "workspace-helper",
        "description": "Reads workspace context.",
        "version": "1.0.0",
        "trust": "Trusted",
        "source_kind": "workspace",
        "keywords": ["workspace"],
        "can_edit": False,
        "can_delete": False,
    },
]

CHANNEL_SURFACES = [
    {
        "kind": "channel",
        "channel": "telegram",
        "direction": "bidirectional",
        "connection": {
            "status": "connected",
            "strategy": "oauth",
            "action": {
                "kind": "open_setup",
                "submit_label": "Reconnect",
            },
        },
    }
]

MOCK_CHANNEL_EXTENSION = {
    "package_ref": {"kind": "extension", "id": "telegram-channel"},
    "display_name": "Telegram Channel",
    "runtime": "first_party",
    "description": "Configured messaging channel.",
    "tools": [],
    "installation_state": "active",
    "surfaces": [{"kind": "channel", "inbound": True, "outbound": True}],
}

MOCK_MCP_EXTENSION = {
    "package_ref": {"kind": "extension", "id": "beta-mcp"},
    "display_name": "Beta MCP",
    "runtime": "mcp",
    "description": "Installed MCP server.",
    "tools": [],
    "installation_state": "setup_needed",
    "surfaces": [{"kind": "tool"}],
}


async def _open_mocked_settings_page(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    tab: str,
    llm_state: dict | None = None,
    llm_requests: list[dict] | None = None,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    browser_messages: list[str] = []
    page.on(
        "console",
        lambda message: browser_messages.append(f"{message.type}: {message.text}"),
    )
    page.on("pageerror", lambda error: browser_messages.append(f"pageerror: {error}"))

    async def fulfill_json(route, payload, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(payload),
            headers={"Cache-Control": "no-store"},
        )

    async def handle_session(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/session" and request.method == "GET":
            await fulfill_json(
                route,
                {
                    "tenant_id": "reborn-v2-e2e",
                    "user_id": "reborn-v2-e2e-user",
                    "capabilities": {"operator_webui_config": True},
                    "features": {"reborn_projects": False},
                },
            )
            return

        await route.continue_()

    async def handle_settings_tools(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/settings/tools" and request.method == "GET":
            await fulfill_json(route, {"entries": MOCK_TOOL_ENTRIES})
            return

        await route.continue_()

    async def handle_skills(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/skills" and request.method == "GET":
            await fulfill_json(
                route,
                {
                    "skills": MOCK_SKILLS,
                    "count": len(MOCK_SKILLS),
                    "auto_activate_learned": True,
                },
            )
            return

        await route.continue_()

    async def handle_extensions(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/extensions" and request.method == "GET":
            await fulfill_json(
                route,
                {"extensions": [MOCK_CHANNEL_EXTENSION, MOCK_MCP_EXTENSION]},
            )
            return

        if path == "/api/webchat/v2/extensions/registry" and request.method == "GET":
            await fulfill_json(route, {"entries": []})
            return

        await route.continue_()

    async def handle_llm(route):
        request = route.request
        path = urlparse(request.url).path
        method = request.method

        if llm_state is None:
            await route.continue_()
            return

        def record(kind: str, payload: dict | None = None) -> None:
            if llm_requests is not None:
                llm_requests.append({"kind": kind, "payload": payload or {}})

        def request_json() -> dict:
            raw = request.post_data or "{}"
            return json.loads(raw)

        def provider_from_payload(payload: dict) -> dict:
            existing = next(
                (
                    provider
                    for provider in llm_state["providers"]
                    if provider["id"] == payload["id"]
                ),
                {},
            )
            return {
                **existing,
                "id": payload["id"],
                "description": payload.get("name") or payload["id"],
                "adapter": payload.get("adapter") or existing.get("adapter"),
                "base_url": payload.get("base_url", existing.get("base_url", "")),
                "default_model": payload.get(
                    "default_model", existing.get("default_model", "")
                ),
                "builtin": False,
                "api_key_set": bool(payload.get("api_key"))
                or existing.get("api_key_set", False),
                "api_key_required": payload.get("adapter") != "ollama",
                "base_url_required": True,
                "accepts_api_key": payload.get("adapter") != "ollama",
            }

        if path == "/api/webchat/v2/llm/providers" and method == "GET":
            await fulfill_json(
                route,
                {
                    "providers": llm_state["providers"],
                    "active": llm_state.get("active"),
                },
            )
            return

        if path == "/api/webchat/v2/llm/providers" and method == "POST":
            payload = request_json()
            record("upsert", payload)
            provider = provider_from_payload(payload)
            llm_state["providers"] = [
                item for item in llm_state["providers"] if item["id"] != provider["id"]
            ] + [provider]
            if payload.get("set_active"):
                llm_state["active"] = {
                    "provider_id": provider["id"],
                    "model": payload.get("model") or provider.get("default_model"),
                }
            await fulfill_json(route, {"provider": provider})
            return

        if path == "/api/webchat/v2/llm/active" and method == "POST":
            payload = request_json()
            record("active", payload)
            llm_state["active"] = {
                "provider_id": payload["provider_id"],
                "model": payload["model"],
            }
            await fulfill_json(route, {"active": llm_state["active"]})
            return

        if path == "/api/webchat/v2/llm/list-models" and method == "POST":
            payload = request_json()
            record("list_models", payload)
            await fulfill_json(
                route,
                {
                    "ok": True,
                    "models": ["acme-fast", "acme-pro"],
                    "message": "models listed",
                },
            )
            return

        if path == "/api/webchat/v2/llm/test-connection" and method == "POST":
            payload = request_json()
            record("test_connection", payload)
            await fulfill_json(
                route,
                {"ok": True, "message": f"probe ok for {payload.get('model')}"},
            )
            return

        if path.startswith("/api/webchat/v2/llm/providers/") and path.endswith(
            "/delete"
        ) and method == "POST":
            provider_id = (
                path.removeprefix("/api/webchat/v2/llm/providers/")
                .removesuffix("/delete")
            )
            record("delete", {"provider_id": provider_id})
            llm_state["providers"] = [
                provider
                for provider in llm_state["providers"]
                if provider["id"] != provider_id
            ]
            await fulfill_json(route, {"success": True})
            return

        await route.continue_()

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/settings/tools**", handle_settings_tools)
    await page.route("**/api/webchat/v2/skills**", handle_skills)
    await page.route("**/api/webchat/v2/extensions**", handle_extensions)
    await page.route("**/api/webchat/v2/llm/**", handle_llm)

    await page.goto(f"{reborn_v2_server}/settings/{tab}?token={REBORN_V2_AUTH_TOKEN}")
    search = page.get_by_placeholder(SEL_V2["settings_search_placeholder"])
    try:
        await expect(search).to_be_visible(timeout=15000)
    except AssertionError as error:
        body_text = await page.locator("body").inner_text(timeout=1000)
        raise AssertionError(
            f"Settings search toolbar did not render on {page.url}.\n"
            f"Browser messages: {browser_messages}\n"
            f"Body text:\n{body_text}"
        ) from error

    return {"context": context, "page": page, "search": search}


def _provider_card(page, provider_id: str):
    return page.locator(
        SEL_V2["llm_provider_card_for"].format(provider_id=provider_id)
    )


async def _choose_select_menu_option(root, label: str, option: str) -> None:
    trigger = root.get_by_role("button", name=label)
    await trigger.click()
    listbox = root.get_by_role("listbox")
    await expect(listbox).to_be_visible(timeout=5000)
    await listbox.get_by_role("option", name=option).click()
    await expect(listbox).to_have_count(0)
    await expect(trigger).to_contain_text(option)


def _mock_llm_state() -> dict:
    return {
        "active": {"provider_id": "openai", "model": "gpt-4.1-mini"},
        "providers": [
            {
                "id": "openai",
                "description": "OpenAI API",
                "adapter": "open_ai_completions",
                "base_url": "https://api.openai.test/v1",
                "default_model": "gpt-4.1-mini",
                "builtin": True,
                "api_key_set": True,
                "api_key_required": True,
                "base_url_required": False,
                "accepts_api_key": True,
            },
            {
                "id": "anthropic",
                "description": "Anthropic API",
                "adapter": "anthropic",
                "base_url": "",
                "default_model": "claude-3-5-sonnet",
                "builtin": True,
                "api_key_set": False,
                "api_key_required": True,
                "base_url_required": False,
                "accepts_api_key": True,
            },
            {
                "id": "legacy-local",
                "description": "Legacy Local",
                "adapter": "open_ai_completions",
                "base_url": "http://localhost:11434/v1",
                "default_model": "legacy-model",
                "builtin": False,
                "api_key_set": True,
                "api_key_required": True,
                "base_url_required": True,
                "accepts_api_key": True,
            },
        ],
    }


async def test_reborn_legacy_settings_tools_search_and_clear(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_settings_page(
        reborn_v2_server,
        reborn_v2_browser,
        tab="tools",
    )
    try:
        page = harness["page"]
        search = harness["search"]

        await expect(page.get_by_text("echo", exact=True)).to_be_visible(timeout=5000)
        await expect(page.get_by_text("search_web", exact=True)).to_be_visible(timeout=5000)

        await search.fill("echo")
        await expect(page.get_by_text("echo", exact=True)).to_be_visible()
        await expect(page.get_by_text("search_web", exact=True)).to_have_count(0)
        await expect(page.get_by_text("1 / 2")).to_be_visible()

        await page.get_by_role("button", name="Clear search").click()
        await expect(search).to_have_value("")
        await expect(page.get_by_text("search_web", exact=True)).to_be_visible()

        await search.fill("missing-tool")
        await expect(page.get_by_text("No tools match the filter.")).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_settings_skills_search_empty_state(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_settings_page(
        reborn_v2_server,
        reborn_v2_browser,
        tab="skills",
    )
    try:
        page = harness["page"]
        search = harness["search"]

        await expect(page.get_by_text("markdown-helper", exact=True)).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("workspace-helper", exact=True)).to_be_visible(
            timeout=5000
        )

        await search.fill("workspace")
        await expect(page.get_by_text("workspace-helper", exact=True)).to_be_visible()
        await expect(page.get_by_text("markdown-helper", exact=True)).to_have_count(0)

        await search.fill("no-such-skill")
        await expect(page.get_by_text('No settings match "no-such-skill"')).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_settings_channels_search(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_settings_page(
        reborn_v2_server,
        reborn_v2_browser,
        tab="channels",
    )
    try:
        page = harness["page"]
        search = harness["search"]

        await expect(page.get_by_text("Telegram Channel", exact=True)).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("Beta MCP", exact=True)).to_have_count(0)

        await search.fill("telegram")
        await expect(page.get_by_text("Telegram Channel", exact=True)).to_be_visible()
        await expect(page.get_by_text("Beta MCP", exact=True)).to_have_count(0)

        await search.fill("nothing-matches-this")
        await expect(
            page.get_by_text('No settings match "nothing-matches-this"')
        ).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_settings_inference_add_test_and_activate_provider(
    reborn_v2_server, reborn_v2_browser
):
    llm_state = _mock_llm_state()
    llm_requests: list[dict] = []
    harness = await _open_mocked_settings_page(
        reborn_v2_server,
        reborn_v2_browser,
        tab="inference",
        llm_state=llm_state,
        llm_requests=llm_requests,
    )
    try:
        page = harness["page"]

        await expect(page.get_by_text("LLM provider", exact=True)).to_be_visible(
            timeout=5000
        )
        await expect(
            page.get_by_text("gpt-4.1-mini", exact=True).first
        ).to_be_visible()
        await expect(_provider_card(page, "openai")).to_be_visible(timeout=5000)
        await expect(_provider_card(page, "legacy-local")).to_be_visible(timeout=5000)

        await page.get_by_role("button", name="Add provider").click()
        dialog = page.get_by_role("dialog")
        await expect(dialog.get_by_role("heading", name="New provider")).to_be_visible()

        await expect(dialog.get_by_role("combobox")).to_have_count(0)
        await _choose_select_menu_option(dialog, "Adapter", "Anthropic")
        await _choose_select_menu_option(dialog, "Adapter", "OpenAI Compatible")

        await dialog.get_by_label("Display name").fill("Acme LLM")
        await expect(dialog.get_by_label("Provider ID")).to_have_value("acme-llm")
        await dialog.get_by_label("Base URL").fill("https://llm.acme.test/v1")
        await dialog.get_by_label("API key").fill("acme-secret")
        await dialog.get_by_label("Default model").fill("stale-model")

        await dialog.get_by_role("button", name="Fetch models").click()
        await expect(dialog.get_by_text("2 models found.")).to_be_visible(timeout=5000)
        await _choose_select_menu_option(dialog, "Default model", "acme-pro")

        await dialog.get_by_role("button", name="Test connection").click()
        await expect(dialog.get_by_text("probe ok for acme-pro")).to_be_visible(
            timeout=5000
        )

        await dialog.get_by_role("button", name="Save").click()
        await expect(page.get_by_text('Added provider "Acme LLM".')).to_be_visible(
            timeout=5000
        )

        acme_card = _provider_card(page, "acme-llm")
        await expect(acme_card).to_be_visible(timeout=5000)
        await acme_card.get_by_role("button", name="Use").click()
        await expect(page.get_by_text("Switched to Acme LLM.")).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("acme-llm", exact=True).first).to_be_visible()
        await expect(page.get_by_text("acme-pro", exact=True).first).to_be_visible()

        assert {
            "kind": "list_models",
            "payload": {
                "adapter": "open_ai_completions",
                "base_url": "https://llm.acme.test/v1",
                "provider_id": "acme-llm",
                "provider_type": "custom",
                "model": "stale-model",
                "api_key": "acme-secret",
            },
        } in llm_requests
        assert {
            "kind": "test_connection",
            "payload": {
                "adapter": "open_ai_completions",
                "base_url": "https://llm.acme.test/v1",
                "provider_id": "acme-llm",
                "provider_type": "custom",
                "model": "acme-pro",
                "api_key": "acme-secret",
            },
        } in llm_requests
        assert any(
            request["kind"] == "upsert"
            and request["payload"]["id"] == "acme-llm"
            and request["payload"]["default_model"] == "acme-pro"
            and request["payload"]["api_key"] == "acme-secret"
            for request in llm_requests
        )
        assert {
            "kind": "active",
            "payload": {"provider_id": "acme-llm", "model": "acme-pro"},
        } in llm_requests
    finally:
        await harness["context"].close()


async def test_reborn_legacy_settings_inference_edit_and_delete_custom_provider(
    reborn_v2_server, reborn_v2_browser
):
    llm_state = _mock_llm_state()
    llm_requests: list[dict] = []
    harness = await _open_mocked_settings_page(
        reborn_v2_server,
        reborn_v2_browser,
        tab="inference",
        llm_state=llm_state,
        llm_requests=llm_requests,
    )
    try:
        page = harness["page"]
        legacy_card = _provider_card(page, "legacy-local")
        await expect(legacy_card).to_be_visible(timeout=5000)

        await legacy_card.get_by_test_id(SEL_V2["llm_provider_disclosure"]).click()
        await legacy_card.get_by_role("button", name="Edit").click()
        dialog = page.get_by_role("dialog")
        await expect(
            dialog.get_by_role("heading", name="Edit provider")
        ).to_be_visible()

        await dialog.get_by_label("Base URL").fill("http://127.0.0.1:11435/v1")
        await dialog.get_by_label("Default model").fill("legacy-v2")
        await dialog.get_by_role("button", name="Save").click()
        await expect(
            page.get_by_text('Updated provider "Legacy Local".')
        ).to_be_visible(timeout=5000)
        await expect(legacy_card.get_by_text("legacy-v2", exact=True)).to_be_visible(
            timeout=5000
        )

        edit_request = next(
            request
            for request in llm_requests
            if request["kind"] == "upsert"
            and request["payload"].get("id") == "legacy-local"
        )
        assert edit_request["payload"]["base_url"] == "http://127.0.0.1:11435/v1"
        assert edit_request["payload"]["default_model"] == "legacy-v2"
        assert "api_key" not in edit_request["payload"]

        page.once("dialog", lambda browser_dialog: browser_dialog.accept())
        await legacy_card.get_by_role("button", name="Delete").click()
        await expect(page.get_by_text("Provider deleted.")).to_be_visible(timeout=5000)
        await expect(_provider_card(page, "legacy-local")).to_have_count(0)
        assert {
            "kind": "delete",
            "payload": {"provider_id": "legacy-local"},
        } in llm_requests
    finally:
        await harness["context"].close()
