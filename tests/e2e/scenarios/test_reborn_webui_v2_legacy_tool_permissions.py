"""Legacy tool-permission coverage ported to Reborn WebChat v2."""

import asyncio
import json
from pathlib import Path
from urllib.parse import quote, unquote, urlparse

import httpx
import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    client_action_id,
    create_thread,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_restartable_server,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
    reborn_bearer_headers,
    send_message,
    wait_for_assistant_message,
)


def _tool_entry(
    name: str,
    *,
    state: str,
    default_state: str = "ask_each_time",
    source: str = "override",
    mutable: bool = True,
    description: str | None = None,
) -> dict:
    return {
        "key": f"tool.{name}",
        "value": {
            "name": name,
            "description": description or f"{name} deterministic test tool.",
            "state": state,
            "default_state": default_state,
            "locked": not mutable,
            "effective_source": source,
        },
        "mutable": mutable,
        "source": source,
    }


async def _open_mocked_tools_page(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    fail_permission_saves: bool = False,
    delay_permission_saves: bool = False,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    auto_approve = {"enabled": False}
    tool_states = {
        "echo": {
            "state": "always_allow",
            "default_state": "ask_each_time",
            "source": "override",
            "mutable": True,
            "description": "Echo text back.",
        },
        "tool.financial": {
            "state": "ask_each_time",
            "default_state": "ask_each_time",
            "source": "locked",
            "mutable": False,
            "description": "Hard-floor approval tool.",
        },
    }
    auto_approve_requests: list[dict] = []
    permission_requests: list[dict] = []
    permission_save_started = asyncio.Event()
    permission_save_release = asyncio.Event()

    def entries():
        return [
            {
                "key": "agent.auto_approve_tools",
                "value": auto_approve["enabled"],
                "mutable": True,
                "source": "override" if auto_approve["enabled"] else "default",
            },
            *[
                _tool_entry(
                    name,
                    state=data["state"],
                    default_state=data["default_state"],
                    source=data["source"],
                    mutable=data["mutable"],
                    description=data["description"],
                )
                for name, data in tool_states.items()
            ],
        ]

    async def fulfill_json(route, payload, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(payload),
            headers={"Cache-Control": "no-store"},
        )

    async def handle_settings_tools(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/settings/tools" and request.method == "GET":
            await fulfill_json(route, {"entries": entries()})
            return

        if path == "/api/webchat/v2/settings/tools" and request.method == "POST":
            body = json.loads(request.post_data or "{}")
            auto_approve_requests.append(body)
            auto_approve["enabled"] = bool(body.get("enabled"))
            await fulfill_json(
                route,
                {
                    "entry": {
                        "key": "agent.auto_approve_tools",
                        "value": auto_approve["enabled"],
                        "mutable": True,
                        "source": "override",
                    }
                },
            )
            return

        if (
            path.startswith("/api/webchat/v2/settings/tools/")
            and request.method == "POST"
        ):
            name = unquote(path.removeprefix("/api/webchat/v2/settings/tools/"))
            body = json.loads(request.post_data or "{}")
            permission_requests.append({"name": name, "body": body})
            permission_save_started.set()
            if delay_permission_saves:
                await permission_save_release.wait()
            if fail_permission_saves:
                await fulfill_json(
                    route,
                    {"kind": "permission_denied", "error": "permission_denied"},
                    status=403,
                )
                return

            tool = tool_states[name]
            if not tool["mutable"]:
                await fulfill_json(
                    route,
                    {"kind": "bad_request", "message": "locked tool"},
                    status=400,
                )
                return

            requested = body.get("state") or "default"
            if requested == "default":
                tool["state"] = tool["default_state"]
                tool["source"] = "default"
            else:
                tool["state"] = requested
                tool["source"] = "override"

            await fulfill_json(
                route,
                {
                    "entry": _tool_entry(
                        name,
                        state=tool["state"],
                        default_state=tool["default_state"],
                        source=tool["source"],
                        mutable=tool["mutable"],
                        description=tool["description"],
                    )
                },
            )
            return

        await route.continue_()

    await page.route("**/api/webchat/v2/settings/tools**", handle_settings_tools)
    await page.goto(f"{reborn_v2_server}/settings/tools?token={REBORN_V2_AUTH_TOKEN}")
    await expect(
        page.get_by_placeholder(SEL_V2["settings_search_placeholder"])
    ).to_be_visible(timeout=15000)
    await expect(page.get_by_text("Tool permissions")).to_be_visible(timeout=5000)

    return {
        "context": context,
        "page": page,
        "auto_approve_requests": auto_approve_requests,
        "permission_requests": permission_requests,
        "permission_save_started": permission_save_started,
        "permission_save_release": permission_save_release,
    }


def _tool_row(page, name: str):
    return page.locator(SEL_V2["settings_tool_row_for"].format(name=name))


def _permission_button(page, name: str):
    return _tool_row(page, name).locator(SEL_V2["settings_tool_permission"])


async def _choose_permission(page, name: str, label: str):
    button = _permission_button(page, name)
    await button.click()
    listbox_id = await button.get_attribute("aria-controls")
    assert listbox_id
    listbox = page.locator(f"#{listbox_id}")
    await expect(listbox).to_be_visible(timeout=5000)
    await listbox.get_by_role("option", name=label).click()
    return button


@pytest.fixture
def reborn_approval_artifact_cleanup():
    yield
    for label in ("first", "second"):
        Path(f"reborn-approval-{label}.txt").unlink(missing_ok=True)


async def _set_real_auto_approve(reborn_v2_server: str, enabled: bool):
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        response = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/settings/tools",
            json={"enabled": enabled},
            timeout=15,
        )
        response.raise_for_status()
        return response.json()


async def _get_real_tool_state(
    client: httpx.AsyncClient, base_url: str, capability_id: str
) -> dict:
    response = await client.get(
        f"{base_url}/api/webchat/v2/settings/tools",
        timeout=15,
    )
    response.raise_for_status()
    for entry in response.json().get("entries", []):
        if entry.get("key") == f"tool.{capability_id}":
            return entry
    raise AssertionError(f"{capability_id} missing from Tools settings")


async def _wait_for_gate_prompt_after_send(
    base_url: str, thread_id: str, content: str
) -> dict:
    url = (
        f"{base_url}/api/webchat/v2/threads/{thread_id}/events"
        f"?token={REBORN_V2_AUTH_TOKEN}"
    )
    timeout = httpx.Timeout(60.0, read=60.0)
    async with httpx.AsyncClient(timeout=timeout) as stream_client:
        async with stream_client.stream("GET", url) as response:
            response.raise_for_status()
            async with httpx.AsyncClient(headers=reborn_bearer_headers()) as action_client:
                await send_message(action_client, base_url, thread_id, content)

            event_name = None
            data_lines: list[str] = []
            async with asyncio.timeout(45):
                async for line in response.aiter_lines():
                    if line.startswith(":"):
                        continue
                    if line.startswith("event:"):
                        event_name = line.removeprefix("event:").strip()
                        continue
                    if line.startswith("data:"):
                        data_lines.append(line.removeprefix("data:").lstrip())
                        continue
                    if line == "":
                        if event_name == "gate" and data_lines:
                            frame = json.loads("\n".join(data_lines))
                            return frame["prompt"]
                        event_name = None
                        data_lines = []
    raise AssertionError("SSE stream closed before a gate prompt arrived")


async def test_reborn_legacy_tool_permissions_tab_visible(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        await expect(page.get_by_text("Always allow eligible tools")).to_be_visible()
        await expect(_tool_row(page, "echo")).to_be_visible(timeout=5000)
        await expect(_tool_row(page, "tool.financial")).to_be_visible(timeout=5000)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_tool_permission_menu_persists_after_reload(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        row = _tool_row(page, "echo")
        await expect(row.locator("select")).to_have_count(0)

        button = _permission_button(page, "echo")
        await expect(button).to_contain_text("Always allow", timeout=5000)

        await _choose_permission(page, "echo", "Ask each time")
        await expect(button).to_contain_text("Ask each time")
        await expect(_tool_row(page, "echo").get_by_text("saved")).to_be_visible(timeout=5000)
        assert harness["permission_requests"][-1] == {
            "name": "echo",
            "body": {"state": "ask_each_time"},
        }

        await page.reload()
        await expect(
            page.get_by_placeholder(SEL_V2["settings_search_placeholder"])
        ).to_be_visible(timeout=15000)
        await expect(_permission_button(page, "echo")).to_contain_text(
            "Ask each time",
            timeout=5000,
        )

        await _choose_permission(page, "echo", "Follow global")
        await expect(_permission_button(page, "echo")).to_contain_text("Follow global")
        assert harness["permission_requests"][-1] == {
            "name": "echo",
            "body": {"state": "default"},
        }
    finally:
        await harness["context"].close()


async def test_reborn_legacy_tool_permission_retains_selection_while_saving(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(
        reborn_v2_server,
        reborn_v2_browser,
        delay_permission_saves=True,
    )
    try:
        page = harness["page"]
        button = _permission_button(page, "echo")
        await expect(button).to_contain_text("Always allow", timeout=5000)

        await _choose_permission(page, "echo", "Ask each time")
        await asyncio.wait_for(harness["permission_save_started"].wait(), timeout=5)

        select = _tool_row(page, "echo").locator(
            SEL_V2["settings_tool_permission_select"]
        )
        await expect(select).to_have_attribute("aria-busy", "true")
        await expect(button).to_contain_text("Ask each time")
        assert harness["permission_requests"][-1] == {
            "name": "echo",
            "body": {"state": "ask_each_time"},
        }

        harness["permission_save_release"].set()
        await expect(_tool_row(page, "echo").get_by_text("saved")).to_be_visible(
            timeout=5000
        )
    finally:
        harness["permission_save_release"].set()
        await harness["context"].close()


async def test_reborn_legacy_tool_permission_save_failure_shows_error(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(
        reborn_v2_server,
        reborn_v2_browser,
        fail_permission_saves=True,
    )
    try:
        page = harness["page"]
        button = _permission_button(page, "echo")
        await expect(button).to_contain_text("Always allow", timeout=5000)

        await _choose_permission(page, "echo", "Ask each time")

        await expect(page.get_by_role("alert")).to_contain_text(
            "Save failed: Permission denied",
            timeout=5000,
        )
        await expect(button).to_contain_text("Always allow", timeout=5000)
        assert harness["permission_requests"][-1] == {
            "name": "echo",
            "body": {"state": "ask_each_time"},
        }
    finally:
        await harness["context"].close()


async def test_reborn_legacy_locked_tool_shows_badge_without_select(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        locked = _tool_row(page, "tool.financial")
        await expect(locked).to_be_visible(timeout=5000)
        await expect(locked.locator(SEL_V2["settings_tool_lock"])).to_be_visible()
        await expect(locked.locator(SEL_V2["settings_tool_permission"])).to_have_count(0)
        await expect(locked.locator("select")).to_have_count(0)
        await expect(locked.get_by_text("Ask each time")).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_auto_approve_switch_persists(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        switch = page.get_by_role("switch", name="Always allow eligible tools")
        await expect(switch).to_have_attribute("aria-checked", "false")
        await switch.click()
        await expect(switch).to_have_attribute("aria-checked", "true")
        assert harness["auto_approve_requests"] == [{"enabled": True}]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_auto_approve_real_api_persists_across_browser_contexts(
    reborn_v2_server,
    reborn_v2_browser,
):
    await _set_real_auto_approve(reborn_v2_server, False)
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    try:
        update = await _set_real_auto_approve(reborn_v2_server, True)
        assert update["entry"]["key"] == "agent.auto_approve_tools"
        assert update["entry"]["value"] is True

        await page.goto(
            f"{reborn_v2_server}/settings/tools?token={REBORN_V2_AUTH_TOKEN}"
        )
        await expect(
            page.get_by_placeholder(SEL_V2["settings_search_placeholder"])
        ).to_be_visible(timeout=15000)
        switch = page.get_by_role("switch", name="Always allow eligible tools")
        await expect(switch).to_have_attribute("aria-checked", "true", timeout=5000)
    finally:
        await context.close()
        await _set_real_auto_approve(reborn_v2_server, False)


async def test_reborn_legacy_tool_permission_real_api_persists_and_rejects_locked(
    reborn_v2_server,
):
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        response = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/settings/tools",
            timeout=15,
        )
        response.raise_for_status()
        entries = response.json().get("entries", [])

        tools = [entry for entry in entries if entry.get("key", "").startswith("tool.")]
        mutable = next((entry for entry in tools if entry.get("mutable") is not False), None)
        locked = next((entry for entry in tools if entry.get("mutable") is False), None)

        if mutable is None:
            pytest.skip("Reborn test catalog has no mutable operator tool")

        capability_id = mutable["key"].removeprefix("tool.")
        update = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/settings/tools/{capability_id}",
            json={"state": "disabled"},
            timeout=15,
        )
        update.raise_for_status()
        assert update.json()["entry"]["value"]["state"] == "disabled"
        assert update.json()["entry"]["mutable"] is True

        reset = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/settings/tools/{capability_id}",
            json={"state": "default"},
            timeout=15,
        )
        reset.raise_for_status()
        assert reset.json()["entry"]["key"] == mutable["key"]

        if locked is not None:
            locked_id = locked["key"].removeprefix("tool.")
            rejected = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/settings/tools/{locked_id}",
                json={"state": "always_allow"},
                timeout=15,
            )
            assert rejected.status_code >= 400


async def test_reborn_legacy_always_approve_survives_reborn_restart(
    reborn_v2_restartable_server,
    reborn_approval_artifact_cleanup,
):
    state, start_server, stop_server = reborn_v2_restartable_server
    capability_id = "builtin.write_file"

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        base_url = state["base_url"]
        reset = await client.post(
            f"{base_url}/api/webchat/v2/settings/tools/{capability_id}",
            json={"state": "default"},
            timeout=15,
        )
        reset.raise_for_status()
        thread_id = await create_thread(client, base_url)

    first_prompt = await _wait_for_gate_prompt_after_send(
        state["base_url"],
        thread_id,
        "reborn write approval file first",
    )
    assert first_prompt["allow_always"] is True
    assert first_prompt["approval_context"]["tool_name"] == capability_id

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        gate_ref = quote(first_prompt["gate_ref"], safe="")
        resolve = await client.post(
            (
                f"{state['base_url']}/api/webchat/v2/threads/{thread_id}/runs/"
                f"{first_prompt['turn_run_id']}/gates/{gate_ref}/resolve"
            ),
            json={
                "client_action_id": client_action_id(),
                "resolution": "approved",
                "always": True,
            },
            timeout=15,
        )
        resolve.raise_for_status()
        await wait_for_assistant_message(client, state["base_url"], thread_id)

        persisted = await _get_real_tool_state(client, state["base_url"], capability_id)
        assert persisted["value"]["state"] == "always_allow"

    await stop_server()
    restarted_url = await start_server()

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        restarted = await _get_real_tool_state(client, restarted_url, capability_id)
        assert restarted["value"]["state"] == "always_allow"

        second_thread_id = await create_thread(client, restarted_url)
        await send_message(
            client,
            restarted_url,
            second_thread_id,
            "reborn write approval file second",
        )
        await wait_for_assistant_message(client, restarted_url, second_thread_id)
