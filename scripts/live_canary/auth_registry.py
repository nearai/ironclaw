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


# Lifecycle ("write + cleanup") cases exercise real provider mutations —
# they send emails, create calendar events, etc., and then clean up.
# Even though each flow is self-cleaning, repeated hourly runs against
# real accounts are not "low-risk / read-only" and must not be the
# default selection for the scheduled lane. Callers must opt in by
# naming these cases explicitly (e.g. `CASES=gmail_roundtrip` or
# `--case gmail_roundtrip`) — see `configured_seeded_cases` below.
LIFECYCLE_CASE_NAMES: frozenset[str] = frozenset(
    {
        "gmail_roundtrip",
        "google_calendar_lifecycle",
        "notion_search_lifecycle",
    }
)


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
        expected_text="google_calendar",
        shared_secret_name="google_oauth_token",
    ),
    "github": SeededProviderCase(
        key="github",
        extension_install_name="github",
        expected_display_name="GitHub",
        response_prompt="read github issue owner/repo#1",
        expected_tool_name="github",
        expected_text="github",
        browser_enabled=True,
        shared_secret_name="github_token",
    ),
    "notion": SeededProviderCase(
        key="notion",
        extension_install_name="notion",
        expected_display_name="Notion",
        response_prompt="search notion for canary",
        expected_tool_name="notion_notion_search",
        expected_text="notion",
        install_kind="mcp_server",
    ),
    # ── Lifecycle write+cleanup canary cases ─────────────────────────────
    #
    # These exercise real provider write operations. Each flow is
    # self-cleaning: create -> verify -> delete/close. The mock LLM drives
    # the multi-step tool chain via match_special_response() in mock_llm.py.
    "gmail_roundtrip": SeededProviderCase(
        key="gmail_roundtrip",
        extension_install_name="gmail",
        expected_display_name="Gmail",
        response_prompt="send an email to user@example.com with subject '[canary] test' and body 'Canary test'",
        expected_tool_name="gmail",
        expected_text="gmail",
        shared_secret_name="google_oauth_token",
        requires_refresh_seed=True,
    ),
    "google_calendar_lifecycle": SeededProviderCase(
        key="google_calendar_lifecycle",
        extension_install_name="google_calendar",
        expected_display_name="Google Calendar",
        response_prompt="create a Google Calendar event titled '[canary] test' for tomorrow at 10am lasting 30 minutes",
        expected_tool_name="google_calendar",
        expected_text="google_calendar",
        shared_secret_name="google_oauth_token",
    ),
    "notion_search_lifecycle": SeededProviderCase(
        key="notion_search_lifecycle",
        extension_install_name="notion",
        expected_display_name="Notion",
        response_prompt="search notion for canary, then search again for test",
        expected_tool_name="notion_notion_search",
        expected_text="notion",
        install_kind="mcp_server",
    ),
}


BROWSER_CASES: dict[str, BrowserProviderCase] = {
    "google": BrowserProviderCase(
        key="google",
        extension_name="gmail",
        expected_extension_name="gmail",
        install_kind=None,
        install_url=None,
        trigger_prompt="check gmail unread",
        expected_tool_name="gmail",
        expected_text="gmail",
        auth_extension_name="gmail",
    ),
    # NOTE: `github` was previously listed here, but the released github
    # WASM tool registers as `auth_summary.method = "manual"` — PAT paste,
    # not OAuth. Activating the extension returns `awaiting_token: True`
    # with no `auth_url`, so the browser-consent probe has nothing to drive.
    # Coverage for github lives in SEEDED_CASES instead, which seeds the
    # PAT directly. Re-add here only after the github tool ships an OAuth
    # flow and the registry's `auth_summary.method` flips to "oauth".
    "notion": BrowserProviderCase(
        key="notion",
        extension_name="notion",
        expected_extension_name="notion",
        install_kind="mcp_server",
        install_url=None,
        trigger_prompt="search notion for canary",
        expected_tool_name="notion_notion_search",
        expected_text="notion",
        auth_extension_name="notion",
    ),
}


def _canary_timestamp() -> str:
    """Short timestamp for unique canary resource names."""
    import time as _time
    return str(int(_time.time()))


def configured_seeded_cases(selected: list[str] | None) -> list[SeededProviderCase]:
    cases: list[SeededProviderCase] = []
    # When no selection is provided (the scheduled-lane default path),
    # exclude lifecycle/mutating cases. The scheduled lane must be
    # low-risk/read-only unless an operator explicitly opts in by
    # naming lifecycle cases via `--case` / `CASES=`.
    if selected:
        names = selected
    else:
        names = [n for n in SEEDED_CASES if n not in LIFECYCLE_CASE_NAMES]
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
        # ── Lifecycle write+cleanup cases ────────────────────────────────
        elif name == "gmail_roundtrip":
            if not google_access:
                continue
            case = replace(case, requires_refresh_seed=bool(google_refresh))
            email = env_str("AUTH_LIVE_GOOGLE_EMAIL") or "canary@example.com"
            ts = _canary_timestamp()
            case = replace(
                case,
                response_prompt=(
                    f"send an email to {email} with subject '[canary] {ts}' "
                    f"and body 'Canary test'. Then list recent messages and confirm "
                    f"it was sent. Finally, trash the sent message."
                ),
            )
        elif name == "google_calendar_lifecycle":
            if not google_access:
                continue
            ts = _canary_timestamp()
            case = replace(
                case,
                response_prompt=(
                    f"create a Google Calendar event titled '[canary] {ts}' "
                    f"for tomorrow at 10am lasting 30 minutes. Then list events "
                    f"to confirm it exists. Finally delete the event."
                ),
            )
        elif name == "notion_search_lifecycle":
            if not env_str("AUTH_LIVE_NOTION_ACCESS_TOKEN"):
                continue
            ts = _canary_timestamp()
            case = replace(
                case,
                response_prompt=(
                    f"search notion for 'canary {ts}', then search again for 'test'"
                ),
            )
        cases.append(case)
    return cases


def configured_browser_cases(selected: list[str] | None) -> list[BrowserProviderCase]:
    cases: list[BrowserProviderCase] = []
    names = selected or list(BROWSER_CASES)
    for name in names:
        case = BROWSER_CASES[name]
        if env_str(f"AUTH_BROWSER_{name.upper()}_STORAGE_STATE_PATH") or env_str(
            f"AUTH_BROWSER_{name.upper()}_USERNAME"
        ):
            cases.append(case)
    return cases

