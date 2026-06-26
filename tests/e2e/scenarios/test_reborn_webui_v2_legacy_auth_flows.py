"""Legacy product-auth prompt coverage ported to Reborn WebUI v2."""

import json

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-legacy-auth-flows"
RUN_ID = "run-legacy-auth-flows"


async def _open_stubbed_auth_thread(reborn_v2_server, reborn_v2_browser):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    manual_token_requests: list[dict] = []
    resolve_requests: list[dict] = []

    await page.add_init_script(
        """
        (() => {
          const streams = [];
          window.__openedAuth = [];
          window.open = (url, target, features) => {
            window.__openedAuth.push({ url, target, features });
            return null;
          };
          class FakeEventSource extends EventTarget {
            constructor(url) {
              super();
              this.url = url;
              this.readyState = 0;
              streams.push(this);
              setTimeout(() => {
                this.readyState = 1;
                if (typeof this.onopen === "function") this.onopen(new Event("open"));
              }, 0);
            }
            close() {
              this.readyState = 2;
            }
          }
          window.EventSource = FakeEventSource;
          window.__emitV2Sse = (type, frame, id = "cursor-auth") => {
            const stream = streams[streams.length - 1];
            if (!stream) throw new Error("no EventSource stream is open");
            const event = new MessageEvent(type, {
              data: JSON.stringify({ type, ...frame }),
              lastEventId: id,
            });
            stream.dispatchEvent(event);
          };
        })();
        """
    )

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
        await fulfill_json(
            route,
            {
                "threads": [
                    {
                        "thread_id": THREAD_ID,
                        "title": "Legacy auth flow port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "seed-user",
                        "kind": "user",
                        "content": "Use a protected integration",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_manual_token(route):
        body = json.loads(route.request.post_data or "{}")
        manual_token_requests.append(body)
        await fulfill_json(
            route,
            {
                "credential_ref": "credential-ref-github",
            },
        )

    async def handle_resolve(route):
        resolve_requests.append(
            {
                "url": route.request.url,
                "body": json.loads(route.request.post_data or "{}"),
            }
        )
        await fulfill_json(
            route,
            {
                "thread_id": THREAD_ID,
                "run_id": RUN_ID,
                "status": "queued",
                "outcome": "resumed",
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{THREAD_ID}/timeline**", handle_timeline)
    await page.route("**/api/reborn/product-auth/manual-token/submit", handle_manual_token)
    await page.route(
        f"**/api/webchat/v2/threads/{THREAD_ID}/runs/**/gates/**/resolve",
        handle_resolve,
    )

    await page.goto(f"{reborn_v2_server}/v2/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
    await expect(page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "Use a protected integration", timeout=15000
    )

    return context, page, manual_token_requests, resolve_requests


async def _emit_auth_prompt(page, *, challenge_kind, gate_ref, authorization_url=None):
    await page.evaluate(
        """
        (prompt) => window.__emitV2Sse("auth_required", { prompt })
        """,
        {
            "turn_run_id": RUN_ID,
            "auth_request_ref": gate_ref,
            "invocation_id": f"invoke-{gate_ref}",
            "challenge_kind": challenge_kind,
            "provider": "github",
            "account_label": "GitHub PAT",
            "authorization_url": authorization_url,
            "expires_at": "2026-06-26T12:00:00Z" if authorization_url else None,
            "headline": "Connect GitHub",
            "body": "GitHub needs credentials before this run can continue.",
        },
    )


async def test_reborn_legacy_manual_token_auth_prompt_submits_and_resumes_gate(
    reborn_v2_server, reborn_v2_browser
):
    context, page, manual_token_requests, resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="manual_token",
            gate_ref="manual-token-gate",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="manual_token")).first
        await expect(gate).to_be_visible(timeout=5000)
        await expect(gate).to_contain_text("Connect GitHub")
        await expect(gate).to_contain_text("GitHub PAT")

        await gate.get_by_role("button", name="Use token").click()
        await expect(gate.get_by_role("alert")).to_contain_text("A token is required.")

        await page.locator(SEL_V2["auth_token_input"]).fill("  ghp_mock_token\n")
        await gate.get_by_role("button", name="Use token").click()
        await expect(gate).to_be_hidden(timeout=5000)

        assert manual_token_requests == [
            {
                "provider": "github",
                "account_label": "GitHub PAT",
                "token": "ghp_mock_token",
                "thread_id": THREAD_ID,
                "run_id": RUN_ID,
                "gate_ref": "manual-token-gate",
            }
        ]
        assert len(resolve_requests) == 1
        assert f"/threads/{THREAD_ID}/runs/{RUN_ID}/gates/manual-token-gate/resolve" in (
            resolve_requests[0]["url"]
        )
        assert resolve_requests[0]["body"]["resolution"] == "credential_provided"
        assert resolve_requests[0]["body"]["credential_ref"] == "credential-ref-github"
        assert resolve_requests[0]["body"]["client_action_id"]
    finally:
        await context.close()


async def test_reborn_legacy_oauth_prompt_opens_https_authorization_only(
    reborn_v2_server, reborn_v2_browser
):
    context, page, _manual_token_requests, resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        auth_url = "https://accounts.example.test/oauth?state=opaque-state"
        await _emit_auth_prompt(
            page,
            challenge_kind="oauth_url",
            gate_ref="oauth-gate",
            authorization_url=auth_url,
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="oauth_url")).first
        await expect(gate).to_be_visible(timeout=5000)
        await expect(gate).to_contain_text("Connect GitHub")
        await expect(gate).to_contain_text("Authorize")

        cta = gate.locator(SEL_V2["auth_oauth_open"])
        await expect(cta).to_have_attribute("href", auth_url)
        await cta.click()
        assert await page.evaluate("() => window.__openedAuth") == [
            {
                "url": auth_url,
                "target": "_blank",
                "features": "noopener,noreferrer",
            }
        ]
        await expect(gate).to_contain_text("Waiting for authorization to complete")

        await gate.get_by_role("button", name="Cancel").click()
        await expect(gate).to_be_hidden(timeout=5000)
        assert len(resolve_requests) == 1
        assert resolve_requests[0]["body"]["resolution"] == "cancelled"
    finally:
        await context.close()


async def test_reborn_legacy_oauth_prompt_rejects_non_https_authorization_url(
    reborn_v2_server, reborn_v2_browser
):
    context, page, _manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="oauth_url",
            gate_ref="oauth-bad-url-gate",
            authorization_url="javascript:alert(1)",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="oauth_url")).first
        await expect(gate).to_be_visible(timeout=5000)
        cta = gate.locator(SEL_V2["auth_oauth_open"])
        await expect(cta).not_to_have_attribute("href", "javascript:alert(1)")

        await cta.click()
        await expect(gate.get_by_role("alert")).to_contain_text("Service unavailable")
        assert await page.evaluate("() => window.__openedAuth") == []
    finally:
        await context.close()
