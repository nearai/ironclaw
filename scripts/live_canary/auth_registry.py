from __future__ import annotations

from dataclasses import dataclass, replace

from scripts.live_canary.common import CanaryError, env_str, required_env


AUTH_SMOKE_TESTS = [
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_oauth_roundtrip",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_oauth_roundtrip",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_oauth_roundtrip_via_browser",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_same_server_multi_user_via_browser",
]

AUTH_FULL_TESTS = AUTH_SMOKE_TESTS + [
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_oauth_provider_error_leaves_extension_unauthed",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_oauth_exchange_failure_leaves_extension_unauthed",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_first_chat_auth_attempt_emits_auth_url",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_chat_first_gmail_installs_prompts_and_retries",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_settings_first_gmail_auth_then_chat_runs",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_settings_first_custom_mcp_auth_then_chat_runs",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_oauth_refresh_on_demand",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_oauth_refresh_on_demand",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_oauth_refresh_on_start",
]

AUTH_CHANNEL_TESTS = [
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_channel_oauth_roundtrip",
]

AUTH_PROFILES: dict[str, list[str]] = {
    "smoke": AUTH_SMOKE_TESTS,
    "full": AUTH_FULL_TESTS,
    "channels": AUTH_CHANNEL_TESTS,
}


@dataclass(frozen=True)
class ExtensionInstall:
    name: str
    expected_display_name: str
    install_kind: str | None = None
    install_url: str | None = None


@dataclass(frozen=True)
class SeededProviderCase:
    key: str
    extension_install_name: str
    expected_display_name: str
    response_prompt: str
    expected_tool_name: str
    expected_text: str
    browser_enabled: bool = False
    install_kind: str | None = None
    install_url: str | None = None
    shared_secret_name: str | None = None
    requires_refresh_seed: bool = False
    extra_installations: tuple[ExtensionInstall, ...] = ()
    expected_tool_names: tuple[str, ...] = ()

    @property
    def installations(self) -> tuple[ExtensionInstall, ...]:
        return (
            ExtensionInstall(
                name=self.extension_install_name,
                expected_display_name=self.expected_display_name,
                install_kind=self.install_kind,
                install_url=self.install_url,
            ),
            *self.extra_installations,
        )

    @property
    def required_tool_names(self) -> tuple[str, ...]:
        return self.expected_tool_names or (self.expected_tool_name,)


@dataclass(frozen=True)
class BrowserProviderCase:
    key: str
    extension_name: str
    expected_extension_name: str
    install_kind: str | None
    install_url: str | None
    trigger_prompt: str
    expected_tool_name: str
    expected_text: str
    auth_extension_name: str | None = None


SEEDED_CASES: dict[str, SeededProviderCase] = {
    "gmail": SeededProviderCase(
        key="gmail",
        extension_install_name="gmail",
        expected_display_name="Gmail",
        response_prompt="check gmail unread",
        expected_tool_name="gmail",
        expected_text="Gmail",
        browser_enabled=True,
        shared_secret_name="google_oauth_token",
        requires_refresh_seed=True,
    ),
    "google_calendar": SeededProviderCase(
        key="google_calendar",
        extension_install_name="google_calendar",
        expected_display_name="Google Calendar",
        response_prompt="list next calendar event",
        expected_tool_name="google_calendar",
        expected_text="Calendar check completed successfully.",
        shared_secret_name="google_oauth_token",
    ),
    "github": SeededProviderCase(
        key="github",
        extension_install_name="github",
        expected_display_name="GitHub",
        response_prompt="read github issue owner/repo#1",
        expected_tool_name="github",
        expected_text="GitHub issue lookup completed successfully.",
        browser_enabled=True,
        shared_secret_name="github_token",
    ),
    "notion": SeededProviderCase(
        key="notion",
        extension_install_name="notion",
        expected_display_name="Notion",
        response_prompt="search notion for canary",
        expected_tool_name="notion_notion_search",
        expected_text="Notion search completed successfully.",
        install_kind="mcp_server",
    ),
    "linear": SeededProviderCase(
        key="linear",
        extension_install_name="linear",
        expected_display_name="Linear",
        response_prompt="search linear for canary",
        expected_tool_name="linear_search_issues",
        expected_text="Linear search completed successfully.",
        install_kind="mcp_server",
    ),
    "ops_workflow": SeededProviderCase(
        key="ops_workflow",
        extension_install_name="gmail",
        expected_display_name="Gmail",
        response_prompt="run auth ops workflow canary",
        expected_tool_name="gmail",
        expected_text="",
        extra_installations=(
            ExtensionInstall("google_calendar", "Google Calendar"),
            ExtensionInstall("google_drive", "Google Drive"),
            ExtensionInstall("google_docs", "Google Docs"),
            ExtensionInstall("google_sheets", "Google Sheets"),
            ExtensionInstall("google_slides", "Google Slides"),
            ExtensionInstall("github", "GitHub"),
            ExtensionInstall("web_search", "Web Search"),
            ExtensionInstall("llm_context", "LLM Context"),
            ExtensionInstall("slack_tool", "Slack Tool"),
            ExtensionInstall("telegram_mtproto", "Telegram Tool"),
            ExtensionInstall("composio", "Composio"),
            ExtensionInstall("notion", "Notion", install_kind="mcp_server"),
            ExtensionInstall("linear", "Linear", install_kind="mcp_server"),
        ),
        expected_tool_names=(
            "gmail",
            "google_calendar",
            "google_drive",
            "google_docs",
            "google_sheets",
            "google_slides",
            "github",
            "web_search",
            "llm_context",
            "slack_tool",
            "telegram_mtproto",
            "composio",
            "notion_notion_search",
            "linear_search_issues",
        ),
    ),
}


OPS_WORKFLOW_REQUIRED_ENVS = (
    "AUTH_LIVE_GOOGLE_ACCESS_TOKEN",
    "AUTH_LIVE_GOOGLE_DOC_ID",
    "AUTH_LIVE_GOOGLE_SHEET_ID",
    "AUTH_LIVE_GOOGLE_SLIDES_ID",
    "AUTH_LIVE_GITHUB_TOKEN",
    "AUTH_LIVE_GITHUB_OWNER",
    "AUTH_LIVE_GITHUB_REPO",
    "AUTH_LIVE_GITHUB_ISSUE_NUMBER",
    "AUTH_LIVE_BRAVE_API_KEY",
    "AUTH_LIVE_SLACK_BOT_TOKEN",
    "AUTH_LIVE_COMPOSIO_API_KEY",
    "AUTH_LIVE_TELEGRAM_API_ID",
    "AUTH_LIVE_TELEGRAM_API_HASH",
    "AUTH_LIVE_TELEGRAM_SESSION_JSON",
    "AUTH_LIVE_NOTION_ACCESS_TOKEN",
    "AUTH_LIVE_NOTION_QUERY",
    "AUTH_LIVE_LINEAR_ACCESS_TOKEN",
    "AUTH_LIVE_LINEAR_QUERY",
)


BROWSER_CASES: dict[str, BrowserProviderCase] = {
    "google": BrowserProviderCase(
        key="google",
        extension_name="gmail",
        expected_extension_name="gmail",
        install_kind=None,
        install_url=None,
        trigger_prompt="check gmail unread",
        expected_tool_name="gmail",
        expected_text="Gmail",
        auth_extension_name="gmail",
    ),
    "github": BrowserProviderCase(
        key="github",
        extension_name="github",
        expected_extension_name="github",
        install_kind=None,
        install_url=None,
        trigger_prompt="read github issue owner/repo#1",
        expected_tool_name="github",
        expected_text="GitHub issue lookup completed successfully.",
        auth_extension_name="github",
    ),
    "notion": BrowserProviderCase(
        key="notion",
        extension_name="notion",
        expected_extension_name="notion",
        install_kind="mcp_server",
        install_url=None,
        trigger_prompt="search notion for canary",
        expected_tool_name="notion_notion_search",
        expected_text="Notion search completed successfully.",
        auth_extension_name="notion",
    ),
}


def configured_seeded_cases(selected: list[str] | None) -> list[SeededProviderCase]:
    cases: list[SeededProviderCase] = []
    names = selected or list(SEEDED_CASES)
    google_access = env_str("AUTH_LIVE_GOOGLE_ACCESS_TOKEN")
    google_refresh = env_str("AUTH_LIVE_GOOGLE_REFRESH_TOKEN")
    if google_refresh and not google_access:
        raise CanaryError(
            "AUTH_LIVE_GOOGLE_ACCESS_TOKEN is required when AUTH_LIVE_GOOGLE_REFRESH_TOKEN is set"
        )

    for name in names:
        case = SEEDED_CASES[name]
        if name in {"gmail", "google_calendar"}:
            if not google_access:
                continue
            if name == "gmail":
                case = replace(case, requires_refresh_seed=bool(google_refresh))
        elif name == "github":
            if not env_str("AUTH_LIVE_GITHUB_TOKEN"):
                continue
            owner = required_env(
                "AUTH_LIVE_GITHUB_OWNER",
                message="AUTH_LIVE_GITHUB_OWNER is required for the selected live-provider case",
            )
            repo = required_env(
                "AUTH_LIVE_GITHUB_REPO",
                message="AUTH_LIVE_GITHUB_REPO is required for the selected live-provider case",
            )
            issue_number = required_env(
                "AUTH_LIVE_GITHUB_ISSUE_NUMBER",
                message="AUTH_LIVE_GITHUB_ISSUE_NUMBER is required for the selected live-provider case",
            )
            case = replace(case, response_prompt=f"read github issue {owner}/{repo}#{issue_number}")
        elif name == "notion":
            if not env_str("AUTH_LIVE_NOTION_ACCESS_TOKEN"):
                continue
            query = required_env(
                "AUTH_LIVE_NOTION_QUERY",
                message="AUTH_LIVE_NOTION_QUERY is required for the selected live-provider case",
            )
            case = replace(case, response_prompt=f"search notion for {query}")
        elif name == "linear":
            if not env_str("AUTH_LIVE_LINEAR_ACCESS_TOKEN"):
                continue
            query = required_env(
                "AUTH_LIVE_LINEAR_QUERY",
                message="AUTH_LIVE_LINEAR_QUERY is required for the selected live-provider case",
            )
            tool_name = env_str("AUTH_LIVE_LINEAR_TOOL_NAME", "linear_search_issues")
            case = replace(
                case,
                response_prompt=f"search linear for {query}",
                expected_tool_name=tool_name,
                expected_tool_names=(tool_name,),
            )
        elif name == "ops_workflow":
            missing = [env_name for env_name in OPS_WORKFLOW_REQUIRED_ENVS if not env_str(env_name)]
            if missing:
                if selected is not None:
                    raise CanaryError(
                        "ops_workflow requires all fixture env vars; missing: "
                        + ", ".join(missing)
                    )
                continue
            linear_tool = env_str("AUTH_LIVE_LINEAR_TOOL_NAME", "linear_search_issues")
            case = replace(
                case,
                expected_tool_names=tuple(
                    linear_tool if tool == "linear_search_issues" else tool
                    for tool in case.expected_tool_names
                ),
            )
        cases.append(case)
    return cases


def configured_browser_cases(selected: list[str] | None) -> list[BrowserProviderCase]:
    cases: list[BrowserProviderCase] = []
    names = selected or list(BROWSER_CASES)
    for name in names:
        case = BROWSER_CASES[name]
        if name == "github":
            if not env_str("GITHUB_OAUTH_CLIENT_ID") or not env_str("GITHUB_OAUTH_CLIENT_SECRET"):
                continue
            owner = env_str("AUTH_BROWSER_GITHUB_OWNER")
            repo = env_str("AUTH_BROWSER_GITHUB_REPO")
            issue_number = env_str("AUTH_BROWSER_GITHUB_ISSUE_NUMBER")
            if not owner or not repo or not issue_number:
                continue
            case = replace(case, trigger_prompt=f"read github issue {owner}/{repo}#{issue_number}")
        if env_str(f"AUTH_BROWSER_{name.upper()}_STORAGE_STATE_PATH") or env_str(
            f"AUTH_BROWSER_{name.upper()}_USERNAME"
        ):
            cases.append(case)
    return cases
