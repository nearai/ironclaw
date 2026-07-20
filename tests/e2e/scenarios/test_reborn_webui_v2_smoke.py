"""Dedicated Reborn WebChat v2 smoke E2E.

This proves the *new* Reborn surface end-to-end: the `ironclaw-reborn serve`
binary boots, serves the React SPA
at `/`, authenticates a bearer caller, and runs one text turn through the
`/api/webchat/v2/*` endpoints against the deterministic mock LLM.

This is intentionally small and complements the Rust composition tests
(`crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs`), which drive the
same router in-process via `tower::ServiceExt::oneshot` with no real TCP
listener or browser. It also differs from `test_reborn_gateway_smoke.py`, which
exercises the legacy `ironclaw` web channel (`/api/chat/*`) under ENGINE_V2 —
NOT the `ironclaw-reborn` binary or the v2 webUI.

Wiring confirmed manually before this test existed:
- The v2 SPA + `serve` subcommand are compiled in unconditionally; the binary
  is `ironclaw-reborn`.
- LLM is selected via `$IRONCLAW_REBORN_HOME/config.toml` `[llm.default]`; the
  built-in `openai` provider (OpenAI `/v1/chat/completions`) is pointed at the
  mock with a `base_url` override and `api_key_env`.
- `IRONCLAW_REBORN_WEBUI_TOKEN` must be >= 32 bytes (it doubles as the SSO
  session-signing key); the user id maps the env-bearer caller.
- `NO_PROXY`/`no_proxy` must cover loopback so the provider's reqwest client
  does not route the mock request through a developer-local HTTP proxy.
"""

import asyncio
import json
import re
import uuid
from urllib.parse import parse_qs, urlparse

import aiohttp
import httpx
from playwright.async_api import expect
from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    create_thread as _create_thread,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
    send_and_settle as _send_and_settle,
    send_message as _send_message,
    wait_for_assistant_message as _wait_for_assistant_message,
)


def _relative_luminance(rgb: list[float]) -> float:
    channels = [
        value / 12.92 if value <= 0.04045 else ((value + 0.055) / 1.055) ** 2.4
        for value in (channel / 255 for channel in rgb)
    ]
    return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]


def _contrast_ratio(foreground: list[float], background: list[float]) -> float:
    foreground_luminance = _relative_luminance(foreground)
    background_luminance = _relative_luminance(background)
    lighter = max(foreground_luminance, background_luminance)
    darker = min(foreground_luminance, background_luminance)
    return (lighter + 0.05) / (darker + 0.05)


async def _effective_colors(locator) -> dict[str, list[float]]:
    return await locator.evaluate(
        """element => {
          const parse = (value) => {
            const channels = value.match(/[\\d.]+/g)?.map(Number) || [];
            const scale = value.trim().startsWith("color(srgb ") ? 255 : 1;
            return [(channels[0] || 0) * scale, (channels[1] || 0) * scale,
              (channels[2] || 0) * scale,
              channels.length > 3 ? channels[3] : 1];
          };
          const over = (front, back) => {
            const alpha = front[3] + back[3] * (1 - front[3]);
            if (alpha === 0) return [0, 0, 0, 0];
            return [
              (front[0] * front[3] + back[0] * back[3] * (1 - front[3])) / alpha,
              (front[1] * front[3] + back[1] * back[3] * (1 - front[3])) / alpha,
              (front[2] * front[3] + back[2] * back[3] * (1 - front[3])) / alpha,
              alpha,
            ];
          };

          const foreground = parse(getComputedStyle(element).color);
          let background = [0, 0, 0, 0];
          for (let node = element; node && background[3] < 1; node = node.parentElement) {
            background = over(background, parse(getComputedStyle(node).backgroundColor));
          }
          background = over(background, [255, 255, 255, 1]);
          return {
            foreground: foreground.slice(0, 3),
            background: background.slice(0, 3),
          };
        }"""
    )


async def _assert_readable(locator, label: str) -> dict[str, list[float]]:
    colors = await _effective_colors(locator)
    ratio = _contrast_ratio(colors["foreground"], colors["background"])
    assert ratio >= 4.5, f"{label} contrast was {ratio:.2f}:1 with colors {colors}"
    return colors


async def _wait_for_automation_named(
    client: httpx.AsyncClient,
    base_url: str,
    name: str,
    *,
    timeout: float = 30.0,
) -> dict:
    last_body: dict = {}
    try:
        async with asyncio.timeout(timeout):
            while True:
                response = await client.get(
                    f"{base_url}/api/webchat/v2/automations",
                    timeout=5,
                )
                response.raise_for_status()
                last_body = response.json()
                for automation in last_body.get("automations", []):
                    if automation.get("name") == name:
                        return automation
                await asyncio.sleep(0.5)
    except TimeoutError:
        raise AssertionError(
            f"Timed out waiting for automation {name!r}. Last body: {last_body}"
        ) from None


async def _install_fake_v2_event_source(page) -> None:
    await page.add_init_script(
        """
        (() => {
          let activeStream = null;
          const currentStream = () => {
            if (!activeStream || activeStream.readyState === 2) {
              throw new Error("no EventSource stream is open");
            }
            return activeStream;
          };
          class FakeEventSource extends EventTarget {
            constructor(url) {
              super();
              this.url = url;
              this.readyState = 0;
              if (activeStream && activeStream.readyState !== 2) {
                activeStream.close();
              }
              activeStream = this;
              setTimeout(() => {
                if (activeStream !== this || this.readyState === 2) return;
                this.readyState = 1;
                if (typeof this.onopen === "function") this.onopen(new Event("open"));
              }, 0);
            }
            close() {
              this.readyState = 2;
              if (activeStream === this) activeStream = null;
            }
          }
          window.EventSource = FakeEventSource;
          window.__emitV2Sse = (type, frame, id = crypto.randomUUID()) => {
            const stream = currentStream();
            const event = new MessageEvent(type, {
              data: JSON.stringify({ type, ...frame }),
              lastEventId: id,
            });
            stream.dispatchEvent(event);
          };
          window.__failLatestV2Sse = (readyState = 2) => {
            const stream = currentStream();
            stream.readyState = readyState;
            if (readyState === 2 && activeStream === stream) activeStream = null;
            if (typeof stream.onerror !== "function") {
              throw new Error("EventSource has no error handler");
            }
            stream.onerror(new Event("error"));
          };
        })();
        """
    )


async def test_reborn_v2_serves_shell_and_gates_auth(reborn_v2_server, reborn_v2_browser):
    """The root-mounted SPA renders the authed shell and anonymous login view."""
    # With a valid token the authenticated chat shell renders.
    authed_ctx = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    authed_page = await authed_ctx.new_page()
    try:
        await authed_page.goto(f"{reborn_v2_server}/?token={REBORN_V2_AUTH_TOKEN}")
        await expect(authed_page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
        await authed_page.wait_for_url(re.compile(r".*/chat(?:[?#].*)?$"), timeout=15000)
        assert urlparse(authed_page.url).path == "/chat"
    finally:
        await authed_ctx.close()

    # Without a token the SPA falls back to the login/connect view.
    anon_ctx = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    anon_page = await anon_ctx.new_page()
    try:
        await anon_page.goto(f"{reborn_v2_server}/")
        await expect(anon_page.locator(SEL_V2["login_token"])).to_be_visible(timeout=15000)
        await anon_page.wait_for_url(re.compile(r".*/login(?:[?#].*)?$"), timeout=15000)
        assert urlparse(anon_page.url).path == "/login"
    finally:
        await anon_ctx.close()


async def test_reborn_v2_legacy_paths_redirect_to_root(
    reborn_v2_server, reborn_v2_browser
):
    """Legacy `/v2` bookmarks redirect to canonical root paths without losing query data."""
    async with httpx.AsyncClient(follow_redirects=False) as client:
        for source, target in [
            ("/v2", "/"),
            ("/v2/", "/"),
            ("/v2?login_ticket=ticket%2B1", "/?login_ticket=ticket%2B1"),
            (
                "/v2/settings/skills?token=old%2Btoken&tab=installed",
                "/settings/skills?token=old%2Btoken&tab=installed",
            ),
        ]:
            response = await client.get(f"{reborn_v2_server}{source}")
            assert response.status_code == 307, source
            assert response.headers.get("location") == target, source

    # Follow a real legacy deep link in Chromium. The token shim removes only
    # the credential query parameter; unrelated query data and the deep route
    # must survive the server redirect and React Router bootstrap.
    context = await reborn_v2_browser.new_context(
        viewport={"width": 1280, "height": 720}
    )
    page = await context.new_page()
    try:
        await page.goto(
            f"{reborn_v2_server}/v2/settings/skills"
            f"?token={REBORN_V2_AUTH_TOKEN}&source=compat"
        )
        toggle = page.get_by_role(
            "button", name=re.compile(r"^Default: (On|Off)$")
        ).first
        await expect(toggle).to_be_visible(timeout=15000)
        parsed = urlparse(page.url)
        assert parsed.path == "/settings/skills"
        assert parse_qs(parsed.query) == {"source": ["compat"]}
    finally:
        await context.close()


async def test_reborn_v2_light_theme_semantic_colors_have_readable_contrast(
    reborn_v2_page,
):
    """Theme-aware controls, success states, and secondary text meet WCAG AA."""
    await reborn_v2_page.evaluate(
        """() => {
          localStorage.setItem("ironclaw:v2-theme", "light");
          document.documentElement.dataset.theme = "light";
        }"""
    )
    await reborn_v2_page.reload()
    await expect(reborn_v2_page.locator(SEL_V2["chat_composer"])).to_be_visible(
        timeout=15000
    )
    assert await reborn_v2_page.locator("html").get_attribute("data-theme") == "light"

    # A slow turn exposes the real danger Button used to cancel an active run.
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("editable composer slow response")
    await composer.press("Enter")
    user_message = reborn_v2_page.locator(SEL_V2["msg_user"]).last
    await expect(user_message).to_contain_text("editable composer slow response", timeout=15000)
    cancel_button = reborn_v2_page.get_by_role("button", name="Cancel").first
    await expect(cancel_button).to_be_visible(timeout=10000)
    await _assert_readable(cancel_button, "light-theme danger button")

    # The message timestamp previously used undefined text-iron-500 and had no
    # emitted color rule. Its replacement must remain readable on the canvas.
    await user_message.hover()
    timestamp = user_message.locator("time")
    await expect(timestamp).to_be_visible()
    await _assert_readable(timestamp, "light-theme secondary timestamp")
    await cancel_button.click()
    await expect(cancel_button).to_have_count(0, timeout=15000)

    origin = await reborn_v2_page.evaluate("location.origin")
    await reborn_v2_page.goto(
        f"{origin}/extensions/registry?token={REBORN_V2_AUTH_TOKEN}"
    )
    install_button = reborn_v2_page.get_by_role("button", name="Install").first
    await expect(install_button).to_be_visible(timeout=15000)
    idle_colors = await _assert_readable(install_button, "light-theme outline button")

    await install_button.hover()
    hover_colors = await _assert_readable(
        install_button, "light-theme outline button on hover"
    )
    assert hover_colors["background"] != idle_colors["background"], (
        "outline button hover state did not change its effective background"
    )

    await reborn_v2_page.mouse.down()
    try:
        pressed_colors = await _assert_readable(
            install_button, "light-theme outline button while pressed"
        )
        assert pressed_colors["background"] != hover_colors["background"], (
            "outline button pressed state did not change its effective background"
        )
    finally:
        await reborn_v2_page.mouse.move(0, 0)
        await reborn_v2_page.mouse.up()

    # The same semantic outline token must remain readable after switching the
    # browser to dark mode; then restore light mode for the success-state check.
    await reborn_v2_page.evaluate(
        "document.documentElement.dataset.theme = 'dark'"
    )
    await _assert_readable(install_button, "dark-theme outline button")
    await reborn_v2_page.evaluate(
        "document.documentElement.dataset.theme = 'light'"
    )

    await reborn_v2_page.goto(
        f"{origin}/settings/skills?token={REBORN_V2_AUTH_TOKEN}"
    )
    toggle_name = re.compile(r"^Default: (On|Off)$")
    toggle = reborn_v2_page.get_by_role("button", name=toggle_name).first
    await expect(toggle).to_be_visible(timeout=15000)
    original_label = await toggle.inner_text()
    restore_label = "Default: Off" if original_label == "Default: On" else "Default: On"
    await toggle.click()
    try:
        restore_toggle = reborn_v2_page.get_by_role("button", name=restore_label).first
        await expect(restore_toggle).to_be_visible(timeout=15000)
        success_banner = reborn_v2_page.locator(SEL_V2["skill_action_result"])
        await expect(success_banner).to_be_visible(timeout=15000)
        await _assert_readable(success_banner, "light-theme success banner")
    finally:
        restore_toggle = reborn_v2_page.get_by_role("button", name=restore_label).first
        if await restore_toggle.count():
            await restore_toggle.click()
            await expect(
                reborn_v2_page.get_by_role("button", name=original_label).first
            ).to_be_visible(timeout=15000)


async def test_reborn_v2_appearance_theme_selection_persists(reborn_v2_page):
    """Appearance controls update the live theme and preserve it across reloads."""
    origin = await reborn_v2_page.evaluate("location.origin")
    await reborn_v2_page.goto(
        f"{origin}/v2/settings/appearance?token={REBORN_V2_AUTH_TOKEN}"
    )

    light_option = reborn_v2_page.locator(SEL_V2["appearance_theme_light"])
    dark_option = reborn_v2_page.locator(SEL_V2["appearance_theme_dark"])
    await expect(light_option).to_be_visible(timeout=15000)
    await expect(dark_option).to_be_visible(timeout=15000)

    await dark_option.click()
    await expect(dark_option).to_be_checked()
    await expect(reborn_v2_page.locator("html")).to_have_attribute(
        "data-theme", "dark"
    )
    await reborn_v2_page.wait_for_function(
        'localStorage.getItem("ironclaw:v2-theme") === "dark"'
    )

    await reborn_v2_page.reload()
    dark_option = reborn_v2_page.locator(SEL_V2["appearance_theme_dark"])
    await expect(dark_option).to_be_checked(timeout=15000)
    await expect(reborn_v2_page.locator("html")).to_have_attribute(
        "data-theme", "dark"
    )

    # Native radios provide the expected arrow-key selection and roving focus.
    await dark_option.press("ArrowLeft")
    light_option = reborn_v2_page.locator(SEL_V2["appearance_theme_light"])
    await expect(light_option).to_be_checked()
    await expect(reborn_v2_page.locator("html")).to_have_attribute(
        "data-theme", "light"
    )
    await reborn_v2_page.wait_for_function(
        'localStorage.getItem("ironclaw:v2-theme") === "light"'
    )

    await reborn_v2_page.reload()
    light_option = reborn_v2_page.locator(SEL_V2["appearance_theme_light"])
    await expect(light_option).to_be_checked(timeout=15000)
    await expect(reborn_v2_page.locator("html")).to_have_attribute(
        "data-theme", "light"
    )


async def test_reborn_v2_settings_import_rejects_unsupported_payloads(
    reborn_v2_page,
):
    """Unsupported imports show one localized error and do not refresh settings."""
    settings_reads = 0

    def count_settings_reads(request) -> None:
        nonlocal settings_reads
        if (
            request.method == "GET"
            and urlparse(request.url).path == "/api/webchat/v2/settings/tools"
        ):
            settings_reads += 1

    reborn_v2_page.on("request", count_settings_reads)
    await reborn_v2_page.keyboard.press("Control+K")
    command_palette = reborn_v2_page.get_by_role(
        "dialog", name=SEL_V2["command_palette_dialog_name"]
    )
    await expect(command_palette).to_be_visible()
    await command_palette.get_by_role(
        "button", name=SEL_V2["command_palette_go_settings_name"]
    ).click()
    await reborn_v2_page.wait_for_url(
        re.compile(r".*/settings(?:[?#].*)?$")
    )
    file_input = reborn_v2_page.locator(SEL_V2["settings_import_file"])
    await expect(file_input).to_have_count(1, timeout=15000)
    await reborn_v2_page.wait_for_timeout(250)
    initial_settings_reads = settings_reads

    for filename, settings in [
        ("empty-settings.json", {}),
        ("unsupported-settings.json", {"agent.model": "example-model"}),
    ]:
        await file_input.set_input_files(
            {
                "name": filename,
                "mimeType": "application/json",
                "buffer": json.dumps({"settings": settings}).encode(),
            }
        )
        status = reborn_v2_page.get_by_role("status").filter(
            has_text="No supported settings found in the selected file"
        )
        await expect(status).to_have_count(1)
        await expect(status).to_have_text(
            "No supported settings found in the selected file"
        )
        await expect(
            reborn_v2_page.get_by_text("Settings imported", exact=True)
        ).to_have_count(0)
        await expect(
            reborn_v2_page.get_by_text(re.compile(r"^Import failed:"))
        ).to_have_count(0)

    await reborn_v2_page.wait_for_timeout(250)
    assert settings_reads == initial_settings_reads, (
        "failed settings imports unexpectedly invalidated the settings query"
    )


async def test_reborn_v2_text_turn_persists(reborn_v2_server):
    """A text turn over /api/webchat/v2/* completes and persists one assistant reply."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

        prompt = "what is 2+2?"
        await _send_message(client, reborn_v2_server, thread_id, prompt)
        assistant = await _wait_for_assistant_message(client, reborn_v2_server, thread_id)
        assert "4" in assistant.get("content", "")

        # Exactly one finalized assistant message — no duplicate terminal response.
        timeline = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline",
            timeout=15,
        )
        timeline.raise_for_status()
        finalized = [
            message
            for message in timeline.json().get("messages", [])
            if message.get("kind") == "assistant"
            and message.get("status") == "finalized"
            and (message.get("content") or "").strip()
        ]
        assert len(finalized) == 1, (
            f"Expected one finalized assistant message, got {len(finalized)}: {finalized}"
        )


async def test_reborn_v2_ui_enter_submits_initial_and_follow_up_messages(
    reborn_v2_page,
):
    """Enter submits both an initial message and a follow-up after success."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("hello there")
    await composer.press("Enter")

    # The user bubble and the streamed assistant reply both render in the shell.
    user_messages = reborn_v2_page.locator(SEL_V2["msg_user"])
    assistant_messages = reborn_v2_page.locator(SEL_V2["msg_assistant"])
    await expect(user_messages.first).to_contain_text(
        "hello there", timeout=15000
    )
    await expect(assistant_messages.first).to_contain_text(
        "Hello", timeout=30000
    )
    await expect(composer).to_have_attribute("data-send-disabled", "false", timeout=15000)

    await composer.fill("follow-up right away")
    await composer.press("Enter")

    await expect(user_messages).to_have_count(2, timeout=15000)
    await expect(user_messages.last).to_contain_text("follow-up right away")
    await expect(assistant_messages).to_have_count(2, timeout=30000)
    await expect(assistant_messages.last).to_contain_text("I understand your request.")


async def test_reborn_v2_automation_rename_persists_from_ui(
    reborn_v2_server, reborn_v2_browser
):
    """Creating an automation through chat can be renamed from /automations."""
    label = f"ui-{uuid.uuid4().hex[:8]}"
    original_name = f"E2E rename original {label}"
    renamed_name = f"E2E rename updated {label}"
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}

    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)
        await _send_message(
            client,
            reborn_v2_server,
            thread_id,
            f"reborn create automation rename target {label}",
        )
        await _wait_for_assistant_message(client, reborn_v2_server, thread_id)
        automation = await _wait_for_automation_named(
            client, reborn_v2_server, original_name
        )
        automation_id = automation["automation_id"]

    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await page.goto(f"{reborn_v2_server}/automations?token={REBORN_V2_AUTH_TOKEN}")
        row_selector = SEL_V2["automation_row_for"].format(id=automation_id)
        row = page.locator(row_selector)
        await expect(row).to_be_visible(timeout=15000)
        await row.locator(
            SEL_V2["automation_name_button_for"].format(id=automation_id)
        ).click()

        await expect(page.locator(SEL_V2["automation_detail"])).to_be_visible(
            timeout=15000
        )
        await expect(page.locator(SEL_V2["automation_detail_title"])).to_contain_text(
            original_name
        )

        await page.locator(SEL_V2["automation_rename_button"]).click()
        rename_input = page.locator(SEL_V2["automation_rename_input"])
        await expect(rename_input).to_have_value(original_name)
        await rename_input.fill(f"  {renamed_name}  ")
        await page.locator(SEL_V2["automation_rename_save"]).click()

        await expect(page.locator(SEL_V2["automation_detail_title"])).to_contain_text(
            renamed_name,
            timeout=15000,
        )
        await expect(row).to_contain_text(renamed_name)

        await page.reload()
        row = page.locator(row_selector)
        await expect(row).to_contain_text(renamed_name, timeout=15000)
    finally:
        await context.close()

    async with httpx.AsyncClient(headers=headers) as client:
        renamed = await _wait_for_automation_named(client, reborn_v2_server, renamed_name)
        assert renamed["automation_id"] == automation_id


async def test_reborn_v2_automation_action_error_toast_is_safe_dismissible_and_cleared_on_retry(
    reborn_v2_server, reborn_v2_page
):
    """Automation mutation toasts stay visible, private, and clear on retry."""
    automation_id = "11111111-2222-3333-4444-555555555555"
    automation_name = "Safe action error regression"
    raw_error = "postgres failed: secret_internal_automation_table"
    attempt_count = 0
    mutation_requests: list[tuple[str, str]] = []
    console_messages: list[str] = []
    retry_started = asyncio.Event()
    release_retry = asyncio.Event()
    retry_completed = asyncio.Event()

    page = reborn_v2_page
    page.on("console", lambda message: console_messages.append(message.text))

    async def handle_automations(route) -> None:
        nonlocal attempt_count
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    {
                        "scheduler_enabled": True,
                        "automations": [
                            {
                                "automation_id": automation_id,
                                "name": automation_name,
                                "source": {
                                    "type": "schedule",
                                    "cron": "0 9 * * *",
                                    "timezone": "UTC",
                                },
                                "state": "active",
                                "next_run_at": "2026-07-18T09:00:00Z",
                                "recent_runs": [],
                            }
                        ],
                    }
                ),
            )
            return

        mutation_requests.append(
            (route.request.method, urlparse(route.request.url).path)
        )
        attempt_count += 1
        if attempt_count <= 2:
            await route.fulfill(
                status=500,
                content_type="text/plain",
                body=raw_error,
            )
            return

        retry_started.set()
        await release_retry.wait()
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"updated": True}),
        )
        retry_completed.set()

    await page.route("**/api/webchat/v2/automations**", handle_automations)
    row_selector = SEL_V2["automation_row_for"].format(id=automation_id)
    error_toast = page.locator(SEL_V2["toast"]).filter(
        has_text="Unable to update the automation. Please try again."
    )

    async def submit_rename(name: str) -> None:
        row = page.locator(row_selector)
        await expect(row).to_be_visible(timeout=15000)
        await row.locator(
            SEL_V2["automation_name_button_for"].format(id=automation_id)
        ).click()
        await page.locator(SEL_V2["automation_rename_button"]).click()
        rename_input = page.locator(SEL_V2["automation_rename_input"])
        await rename_input.fill(name)
        await page.locator(SEL_V2["automation_rename_save"]).click()

    try:
        await page.goto(f"{reborn_v2_server}/automations?token={REBORN_V2_AUTH_TOKEN}")

        await submit_rename("First failed rename")
        await expect(error_toast).to_be_visible(timeout=10000)
        await expect(error_toast).to_have_text(
            "Unable to update the automation. Please try again."
        )
        await expect(error_toast).not_to_contain_text(raw_error)
        assert not any(raw_error in message for message in console_messages)
        await error_toast.get_by_role("button", name="Dismiss").click()
        await expect(error_toast).to_have_count(0, timeout=3000)

        pause_button = page.get_by_role(
            "button", name=f"Pause: {automation_name}", exact=True
        )
        await pause_button.click()
        await expect(error_toast).to_be_visible(timeout=10000)
        assert not any(raw_error in message for message in console_messages)

        await pause_button.click()
        await asyncio.wait_for(retry_started.wait(), timeout=10)
        await expect(error_toast).to_have_count(0)

        release_retry.set()
        await asyncio.wait_for(retry_completed.wait(), timeout=10)
        await expect(error_toast).to_have_count(0)
        assert not any(raw_error in message for message in console_messages)
        assert mutation_requests == [
            (
                "POST",
                f"/api/webchat/v2/automations/{automation_id}",
            ),
            (
                "POST",
                f"/api/webchat/v2/automations/{automation_id}/pause",
            ),
            (
                "POST",
                f"/api/webchat/v2/automations/{automation_id}/pause",
            ),
        ]
    finally:
        release_retry.set()


async def test_reborn_v2_automation_failed_run_actions_are_clickable(
    reborn_v2_server, reborn_v2_browser
):
    """Failed automation runs expose working Open run and scoped Logs actions."""
    automation_id = "11111111-2222-3333-4444-555555555555"
    thread_id = "thread-failed-automation"
    run_id = "22222222-3333-4444-5555-666666666666"
    requested_log_queries: list[dict[str, list[str]]] = []
    logs_requested = asyncio.Event()

    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    async def fulfill_json(route, body, status=200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def handle_session(route) -> None:
        await fulfill_json(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": USER_ID,
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def handle_automations(route) -> None:
        await fulfill_json(
            route,
            {
                "scheduler_enabled": True,
                "automations": [
                    {
                        "automation_id": automation_id,
                        "name": "Failed run action regression",
                        "source": {
                            "type": "schedule",
                            "cron": "0 9 * * *",
                            "timezone": "UTC",
                        },
                        "state": "active",
                        "next_run_at": "2026-07-10T09:00:00Z",
                        "recent_runs": [
                            {
                                "status": "error",
                                "fire_slot": "2026-07-09T09:00:00Z",
                                "submitted_at": "2026-07-09T09:00:01Z",
                                "completed_at": "2026-07-09T09:00:42Z",
                                "thread_id": thread_id,
                                "run_id": run_id,
                            }
                        ],
                    }
                ],
            },
        )

    async def handle_threads(route) -> None:
        await fulfill_json(
            route,
            {
                "threads": [
                    {
                        "thread_id": thread_id,
                        "title": "Failed automation thread",
                        "created_at": "2026-07-09T09:00:01Z",
                        "updated_at": "2026-07-09T09:00:42Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route) -> None:
        await fulfill_json(route, {"messages": [], "next_cursor": None})

    async def handle_logs(route) -> None:
        parsed = urlparse(route.request.url)
        requested_log_queries.append(parse_qs(parsed.query))
        logs_requested.set()
        await fulfill_json(
            route,
            {
                "logs": {
                    "source": "in_memory_tracing",
                    "entries": [
                        {
                            "id": "automation-failed-log",
                            "timestamp": "2026-07-09T09:00:42Z",
                            "level": "error",
                            "target": "ironclaw::automation",
                            "message": "failed automation run log",
                            "thread_id": thread_id,
                            "run_id": run_id,
                        }
                    ],
                    "next_cursor": None,
                    "tail_supported": True,
                    "follow_supported": False,
                },
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/automations**", handle_automations)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/timeline**", handle_timeline)
    await page.route("**/api/webchat/v2/logs**", handle_logs)
    row_selector = SEL_V2["automation_row_for"].format(id=automation_id)

    async def select_automation() -> None:
        row = page.locator(row_selector)
        await expect(row).to_be_visible(timeout=15000)
        await row.locator(
            SEL_V2["automation_name_button_for"].format(id=automation_id)
        ).click()
        await expect(page.locator(SEL_V2["automation_detail"])).to_be_visible(
            timeout=15000
        )

    try:
        await page.goto(f"{reborn_v2_server}/automations?token={REBORN_V2_AUTH_TOKEN}")
        await select_automation()

        open_run = page.locator(SEL_V2["automation_run_open"]).first
        logs = page.locator(SEL_V2["automation_run_logs"]).first
        await expect(open_run).to_be_enabled()
        await expect(logs).to_be_enabled()

        await open_run.click()
        await page.wait_for_url(f"**/chat/{thread_id}", timeout=10000)

        await page.goto(f"{reborn_v2_server}/automations?token={REBORN_V2_AUTH_TOKEN}")
        await select_automation()
        await page.locator(SEL_V2["automation_run_logs"]).first.click()
        await asyncio.wait_for(logs_requested.wait(), timeout=10)

        assert "/logs" in page.url
        first_query = requested_log_queries[0]
        assert first_query.get("thread_id") == [thread_id], first_query
        assert first_query.get("run_id") == [run_id], first_query
    finally:
        await context.close()


async def test_reborn_v2_composer_accepts_draft_while_run_is_processing(reborn_v2_page):
    """The composer stays editable while the current assistant run is still active."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("editable composer slow response")
    await composer.press("Enter")

    await expect(reborn_v2_page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "editable composer slow response", timeout=15000
    )
    await expect(
        reborn_v2_page.locator(SEL_V2["typing_indicator"])
    ).to_be_visible(timeout=15000)

    await expect(composer).to_be_enabled()
    await composer.fill("draft while the reply is still running")
    await expect(composer).to_have_value("draft while the reply is still running")

    await composer.press("Enter")
    await expect(reborn_v2_page.locator(SEL_V2["msg_user"])).to_have_count(1, timeout=1000)


async def test_reborn_v2_disconnected_run_shows_status_and_stops_typing(
    reborn_v2_server, reborn_v2_browser
) -> None:
    """A disconnected active run shows transport status and stops spinning."""
    thread_id = "thread-disconnected-run"
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await _install_fake_v2_event_source(page)

    async def fulfill_json(route, body, status=200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def handle_session(route) -> None:
        await fulfill_json(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": USER_ID,
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def handle_threads(route) -> None:
        await fulfill_json(
            route,
            {
                "threads": [
                    {
                        "thread_id": thread_id,
                        "title": "Disconnected run regression",
                        "created_at": "2026-06-02T00:00:00Z",
                        "updated_at": "2026-06-02T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route) -> None:
        await fulfill_json(route, {"messages": [], "next_cursor": None})

    async def handle_send(route) -> None:
        await fulfill_json(
            route,
            {
                "thread_id": thread_id,
                "run_id": "run-disconnected",
                "status": "running",
            },
            status=202,
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/timeline**", handle_timeline)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/messages", handle_send)

    try:
        await page.goto(f"{reborn_v2_server}/chat/{thread_id}?token={REBORN_V2_AUTH_TOKEN}")
        composer = page.locator(SEL_V2["chat_composer"])
        await expect(composer).to_be_visible(timeout=15000)
        connection_status = page.locator(SEL_V2["connection_status"])

        await context.set_offline(True)
        await expect(connection_status).to_have_text("Reconnecting...", timeout=5000)
        await expect(connection_status).to_have_css("position", "static")
        assert await connection_status.evaluate("node => Boolean(node.closest('header'))")
        await expect(connection_status).to_be_in_viewport()

        await page.set_viewport_size({"width": 390, "height": 844})
        connection_status_toggle = page.locator(SEL_V2["connection_status_toggle"])
        connection_status_label = page.locator(SEL_V2["connection_status_label"])
        disclosure_id = await connection_status_label.get_attribute("id")
        assert disclosure_id
        await expect(connection_status_label).to_be_hidden()
        await expect(connection_status_label).to_have_attribute("aria-hidden", "true")
        await expect(connection_status_toggle).to_have_attribute("aria-expanded", "false")
        await expect(connection_status_toggle).to_have_attribute("aria-controls", disclosure_id)
        await expect(connection_status_toggle).to_be_in_viewport()

        await connection_status_toggle.click()
        await expect(connection_status_toggle).to_have_attribute("aria-expanded", "true")
        await expect(connection_status_label).to_have_attribute("aria-hidden", "false")
        await expect(connection_status_label).to_be_visible()
        await expect(connection_status_label).to_have_text("Reconnecting...")
        await expect(connection_status_label).to_have_css("position", "absolute")
        await expect(connection_status_toggle).to_be_in_viewport()
        await expect(connection_status_label).to_be_in_viewport()
        await expect(page.locator(SEL_V2["header_logs_link"])).to_be_visible()
        await expect(page.locator(SEL_V2["header_docs_link"])).to_be_visible()

        await page.set_viewport_size({"width": 1280, "height": 720})
        await context.set_offline(False)
        await expect(connection_status).to_have_count(0, timeout=5000)

        await composer.fill("summarize 3 X/Twitter posts")
        await composer.press("Enter")
        await expect(page.locator(SEL_V2["typing_indicator"])).to_be_visible(timeout=5000)

        await page.evaluate("() => window.__failLatestV2Sse(0)")
        await expect(connection_status).to_have_text("Reconnecting...", timeout=5000)

        await page.evaluate("() => window.__failLatestV2Sse(2)")
        await expect(connection_status).to_have_text("Disconnected", timeout=5000)

        await expect(page.locator(SEL_V2["typing_indicator"])).to_have_count(0, timeout=5000)
        await expect(page.locator(SEL_V2["msg_error"]).last).to_contain_text(
            "Connection to the server was lost. Please reconnect and try again.",
            timeout=5000,
        )
    finally:
        await context.close()


async def test_reborn_v2_approval_gate_blocks_composer_send(
    reborn_v2_server, reborn_v2_browser
):
    """An open approval gate shows the warning and blocks new sends locally."""
    thread_id = "thread-approval-blocked"
    send_requests: list[dict] = []
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await _install_fake_v2_event_source(page)

    async def fulfill_json(route, body, status=200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def handle_session(route) -> None:
        await fulfill_json(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": USER_ID,
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def handle_threads(route) -> None:
        await fulfill_json(
            route,
            {
                "threads": [
                    {
                        "thread_id": thread_id,
                        "title": "Approval blocked regression",
                        "created_at": "2026-06-25T00:00:00Z",
                        "updated_at": "2026-06-25T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route) -> None:
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "seed-user",
                        "kind": "user",
                        "content": "trigger approval",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-25T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_send(route) -> None:
        send_requests.append(json.loads(route.request.post_data or "{}"))
        await fulfill_json(route, {"thread_id": thread_id}, status=202)

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/timeline**", handle_timeline)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/messages", handle_send)

    try:
        await page.goto(f"{reborn_v2_server}/chat/{thread_id}?token={REBORN_V2_AUTH_TOKEN}")
        await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
        await expect(page.locator(SEL_V2["msg_user"]).first).to_contain_text(
            "trigger approval", timeout=15000
        )

        await page.evaluate(
            """
            () => window.__emitV2Sse("gate", {
              prompt: {
                turn_run_id: "run-gated",
                gate_ref: "gate-shell",
                invocation_id: "invoke-shell",
                headline: "Approval required",
                body: "Allow shell to inspect the workspace?",
                allow_always: false,
                approval_context: {
                  tool_name: "builtin.shell",
                  reason: "Allow shell to inspect the workspace?",
                  action: { label: "Run command" },
                  destination: { label: "Local workspace" },
                  details: [{ label: "Command", value: "pwd" }]
                }
              }
            })
            """
        )

        await expect(page.locator(SEL_V2["approval_card"]).first).to_be_visible(timeout=5000)
        await expect(
            page.get_by_text("Resolve the approval request before sending another message.")
        ).to_be_visible(timeout=5000)

        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("new message while approval is open")
        await composer.press("Enter")
        await expect(page.locator(SEL_V2["msg_user"])).to_have_count(1, timeout=1000)
        assert send_requests == []
    finally:
        await context.close()


async def test_reborn_v2_unscoped_activity_stays_with_previous_reply(
    reborn_v2_server, reborn_v2_browser
):
    """POST-seeded run ids keep delayed unscoped activity before its reply.

    This remains a browser E2E because the regression crosses the React-only
    seam from useChat's submit response into useChatEvents and MessageList DOM
    grouping; the Rust integration harness cannot observe that client boundary.
    """
    thread_id = "thread-unscoped-activity-order"
    run_id = "run-unscoped-activity-order"
    send_requests: list[dict] = []
    timeline_messages: list[dict] = []
    release_second_send = asyncio.Event()
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await _install_fake_v2_event_source(page)

    async def fulfill_json(route, body, status=200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def handle_session(route) -> None:
        await fulfill_json(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": USER_ID,
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def handle_threads(route) -> None:
        await fulfill_json(
            route,
            {
                "threads": [
                    {
                        "thread_id": thread_id,
                        "title": "Unscoped activity ordering regression",
                        "created_at": "2026-07-08T13:00:00Z",
                        "updated_at": "2026-07-08T13:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route) -> None:
        await fulfill_json(route, {"messages": timeline_messages, "next_cursor": None})

    async def handle_send(route) -> None:
        send_requests.append(json.loads(route.request.post_data or "{}"))
        if len(send_requests) == 1:
            await fulfill_json(
                route,
                {
                    "thread_id": thread_id,
                    "accepted_message_ref": "msg:first-user",
                    "run_id": run_id,
                    "status": "running",
                },
                status=202,
            )
            return

        await release_second_send.wait()
        await fulfill_json(
            route,
            {
                "thread_id": thread_id,
                "run_id": "run-follow-up",
                "status": "running",
            },
            status=202,
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/timeline**", handle_timeline)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/messages", handle_send)

    try:
        await page.goto(f"{reborn_v2_server}/chat/{thread_id}?token={REBORN_V2_AUTH_TOKEN}")
        composer = page.locator(SEL_V2["chat_composer"])
        await expect(composer).to_be_visible(timeout=15000)

        await composer.fill("connect my Google tools")
        await composer.press("Enter")
        await expect(page.locator(SEL_V2["msg_user"]).first).to_contain_text(
            "connect my Google tools", timeout=15000
        )

        timeline_messages[:] = [
            {
                "message_id": "first-user",
                "kind": "user",
                "content": "connect my Google tools",
                "sequence": 1,
                "status": "accepted",
                "created_at": "2026-07-08T13:00:00Z",
                "turn_run_id": run_id,
            },
            {
                "message_id": "first-assistant",
                "kind": "assistant",
                "content": "Gmail, Calendar, Drive, and Sheets are connected.",
                "sequence": 2,
                "status": "finalized",
                "created_at": "2026-07-08T13:00:10Z",
                "updated_at": "2026-07-08T13:00:10Z",
                "turn_run_id": run_id,
            },
        ]
        await page.evaluate(
            """
            (runId) => {
              window.__emitV2Sse("projection_update", {
                state: {
                  items: [
                    { run_status: { run_id: runId, status: "completed" } }
                  ]
                }
              }, "cursor-terminal");
              window.__emitV2Sse("final_reply", {
                reply: {
                  turn_run_id: runId,
                  text: "Gmail, Calendar, Drive, and Sheets are connected.",
                  generated_at: "2026-07-08T13:00:10Z"
                }
              }, "cursor-final");
            }
            """,
            run_id,
        )
        await expect(page.locator(SEL_V2["msg_assistant"]).first).to_contain_text(
            "Gmail, Calendar, Drive, and Sheets are connected.",
            timeout=5000,
        )

        await composer.fill("thanks")
        await composer.press("Enter")
        await expect(page.locator(SEL_V2["msg_user"])).to_have_count(2, timeout=5000)

        await page.evaluate(
            """
            () => window.__emitV2Sse("capability_activity", {
              activity: {
                invocation_id: "invocation-google-connect",
                capability_id: "builtin.extension_search",
                status: "completed",
                subtitle: "Google tools"
              }
            }, "cursor-delayed-activity")
            """
        )
        await expect(page.locator(SEL_V2["activity_run"]).first).to_be_visible(
            timeout=5000
        )

        order = await page.locator(SEL_V2["message_list_content"]).evaluate(
            """
            (node) => Array.from(node.children)
              .map((child) => {
                const marker = child.getAttribute("data-testid");
                if (marker === "msg-user") return "user";
                if (marker === "activity-run") return "activity";
                if (marker === "msg-assistant") return "assistant";
                return null;
              })
              .filter(Boolean)
            """
        )
        assert order == ["user", "activity", "assistant", "user"], order
    finally:
        release_second_send.set()
        await context.close()


async def test_reborn_v2_desktop_sidebar_can_collapse_and_persist(reborn_v2_page):
    """Desktop users can collapse the sidebar, and the preference survives reload."""
    sidebar = reborn_v2_page.locator(SEL_V2["sidebar"])
    toggle = reborn_v2_page.locator(SEL_V2["sidebar_toggle"])

    await expect(toggle).to_be_visible(timeout=15000)
    await expect(sidebar).to_be_visible(timeout=15000)

    await toggle.click()
    await expect(sidebar).to_be_hidden(timeout=5000)
    await reborn_v2_page.wait_for_function(
        "() => localStorage.getItem('ironclaw:v2-sidebar-open') === 'false'",
        timeout=5000,
    )

    await reborn_v2_page.reload()
    await expect(reborn_v2_page.locator(SEL_V2["chat_composer"])).to_be_visible(
        timeout=15000
    )
    await expect(sidebar).to_be_hidden(timeout=5000)

    await toggle.click()
    await expect(sidebar).to_be_visible(timeout=5000)
    await reborn_v2_page.wait_for_function(
        "() => localStorage.getItem('ironclaw:v2-sidebar-open') === 'true'",
        timeout=5000,
    )


async def test_reborn_v2_messages_omit_identity_labels(reborn_v2_page):
    """User and assistant messages render content without persistent identity labels."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("hello there")
    await composer.press("Enter")

    # Message bubbles retain content while omitting redundant identity labels.
    user_bubble = reborn_v2_page.locator(SEL_V2["msg_user"]).first
    await expect(user_bubble).to_contain_text("hello there", timeout=15000)
    await expect(user_bubble).not_to_contain_text("You")

    assistant_bubble = reborn_v2_page.locator(SEL_V2["msg_assistant"]).first
    await expect(assistant_bubble).to_contain_text("Hello", timeout=30000)
    await expect(assistant_bubble).not_to_contain_text("IronClaw")


async def test_reborn_v2_response_links_open_in_new_tab(reborn_v2_page):
    """Links inside an assistant reply open in a new tab."""
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])
    await composer.fill("link test")
    await composer.press("Enter")

    link = (
        reborn_v2_page.locator(SEL_V2["msg_assistant"])
        .get_by_role("link", name="the pull request")
    )
    await expect(link).to_be_visible(timeout=30000)
    assert await link.get_attribute("target") == "_blank", "link must open in a new tab"
    rel = await link.get_attribute("rel") or ""
    assert "noopener" in rel, f"link must be noopener, got rel={rel!r}"


async def test_reborn_v2_logs_page_passes_scope_to_api_and_renders_context(
    reborn_v2_page, reborn_v2_server
):
    """The browser logs route passes URL scope to the API and renders scoped entries."""
    requested_queries: list[dict[str, list[str]]] = []
    logs_requested = asyncio.Event()

    async def handle_operator_logs(route) -> None:
        parsed = urlparse(route.request.url)
        requested_queries.append(parse_qs(parsed.query))
        logs_requested.set()
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "status": "available",
                    "logs": {
                        "source": "in_memory_tracing",
                        "entries": [
                            {
                                "id": "ui-log-1",
                                "timestamp": "2026-06-12T10:11:12.123Z",
                                "level": "info",
                                "target": "ironclaw::ui::logs",
                                "message": "scoped log from browser fixture",
                                "thread_id": "thread-ui",
                                "run_id": "run-ui",
                                "tool_call_id": "tool-call-ui",
                                "tool_name": "shell",
                                "source": "slack",
                            }
                        ],
                        "next_cursor": None,
                        "tail_supported": True,
                        "follow_supported": False,
                    },
                }
            ),
        )

    await reborn_v2_page.route("**/api/webchat/v2/operator/logs**", handle_operator_logs)
    await reborn_v2_page.goto(
        f"{reborn_v2_server}/logs"
        "?thread_id=thread-ui&run_id=run-ui&tool_call_id=tool-call-ui&source=slack"
    )

    await asyncio.wait_for(logs_requested.wait(), timeout=10)
    first_query = requested_queries[0]
    assert first_query.get("thread_id") == ["thread-ui"], first_query
    assert first_query.get("run_id") == ["run-ui"], first_query
    assert first_query.get("tool_call_id") == ["tool-call-ui"], first_query
    assert first_query.get("source") == ["slack"], first_query
    assert first_query.get("limit") == ["500"], first_query

    await expect(
        reborn_v2_page.locator(SEL_V2["logs_scope_toolbar"])
    ).to_be_visible(timeout=10000)
    await expect(
        reborn_v2_page.locator(SEL_V2["logs_scope_chip"].format(key="thread_id"))
    ).to_contain_text("thread-ui")
    await expect(
        reborn_v2_page.locator(SEL_V2["logs_scope_chip"].format(key="run_id"))
    ).to_contain_text("run-ui")

    entry = reborn_v2_page.locator(SEL_V2["logs_entry"]).first
    await expect(entry.locator(SEL_V2["logs_entry_message"])).to_contain_text(
        "scoped log from browser fixture"
    )

    await entry.locator(SEL_V2["logs_entry_row"]).click()
    context = entry.locator(SEL_V2["logs_entry_context"])
    await expect(
        context.locator(SEL_V2["logs_context_chip"].format(key="tool_call_id"))
    ).to_contain_text("tool-call-ui")
    await expect(
        context.locator(SEL_V2["logs_context_chip"].format(key="tool_name"))
    ).to_contain_text("shell")
    await expect(
        context.locator(SEL_V2["logs_context_chip"].format(key="source"))
    ).to_contain_text("slack")


async def test_reborn_v2_logs_deep_link_loads_scoped_conversation_on_first_open(
    reborn_v2_server, reborn_v2_browser
):
    """A non-admin logs deep link reads URL scope before active chat state exists."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    requested_queries: list[dict[str, list[str]]] = []
    operator_logs_requested = False
    logs_requested = asyncio.Event()

    async def fulfill_json(route, body, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def handle_session(route):
        await fulfill_json(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": USER_ID,
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def handle_threads(route):
        await fulfill_json(route, {"threads": [], "next_cursor": None})

    async def handle_logs(route):
        parsed = urlparse(route.request.url)
        requested_queries.append(parse_qs(parsed.query))
        logs_requested.set()
        await fulfill_json(
            route,
            {
                "logs": {
                    "source": "in_memory_tracing",
                    "entries": [
                        {
                            "id": "direct-log-1",
                            "timestamp": "2026-07-08T10:11:12.123Z",
                            "level": "info",
                            "target": "ironclaw::ui::logs",
                            "message": "direct scoped deep link log",
                            "thread_id": "thread-direct",
                            "run_id": "run-direct",
                        }
                    ],
                    "next_cursor": None,
                    "tail_supported": True,
                    "follow_supported": False,
                },
            },
        )

    async def handle_operator_logs(route):
        nonlocal operator_logs_requested
        operator_logs_requested = True
        await fulfill_json(route, {"logs": {"entries": []}}, status=403)

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads**", handle_threads)
    await page.route("**/api/webchat/v2/logs**", handle_logs)
    await page.route("**/api/webchat/v2/operator/logs**", handle_operator_logs)

    try:
        await page.goto(
            f"{reborn_v2_server}/logs"
            "?thread_id=thread-direct&run_id=run-direct"
            f"&token={REBORN_V2_AUTH_TOKEN}"
        )

        await asyncio.wait_for(logs_requested.wait(), timeout=10)
        first_query = requested_queries[0]
        assert first_query.get("thread_id") == ["thread-direct"], first_query
        assert first_query.get("run_id") == ["run-direct"], first_query
        assert first_query.get("limit") == ["500"], first_query
        assert not operator_logs_requested

        await expect(page.locator(SEL_V2["logs_scope_toolbar"])).to_be_visible(
            timeout=10000
        )
        await expect(
            page.locator(SEL_V2["logs_scope_chip"].format(key="thread_id"))
        ).to_contain_text("thread-direct")
        await expect(
            page.locator(SEL_V2["logs_scope_chip"].format(key="run_id"))
        ).to_contain_text("run-direct")
        entry = page.locator(SEL_V2["logs_entry"]).first
        await expect(entry.locator(SEL_V2["logs_entry_message"])).to_contain_text(
            "direct scoped deep link log"
        )
    finally:
        await context.close()


async def test_reborn_v2_thread_list_and_delete(reborn_v2_server):
    """Threads are listed for the caller and deletion removes the thread and transcript."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        keep_id = await _create_thread(client, reborn_v2_server)
        drop_id = await _create_thread(client, reborn_v2_server)

        listed = await client.get(f"{reborn_v2_server}/api/webchat/v2/threads", timeout=15)
        listed.raise_for_status()
        ids = {thread["thread_id"] for thread in listed.json().get("threads", [])}
        assert {keep_id, drop_id} <= ids, f"both threads should be listed, got {ids}"

        deleted = await client.request(
            "DELETE", f"{reborn_v2_server}/api/webchat/v2/threads/{drop_id}", timeout=15
        )
        assert deleted.status_code == 200, deleted.text

        # Transcript is gone (404, not an empty timeline) and re-delete is idempotent-404.
        gone = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{drop_id}/timeline", timeout=15
        )
        assert gone.status_code == 404, gone.text
        re_delete = await client.request(
            "DELETE", f"{reborn_v2_server}/api/webchat/v2/threads/{drop_id}", timeout=15
        )
        assert re_delete.status_code == 404, re_delete.text

        relisted = await client.get(f"{reborn_v2_server}/api/webchat/v2/threads", timeout=15)
        relisted.raise_for_status()
        remaining = {thread["thread_id"] for thread in relisted.json().get("threads", [])}
        assert drop_id not in remaining, "deleted thread must not reappear in the list"
        assert keep_id in remaining, "untouched thread must remain in the list"


async def test_reborn_v2_thread_delete_uses_shared_confirmation_dialog(
    reborn_v2_server, reborn_v2_page
):
    """The sidebar uses the in-app dialog and deletes only after confirmation."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

    native_dialogs: list[str] = []

    async def dismiss_native_dialog(dialog) -> None:
        native_dialogs.append(dialog.type)
        await dialog.dismiss()

    reborn_v2_page.on("dialog", dismiss_native_dialog)
    await reborn_v2_page.goto(
        f"{reborn_v2_server}/chat?token={REBORN_V2_AUTH_TOKEN}"
    )
    delete_button = reborn_v2_page.locator(
        SEL_V2["thread_delete_for"].format(id=thread_id)
    )
    await expect(delete_button).to_be_visible(timeout=15000)

    await delete_button.click()
    confirmation = reborn_v2_page.get_by_role("dialog", name="Delete chat")
    await expect(confirmation).to_be_visible()
    await confirmation.locator(SEL_V2["confirm_dialog_cancel"]).click()
    await expect(confirmation).to_have_count(0)

    async with httpx.AsyncClient(headers=headers) as client:
        timeline = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline",
            timeout=15,
        )
        assert timeline.status_code == 200, timeline.text

    await delete_button.click()
    await expect(confirmation).to_be_visible()
    async with reborn_v2_page.expect_response(
        lambda response: response.request.method == "DELETE"
        and response.url.endswith(f"/api/webchat/v2/threads/{thread_id}")
    ) as response_info:
        await confirmation.locator(SEL_V2["confirm_dialog_confirm"]).click()
    assert (await response_info.value).status == 200

    await expect(delete_button).to_have_count(0, timeout=15000)
    assert native_dialogs == []


async def test_reborn_v2_ui_delete_removes_sidebar_thread_without_refetch(
    reborn_v2_server, reborn_v2_page
):
    """A successful delete updates the rendered sidebar before list revalidation returns."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        keep_id = await _create_thread(client, reborn_v2_server)
        drop_id = await _create_thread(client, reborn_v2_server)

    page = reborn_v2_page
    # The shared page is opened before this test creates its API fixtures, so
    # reload once during setup to populate the sidebar. No reload occurs after
    # deletion; the assertion below runs while list revalidation is blocked.
    await page.reload()
    release_refetch = asyncio.Event()
    refetch_started = asyncio.Event()
    refetch_finished = asyncio.Event()

    async def delay_thread_list_refetch(route, _request) -> None:
        refetch_started.set()
        try:
            await release_refetch.wait()
            await route.continue_()
        finally:
            refetch_finished.set()

    try:
        await page.goto(f"{reborn_v2_server}/?token={REBORN_V2_AUTH_TOKEN}")
        await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)

        keep_button = page.locator(SEL_V2["thread_delete_for"].format(id=keep_id))
        drop_button = page.locator(SEL_V2["thread_delete_for"].format(id=drop_id))
        await expect(keep_button).to_have_count(1, timeout=15000)
        await expect(drop_button).to_have_count(1, timeout=15000)

        # Hold the delete-triggered list revalidation open. The deleted row must
        # disappear from the local React Query cache before this request returns.
        thread_list_pattern = "**/api/webchat/v2/threads"
        await page.route(thread_list_pattern, delay_thread_list_refetch)
        await drop_button.click()
        confirmation = page.get_by_role("dialog", name="Delete chat")
        await expect(confirmation).to_be_visible()
        await confirmation.locator(SEL_V2["confirm_dialog_confirm"]).click()
        await asyncio.wait_for(refetch_started.wait(), timeout=5)

        await expect(drop_button).to_have_count(0, timeout=2000)
        await expect(keep_button).to_have_count(1)
    finally:
        release_refetch.set()
        if refetch_started.is_set():
            await asyncio.wait_for(refetch_finished.wait(), timeout=5)
        await page.unroute("**/api/webchat/v2/threads", delay_thread_list_refetch)


async def test_reborn_v2_timeline_pagination(reborn_v2_server):
    """Timeline honors `limit` and pages older messages via the opaque `next_cursor`."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

        # Two settled turns -> >= 4 messages, enough to force a second page at limit=2.
        await _send_and_settle(client, reborn_v2_server, thread_id, "hello one", expected=1)
        await _send_and_settle(client, reborn_v2_server, thread_id, "hello two", expected=2)

        page1 = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline",
            params={"limit": 2},
            timeout=15,
        )
        page1.raise_for_status()
        page1_body = page1.json()
        assert len(page1_body["messages"]) == 2, page1_body
        cursor = page1_body.get("next_cursor")
        assert cursor, f"a thread with >2 messages must expose next_cursor: {page1_body}"

        # httpx URL-encodes the opaque cursor (it is JSON like {"before_message_sequence":N}).
        page2 = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline",
            params={"limit": 2, "cursor": cursor},
            timeout=15,
        )
        page2.raise_for_status()
        page2_body = page2.json()
        assert page2_body["messages"], f"cursor page must return older messages: {page2_body}"

        page1_seq = {m["sequence"] for m in page1_body["messages"]}
        page2_seq = {m["sequence"] for m in page2_body["messages"]}
        assert page1_seq.isdisjoint(page2_seq), (
            f"paged messages must not overlap: page1={page1_seq} page2={page2_seq}"
        )


async def test_reborn_v2_sse_streams_run_lifecycle(reborn_v2_server):
    """The SSE stream opens via the `?token=` shim and reports the run reaching completion.

    The browser's `EventSource` cannot set an `Authorization` header, so
    `GET /events` accepts `?token=` instead of a bearer (the only v2 route that
    does). The stream is projection-based: it carries run lifecycle status
    (`queued` -> `running` -> `completed`), not the reply text.
    """
    bearer = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=bearer) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

    events_url = (
        f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/events"
        f"?token={REBORN_V2_AUTH_TOKEN}"
    )
    client_timeout = aiohttp.ClientTimeout(total=45, sock_read=45)
    async with aiohttp.ClientSession(timeout=client_timeout) as session:
        # No Authorization header — only the `?token=` query shim authenticates.
        async with session.get(
            events_url, headers={"Accept": "text/event-stream"}
        ) as response:
            assert response.status == 200, (
                f"events stream must open via ?token= shim, got {response.status}"
            )

            # Submit the turn now that the stream is live, then read lifecycle frames.
            async with httpx.AsyncClient(headers=bearer) as client:
                await _send_message(client, reborn_v2_server, thread_id, "hello sse")

            async with asyncio.timeout(45):
                while True:
                    raw = await response.content.readline()
                    assert raw, "SSE stream closed before the run completed"
                    line = raw.decode("utf-8", errors="replace")
                    if '"status":"completed"' in line:
                        return


async def test_reborn_v2_bearer_auth_and_token_shim_scope(reborn_v2_server):
    """v2 routes require a bearer; the `?token=` shim authenticates only the events route."""
    bearer = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=bearer) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

    async with httpx.AsyncClient() as anon:
        # No credentials at all -> 401 on session, list, and timeline.
        for path in (
            "/api/webchat/v2/session",
            "/api/webchat/v2/threads",
            f"/api/webchat/v2/threads/{thread_id}/timeline",
        ):
            response = await anon.get(f"{reborn_v2_server}{path}", timeout=15)
            assert response.status_code == 401, f"{path} without bearer: {response.status_code}"

        # A malformed bearer is rejected.
        bad = await anon.get(
            f"{reborn_v2_server}/api/webchat/v2/session",
            headers={"Authorization": "Bearer not-a-valid-token"},
            timeout=15,
        )
        assert bad.status_code == 401, bad.text

        # The `?token=` shim must NOT authenticate a non-events route (timeline).
        shimmed = await anon.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline"
            f"?token={REBORN_V2_AUTH_TOKEN}",
            timeout=15,
        )
        assert shimmed.status_code == 401, (
            f"?token= must not authenticate timeline, got {shimmed.status_code}"
        )
