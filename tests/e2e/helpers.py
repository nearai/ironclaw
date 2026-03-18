"""Shared helpers for E2E tests."""

import asyncio
import hashlib
import hmac
import re
import time

import httpx

# -- DOM Selectors --------------------------------------------------------
# Keep all selectors in one place so changes to the frontend only need
# one update.

SEL = {
    # Auth
    "auth_screen": "#auth-screen",
    "token_input": "#token-input",
    # Connection
    "sse_status": "#sse-status",
    # Tabs
    "tab_button": '.tab-bar button[data-tab="{tab}"]',
    "tab_panel": "#tab-{tab}",
    # Chat
    "chat_input": "#chat-input",
    "chat_messages": "#chat-messages",
    "message_user": "#chat-messages .message.user",
    "message_assistant": "#chat-messages .message.assistant",
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
    # Extensions tab – sections
    "extensions_list":          "#extensions-list",
    "available_wasm_list":      "#available-wasm-list",
    "mcp_servers_list":         "#mcp-servers-list",
    "tools_tbody":              "#tools-tbody",
    "tools_empty":              "#tools-empty",
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
    "ext_error":                ".ext-error",
    "ext_tools":                ".ext-tools",
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
    # WASM channel progress stepper
    "ext_stepper":              ".ext-stepper",
    "stepper_step":             ".stepper-step",
    "stepper_circle":           ".stepper-circle",
    # Toast notifications
    "toast":                    ".toast",
    "toast_success":            ".toast.toast-success",
    "toast_error":              ".toast.toast-error",
    "toast_info":               ".toast.toast-info",
    # Jobs / routines
    "jobs_tbody":               "#jobs-tbody",
    "job_row":                  "#jobs-tbody .job-row",
    "jobs_empty":               "#jobs-empty",
    "routines_tbody":           "#routines-tbody",
    "routine_row":              "#routines-tbody .routine-row",
    "routines_empty":           "#routines-empty",
}

TABS = ["chat", "memory", "jobs", "routines", "extensions", "skills"]

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

def auth_headers() -> dict[str, str]:
    """Return Authorization header dict for authenticated API calls."""
    return {"Authorization": f"Bearer {AUTH_TOKEN}"}


async def api_get(base_url: str, path: str, **kwargs) -> httpx.Response:
    """Make an authenticated GET request to the ironclaw API."""
    async with httpx.AsyncClient() as client:
        return await client.get(
            f"{base_url}{path}",
            headers=auth_headers(),
            timeout=kwargs.pop("timeout", 10),
            **kwargs,
        )


async def api_post(base_url: str, path: str, **kwargs) -> httpx.Response:
    """Make an authenticated POST request to the ironclaw API."""
    async with httpx.AsyncClient() as client:
        return await client.post(
            f"{base_url}{path}",
            headers=auth_headers(),
            timeout=kwargs.pop("timeout", 10),
            **kwargs,
        )


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
