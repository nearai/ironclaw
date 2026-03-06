"""Shared helpers for E2E tests."""

import asyncio
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
}

TABS = ["chat", "memory", "jobs", "routines", "extensions", "skills"]

# Auth token used across all tests
AUTH_TOKEN = "e2e-test-token"


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
