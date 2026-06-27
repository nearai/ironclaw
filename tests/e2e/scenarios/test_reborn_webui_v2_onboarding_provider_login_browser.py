"""Stubbed-browser WebUI v2 first-run onboarding provider-login matrix.

These tests drive the committed /welcome onboarding screen in Chromium through
the real ironclaw-reborn serve binary while stubbing only the v2 provider-login
endpoints. They cover the browser-only REBCLI-069 onboarding rows without live
NEAR AI or Codex provider calls.
"""

import json
import re
from urllib.parse import urlparse

import pytest
from playwright.async_api import async_playwright, expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


class OnboardingProviderLoginStub:
    def __init__(self):
        self.provider_requests: list[str] = []
        self.nearai_requests: list[dict] = []
        self.codex_requests: list[dict] = []

    async def install(self, page) -> None:
        await page.route(re.compile(r".*/api/webchat/v2/session$"), self._session)
        await page.route(re.compile(r".*/api/webchat/v2/threads.*"), self._threads)
        await page.route(
            re.compile(r".*/api/webchat/v2/llm/providers$"),
            self._providers,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/llm/nearai/login$"),
            self._nearai_login,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/llm/codex/login$"),
            self._codex_login,
        )

    async def _fulfill(self, route, body, status: int = 200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def _session(self, route) -> None:
        await self._fulfill(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": "onboarding-provider-browser-user",
                "capabilities": {"operator_webui_config": True},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def _threads(self, route) -> None:
        await self._fulfill(route, {"threads": [], "next_cursor": None})

    async def _providers(self, route) -> None:
        self.provider_requests.append(route.request.headers.get("authorization", ""))
        await self._fulfill(
            route,
            {
                "providers": [
                    {
                        "id": "nearai",
                        "description": "NEAR AI",
                        "adapter": "nearai",
                        "builtin": True,
                        "api_key_required": True,
                        "accepts_api_key": True,
                        "base_url_required": False,
                        "default_model": "nearai-default",
                        "api_key_set": False,
                    },
                    {
                        "id": "openai_codex",
                        "description": "ChatGPT subscription",
                        "adapter": "openai_codex",
                        "builtin": True,
                        "api_key_required": False,
                        "accepts_api_key": False,
                        "base_url_required": False,
                        "default_model": "gpt-5-codex",
                        "api_key_set": False,
                    },
                    {
                        "id": "openai",
                        "description": "OpenAI API",
                        "adapter": "openai",
                        "builtin": True,
                        "api_key_required": True,
                        "accepts_api_key": True,
                        "base_url_required": False,
                        "default_model": "gpt-4o",
                        "api_key_set": False,
                    },
                ],
                "active": None,
            },
        )

    async def _nearai_login(self, route) -> None:
        self.nearai_requests.append(
            {
                "authorization": route.request.headers.get("authorization", ""),
                "body": json.loads(route.request.post_data or "{}"),
            }
        )
        await self._fulfill(route, {"auth_url": "https://private.near.ai/login"})

    async def _codex_login(self, route) -> None:
        self.codex_requests.append(
            {
                "authorization": route.request.headers.get("authorization", ""),
                "body": route.request.post_data or "",
            }
        )
        await self._fulfill(
            route,
            {
                "user_code": "SMOKE-CODE",
                "verification_uri": "https://chatgpt.com/activate",
            },
        )


async def _open_public_onboarding_page(
    reborn_v2_server,
    stub: OnboardingProviderLoginStub,
    *,
    viewport: dict[str, int] | None = None,
):
    parsed = urlparse(reborn_v2_server)
    public_base = f"{parsed.scheme}://app.example.test:{parsed.port}"
    playwright = await async_playwright().start()
    browser = await playwright.chromium.launch(
        headless=True,
        args=[
            "--host-resolver-rules=MAP app.example.test 127.0.0.1",
            "--no-proxy-server",
        ],
    )
    context = await browser.new_context(
        viewport=viewport or {"width": 1280, "height": 720},
    )
    await context.add_init_script(
        """
        (() => {
          const opened = [];
          window.__providerLoginOpenedUrls = opened;
          window.open = (url) => {
            const popup = {
              closed: false,
              opener: window,
              close() { this.closed = true; },
              location: {},
            };
            Object.defineProperty(popup.location, "href", {
              get() { return this._href || ""; },
              set(value) {
                this._href = value;
                opened.push(value);
              },
            });
            opened.push(url);
            return popup;
          };
        })();
        """
    )
    page = await context.new_page()
    await stub.install(page)
    await page.goto(
        f"{public_base}/v2/welcome?token={REBORN_V2_AUTH_TOKEN}",
        wait_until="domcontentloaded",
    )
    await expect(page).to_have_url(
        re.compile(r"app\.example\.test:\d+/v2/welcome/?$"),
        timeout=15000,
    )
    await expect(page.get_by_role("heading", name="Welcome to IronClaw")).to_be_visible(
        timeout=15000,
    )
    await expect(
        page.locator(SEL_V2["llm_provider_card_for"].format(id="nearai"))
    ).to_be_visible(timeout=15000)
    await expect(
        page.locator(SEL_V2["llm_provider_card_for"].format(id="openai_codex"))
    ).to_be_visible(timeout=15000)
    return playwright, browser, context, page, public_base


async def _close_public_browser(playwright, browser, context) -> None:
    await context.close()
    await browser.close()
    await playwright.stop()


async def test_onboarding_provider_login_browser_starts_nearai_google_flow(
    reborn_v2_server,
):
    stub = OnboardingProviderLoginStub()
    playwright, browser, context, page, public_base = await _open_public_onboarding_page(
        reborn_v2_server,
        stub,
    )
    try:
        await page.locator(SEL_V2["llm_provider_nearai_setup_menu"]).click()
        await expect(
            page.locator(SEL_V2["llm_provider_nearai_setup_menu_items"])
        ).to_be_visible(timeout=10000)
        await page.locator(SEL_V2["llm_provider_nearai_google_login"]).click()
        await expect(page.locator(SEL_V2["llm_provider_nearai_waiting"])).to_be_visible(
            timeout=10000
        )
        assert stub.nearai_requests == [
            {
                "authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}",
                "body": {"provider": "google", "origin": public_base},
            }
        ]
        opened = []
        for _ in range(50):
            opened = await page.evaluate("window.__providerLoginOpenedUrls")
            if len(opened) == 2:
                break
            await page.wait_for_timeout(200)
        assert opened == ["about:blank", "https://private.near.ai/login"]
    finally:
        await _close_public_browser(playwright, browser, context)


async def test_onboarding_provider_login_browser_starts_codex_device_flow(
    reborn_v2_server,
):
    stub = OnboardingProviderLoginStub()
    playwright, browser, context, page, _public_base = await _open_public_onboarding_page(
        reborn_v2_server,
        stub,
    )
    try:
        await page.locator(SEL_V2["llm_provider_codex_login"]).click()
        await expect(page.locator(SEL_V2["llm_provider_codex_code"])).to_contain_text(
            "SMOKE-CODE",
            timeout=10000,
        )
        await expect(page.locator(SEL_V2["llm_provider_codex_waiting"])).to_be_visible(
            timeout=10000
        )
        assert stub.codex_requests == [
            {
                "authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}",
                "body": "",
            }
        ]
        opened = await page.evaluate("window.__providerLoginOpenedUrls")
        assert opened == ["about:blank", "https://chatgpt.com/activate"]
    finally:
        await _close_public_browser(playwright, browser, context)


async def test_onboarding_provider_login_browser_mobile_menu_stays_in_viewport(
    reborn_v2_server,
):
    stub = OnboardingProviderLoginStub()
    playwright, browser, context, page, _public_base = await _open_public_onboarding_page(
        reborn_v2_server,
        stub,
        viewport={"width": 390, "height": 844},
    )
    try:
        await page.locator(SEL_V2["llm_provider_nearai_setup_menu"]).click()
        menu = page.locator(SEL_V2["llm_provider_nearai_setup_menu_items"])
        await expect(menu).to_be_visible(timeout=10000)
        bounds = await menu.bounding_box()
        assert bounds is not None
        assert bounds["x"] >= 0
        assert bounds["x"] + bounds["width"] <= 390
        assert await page.evaluate("document.documentElement.scrollWidth") <= 390
    finally:
        await _close_public_browser(playwright, browser, context)
