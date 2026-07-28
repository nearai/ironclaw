"""Legacy product-auth prompt coverage ported to Reborn WebUI v2."""

import json
import re
from urllib.parse import unquote, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-legacy-auth-flows"
THREAD_B_ID = "thread-legacy-auth-flows-other"
RUN_ID = "run-legacy-auth-flows"
GITHUB_PAT_RE = re.compile(
    r"(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]+",
    re.IGNORECASE,
)


def _assert_no_github_pat(text: str, *, context: str) -> None:
    match = GITHUB_PAT_RE.search(text)
    assert match is None, f"GitHub PAT leaked into {context}: {match.group()!r}"


def _fake_github_pat() -> str:
    return "".join(("gh", "p", "_", "x" * 36))


async def _open_stubbed_auth_thread(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    manual_token_status=200,
    manual_token_body=None,
    thread_records: list[dict] | None = None,
    timelines: dict[str, list[dict]] | None = None,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    manual_token_requests: list[dict] = []
    resolve_requests: list[dict] = []
    default_thread_records = [
        {
            "thread_id": THREAD_ID,
            "title": "Legacy auth flow port",
            "created_at": "2026-06-26T00:00:00Z",
            "updated_at": "2026-06-26T00:00:00Z",
        }
    ]
    default_timelines = {
        THREAD_ID: [
            {
                "message_id": "seed-user",
                "kind": "user",
                "content": "Use a protected integration",
                "sequence": 1,
                "status": "accepted",
                "created_at": "2026-06-26T00:00:00Z",
            }
        ]
    }
    thread_records = thread_records or default_thread_records
    timelines = timelines or default_timelines

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
                    "max_count": 4,
                    "max_file_bytes": 1048576,
                    "max_total_bytes": 4194304,
                },
            },
        )

    async def handle_threads(route):
        await fulfill_json(
            route,
            {
                "threads": thread_records,
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        parsed = urlparse(route.request.url)
        thread_id = unquote(parsed.path.split("/threads/", 1)[1].split("/timeline", 1)[0])
        await fulfill_json(
            route,
            {
                "messages": timelines.get(thread_id, []),
                "next_cursor": None,
            },
        )

    async def handle_manual_token(route):
        body = json.loads(route.request.post_data or "{}")
        manual_token_requests.append(body)
        await fulfill_json(
            route,
            manual_token_body
            if manual_token_body is not None
            else {
                "credential_ref": "credential-ref-github",
            },
            status=manual_token_status,
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
    await page.route("**/api/webchat/v2/threads/*/timeline**", handle_timeline)
    await page.route("**/api/reborn/product-auth/manual-token/submit", handle_manual_token)
    await page.route(
        f"**/api/webchat/v2/threads/{THREAD_ID}/runs/**/gates/**/resolve",
        handle_resolve,
    )

    await page.goto(f"{reborn_v2_server}/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
    await expect(page.locator(SEL_V2["msg_user"]).first).to_contain_text(
        "Use a protected integration", timeout=15000
    )

    return context, page, manual_token_requests, resolve_requests


async def _emit_auth_prompt(
    page,
    *,
    challenge_kind,
    gate_ref,
    authorization_url=None,
    provider="github",
    account_label="GitHub PAT",
    headline="Connect GitHub",
    body="GitHub needs credentials before this run can continue.",
):
    await page.evaluate(
        """
        (prompt) => window.__emitV2Sse("auth_required", { prompt })
        """,
        {
            "turn_run_id": RUN_ID,
            "auth_request_ref": gate_ref,
            "invocation_id": f"invoke-{gate_ref}",
            "challenge_kind": challenge_kind,
            "provider": provider,
            "account_label": account_label,
            "authorization_url": authorization_url,
            "expires_at": "2026-06-26T12:00:00Z" if authorization_url else None,
            "headline": headline,
            "body": body,
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

        await page.locator(SEL_V2["auth_token_input"]).fill("  manual-token-placeholder\n")
        await gate.get_by_role("button", name="Use token").click()
        await expect(gate).to_be_hidden(timeout=5000)

        assert manual_token_requests == [
            {
                "provider": "github",
                "account_label": "GitHub PAT",
                "token": "manual-token-placeholder",
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


async def test_reborn_legacy_manual_token_not_retained_in_browser(
    reborn_v2_server, reborn_v2_browser
):
    context, page, manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="manual_token",
            gate_ref="manual-token-no-leak-gate",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="manual_token")).first
        await expect(gate).to_be_visible(timeout=5000)

        fake_github_pat = _fake_github_pat()
        await page.locator(SEL_V2["auth_token_input"]).fill(f" {fake_github_pat} ")
        await gate.get_by_role("button", name="Use token").click()
        await expect(gate).to_be_hidden(timeout=5000)
        await expect(page.locator(SEL_V2["auth_token_input"])).to_have_count(0)

        assert manual_token_requests[-1]["token"] == fake_github_pat
        _assert_no_github_pat(await page.inner_text("body"), context="visible page text")
        _assert_no_github_pat(
            await page.evaluate("() => document.body.innerHTML"),
            context="page HTML",
        )
        browser_storage = await page.evaluate(
            """
            () => JSON.stringify({
              localStorage: Object.entries(window.localStorage),
              sessionStorage: Object.entries(window.sessionStorage),
            })
            """
        )
        _assert_no_github_pat(browser_storage, context="browser storage")
    finally:
        await context.close()


async def test_reborn_legacy_manual_token_submit_failure_keeps_gate_retryable(
    reborn_v2_server, reborn_v2_browser
):
    context, page, manual_token_requests, resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server,
        reborn_v2_browser,
        manual_token_status=400,
        manual_token_body={"error": "invalid_credential", "kind": "bad_request"},
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="manual_token",
            gate_ref="manual-token-failure-gate",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="manual_token")).first
        await expect(gate).to_be_visible(timeout=5000)
        token_input = page.locator(SEL_V2["auth_token_input"])
        await token_input.fill("wrong-token")

        submit = gate.get_by_role("button", name="Use token")
        await submit.click()

        await expect(gate.get_by_role("alert")).to_contain_text("Could not save the token.")
        await expect(gate).to_be_visible()
        await expect(token_input).to_be_enabled()
        await expect(submit).to_be_enabled()
        assert manual_token_requests == [
            {
                "provider": "github",
                "account_label": "GitHub PAT",
                "token": "wrong-token",
                "thread_id": THREAD_ID,
                "run_id": RUN_ID,
                "gate_ref": "manual-token-failure-gate",
            }
        ]
        assert resolve_requests == []
    finally:
        await context.close()


async def test_reborn_legacy_manual_token_cancel_resolves_gate_without_token_submit(
    reborn_v2_server, reborn_v2_browser
):
    context, page, manual_token_requests, resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="manual_token",
            gate_ref="manual-token-cancel-gate",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="manual_token")).first
        await expect(gate).to_be_visible(timeout=5000)
        await gate.get_by_role("button", name="Cancel").click()
        await expect(gate).to_be_hidden(timeout=5000)

        assert manual_token_requests == []
        assert len(resolve_requests) == 1
        assert f"/threads/{THREAD_ID}/runs/{RUN_ID}/gates/manual-token-cancel-gate/resolve" in (
            resolve_requests[0]["url"]
        )
        assert resolve_requests[0]["body"]["resolution"] == "declined"
        assert resolve_requests[0]["body"]["always"] is False
        assert resolve_requests[0]["body"]["client_action_id"]
    finally:
        await context.close()


async def test_reborn_legacy_auth_prompt_does_not_duplicate_as_assistant_text(
    reborn_v2_server, reborn_v2_browser
):
    context, page, _manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        auth_body = (
            "Paste your GitHub personal access token below. "
            "Authentication required before the protected integration can continue."
        )
        await page.evaluate(
            """
            (prompt) => window.__emitV2Sse("auth_required", { prompt })
            """,
            {
                "turn_run_id": RUN_ID,
                "auth_request_ref": "duplicate-response-auth-gate",
                "invocation_id": "invoke-duplicate-response-auth-gate",
                "challenge_kind": "manual_token",
                "provider": "github",
                "account_label": "GitHub PAT",
                "headline": "Connect GitHub",
                "body": auth_body,
            },
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="manual_token")).first
        await expect(gate).to_be_visible(timeout=5000)
        await expect(gate).to_contain_text(auth_body)
        await expect(
            page.locator(SEL_V2["msg_assistant"]).filter(has_text="Paste your GitHub")
        ).to_have_count(0)
        await expect(page.locator(SEL_V2["auth_gate"])).to_have_count(1)
    finally:
        await context.close()


async def test_reborn_legacy_auth_prompt_replaces_existing_prompt(
    reborn_v2_server, reborn_v2_browser
):
    context, page, manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="manual_token",
            gate_ref="manual-token-first-gate",
            provider="github",
            account_label="GitHub PAT",
            headline="Connect GitHub",
            body="First token prompt",
        )
        first_gate = page.locator(SEL_V2["auth_gate_for"].format(kind="manual_token")).first
        await expect(first_gate).to_be_visible(timeout=5000)
        await expect(first_gate).to_contain_text("First token prompt")

        await _emit_auth_prompt(
            page,
            challenge_kind="manual_token",
            gate_ref="manual-token-second-gate",
            provider="slack",
            account_label="Slack token",
            headline="Connect Slack",
            body="Second token prompt",
        )

        await expect(page.locator(SEL_V2["auth_gate"])).to_have_count(1)
        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="manual_token")).first
        await expect(gate).to_contain_text("Connect Slack")
        await expect(gate).to_contain_text("Slack token")
        await expect(gate).to_contain_text("Second token prompt")
        await expect(
            page.locator(SEL_V2["auth_gate"]).filter(has_text="First token prompt")
        ).to_have_count(0)
        await page.locator(SEL_V2["auth_token_input"]).fill("xoxb-second-token")
        await gate.get_by_role("button", name="Use token").click()
        await expect(gate).to_be_hidden(timeout=5000)
        assert manual_token_requests == [
            {
                "provider": "slack",
                "account_label": "Slack token",
                "token": "xoxb-second-token",
                "thread_id": THREAD_ID,
                "run_id": RUN_ID,
                "gate_ref": "manual-token-second-gate",
            }
        ]
    finally:
        await context.close()


async def test_reborn_legacy_auth_prompt_does_not_block_other_thread(
    reborn_v2_server, reborn_v2_browser
):
    """Port foreign-thread auth isolation to Reborn's route-scoped auth gate state."""
    context, page, _manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server,
        reborn_v2_browser,
        thread_records=[
            {
                "thread_id": THREAD_ID,
                "title": "Auth Thread A",
                "created_at": "2026-06-26T00:00:00Z",
                "updated_at": "2026-06-26T00:00:00Z",
            },
            {
                "thread_id": THREAD_B_ID,
                "title": "Auth Thread B",
                "created_at": "2026-06-26T00:01:00Z",
                "updated_at": "2026-06-26T00:01:00Z",
            },
        ],
        timelines={
            THREAD_ID: [
                {
                    "message_id": "thread-a-user",
                    "kind": "user",
                    "content": "Use a protected integration",
                    "sequence": 1,
                    "status": "accepted",
                    "created_at": "2026-06-26T00:00:00Z",
                }
            ],
            THREAD_B_ID: [
                {
                    "message_id": "thread-b-user",
                    "kind": "user",
                    "content": "Thread B can continue",
                    "sequence": 1,
                    "status": "accepted",
                    "created_at": "2026-06-26T00:01:00Z",
                }
            ],
        },
    )
    send_requests: list[dict] = []

    async def handle_send_b(route):
        send_requests.append(json.loads(route.request.post_data or "{}"))
        await route.fulfill(
            status=202,
            content_type="application/json",
            body=json.dumps(
                {
                    "thread_id": THREAD_B_ID,
                    "run_id": "run-thread-b-auth-send",
                    "status": "queued",
                }
            ),
        )

    await page.route(f"**/api/webchat/v2/threads/{THREAD_B_ID}/messages", handle_send_b)

    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="manual_token",
            gate_ref="manual-token-thread-a-gate",
            headline="Connect Thread A",
            body="Thread A needs credentials before it can continue.",
        )
        await expect(page.locator(SEL_V2["auth_gate"])).to_have_count(1)
        await expect(page.locator(SEL_V2["chat_composer"])).to_have_attribute(
            "data-send-disabled",
            "true",
        )

        await page.locator(SEL_V2["sidebar_button"]).filter(
            has_text="Auth Thread B"
        ).first.click()
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="Thread B can continue")
        ).to_be_visible(timeout=15000)
        await expect(page.locator(SEL_V2["auth_gate"])).to_have_count(0)

        composer = page.locator(SEL_V2["chat_composer"])
        await expect(composer).to_have_attribute(
            "data-send-disabled",
            "false",
            timeout=5000,
        )
        await composer.fill("send while other thread needs auth")
        await composer.press("Enter")

        await expect(page.locator(SEL_V2["msg_user"]).last).to_contain_text(
            "send while other thread needs auth",
            timeout=10000,
        )
        assert len(send_requests) == 1
        assert send_requests[0]["content"] == "send while other thread needs auth"
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

        await page.evaluate(
            """
            () => {
              window.__openedAuth = [];
              window.open = (url, target, features) => {
                const popup = {
                  closed: false,
                  close() { this.closed = true; },
                  location: {
                    _href: url,
                    get href() { return this._href; },
                    set href(value) {
                      this._href = value;
                      window.__openedAuth.push({ kind: "navigate", url: value });
                    },
                  },
                };
                window.__openedAuth.push({ kind: "open", url, target, features });
                return popup;
              };
            }
            """
        )
        cta = gate.locator(SEL_V2["auth_oauth_open"])
        await expect(cta).to_have_attribute("href", auth_url)
        await cta.click()
        assert await page.evaluate("() => window.__openedAuth") == [
            {
                "kind": "open",
                "url": "about:blank",
                "target": "_blank",
                "features": "width=600,height=600",
            },
            {
                "kind": "navigate",
                "url": auth_url,
            }
        ]
        await expect(gate).to_contain_text("Waiting for GitHub")

        await gate.get_by_role("button", name="Cancel").click()
        await expect(gate).to_be_hidden(timeout=5000)
        assert len(resolve_requests) == 1
        assert resolve_requests[0]["body"]["resolution"] == "declined"
    finally:
        await context.close()


async def test_reborn_legacy_notion_oauth_prompt_renders_provider_label(
    reborn_v2_server, reborn_v2_browser
):
    context, page, _manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="oauth_url",
            gate_ref="notion-oauth-gate",
            authorization_url="https://api.notion.com/v1/oauth/authorize?state=notion-state",
            provider="notion",
            account_label="Notion workspace",
            headline="Connect Notion",
            body="Notion needs authorization before this run can continue.",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="oauth_url")).first
        await expect(gate).to_be_visible(timeout=5000)
        await expect(gate).to_contain_text("Connect Notion")
        await expect(gate).to_contain_text("Notion workspace")
        await expect(gate).to_contain_text("Open Notion authorization")
    finally:
        await context.close()


async def test_reborn_legacy_gsuite_oauth_prompt_renders_authorization_action(
    reborn_v2_server, reborn_v2_browser
):
    context, page, _manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="oauth_url",
            gate_ref="gsuite-oauth-gate",
            authorization_url="https://accounts.google.com/o/oauth2/v2/auth?state=gsuite-state",
            provider="google",
            account_label="Google Workspace",
            headline="Connect Google Workspace",
            body="Google Workspace needs authorization before this run can continue.",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="oauth_url")).first
        await expect(gate).to_be_visible(timeout=5000)
        await expect(gate).to_contain_text("Connect Google Workspace")
        await expect(gate).to_contain_text("Google Workspace")
        await expect(gate.locator(SEL_V2["auth_oauth_open"])).to_have_attribute(
            "href",
            "https://accounts.google.com/o/oauth2/v2/auth?state=gsuite-state",
        )
        await expect(gate).to_contain_text("Open Google authorization")
    finally:
        await context.close()


async def test_reborn_legacy_oauth_callback_completion_clears_matching_gate(
    reborn_v2_server, reborn_v2_browser
):
    context, page, _manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="oauth_url",
            gate_ref="oauth-callback-gate",
            authorization_url="https://accounts.example.test/oauth?state=callback-state",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="oauth_url")).first
        await expect(gate).to_be_visible(timeout=5000)

        await page.evaluate(
            """
            (payload) => {
              const key = "ironclaw:product-auth:oauth-complete";
              const value = JSON.stringify(payload);
              window.localStorage.setItem(key, value);
              window.dispatchEvent(new StorageEvent("storage", { key, newValue: value }));
            }
            """,
            {
                "type": "ironclaw:product-auth:oauth-complete",
                "status": "completed",
                "completedAt": 1924617600000,
                "continuation": {
                    "type": "turn_gate_resume",
                    "turn_run_ref": RUN_ID,
                    "gate_ref": "oauth-callback-gate",
                },
            },
        )

        await expect(gate).to_be_hidden(timeout=5000)
        await expect(page.locator(SEL_V2["auth_gate"])).to_have_count(0)
    finally:
        await context.close()


async def test_reborn_legacy_oauth_callback_for_other_gate_keeps_prompt_visible(
    reborn_v2_server, reborn_v2_browser
):
    context, page, _manual_token_requests, _resolve_requests = await _open_stubbed_auth_thread(
        reborn_v2_server, reborn_v2_browser
    )
    try:
        await _emit_auth_prompt(
            page,
            challenge_kind="oauth_url",
            gate_ref="oauth-active-gate",
            authorization_url="https://accounts.example.test/oauth?state=active-state",
        )

        gate = page.locator(SEL_V2["auth_gate_for"].format(kind="oauth_url")).first
        await expect(gate).to_be_visible(timeout=5000)

        await page.evaluate(
            """
            (payload) => {
              const key = "ironclaw:product-auth:oauth-complete";
              const value = JSON.stringify(payload);
              window.localStorage.setItem(key, value);
              window.dispatchEvent(new StorageEvent("storage", { key, newValue: value }));
            }
            """,
            {
                "type": "ironclaw:product-auth:oauth-complete",
                "status": "completed",
                "completedAt": 1924617600000,
                "continuation": {
                    "type": "turn_gate_resume",
                    "turn_run_ref": RUN_ID,
                    "gate_ref": "oauth-other-gate",
                },
            },
        )

        await expect(gate).to_be_visible(timeout=1000)
        await expect(gate).to_contain_text("Connect GitHub")
        await expect(page.locator(SEL_V2["auth_gate"])).to_have_count(1)
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
