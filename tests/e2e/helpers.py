"""Shared helpers for E2E tests."""

import asyncio
import re
import time
from contextlib import asynccontextmanager

import aiohttp
import httpx

EMULATE_GOOGLE_BEARER = "mock-refreshed-access-token"
EMULATE_GOOGLE_SECONDARY_BEARER = "emulate-google-secondary-token"
EMULATE_SLACK_BEARER = "emulate-slack-token"
EMULATE_SLACK_LIMITED_BEARER = "emulate-slack-limited-token"
EMULATE_GITHUB_BEARER = "ghp_emulate_github_token"
EMULATE_GITHUB_SECONDARY_BEARER = "ghp_emulate_github_secondary_token"

# Bearer token for the canonical WebUI surface (`ironclaw serve`).
# Must be >= 32 bytes because `serve` also uses it as the SSO session-signing
# key.
REBORN_V2_AUTH_TOKEN = "e2e-reborn-v2-bearer-token-0123456789abcdef"

# Selectors for the React SPA served at `/`.
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
    "toast": "[data-testid='toast']",
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


@asynccontextmanager
async def sse_stream(
    base_url: str,
    path: str,
    *,
    token: str = REBORN_V2_AUTH_TOKEN,
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
