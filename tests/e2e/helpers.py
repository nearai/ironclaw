"""Shared helpers for E2E tests."""

import asyncio
import hashlib
import hmac
import os
import re
import signal
import socket
import time
import uuid
from contextlib import asynccontextmanager
from typing import AsyncIterator

import pytest

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
    # Projects control-room
    "projects_cards":           "#cr-cards",
    "projects_card":            ".cr-card",
    "projects_card_by_id":      '.cr-card[data-id="{id}"]',
    "projects_drill":           "#cr-drill",
    "projects_drill_name":      ".cr-drill-name",
    "projects_detail":          "#cr-detail",
    "projects_mission_card":    ".cr-mission-card",
    "projects_activity_row":    ".cr-activity-row",
    "projects_activity_row_by_id": '.cr-activity-row[data-id="{id}"]',
    "projects_thread_title":    ".cr-thread-title",
    "projects_thread_subtitle": ".cr-thread-subtitle",
    "projects_thread_brief":    ".cr-thread-brief",
    "projects_thread_meta":     ".cr-thread-meta-grid",
    "projects_thread_timeline": ".cr-thread-timeline",
    "projects_thread_message":  ".cr-thread-message",
    # Canonical Missions detail surface
    "missions_detail":          "#mission-detail",
    "missions_detail_title":    ".ms-detail-title",
}

TABS = ["chat", "memory", "jobs", "routines", "settings"]

# Auth token used across all tests
AUTH_TOKEN = "e2e-test-token"
OWNER_SCOPE_ID = "e2e-owner-scope"
HTTP_WEBHOOK_SECRET = "e2e-http-webhook-secret"
EMULATE_GOOGLE_BEARER = "mock-refreshed-access-token"
EMULATE_GOOGLE_SECONDARY_BEARER = "emulate-google-secondary-token"
EMULATE_SLACK_BEARER = "emulate-slack-token"
EMULATE_SLACK_LIMITED_BEARER = "emulate-slack-limited-token"
EMULATE_GITHUB_BEARER = "ghp_emulate_github_token"
EMULATE_GITHUB_SECONDARY_BEARER = "ghp_emulate_github_secondary_token"

# Bearer token for the Reborn WebUI v2 surface (`ironclaw-reborn serve`).
# Must be >= 32 bytes: `serve` also uses this value as the SSO session-signing
# key and refuses to bind with a shorter secret. Distinct from AUTH_TOKEN,
# which targets the legacy `ironclaw` web channel.
REBORN_V2_AUTH_TOKEN = "e2e-reborn-v2-bearer-token-0123456789abcdef"

# Selectors for the Reborn WebUI v2 React SPA (served at /). The shell
# DOM differs entirely from the legacy gateway in SEL, so keep these separate.
SEL_V2 = {
    "root":           "#v2-root",          # SPA mount point (index.html)
    "login_token":    "#v2-token",         # token input on the login/connect view
    "admin_new_user_button_name": "New user",
    "admin_create_form": "form",
    "admin_display_name_input": 'input[type="text"]',
    "admin_email_input": 'input[type="email"]',
    "admin_create_user_button_name": "Create user",
    "admin_token_created_text": "Token created",
    "admin_token_value": "code",
    "admin_token_description_text": "Copy this now — it will not be shown again.",
    "admin_create_token_button_name": "Create token",
    "admin_current_role_button_name": "Current role",
    "admin_save_role_button_name": "Save role",
    "admin_member_role_name": "Member",
    "admin_admin_role_name": "Admin",
    "admin_active_status_name": "Active",
    "admin_suspended_status_name": "Suspended",
    "admin_suspend_button_name": "Suspend",
    "admin_activate_button_name": "Activate",
    "admin_user_secrets_panel": "[data-testid='admin-user-secrets-panel']",
    "admin_secret_handle_input": "[data-testid='admin-secret-handle']",
    "admin_secret_value_input": "[data-testid='admin-secret-value']",
    "admin_secret_save": "[data-testid='admin-secret-save']",
    "admin_secret_status": "[data-testid='admin-secret-status']",
    "admin_secret_row_for": (
        "[data-testid='admin-secret-row'][data-secret-handle='{handle}']"
    ),
    "admin_secret_replace_for": (
        "[data-testid='admin-secret-replace'][data-secret-handle='{handle}']"
    ),
    "admin_secret_delete_for": (
        "[data-testid='admin-secret-delete'][data-secret-handle='{handle}']"
    ),
    "admin_secret_delete_dialog": "[data-testid='admin-secret-delete-dialog']",
    "admin_secret_delete_confirm": "[data-testid='admin-secret-delete-confirm']",
    "sidebar":        "#gateway-sidebar",  # app navigation sidebar
    "sidebar_button": "#gateway-sidebar button",
    "nav_workspace": "[data-testid='nav-workspace']",
    "workspace_heading": "[data-testid='workspace-heading']",
    "workspace_download": "[data-testid='workspace-download']",
    "workspace_directory_entry_for": (
        "[data-testid='workspace-directory-entry'][data-entry-path='{path}']"
    ),
    "workspace_tree_entry": "[data-testid='workspace-tree-entry']",
    "thread_delete_for": (
        '[data-testid="thread-delete"][data-thread-id="{id}"]'
    ),
    "confirm_dialog_cancel": '[data-testid="confirm-dialog-cancel"]',
    "confirm_dialog_confirm": '[data-testid="confirm-dialog-confirm"]',
    "sidebar_toggle": "button[aria-label='Toggle sidebar']",
    "sign_out_button": "button[title='Sign out']",
    "appearance_theme_light": "[data-testid='appearance-theme-light']",
    "appearance_theme_dark": "[data-testid='appearance-theme-dark']",
    "chat_composer":  "[data-testid='chat-composer']",  # message textarea on /chat
    "attachment_file_input": "input[type=file][multiple]",
    "typing_indicator": "[data-testid='typing-indicator']",
    "connection_status": "[data-testid='connection-status']",
    "connection_status_toggle": "[data-testid='connection-status-toggle']",
    "connection_status_label": "[data-testid='connection-status-label']",
    "msg_user":       "[data-testid='msg-user']",       # user message bubble
    "msg_assistant":  "[data-testid='msg-assistant']",  # assistant message bubble
    "msg_system":     "[data-testid='msg-system']",     # system notice bubble
    "msg_error":      "[data-testid='msg-error']",
    "message_copy_button": "button[title]",
    "message_list_scroll": "[data-testid='message-list-scroll']",
    "message_list_content": "[data-testid='message-list-content']",
    "message_list_load_older": "[data-testid='message-list-load-older']",
    "notification_bell": "[data-testid='notification-bell']",
    "notification_panel": "[data-testid='notification-panel']",
    "notification_row": "[data-testid='notification-row']",
    "notification_unread_dot": "[data-testid='notification-unread-dot']",
    "toast": "[data-testid='toast']",
    "toast_dismiss": "[data-testid='toast-dismiss']",
    "toast_viewport": "[data-rht-toaster]",
    "header_logs_link": "[data-testid='header-logs-link']",
    "header_docs_link": "[data-testid='header-docs-link']",
    "command_palette_dialog_name": "Command palette",
    "command_palette_search_placeholder": "Type a command or search",
    "command_palette_go_settings_name": "Go to Settings",
    "auth_gate":      "[data-testid='auth-gate']",
    "auth_gate_for":  "[data-testid='auth-gate'][data-auth-challenge='{kind}']",
    "auth_token_input": "[data-testid='auth-token-input']",
    "auth_oauth_open": "[data-testid='auth-oauth-open']",
    "channel_connect_card": "[data-testid='channel-connect-card']",
    "channel_connect_card_for": (
        "[data-testid='channel-connect-card'][data-channel='{channel}']"
        "[data-strategy='{strategy}']"
    ),
    "channel_connect_dismiss": "[data-testid='channel-connect-dismiss']",
    "extension_card_for": (
        "[data-testid='extension-card'][data-extension-id='{id}']"
    ),
    "pairing_section": "[data-testid='pairing-section']",
    "pairing_code_input": "[data-testid='pairing-code-input']",
    "pairing_submit": "[data-testid='pairing-submit']",
    "pairing_success": "[data-testid='pairing-success']",
    "pairing_error": "[data-testid='pairing-error']",
    "approval_card":  "[data-testid='approval-card']",  # approval gate card
    "busy_gate_notice": "[data-testid='busy-gate-notice']",  # gate busy notice
    "activity_run":   "[data-testid='activity-run']",
    "activity_run_toggle": "[data-testid='activity-run-toggle']",
    "activity_run_items": "[data-testid='activity-run-items']",
    "tool_activity_card": "[data-testid='tool-activity-card']",
    "tool_activity_card_for": "[data-testid='tool-activity-card'][data-tool-name='{name}']",
    "tool_activity_toggle": "[data-testid='tool-activity-toggle']",
    "tool_activity_detail": "[data-testid='tool-activity-detail']",
    "projects_grid": "[data-testid='projects-grid']",
    "projects_search_input": "[data-testid='projects-search-input']",
    "project_card": "[data-testid='project-card']",
    "project_card_for": "[data-testid='project-card'][data-project-id='{id}']",
    "project_open_workspace": "[data-testid='project-open-workspace']",
    "project_workspace": "[data-testid='project-workspace']",
    "project_workspace_for": "[data-testid='project-workspace'][data-project-id='{id}']",
    "project_workspace_title": "[data-testid='project-workspace-title']",
    "project_filesystem_entry_for": (
        "[data-testid='project-filesystem-entry'][data-entry-path='{path}']"
    ),
    # Download chip for an agent-produced workspace file; `{path}` selects one.
    # Clicking a chip opens the shared attachment preview modal, whose footer
    # carries the Download action.
    "project_file_chip": "[data-testid='project-file-chip']",
    "project_file_chip_for": "[data-testid='project-file-chip'][data-file-path='{path}']",
    # Inline one-click download icon on a project-file chip; `{path}` scopes it
    # to the chip's adjacent sibling so each chip's download is addressable.
    "project_file_download_for": (
        "[data-testid='project-file-chip'][data-file-path='{path}'] "
        "+ [data-testid='project-file-download']"
    ),
    # Download action inside the shared attachment preview modal.
    "attachment_download": "[data-testid='attachment-download']",
    "logs_scope_toolbar": "[data-testid='logs-scope-toolbar']",
    "logs_scope_chip": "[data-testid='logs-scope-chip'][data-scope-key='{key}']",
    "logs_entry": "[data-testid='logs-entry']",
    "logs_entry_row": "[data-testid='logs-entry-row']",
    "logs_entry_message": "[data-testid='logs-entry-message']",
    "logs_entry_context": "[data-testid='logs-entry-context']",
    "logs_context_chip": "[data-testid='logs-context-chip'][data-context-key='{key}']",
    "settings_search_placeholder": "Search settings...",
    "settings_import_file": 'input[type="file"][accept=".json,application/json"]',
    "settings_tool_row_for": (
        "[data-testid='settings-tool-row'][data-tool-name='{name}']"
    ),
    "settings_tool_permission": (
        "[data-testid='settings-tool-permission-select'] button[aria-haspopup='listbox']"
    ),
    "settings_tool_lock": "[data-testid='settings-tool-lock']",
    "skill_action_result": "[data-testid='skill-action-result']",
    "llm_provider_card_for": (
        "[data-testid='llm-provider-card'][data-provider-id='{provider_id}']"
    ),
    "llm_provider_disclosure": "llm-provider-disclosure",
    "automation_row_for": (
        "[data-testid='automation-row'][data-automation-id='{id}']"
    ),
    "automation_name_button_for": (
        "[data-testid='automation-name-button'][data-automation-id='{id}']"
    ),
    "automation_detail": "[data-testid='automation-detail-panel']",
    "automation_detail_title": "[data-testid='automation-detail-title']",
    "automation_rename_button": "[data-testid='automation-rename-button']",
    "automation_rename_input": "[data-testid='automation-rename-input']",
    "automation_rename_save": "[data-testid='automation-rename-save']",
    "automation_run_open": "[data-testid='automation-run-open']",
    "automation_run_logs": "[data-testid='automation-run-logs']",
    "skills_card": "#skills-list .ext-card",
    "skill_name_placeholder": "skill-name",
    "skill_content_placeholder": "---\\nname: example\\ndescription: ...\\n---\\n",
}


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
    await page.goto(f"{base_url}/?token={token}", timeout=15000)
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
    expected_text_contains: str | None = None,
) -> dict[str, str]:
    """Send a chat message and wait for the next terminal visible outcome.

    Returns a dict with:
    - ``role``: ``assistant`` or ``system``
    - ``text``: rendered text of the newest terminal message

    The default predicate waits for the assistant message to fully settle —
    ``data-streaming`` attribute cleared, input re-enabled, and this send's
    optimistic pending marker removed. On slow CI runners under heavy
    parallelism the streaming condition can race with SSE reconnects (chunks
    arrive during the reconnect window, the attribute-clearing delta is lost,
    predicate never flips). Callers that already assert on specific response
    text can pass ``expected_text_contains=`` to short-circuit as soon as that
    substring appears in the assistant bubble, but the helper still waits for
    this send's pending marker to clear so late history renders from previous
    turns cannot be mistaken for the new response.
    """
    chat_input = await ensure_writable_chat_input(page)

    # The page fixture waits for auth/SSE readiness, but the first
    # loadHistory() call may still be replacing the skeleton with older
    # persisted messages. Count terminal bubbles only after that settles;
    # otherwise a late history-rendered assistant from a previous turn can
    # satisfy the "new assistant" predicate for the message we are about to
    # send.
    await page.wait_for_function(
        """() => !document.querySelector('#chat-messages .skeleton-container')""",
        timeout=timeout,
    )

    pending_marker = await page.evaluate(
        """() => ({
            threadId: typeof currentThreadId !== 'undefined' ? currentThreadId : null,
            pendingId: typeof _nextPendingId !== 'undefined' ? _nextPendingId : null,
        })"""
    )

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
            expectedContains,
            pendingThreadId,
            pendingId,
        }) => {
            const pendingForThisSendCleared = () => {
                if (!pendingThreadId || pendingId === null || pendingId === undefined) return true;
                if (typeof _pendingUserMessages === 'undefined') return true;
                const pending = _pendingUserMessages.get(pendingThreadId);
                return !pending || !pending.some((p) => p.id === pendingId);
            };

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
            if (assistants.length > assistantCount) {
                const last = assistants[assistants.length - 1];
                const content = last.querySelector('.message-content');
                const text = ((content && content.innerText) || last.innerText || '').trim();
                if (text.length > 0 && pendingForThisSendCleared()) {
                    if (expectedContains && text.includes(expectedContains)) {
                        return { role: 'assistant', text };
                    }
                    if (!expectedContains
                        && input && !input.disabled
                        && !last.hasAttribute('data-streaming')) {
                        return { role: 'assistant', text };
                    }
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
            "expectedContains": expected_text_contains,
            "pendingThreadId": pending_marker["threadId"],
            "pendingId": pending_marker["pendingId"],
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


# ---------------------------------------------------------------------------
# Process helpers (shared across v2 auth E2E scenario files)
# ---------------------------------------------------------------------------

def _forward_coverage_env(env: dict) -> None:
    """Copy LLVM coverage env vars into *env* so coverage data is collected."""
    for key in os.environ:
        if key.startswith(
            ("CARGO_LLVM_COV", "LLVM_", "CARGO_ENCODED_RUSTFLAGS", "CARGO_INCREMENTAL")
        ):
            env[key] = os.environ[key]


async def stop_process(proc, sig: int = signal.SIGINT, timeout: float = 5) -> None:
    """Gracefully stop *proc*, escalating to SIGKILL on timeout."""
    async def _drain() -> None:
        try:
            await asyncio.wait_for(proc.communicate(), timeout=1)
        except asyncio.TimeoutError:
            # Release transports now rather than waiting for GC.
            for pipe in (proc.stdout, proc.stderr):
                if pipe is not None:
                    try:
                        pipe.feed_eof()
                    except Exception:
                        pass
        except ValueError:
            pass

    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        await _drain()
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()
    await _drain()


async def wait_for_pending_auth_gate(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Poll /api/chat/history until a pending *authentication* gate appears.

    Returns the gate dict.  Raises ``AssertionError`` on timeout.
    The gate kind is validated so that approval / confirmation gates do not
    satisfy the check and produce false positives.
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        r = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15)
        if r.status_code == 200:
            gate = r.json().get("pending_gate")
            if isinstance(gate, dict) and gate.get("request_id"):
                resume = gate.get("resume_kind")
                if not isinstance(resume, dict):
                    resume = {}
                if (
                    resume.get("kind") == "authentication"
                    or gate.get("gate_name") in ("auth", "authentication")
                ):
                    return gate
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for pending auth gate in thread {thread_id}"
    )


def _reserve_loopback_port() -> int:
    """Return an available loopback port for a short-lived E2E server."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


_ENGINE_V2_BASE_ENV = {
    "RUST_LOG": "ironclaw=debug",
    "RUST_BACKTRACE": "1",
    "ENGINE_V2": "true",
    "AGENT_AUTO_APPROVE_TOOLS": "true",
    "HTTP_ALLOW_LOCALHOST": "true",
    "SECRETS_MASTER_KEY": hashlib.sha256(b"ironclaw-4112-e2e-master-key").hexdigest(),
    "GATEWAY_ENABLED": "true",
    "GATEWAY_HOST": "127.0.0.1",
    "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
    "CLI_ENABLED": "false",
    "LLM_BACKEND": "openai_compatible",
    "LLM_API_KEY": "mock-api-key",
    "LLM_MODEL": "mock-model",
    "DATABASE_BACKEND": "libsql",
    "SANDBOX_ENABLED": "false",
    "SKILLS_ENABLED": "true",
    "ROUTINES_ENABLED": "false",
    "HEARTBEAT_ENABLED": "false",
    "EMBEDDING_ENABLED": "false",
    "WASM_ENABLED": "false",
    "ONBOARD_COMPLETED": "true",
    "IRONCLAW_DISABLE_OS_KEYCHAIN": "1",
}


@asynccontextmanager
async def _start_engine_v2_server(
    ironclaw_binary: str,
    *,
    mock_llm_server: str,
    port: int,
    home_dir: str,
    db_path: str,
    user_id: str,
    label: str,
    env_overrides: dict[str, str] | None = None,
) -> AsyncIterator[str]:
    """Start an ENGINE_V2 ironclaw process and yield its base URL.

    Centralises the subprocess-lifecycle boilerplate shared across all three
    v2 auth E2E server fixtures.
    """
    env: dict[str, str] = {
        **_ENGINE_V2_BASE_ENV,
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "GATEWAY_PORT": str(port),
        "GATEWAY_USER_ID": user_id,
        "IRONCLAW_OWNER_ID": user_id,
        "LLM_BASE_URL": mock_llm_server,
        "LIBSQL_PATH": db_path,
    }
    if env_overrides:
        env.update(env_overrides)
    _forward_coverage_env(env)

    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary, "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.DEVNULL,
        stderr=asyncio.subprocess.DEVNULL,
        env=env,
    )
    base_url = f"http://127.0.0.1:{port}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=60)
        yield base_url
    except TimeoutError:
        if proc.returncode is None:
            await stop_process(proc, timeout=2)
        pytest.fail(f"{label} failed to start on port {port}")
    finally:
        if proc.returncode is None:
            await stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await stop_process(proc, sig=signal.SIGTERM, timeout=5)
