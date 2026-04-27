"""Shared helpers for E2E tests."""

import asyncio
import hashlib
import hmac
import re
import time
import uuid
from contextlib import asynccontextmanager

import aiohttp
import httpx

# -- DOM Selectors --------------------------------------------------------
# Keep all selectors in one place so changes to the frontend only need
# one update.

SEL = {
    # Auth
    "auth_screen": "#auth-screen",
    "token_input": "#token-input",
    # Tabs
    # Scope to the main tab-bar buttons only. `.status-logs-btn` covers the
    # right-aligned auxiliary buttons (logs, docs link) and `.tab-btn` covers
    # widget-injected tabs added by `_addWidgetTab`. Excluding both keeps the
    # selector a single match under Playwright strict mode even if a widget
    # or auxiliary button is ever introduced with a colliding `data-tab` id.
    "tab_button": '.tab-bar > button[data-tab="{tab}"]:not(.status-logs-btn):not(.tab-btn)',
    "tab_panel": "#tab-{tab}",
    # Chat
    "chat_input": "#chat-input",
    "chat_messages": "#chat-messages",
    "attach_btn": "#attach-btn",
    "attachment_input": "#image-file-input",
    "slash_autocomplete": "#slash-autocomplete",
    "slash_item": "#slash-autocomplete .slash-ac-item",
    "message_user": "#chat-messages .message.user",
    "message_assistant": "#chat-messages .message.assistant",
    "message_system": "#chat-messages .message.system",
    "message_attachments": "#chat-messages .message.user .message-attachments",
    # Skills
    "skill_search_input": "#skill-search-input",
    "skill_search_results": "#skill-search-results",
    "skill_search_result": ".skill-search-result",
    "skill_installed": "#skills-list .ext-card",
    # SSE status
    "sse_dot": "#sse-dot",
    # Approval overlay
    "approval_card": ".approval-card",
    "approval_header": ".approval-header",
    "approval_tool_name": ".approval-tool-name",
    "approval_description": ".approval-description",
    "approval_params_toggle": ".approval-params-toggle",
    "approval_params": ".approval-params",
    "approval_actions": ".approval-actions",
    "approval_approve_btn": ".approval-actions button.approve",
    "approval_always_btn": ".approval-actions button.always",
    "approval_deny_btn": ".approval-actions button.deny",
    "approval_resolved": ".approval-resolved",
    # Settings subtabs
    "settings_subtab":          '.settings-subtab[data-settings-subtab="{subtab}"]',
    "settings_subpanel":        "#settings-{subtab}",
    # Extensions section
    "extensions_list":          "#extensions-list",
    "available_wasm_list":      "#available-wasm-list",
    "mcp_servers_list":         "#mcp-servers-list",
    # Extensions tab – cards
    "ext_card_installed":       "#extensions-list .ext-card",
    "ext_card_available":       "#available-wasm-list .ext-card.ext-available",
    "ext_card_mcp":             "#mcp-servers-list .ext-card",
    "ext_name":                 ".ext-name",
    "ext_kind":                 ".ext-kind",
    "ext_auth_dot":             ".ext-auth-dot",
    "ext_auth_dot_authed":      ".ext-auth-dot.authed",
    "ext_auth_dot_unauthed":    ".ext-auth-dot.unauthed",
    "ext_active_label":         ".ext-active-label",
    "ext_pairing_label":        ".ext-pairing-label",
    "ext_pairing":              ".ext-pairing",
    "ext_error":                ".ext-error",
    "ext_tools":                ".ext-tools",
    "pairing_heading":          ".pairing-heading",
    "pairing_help":             ".pairing-help:not(.pairing-restart)",
    "pairing_input":            ".pairing-input",
    "pairing_manual_input":     ".pairing-manual-input",
    "pairing_manual_submit":    ".pairing-manual-submit",
    "pairing_row":              ".pairing-row",
    "pairing_code":             ".pairing-code",
    "pairing_sender":           ".pairing-sender",
    # Extensions tab – action buttons
    "ext_install_btn":          ".btn-ext.install",
    "ext_remove_btn":           ".btn-ext.remove",
    "ext_activate_btn":         ".btn-ext.activate",
    "ext_configure_btn":        ".btn-ext.configure",
    # Configure modal
    "configure_overlay":        ".configure-overlay",
    "configure_modal":          ".configure-modal",
    "configure_field":          ".configure-field",
    "configure_input":          ".configure-modal input[type='password']",
    "configure_save_btn":       ".configure-actions button.btn-ext.activate",
    "configure_cancel_btn":     ".configure-actions button.btn-ext.remove",
    "field_provided":           ".field-provided",
    "field_autogen":            ".field-autogen",
    "field_optional":           ".field-optional",
    # Auth card (SSE-triggered, injected into chat-messages)
    "auth_card":                ".auth-card",
    "auth_header":              ".auth-header",
    "auth_instructions":        ".auth-instructions",
    "auth_oauth_btn":           ".auth-oauth",
    "auth_token_input":         ".auth-token-input input",
    "auth_submit_btn":          ".auth-submit",
    "auth_cancel_btn":          ".auth-cancel",
    "auth_error":               ".auth-error",
    "setup_card":               ".setup-card",
    "setup_form":               ".setup-form",
    "setup_input":              ".setup-input",
    "setup_next_step":          ".setup-next-step",
    "pairing_card":             ".pairing-card",
    "pairing_submit_btn":       ".pairing-submit",
    "pairing_cancel_btn":       ".pairing-cancel",
    "pairing_restart":          ".pairing-restart",
    # WASM channel progress stepper
    "ext_stepper":              ".ext-stepper",
    "stepper_step":             ".stepper-step",
    "stepper_circle":           ".stepper-circle",
    # Confirm modal (custom, replaces window.confirm)
    "confirm_modal":            "#confirm-modal",
    "confirm_modal_btn":        "#confirm-modal-btn",
    "confirm_modal_cancel":     "#confirm-modal-cancel-btn",
    # Channels subtab – cards
    "channels_ext_card":        "#settings-channels-content .ext-card",
    "ext_onboarding":           ".ext-onboarding",
    "ext_onboarding_title":     ".ext-onboarding-title",
    "ext_onboarding_text":      ".ext-onboarding-text",
    # Toast notifications
    "toast":                    ".toast",
    "toast_success":            ".toast.toast-success",
    "toast_error":              ".toast.toast-error",
    "toast_info":               ".toast.toast-info",
    # Jobs / missions / routines
    "jobs_tbody":               "#jobs-tbody",
    "job_row":                  "#jobs-tbody .job-row",
    "jobs_empty":               "#jobs-empty",
    "missions_summary":         "#missions-summary",
    "missions_table":           "#missions-table",
    "missions_tbody":           "#missions-tbody",
    "missions_empty":           "#missions-empty",
    "active_work_strip":        "#active-work-strip",
    "routines_tbody":           "#routines-tbody",
    "routine_row":              "#routines-tbody .routine-row",
    "routines_empty":           "#routines-empty",
    # Plan mode
    "plan_container":           ".plan-container",
    "plan_steps":               ".plan-step",
    "plan_step_completed":      '.plan-step[data-status="completed"]',
    "plan_step_pending":        '.plan-step[data-status="pending"]',
    "plan_step_running":        '.plan-step[data-status="in_progress"]',
    "plan_status_badge":        ".plan-status-badge",
    "plan_title":               ".plan-title",
    "plan_summary":             ".plan-summary",
    # Settings search
    "settings_search_input":    "#settings-search-input",
    "settings_search_empty":    ".settings-search-empty",
    # Tool permissions (Settings → Tools tab)
    "tools_tab":                "button[data-settings-subtab='tools']",
    "tool_permission_row":      ".tool-permission-row",
    "tool_permission_toggle":   ".tool-permission-toggle",
    "tool_lock_icon":           ".tool-lock-icon",
    "tool_default_badge":       ".tool-default-badge",
    # User management (Settings → Users tab)
    "users_tbody":              "#users-tbody",
    "users_tbody_row":          "#users-tbody tr",
    # Activity / tool cards (live and history)
    "activity_group":           ".activity-group",
    "activity_tool_card":       ".activity-tool-card",
    "activity_tool_name":       ".activity-tool-name",
    "activity_tool_output":     ".activity-tool-output",
    "activity_summary":         ".activity-summary",
    "activity_cards_container": ".activity-cards-container",
    "activity_tool_body":       ".activity-tool-body",
    "activity_thinking":        ".activity-thinking",
    "activity_thinking_text":   ".activity-thinking-text",
    # Thread processing indicator
    "thread_processing":        ".thread-processing",
}

TABS = ["chat", "memory", "jobs", "routines", "settings"]

# Auth token used across all tests
AUTH_TOKEN = "e2e-test-token"
OWNER_SCOPE_ID = "e2e-owner-scope"
HTTP_WEBHOOK_SECRET = "e2e-http-webhook-secret"


async def wait_for_ready(url: str, *, timeout: float = 60, interval: float = 0.5):
    """Poll a URL until it returns 200 or timeout."""
    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            try:
                resp = await client.get(url, timeout=5)
                if resp.status_code == 200:
                    return
            except (httpx.ConnectError, httpx.ReadError, httpx.TimeoutException):
                pass
            await asyncio.sleep(interval)
    raise TimeoutError(f"Service at {url} not ready after {timeout}s")


async def wait_for_port_line(process, pattern: str, *, timeout: float = 60) -> int:
    """Read process stdout line by line until a port-bearing line matches."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        try:
            line = await asyncio.wait_for(process.stdout.readline(), timeout=remaining)
        except asyncio.TimeoutError:
            break
        decoded = line.decode("utf-8", errors="replace").strip()
        if match := re.search(pattern, decoded):
            return int(match.group(1))
    raise TimeoutError(f"Port pattern '{pattern}' not found in stdout after {timeout}s")


# -- API helpers -----------------------------------------------------------

def auth_headers(token: str = AUTH_TOKEN) -> dict[str, str]:
    """Return Authorization header dict for authenticated API calls."""
    return {"Authorization": f"Bearer {token}"}


async def api_get(base_url: str, path: str, *, token: str = AUTH_TOKEN, **kwargs) -> httpx.Response:
    """Make an authenticated GET request to the ironclaw API."""
    async with httpx.AsyncClient() as client:
        return await client.get(
            f"{base_url}{path}",
            headers=auth_headers(token),
            timeout=kwargs.pop("timeout", 10),
            **kwargs,
        )


async def api_post(base_url: str, path: str, *, token: str = AUTH_TOKEN, **kwargs) -> httpx.Response:
    """Make an authenticated POST request to the ironclaw API."""
    async with httpx.AsyncClient() as client:
        return await client.post(
            f"{base_url}{path}",
            headers=auth_headers(token),
            timeout=kwargs.pop("timeout", 10),
            **kwargs,
        )


@asynccontextmanager
async def sse_stream(
    base_url: str,
    path: str = "/api/chat/events",
    *,
    token: str = AUTH_TOKEN,
    params: dict[str, str] | None = None,
    headers: dict[str, str] | None = None,
    timeout: float = 45,
):
    """Open an authenticated SSE stream and yield the aiohttp response."""
    request_headers = {
        "Accept": "text/event-stream",
        "Authorization": f"Bearer {token}",
    }
    if headers:
        request_headers.update(headers)
    client_timeout = aiohttp.ClientTimeout(total=timeout, sock_read=timeout)
    async with aiohttp.ClientSession(timeout=client_timeout) as session:
        async with session.get(
            f"{base_url}{path}",
            params=params,
            headers=request_headers,
        ) as response:
            yield response


async def wait_for_sse_line(response, *, predicate, timeout: float = 40) -> str:
    """Read SSE lines until ``predicate`` matches or the timeout expires."""
    async with asyncio.timeout(timeout):
        while True:
            line = await response.content.readline()
            if not line:
                raise AssertionError("SSE stream closed before a matching line arrived")
            decoded = line.decode("utf-8", errors="replace").rstrip("\r\n")
            if predicate(decoded):
                return decoded


async def wait_for_sse_comment(response, timeout: float = 40) -> str:
    """Wait for the next SSE keepalive/comment line."""
    return await wait_for_sse_line(
        response,
        predicate=lambda line: line.startswith(":"),
        timeout=timeout,
    )


async def create_member_user(
    base_url: str,
    *,
    display_name: str | None = None,
    email: str | None = None,
) -> dict[str, str]:
    """Create a member user through the real admin API and return credentials."""
    suffix = uuid.uuid4().hex[:8]
    payload = {
        "display_name": display_name or f"E2E Member {suffix}",
        "role": "member",
    }
    if email is not None:
        payload["email"] = email
    else:
        payload["email"] = f"e2e-member-{suffix}@example.test"

    response = await api_post(base_url, "/api/admin/users", json=payload)
    response.raise_for_status()
    body = response.json()
    return {"id": body["id"], "token": body["token"], "display_name": body["display_name"]}


async def open_authed_page(browser, base_url: str, *, token: str = AUTH_TOKEN):
    """Open a fresh authenticated page using the given bearer token query param."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{base_url}/?token={token}", wait_until="networkidle", timeout=15000)
    await page.locator(SEL["auth_screen"]).wait_for(state="hidden", timeout=10000)
    return context, page


async def ensure_writable_chat_input(page, *, timeout: int = 10000):
    """Return the chat input, switching to a fresh writable thread when needed."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=timeout)
    if await chat_input.evaluate("el => !!el.disabled"):
        await page.keyboard.press("Control+n")
        await page.wait_for_function(
            """selector => {
                const input = document.querySelector(selector);
                return !!input && !input.disabled;
            }""",
            arg=SEL["chat_input"],
            timeout=timeout,
        )
    return chat_input


async def send_chat_and_wait_for_terminal_message(
    page,
    message: str,
    *,
    timeout: int = 30000,
) -> dict[str, str]:
    """Send a chat message and wait for the next terminal visible outcome.

    Returns a dict with:
    - ``role``: ``assistant`` or ``system``
    - ``text``: rendered text of the newest terminal message
    """
    chat_input = await ensure_writable_chat_input(page)

    assistant_sel = SEL["message_assistant"]
    system_sel = SEL["message_system"]
    before_assistant = await page.locator(assistant_sel).count()
    before_system = await page.locator(system_sel).count()

    await chat_input.fill(message)
    await chat_input.press("Enter")

    handle = await page.wait_for_function(
        """({
            assistantSelector,
            systemSelector,
            chatInputSelector,
            assistantCount,
            systemCount,
        }) => {
            const input = document.querySelector(chatInputSelector);
            const systems = document.querySelectorAll(systemSelector);
            if (systems.length > systemCount) {
                const last = systems[systems.length - 1];
                const content = last.querySelector('.message-content');
                return {
                    role: 'system',
                    text: ((content && content.innerText) || last.innerText || '').trim(),
                };
            }

            const assistants = document.querySelectorAll(assistantSelector);
            if (assistants.length > assistantCount && input && !input.disabled) {
                const last = assistants[assistants.length - 1];
                const content = last.querySelector('.message-content');
                const text = ((content && content.innerText) || last.innerText || '').trim();
                if (text.length > 0 && !last.hasAttribute('data-streaming')) {
                    return {
                        role: 'assistant',
                        text,
                    };
                }
            }

            return null;
        }""",
        arg={
            "assistantSelector": assistant_sel,
            "systemSelector": system_sel,
            "chatInputSelector": SEL["chat_input"],
            "assistantCount": before_assistant,
            "systemCount": before_system,
        },
        timeout=timeout,
    )
    return await handle.json_value()


def signed_http_webhook_headers(body: bytes) -> dict[str, str]:
    """Return headers for the owner-scoped HTTP webhook channel."""
    digest = hmac.new(
        HTTP_WEBHOOK_SECRET.encode("utf-8"),
        body,
        hashlib.sha256,
    ).hexdigest()
    return {
        "Content-Type": "application/json",
        "X-Hub-Signature-256": f"sha256={digest}",
    }
