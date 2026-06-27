"""Stubbed-browser Reborn WebUI v2 login/session matrix.

These tests drive the committed WebUI v2 bundle in Chromium while stubbing the
public auth/session browser API contracts. They cover the browser-only
REBCLI-064 matrix rows without live SSO providers or live LLM calls.
"""

import json
import re
from urllib.parse import parse_qs, urlparse

import pytest
from playwright.async_api import expect

from helpers import SEL_V2


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


TOKEN_KEY = "ironclaw_token"


class StubbedLoginSession:
    def __init__(
        self,
        *,
        valid_tokens=None,
        providers=None,
        exchange_tokens=None,
        exchange_failures=None,
        logout_status=200,
    ):
        self.valid_tokens = set(valid_tokens or [])
        self.providers = list(providers or [])
        self.exchange_tokens = dict(exchange_tokens or {})
        self.exchange_failures = dict(exchange_failures or {})
        self.logout_status = logout_status
        self.session_requests: list[str] = []
        self.provider_requests: list[str] = []
        self.exchange_requests: list[dict] = []
        self.logout_requests: list[str] = []

    async def install(self, page) -> None:
        await page.route(re.compile(r".*/api/webchat/v2/session$"), self._session)
        await page.route(
            re.compile(r".*/api/webchat/v2/threads(?:\?.*)?$"),
            self._threads,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/threads/[^/]+/timeline(?:\?.*)?$"),
            self._timeline,
        )
        await page.route(re.compile(r".*/auth/providers$"), self._providers)
        await page.route(
            re.compile(r".*/auth/session/exchange$"),
            self._exchange,
        )
        await page.route(re.compile(r".*/auth/logout$"), self._logout)

    async def _fulfill(self, route, body, status: int = 200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    def _bearer(self, route) -> str:
        header = route.request.headers.get("authorization", "")
        return header.removeprefix("Bearer ").strip()

    async def _session(self, route) -> None:
        token = self._bearer(route)
        self.session_requests.append(token)
        if token not in self.valid_tokens:
            await self._fulfill(route, {"error": "unauthorized"}, status=401)
            return
        await self._fulfill(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": f"user-{token}",
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_count": 4,
                    "max_file_bytes": 1024,
                    "max_total_bytes": 4096,
                },
            },
        )

    async def _threads(self, route) -> None:
        await self._fulfill(route, {"threads": [], "next_cursor": None})

    async def _timeline(self, route) -> None:
        await self._fulfill(route, {"messages": [], "next_cursor": None})

    async def _providers(self, route) -> None:
        self.provider_requests.append(route.request.headers.get("authorization", ""))
        await self._fulfill(route, {"providers": self.providers})

    async def _exchange(self, route) -> None:
        body = json.loads(route.request.post_data or "{}")
        self.exchange_requests.append(
            {
                "body": body,
                "authorization": route.request.headers.get("authorization", ""),
            }
        )
        ticket = str(body.get("ticket") or "")
        if ticket in self.exchange_failures:
            await self._fulfill(
                route,
                {"error": self.exchange_failures[ticket]},
                status=401,
            )
            return
        token = self.exchange_tokens.get(ticket)
        if not token:
            await self._fulfill(route, {"error": "invalid_ticket"}, status=401)
            return
        await self._fulfill(route, {"token": token})

    async def _logout(self, route) -> None:
        self.logout_requests.append(self._bearer(route))
        await self._fulfill(route, {"ok": self.logout_status < 400}, status=self.logout_status)


async def _open(page, base_url: str, stub: StubbedLoginSession, path="/v2/chat"):
    await stub.install(page)
    await page.goto(f"{base_url}{path}")


async def _stored_token(page):
    return await page.evaluate(f"sessionStorage.getItem('{TOKEN_KEY}')")


async def _seed_token(context, token: str):
    await context.add_init_script(
        f"sessionStorage.setItem({json.dumps(TOKEN_KEY)}, {json.dumps(token)});"
    )


async def _expect_authenticated_chat(page, token: str, stub: StubbedLoginSession):
    await expect(page).to_have_url(re.compile(r"/v2/chat"), timeout=15000)
    await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
    assert await _stored_token(page) == token
    assert token in stub.session_requests
    assert "token=" not in page.url
    assert "login_ticket=" not in page.url


async def _submit_login_token(page, token: str):
    await expect(page.locator(SEL_V2["login_token"])).to_be_visible(timeout=15000)
    await page.locator(SEL_V2["login_token"]).fill(token)
    await page.get_by_role("button", name="Connect").click()


async def _open_authenticated_mobile(page, base_url, stub, token):
    await _open(page, base_url, stub, path="/v2/chat")
    await _expect_authenticated_chat(page, token, stub)
    await page.get_by_label("Toggle sidebar").click()
    await expect(page.get_by_role("button", name="Sign out")).to_be_visible(timeout=5000)


async def test_login_browser_manual_token_trim_authenticates_chat(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedLoginSession(valid_tokens={"manual-smoke-token"})
    try:
        await _open(page, reborn_v2_server, stub)
        await _submit_login_token(page, "  manual-smoke-token  ")
        await _expect_authenticated_chat(page, "manual-smoke-token", stub)
    finally:
        await context.close()


async def test_login_browser_rejected_manual_token_clears_session(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedLoginSession()
    try:
        await _open(page, reborn_v2_server, stub)
        await _submit_login_token(page, "expired-smoke-token")
        await expect(page).to_have_url(re.compile(r"/v2/login"), timeout=15000)
        await expect(page.get_by_text("Your session expired")).to_be_visible(timeout=5000)
        assert await _stored_token(page) is None
        assert stub.session_requests == ["expired-smoke-token"]
        assert "token=" not in page.url
    finally:
        await context.close()


async def test_login_browser_mobile_layout_has_no_horizontal_overflow(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(
        viewport={"width": 390, "height": 844}, is_mobile=True
    )
    page = await context.new_page()
    stub = StubbedLoginSession()
    try:
        await _open(page, reborn_v2_server, stub)
        await expect(page.locator(SEL_V2["login_token"])).to_be_visible(timeout=15000)
        overflow = await page.evaluate(
            """() => Math.max(
              document.documentElement.scrollWidth - document.documentElement.clientWidth,
              document.body.scrollWidth - document.body.clientWidth
            )"""
        )
        assert overflow <= 1
        for locator in [
            page.locator(SEL_V2["login_token"]),
            page.get_by_role("button", name="Connect"),
            page.get_by_label(re.compile(r"Switch to (light|dark)")),
        ]:
            box = await locator.bounding_box()
            assert box and box["width"] > 0 and box["x"] >= -1 and box["x"] + box["width"] <= 391
    finally:
        await context.close()


async def test_login_browser_oauth_provider_links_preserve_redirect(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedLoginSession(providers=["github", "unknown", "apple", "google"])
    try:
        await _open(page, reborn_v2_server, stub)
        links = page.get_by_role("link", name=re.compile(r"Continue with"))
        await expect(links).to_have_count(3, timeout=5000)
        labels = [await links.nth(i).inner_text() for i in range(3)]
        assert labels == ["Continue with Google", "Continue with GitHub", "Continue with Apple"]
        for index, provider in enumerate(["google", "github", "apple"]):
            href = await links.nth(index).get_attribute("href")
            parsed = urlparse(href)
            assert parsed.path == f"/auth/login/{provider}"
            assert parse_qs(parsed.query)["redirect_after"] == ["/v2/chat"]
        assert stub.provider_requests == [""]
    finally:
        await context.close()


async def test_login_browser_oauth_ticket_exchange_authenticates_chat(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedLoginSession(
        valid_tokens={"oauth-session-smoke-token"},
        exchange_tokens={"oauth-smoke-ticket": "oauth-session-smoke-token"},
    )
    try:
        await _open(
            page,
            reborn_v2_server,
            stub,
            path="/v2?login_ticket=oauth-smoke-ticket&next=%2Fv2%2Fchat#state",
        )
        await _expect_authenticated_chat(page, "oauth-session-smoke-token", stub)
        assert stub.exchange_requests == [
            {"body": {"ticket": "oauth-smoke-ticket"}, "authorization": ""}
        ]
    finally:
        await context.close()


async def test_login_browser_rejected_oauth_ticket_returns_to_login(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedLoginSession(exchange_failures={"expired-oauth-ticket": "expired"})
    try:
        await _open(page, reborn_v2_server, stub, path="/v2?login_ticket=expired-oauth-ticket")
        await expect(page).to_have_url(re.compile(r"/v2/login"), timeout=15000)
        await expect(page.get_by_text("Could not complete sign-in")).to_be_visible(timeout=5000)
        assert await _stored_token(page) is None
        assert stub.exchange_requests == [
            {"body": {"ticket": "expired-oauth-ticket"}, "authorization": ""}
        ]
        assert stub.session_requests == []
        assert "login_ticket=" not in page.url
    finally:
        await context.close()


@pytest.mark.parametrize("logout_status", [200, 503])
async def test_login_browser_sign_out_always_clears_local_session(
    reborn_v2_server, reborn_v2_browser, logout_status
):
    context = await reborn_v2_browser.new_context(
        viewport={"width": 390, "height": 844}, is_mobile=True
    )
    token = "logout-failure-smoke-token" if logout_status >= 400 else "logout-smoke-token"
    await _seed_token(context, token)
    page = await context.new_page()
    stub = StubbedLoginSession(valid_tokens={token}, logout_status=logout_status)
    try:
        await _open_authenticated_mobile(page, reborn_v2_server, stub, token)
        await page.get_by_role("button", name="Sign out").click()
        await expect(page).to_have_url(re.compile(r"/v2/login"), timeout=15000)
        await expect(page.get_by_role("button", name="Connect")).to_be_visible(timeout=5000)
        assert await _stored_token(page) is None
        assert stub.logout_requests == [token]
    finally:
        await context.close()


async def test_login_browser_existing_session_ignores_unsolicited_query_token(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    await _seed_token(context, "working-smoke-token")
    page = await context.new_page()
    stub = StubbedLoginSession(valid_tokens={"working-smoke-token"})
    try:
        await _open(
            page,
            reborn_v2_server,
            stub,
            path="/v2/chat?token=unsolicited-smoke-token",
        )
        await _expect_authenticated_chat(page, "working-smoke-token", stub)
        assert "unsolicited-smoke-token" not in page.url
        assert "unsolicited-smoke-token" not in stub.session_requests
    finally:
        await context.close()


async def test_login_browser_fragment_token_wins_and_scrubs_query_token(
    reborn_v2_server, reborn_v2_browser
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedLoginSession(valid_tokens={"fragment-smoke-token"})
    try:
        await _open(
            page,
            reborn_v2_server,
            stub,
            path="/v2/chat?token=query-smoke-token#token=fragment-smoke-token&mode=debug",
        )
        await _expect_authenticated_chat(page, "fragment-smoke-token", stub)
        assert "query-smoke-token" not in page.url
        assert "fragment-smoke-token" not in page.url
        assert page.url.endswith("#mode=debug")
    finally:
        await context.close()


@pytest.mark.parametrize(
    ("code", "expected"),
    [
        ("invalid_state", "Your sign-in session expired. Please try again."),
        ("future_code", "Could not complete sign-in. Please try again."),
    ],
)
async def test_login_browser_login_error_banner_maps_and_scrubs_callback_code(
    reborn_v2_server, reborn_v2_browser, code, expected
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = StubbedLoginSession()
    try:
        await _open(
            page,
            reborn_v2_server,
            stub,
            path=f"/v2/login?login_error={code}&tab=login#callback",
        )
        await expect(page.get_by_text(expected)).to_be_visible(timeout=5000)
        assert await _stored_token(page) is None
        assert stub.session_requests == []
        assert "login_error=" not in page.url
        assert "tab=login" in page.url
        assert page.url.endswith("#callback")
    finally:
        await context.close()
