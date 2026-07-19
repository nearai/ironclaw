"""Live QA runner for Reborn WebUI v2.

This lane intentionally starts the standalone ``ironclaw serve`` binary
and drives the React WebUI v2 surface with Playwright. It does not use the
legacy gateway stack and does not mock the LLM provider.
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import hmac
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
import urllib.parse
import uuid
from contextlib import closing
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Awaitable, Callable

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.live_canary.common import (  # noqa: E402
    DEFAULT_VENV,
    ProbeResult,
    bootstrap_python,
    env_secret,
    install_playwright,
    reserve_loopback_port,
    run,
    stop_process,
    wait_for_ready,
    write_results,
)
from scripts.reborn_webui_v2_live_qa.case_matrix import (  # noqa: E402
    CaseFn,
    CaseSpec,
    QA_SHEET_CASES,
    QA_SHEET_TAB,
    QA_SHEET_URL,
    qa_row_sort_key,
)
from scripts.reborn_webui_v2_live_qa.errors import LiveQaError  # noqa: E402
from scripts.reborn_webui_v2_live_qa.external_auth_helpers import (  # noqa: E402
    _github_auth_preflight,
    _materialize_telegram_env_for_reborn,
    _seed_generated_github_product_auth_if_configured,
    _telegram_preflight,
)
from scripts.reborn_webui_v2_live_qa.env_helpers import (  # noqa: E402
    _env_present,
    _env_value,
    _first_env_value,
    _non_empty_env,
    _section_env_name,
)
from scripts.reborn_webui_v2_live_qa.google_api_helpers import (  # noqa: E402
    _create_google_spreadsheet_fixture,
    _extract_google_document_id,
    _extract_google_spreadsheet_id,
    _gmail_delivery_target_email,
    _gmail_message_contains_marker,
    _gmail_profile_email,
    _google_drive_file_id_by_name,
    _google_sheet_contains_marker,
    _wait_for_gmail_marker,
    _wait_for_google_sheet_marker,
)
from scripts.reborn_webui_v2_live_qa.google_auth_helpers import (  # noqa: E402
    _google_credential_action_for_block,
    _google_product_auth_env_status,
    _google_product_auth_preflight,
    _google_required_env_for_block,
    _google_runtime_access_token,
    _materialize_google_oauth_env_for_reborn,
    _seed_generated_google_product_auth_if_configured,
)
from scripts.reborn_webui_v2_live_qa.root_filesystem import (  # noqa: E402
    _decrypt_filesystem_secret,
    _encrypt_filesystem_secret,
    _put_root_filesystem_json,
    _root_filesystem_create_table,
    _root_filesystem_json,
    _root_filesystem_secret_by_handle,
)
from scripts.reborn_webui_v2_live_qa.green_run_explanation import (  # noqa: E402
    write_green_run_explanation,
)
from scripts.reborn_webui_v2_live_qa.semantic_judge import (  # noqa: E402
    _compact_json,
    _judge_assistant_reply_completion,
    _semantic_judge_passed,
)
from scripts.reborn_webui_v2_live_qa.slack_helpers import (  # noqa: E402
    SLACK_BOT_TOKEN_ENV,
    SLACK_OAUTH_CLIENT_ID_ENV,
    SLACK_OAUTH_CLIENT_SECRET_ENV,
    SLACK_PERSONAL_ACCESS_TOKEN_ENV,
    SLACK_PERSONAL_ACCESS_TOKEN_ENV_NAMES,
    SLACK_SECOND_USER_TOKEN_ENV,
    SLACK_SIGNING_SECRET_ENV,
    _disable_slack_in_config,
    _discover_slack_dm_route_channel,
    _has_live_slack_env,
    _has_slack_delivery_target,
    _materialize_slack_env_from_reborn_home,
    _persisted_slack_personal_dm_channel_id,
    _remove_legacy_slack_setup_fields,
    _remove_dm_slack_channel_routes,
    _seed_generated_slack_product_auth_if_configured,
    _seed_slack_personal_dm_target,
    _set_slack_section_key,
    _slack_inbound_user_id_for_cases,
    _slack_personal_auth_preflight,
    _slack_setup_payload,
    _slack_setup_preflight,
    _slack_auth_test,
    _slack_config_value,
    _slack_enabled,
)
from scripts.reborn_webui_v2_live_qa.text_match import (  # noqa: E402
    required_text_matches,
)

QA_SHEET_PROMPTS: dict[str, str] = {
    "qa_2a_gmail_connect": """In WebUI, ask IronClaw “connect to Gmail.” Go through the auth flow.
Expected result: Gmail is connected""",
    "qa_2b_calendar_connect": """In WebUI, ask IronClaw “connect to Google Calendar.” Go through the auth flow.
Expected result: Google Calendar is connected""",
    "qa_2c_drive_connect": """In WebUI, ask IronClaw “connect to Google Drive.” Go through the auth flow.
Expected result: Google Drive is connected""",
    "qa_2d_calendar_prep_live_chat": """In WebUI, ask IronClaw “For my next meeting, find information about the company that I am meeting with from my Google Docs and find the latest news.”
Expected result: Reference a Google Doc and the latest news""",
    "qa_2e_calendar_prep_email_routine": """In WebUI, ask IronClaw, “Every 30 minutes, send me an email with a summary for my next meeting, including info about the company I will meet, based on the Google Drive docs and the latest news.”
Expected results: Routine created""",
    "qa_3a_slack_connect": """In WebUI, ask IronClaw "connect to Slack." Go through the flow.
Expected result: Slack is connected""",
    "qa_3b_endpoint_status_live_chat": """In WebUI, ask IronClaw "check if near.ai returns a 200 status."
Expected result: IronClaw reports the endpoint's current HTTP status""",
    "qa_3c_endpoint_status_slack_routine": """In WebUI, ask IronClaw, "Every 5 minutes, ping [endpoint URL] checking if it returns a 200 status and send result in a DM in slack."
Expected result: Routine created""",
    "qa_4a_gmail_connect": """In WebUI, ask IronClaw "connect to Gmail." Go through the flow w/ Gmail.
Expected result: Gmail is connected""",
    "qa_4b_github_connect": """In WebUI, ask IronClaw "connect to GitHub." Go through the auth flow.
Expected result: GitHub is connected""",
    "qa_4c_github_release_live_chat": """In WebUI, ask IronClaw "summarize the latest release from https://github.com/nearai/ironclaw."
Expected result: summary of the most recent release""",
    "qa_4d_github_release_slack_routine": """In WebUI, ask IronClaw, "Every 5 minutes, check https://github.com/nearai/ironclaw for latest releases and send me a Slack DM summarizing any new ones."
Expected result: Routine created""",
    "qa_5a_slack_connect": """In WebUI, ask IronClaw "connect to Slack." Go through the auth flow.
Expected result: Slack is connected""",
    "qa_5b_drive_connect": """In WebUI, ask IronClaw "connect to Google Drive." Go through the auth flow.
Expected result: Google Drive is connected""",
    "qa_5c_strategy_doc_knowledge_base": """In WebUI, ask IronClaw "use the NEAR AI Strategy doc in my Google Drive as your knowledge base for answering strategy questions."
Expected result: IronClaw references the doc and confirms it can answer from it""",
    "qa_5d_slack_strategy_doc_answer": """In Slack, in a DM with IronClawm, ask a detailed strategy question about a Google doc (providing a link)
Expected result: Slack reply that answers the question, grounded in the strategy doc""",
    "qa_6a_gmail_connect": """In WebUI, ask IronClaw "connect to Gmail." Go through the auth flow.
Expected result: Gmail is connected""",
    "qa_6b_sheets_connect": """In WebUI, ask IronClaw "connect to Google Sheets." Go through the auth flow.
Expected result: Google Sheets is connected""",
    "qa_6c_gmail_to_sheet_live_chat": """In WebUI, ask IronClaw "check my recent emails and add any from a near.ai address to my Google Sheet called ABC."
Expected result: ABC sheet has new rows for each near.ai inbound email""",
    "qa_6d_gmail_to_sheet_routine": """In WebUI, ask IronClaw, "Every 30 minutes, check my inbox and add any new emails from a near.ai address to my Google Sheet called ABC."
Expected result: Routine created""",
    "qa_7a_slack_product_channel_connect": """Verify Slack DM delivery target preflight configuration before Slack workflow cases run.
Expected result: Slack DM delivery target is configured""",
    "qa_7b_sheets_connect": """In WebUI, ask IronClaw "connect to Google Sheets." Go through the auth flow.
Expected result: Google Sheets is connected""",
    "qa_7c_slack_bug_logger_routine": """In WebUI, ask IronClaw "whenever I send a slack message starting with 'bug:', add it as a row to my bug logging Google Sheet."
Expected result: Routine/trigger created""",
    "qa_7d_slack_bug_message_trigger": """In Slack, send a message starting with "bug:"
Expected result: Routine created""",
    "qa_8a_slack_connect": """In WebUI, ask IronClaw "connect to Slack." Go through the auth flow.
Expected result: Slack is connected""",
    "qa_8b_hn_keyword_live_chat": """In WebUI, ask IronClaw "search Hacker News for any recent posts mentioning 'IronClaw' or 'NEAR AI'."
Expected result: IronClaw reports any matching HN posts""",
    "qa_8c_hn_keyword_slack_routine": """In WebUI, ask IronClaw, "Every hour, check Hacker News for new posts mentioning 'IronClaw' or 'NEAR AI' and send a summary to Slack DM."
Expected result: Routine created""",
    "qa_9a_slack_connect": """In WebUI, ask IronClaw "connect to Slack." Go through the auth flow.
Expected result: Slack is connected""",
}


def _qa_sheet_prompt(case_name: str) -> str:
    try:
        return QA_SHEET_PROMPTS[case_name]
    except KeyError as exc:
        raise AssertionError(f"QA sheet prompt is not hardcoded for {case_name}") from exc

DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "reborn-webui-v2-live-qa"
DEFAULT_REBORN_HOME = Path("/tmp/ironclaw-reborn-real-slack")
AUTH_TOKEN = "reborn-webui-v2-live-qa-token-0123456789abcdef"
DEFAULT_USER_ID = "reborn-webui-v2-live-qa-user"
ENDPOINT_STATUS_URL = "https://near.ai"
PROVIDER = "reborn-webui-v2"
MODE = "live"
# Live QA is model- and network-nondeterministic: the same commit can pass then
# flake red hours later. Retry a transient (assertion/behavioral) case failure up
# to this many total attempts before recording a red. Default 2 = one retry;
# override via REBORN_WEBUI_V2_LIVE_QA_CASE_ATTEMPTS.
try:
    LIVE_QA_CASE_ATTEMPTS = max(
        1, int(os.environ.get("REBORN_WEBUI_V2_LIVE_QA_CASE_ATTEMPTS", "2"))
    )
except ValueError:
    LIVE_QA_CASE_ATTEMPTS = 2
HN_KEYWORD_SEARCH_URL = (
    "https://hn.algolia.com/api/v1/search_by_date"
    "?query=NEAR%20AI&tags=story&hitsPerPage=1"
)
EXTENSION_SEARCH_CAPABILITY_ID = "builtin.extension_search"
EXTENSION_INSTALL_CAPABILITY_ID = "builtin.extension_install"
EXTENSION_ACTIVATE_CAPABILITY_ID = "builtin.extension_activate"
OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID = "builtin.outbound_delivery_targets_list"
QA_7C_BUG_LOGGING_SHEET_TITLE = "bug logging Google Sheet"
SLACK_EXTENSION_REQUIREMENT = {
    "package_id": "slack",
    "display_name": "Slack",
    "required_tools": [
        "slack.list_conversations",
        "slack.get_conversation_info",
        "slack.get_conversation_history",
    ],
}


def _qa_7c_bug_logger_prompt(
    *,
    sheet_fixture: dict[str, object],
    routine_name: str,
) -> str:
    title = str(
        sheet_fixture.get("title") or QA_7C_BUG_LOGGING_SHEET_TITLE
    ).strip()
    spreadsheet_url = str(sheet_fixture.get("spreadsheet_url") or "").strip()
    spreadsheet_id = str(sheet_fixture.get("spreadsheet_id") or "").strip()
    sheet_name = str(sheet_fixture.get("sheet_name") or "Sheet1").strip()
    header_columns = "Summary, Reporter, Slack Timestamp, Status, QA Marker"
    return (
        f"{_qa_sheet_prompt('qa_7c_slack_bug_logger_routine')}\n\n"
        "Use this exact Google Sheet for \"my bug logging Google Sheet\":\n"
        f"- Title: {title}\n"
        f"- Spreadsheet URL: {spreadsheet_url}\n"
        f"- Spreadsheet ID: {spreadsheet_id}\n"
        f"- Sheet/tab name: {sheet_name}\n"
        f"- Header columns: {header_columns}\n\n"
        f"Create the routine/trigger named `{routine_name}`. Do not ask for "
        "spreadsheet details; use the sheet information above."
    )


class LiveQaContext:
    def __init__(
        self,
        *,
        base_url: str,
        output_dir: Path,
        reborn_home: Path,
        env: dict[str, str],
    ) -> None:
        self.base_url = base_url
        self.output_dir = output_dir
        self.reborn_home = reborn_home
        self.env = env


@dataclass
class PreparedRebornHome:
    path: Path
    env: dict[str, str] = field(default_factory=dict)
    preflight: dict[str, object] = field(default_factory=dict)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run live Playwright QA checks against Reborn WebUI v2."
    )
    parser.add_argument(
        "--case",
        action="append",
        choices=sorted(CASES),
        default=[],
        help="Limit the run to a case. May be repeated. Default runs the promoted suite.",
    )
    parser.add_argument(
        "--all-cases",
        action="store_true",
        help="Run every QA-sheet case, including cases normally gated by live credentials.",
    )
    parser.add_argument(
        "--non-telegram-qa-cases",
        action="store_true",
        help=(
            "Run every implemented QA-sheet case except Telegram cases. This is "
            "the full current live QA target."
        ),
    )
    parser.add_argument(
        "--reborn-home",
        type=Path,
        default=Path(
            os.environ.get("REBORN_WEBUI_V2_LIVE_QA_HOME", DEFAULT_REBORN_HOME)
        ),
        help=(
            "Source Reborn home to copy for the run. Defaults to "
            "REBORN_WEBUI_V2_LIVE_QA_HOME or /tmp/ironclaw-reborn-real-slack."
        ),
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help=f"Artifacts directory (default: {DEFAULT_OUTPUT_DIR})",
    )
    parser.add_argument(
        "--venv",
        type=Path,
        default=DEFAULT_VENV,
        help=f"Virtualenv path (default: {DEFAULT_VENV})",
    )
    parser.add_argument(
        "--playwright-install",
        choices=("auto", "with-deps", "plain", "skip"),
        default="auto",
    )
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--skip-python-bootstrap", action="store_true")
    parser.add_argument(
        "--require-slack-live",
        action="store_true",
        help=(
            "Require real Slack host env vars and keep [slack].enabled=true. "
            "Without this, non-Slack cases disable Slack in the copied temp home "
            "when Slack env vars are absent."
        ),
    )
    args = parser.parse_args()
    selected_modes = sum(
        [
            bool(args.case),
            args.all_cases,
            args.non_telegram_qa_cases,
        ]
    )
    if selected_modes > 1:
        parser.error(
            "--case, --all-cases, and --non-telegram-qa-cases are mutually exclusive"
        )
    return args


def _cargo_target_dir() -> Path:
    env_target = os.environ.get("CARGO_TARGET_DIR")
    if env_target:
        return Path(env_target)
    cargo_config = Path.home() / ".cargo" / "config.toml"
    if cargo_config.exists():
        for line in cargo_config.read_text(encoding="utf-8", errors="ignore").splitlines():
            line = line.strip()
            if line.startswith("target-dir"):
                _, _, value = line.partition("=")
                value = value.strip().strip('"').strip("'")
                if value:
                    return Path(value)
    return ROOT / "target"


def _reborn_binary() -> Path:
    return _cargo_target_dir() / "debug" / "ironclaw"


def build_reborn_binary() -> Path:
    features = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_FEATURES",
        "webui-v2-beta,slack-v2-host-beta",
    )
    build_env = os.environ.copy()
    build_env.setdefault("CARGO_PROFILE_DEV_DEBUG", "0")
    build_env.setdefault("CARGO_INCREMENTAL", "0")
    run(
        [
            "cargo",
            "build",
            "-p",
            "ironclaw_reborn_cli",
            "--features",
            features,
            "--bin",
            "ironclaw",
        ],
        cwd=ROOT,
        env=build_env,
    )
    binary = _reborn_binary()
    if not binary.exists():
        message = f"ironclaw binary was not produced at {binary}"
        raise LiveQaError(message)
    return binary


def _config_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        raise LiveQaError(f"failed to read Reborn config {path}: {exc}") from exc


def _referenced_env_names(config_text: str) -> set[str]:
    names: set[str] = set()
    for key in ("api_key_env", "signing_secret_env", "bot_token_env"):
        for match in re.finditer(rf"^\s*{key}\s*=\s*\"([A-Za-z_][A-Za-z0-9_]*)\"", config_text, re.MULTILINE):
            names.add(match.group(1))
    return names


def _write_minimal_reborn_config(path: Path, *, include_slack: bool) -> None:
    api_key_env = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV",
        "NEARAI_API_KEY" if os.environ.get("NEARAI_API_KEY") else "LIVE_OPENAI_COMPATIBLE_API_KEY",
    )
    api_key = env_secret(api_key_env)
    if not api_key:
        raise LiveQaError(
            f"Reborn home is missing config.toml and {api_key_env} is unset; "
            "set REBORN_WEBUI_V2_LIVE_QA_HOME to a complete Reborn home or provide live LLM env."
        )
    provider_id = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_PROVIDER_ID",
        "nearai",
    )
    model = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_MODEL",
        os.environ.get("LIVE_OPENAI_COMPATIBLE_MODEL", "deepseek-ai/DeepSeek-V4-Flash"),
    )
    base_url = os.environ.get("REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL")
    if provider_id != "nearai" and not base_url:
        base_url = os.environ.get("LIVE_OPENAI_COMPATIBLE_BASE_URL", "https://cloud-api.near.ai/v1")
    llm_default_lines = [
        f'provider_id = "{provider_id}"',
        f'model = "{model}"',
        f'api_key_env = "{api_key_env}"',
    ]
    if base_url:
        llm_default_lines.append(f'base_url = "{base_url}"')
    slack_lines: list[str] = []
    if include_slack:
        slack_lines = [
            "[slack]",
            "enabled = true",
            "",
        ]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "\n".join(
            [
                'api_version = "ironclaw.runtime/v1"',
                "",
                "[boot]",
                'profile = "local-dev"',
                "",
                "[llm]",
                "",
                "[llm.default]",
                *llm_default_lines,
                "",
                *slack_lines,
            ]
        ),
        encoding="utf-8",
    )


def _auth_user_id() -> str:
    configured = os.environ.get("REBORN_WEBUI_V2_LIVE_QA_USER_ID", "").strip()
    if configured:
        return configured
    home = Path(os.environ.get("REBORN_WEBUI_V2_LIVE_QA_HOME", DEFAULT_REBORN_HOME))
    discovered = _persisted_google_user_id(home)
    return discovered or DEFAULT_USER_ID


def _persisted_google_user_id(reborn_home: Path) -> str | None:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    with closing(sqlite3.connect(db_path)) as db:
        row = db.execute(
            "SELECT contents FROM root_filesystem_entries "
            "WHERE path LIKE '/tenants/reborn-cli/shared/reborn-identity/external/%/oauth/Z29vZ2xl/%' "
            "ORDER BY path LIMIT 1",
        ).fetchone()
    if not row:
        return None
    try:
        payload = json.loads(row[0])
    except (TypeError, json.JSONDecodeError):
        return None
    user_id = str(payload.get("user_id") or "").strip()
    return user_id or None


def prepare_reborn_home(
    args: argparse.Namespace,
    selected_cases: list[str],
    *,
    case_name: str | None = None,
) -> PreparedRebornHome:
    args.output_dir.mkdir(parents=True, exist_ok=True)
    needs_slack = any(CASES[name].requires_slack for name in selected_cases)
    needs_slack_target = any(CASES[name].requires_slack_target for name in selected_cases)
    needs_slack_personal_auth = any(
        CASES[name].requires_slack_personal_auth for name in selected_cases
    )
    needs_google_product_auth = any(
        CASES[name].requires_google_product_auth for name in selected_cases
    )
    needs_telegram = any(CASES[name].requires_telegram for name in selected_cases)
    needs_github_auth = any(CASES[name].requires_github_auth for name in selected_cases)
    auth_user_id = _auth_user_id()
    source_home = args.reborn_home
    if not source_home.exists():
        source_home = create_generated_reborn_home(
            args.output_dir / "generated-reborn-home",
            include_slack=needs_slack,
        )

    prepared_home = args.output_dir / "reborn-home"
    if case_name:
        prepared_home = prepared_home / case_name
    if prepared_home.exists():
        shutil.rmtree(prepared_home)

    def _ignore(_dir: str, names: list[str]) -> set[str]:
        return {name for name in names if name.endswith(".lock")}

    prepared_home.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source_home, prepared_home, ignore=_ignore)
    config_path = prepared_home / "config.toml"
    if not config_path.exists() and (prepared_home / "local-dev" / "reborn-local-dev.db").exists():
        _write_minimal_reborn_config(config_path, include_slack=needs_slack)
    legacy_setup_cleanup = _remove_legacy_slack_setup_fields(config_path)
    stale_dm_route_cleanup = _remove_dm_slack_channel_routes(config_path)
    route_configured_from_env = False
    inbound_user_id = _slack_inbound_user_id_for_cases(
        selected_cases,
    )
    config = _config_text(config_path)
    secret_env: dict[str, str] = {}
    secret_preflight: dict[str, object] = {"materialized": False}
    google_env, google_env_preflight = _materialize_google_oauth_env_for_reborn(
        prepared_home,
    )
    telegram_env, telegram_env_preflight = _materialize_telegram_env_for_reborn()

    if _slack_enabled(config) and not _has_live_slack_env():
        secret_env, secret_preflight = _materialize_slack_env_from_reborn_home(
            prepared_home,
            config,
        )
    process_env = {**secret_env, **google_env, **telegram_env}
    path_secret_env: dict[str, str] = {}
    for name in _referenced_env_names(config):
        value = env_secret(name)
        if value and not process_env.get(name):
            path_secret_env[name] = value
    process_env.update(path_secret_env)
    slack_route_discovery: dict[str, object] = {"checked": False}
    if (
        needs_slack_target
        and _slack_enabled(config)
        and _has_live_slack_env(process_env)
        and not _has_slack_delivery_target(config, prepared_home, auth_user_id)
    ):
        slack_route_discovery = _discover_slack_dm_route_channel(process_env)
        channel_id = (
            str(slack_route_discovery.get("channel_id") or "").strip()
            if slack_route_discovery.get("ok")
            else ""
        )
        if channel_id:
            slack_user_id = str(slack_route_discovery.get("dm_user_id") or "").strip()
            slack_route_discovery["personal_dm_seed"] = _seed_slack_personal_dm_target(
                prepared_home,
                config,
                auth_user_id=auth_user_id,
                slack_user_id=slack_user_id,
                dm_channel_id=channel_id,
            )
            config = _config_text(config_path)

    missing = sorted(name for name in _referenced_env_names(config) if not _env_present(name, process_env))
    missing = [name for name in missing if not name.startswith("IRONCLAW_REBORN_SLACK_")]
    if missing:
        raise LiveQaError(
            "Reborn config references unset live env vars: " + ", ".join(missing)
        )

    slack_enabled = _slack_enabled(config)
    slack_setup = (
        _slack_setup_preflight(prepared_home, config, process_env)
        if slack_enabled
        else {"configured": False}
    )
    slack_target_present = _has_slack_delivery_target(config, prepared_home, auth_user_id)
    slack_auth = (
        _slack_auth_test(config, process_env)
        if slack_enabled and _has_live_slack_env(process_env)
        else {"checked": False, "ok": False, "error": "Slack env unavailable"}
    )
    if args.require_slack_live and needs_slack and not slack_enabled:
        raise LiveQaError(
            "selected cases require live Slack, but [slack].enabled is not true "
            "in the prepared Reborn config."
        )
    if slack_enabled and not _has_live_slack_env(process_env):
        if args.require_slack_live:
            raise LiveQaError(
                "Reborn config enables Slack, but live Slack env vars are missing "
                "(expected IRONCLAW_REBORN_SLACK_SIGNING_SECRET and "
                "IRONCLAW_REBORN_SLACK_BOT_TOKEN unless overridden in config)."
            )
        if not needs_slack:
            _disable_slack_in_config(config_path)
            print(
                "[reborn-webui-v2-live-qa] Slack disabled in copied temp home because "
                "Slack live env vars are not present and no Slack case was selected.",
                flush=True,
            )
    if args.require_slack_live and needs_slack and slack_enabled and not slack_auth.get("ok"):
        raise LiveQaError(
            "selected cases require live Slack, but Slack auth.test failed: "
            f"{slack_auth.get('error') or 'unknown Slack auth error'}"
        )
    elif secret_env:
        print(
            "[reborn-webui-v2-live-qa] Slack env materialized from copied Reborn home "
            "for the child serve process.",
            flush=True,
        )
    google_preflight = _google_product_auth_preflight(
        prepared_home,
        auth_user_id,
        process_env,
    )
    google_preflight["requires_google_product_auth"] = needs_google_product_auth
    google_preflight["env_materialization"] = google_env_preflight
    telegram_preflight = _telegram_preflight(
        prepared_home,
        process_env,
        telegram_env_preflight,
        requires_telegram=needs_telegram,
    )
    github_preflight = _github_auth_preflight(
        prepared_home,
        process_env,
        requires_github_auth=needs_github_auth,
    )
    slack_personal_auth_preflight = _slack_personal_auth_preflight(
        prepared_home,
        auth_user_id,
        process_env,
        requires_slack_personal_auth=needs_slack_personal_auth,
    )
    return PreparedRebornHome(
        path=prepared_home,
        env=process_env,
        preflight={
            "slack": {
                "enabled_in_config": slack_enabled,
                "env_present": _has_live_slack_env(process_env),
                "requires_slack": needs_slack,
                "requires_delivery_target": needs_slack_target,
                "delivery_target_present": slack_target_present,
                "route_configured_from_env": route_configured_from_env,
                "route_discovery": slack_route_discovery,
                "stale_dm_route_cleanup": stale_dm_route_cleanup,
                "legacy_setup_cleanup": legacy_setup_cleanup,
                "inbound_user_id": inbound_user_id,
                "auth_user_id": auth_user_id,
                "setup": slack_setup,
                "auth_test": slack_auth,
                "secret_source": secret_preflight,
                "path_secret_env_names": sorted(path_secret_env),
            },
            "google_product_auth": google_preflight,
            "telegram": telegram_preflight,
            "github_auth": github_preflight,
            "slack_personal_auth": slack_personal_auth_preflight,
        },
    )


def create_generated_reborn_home(path: Path, *, include_slack: bool = False) -> Path:
    provider_id = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_PROVIDER_ID",
        "nearai",
    )
    model = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_MODEL",
        os.environ.get("LIVE_OPENAI_COMPATIBLE_MODEL", "deepseek-ai/DeepSeek-V4-Flash"),
    )
    path.mkdir(parents=True, exist_ok=True)
    _write_minimal_reborn_config(path / "config.toml", include_slack=include_slack)
    google_seed = _seed_generated_google_product_auth_if_configured(path, _auth_user_id())
    github_seed = _seed_generated_github_product_auth_if_configured(path, _auth_user_id())
    slack_seed = (
        _seed_generated_slack_product_auth_if_configured(path, _auth_user_id())
        if include_slack
        else {"seeded": False}
    )
    api_key_env = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV",
        "NEARAI_API_KEY" if os.environ.get("NEARAI_API_KEY") else "LIVE_OPENAI_COMPATIBLE_API_KEY",
    )
    print(
        "[reborn-webui-v2-live-qa] Generated temp Reborn home from live LLM env "
        f"(provider_id={provider_id}, model={model}, api_key_env={api_key_env}).",
        flush=True,
    )
    if google_seed.get("seeded"):
        print(
            "[reborn-webui-v2-live-qa] Seeded generated Reborn home with "
            "AUTH_LIVE_GOOGLE_* product-auth credentials for Google live cases.",
            flush=True,
        )
    if github_seed.get("seeded"):
        print(
            "[reborn-webui-v2-live-qa] Seeded generated Reborn home with "
            "GitHub product-auth credentials for GitHub live cases.",
            flush=True,
        )
    if slack_seed.get("seeded"):
        print(
            "[reborn-webui-v2-live-qa] Seeded generated Reborn home with "
            "Slack personal product-auth credentials for Slack live cases.",
            flush=True,
        )
    return path


def server_env(
    reborn_home: Path,
    process_home: Path,
    extra_env: dict[str, str] | None = None,
) -> dict[str, str]:
    process_home.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    if extra_env:
        env.update(extra_env)
    env.update(
        {
            "HOME": str(process_home),
            "IRONCLAW_REBORN_HOME": str(reborn_home),
            "IRONCLAW_REBORN_PROFILE": "local-dev",
            "IRONCLAW_REBORN_WEBUI_TOKEN": AUTH_TOKEN,
            "IRONCLAW_REBORN_WEBUI_USER_ID": _auth_user_id(),
            "NO_PROXY": "127.0.0.1,localhost,::1",
            "no_proxy": "127.0.0.1,localhost,::1",
            "RUST_BACKTRACE": "1",
            "RUST_LOG": os.environ.get(
                "RUST_LOG",
                "ironclaw=warn,ironclaw_runner=warn,ironclaw_webui=info",
            ),
        }
    )
    env.setdefault("IRONCLAW_TRIGGER_POLLER_ENABLED", "true")
    env.setdefault("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS", "1")
    return env


async def start_reborn_server(
    binary: Path,
    reborn_home: Path,
    output_dir: Path,
    extra_env: dict[str, str] | None = None,
) -> tuple[subprocess.Popen[str], str]:
    port = reserve_loopback_port()
    base_url = f"http://127.0.0.1:{port}"
    process_extra_env = dict(extra_env or {})
    if (
        _env_present("IRONCLAW_REBORN_GOOGLE_CLIENT_ID", process_extra_env)
        and not _env_present("IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI", process_extra_env)
        and not _env_present("GOOGLE_OAUTH_REDIRECT_URI", process_extra_env)
    ):
        process_extra_env["IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI"] = (
            f"{base_url}/api/reborn/product-auth/oauth/google/callback"
        )
    slack_oauth_client_configured = _env_present(
        SLACK_OAUTH_CLIENT_ID_ENV,
        process_extra_env,
    )
    config_path = reborn_home / "config.toml"
    if not slack_oauth_client_configured and config_path.exists():
        config_text = _config_text(config_path)
        if _slack_enabled(config_text):
            slack_oauth_client_configured = bool(
                _slack_setup_preflight(
                    reborn_home,
                    config_text,
                    process_extra_env,
                ).get("oauth_client_id_configured")
            )
    if (
        slack_oauth_client_configured
        and _env_present(SLACK_OAUTH_CLIENT_SECRET_ENV, process_extra_env)
        and not _env_present(
            "IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI",
            process_extra_env,
        )
    ):
        process_extra_env["IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"] = (
            f"{base_url}/api/reborn/product-auth/oauth/slack_personal/callback"
        )
    stdout_path = output_dir / "ironclaw-reborn-serve.stdout.log"
    stderr_path = output_dir / "ironclaw-reborn-serve.stderr.log"
    workspace_dir = output_dir / "workspace"
    workspace_dir.mkdir(parents=True, exist_ok=True)
    out = stdout_path.open("a", encoding="utf-8")
    err = stderr_path.open("a", encoding="utf-8")
    separator = f"\n--- ironclaw serve start {time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} ---\n"
    out.write(separator)
    err.write(separator)
    out.flush()
    err.flush()
    proc = subprocess.Popen(
        [
            str(binary),
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
        ],
        stdin=subprocess.DEVNULL,
        stdout=out,
        stderr=err,
        text=True,
        env=server_env(reborn_home, output_dir / "os-home", process_extra_env),
        cwd=workspace_dir,
    )
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=90.0)
    except Exception as exc:
        stop_process(proc)
        tail = ""
        if stderr_path.exists():
            tail = "\n".join(stderr_path.read_text(encoding="utf-8", errors="replace").splitlines()[-80:])
        raise LiveQaError(
            f"ironclaw serve did not become healthy at {base_url}: {exc}\n{tail}"
        ) from exc
    return proc, base_url


async def _apply_slack_setup_api_after_start(
    *,
    base_url: str,
    prepared_home: PreparedRebornHome,
) -> dict[str, object]:
    config_text = _config_text(prepared_home.path / "config.toml")
    if not _slack_enabled(config_text):
        return {"applied": False, "reason": "slack_disabled"}
    payload, preflight = _slack_setup_payload(
        prepared_home.path,
        config_text,
        prepared_home.env,
    )
    if payload is None:
        return {"applied": False, "reason": "setup_payload_missing", **preflight}
    try:
        import httpx

        async with httpx.AsyncClient(timeout=30.0) as client:
            response = await client.put(
                f"{base_url}/api/webchat/v2/channels/slack/setup",
                headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
                json=payload,
            )
            if response.status_code < 200 or response.status_code >= 300:
                raise LiveQaError(
                    "Slack setup API returned HTTP "
                    f"{response.status_code}; response body omitted because "
                    "this endpoint handles Slack secrets"
                )
            status = response.json()
    except LiveQaError:
        raise
    except Exception as exc:
        raise LiveQaError(f"Slack setup API call failed: {type(exc).__name__}: {exc}") from exc
    if not isinstance(status, dict):
        raise LiveQaError(f"Slack setup API returned non-object JSON: {status!r}")
    required_flags = ["configured", "bot_token_configured", "signing_secret_configured"]
    if payload.get("oauth_client_id") or payload.get("oauth_client_secret"):
        required_flags.extend(["oauth_client_id_configured", "oauth_client_secret_configured"])
    missing_flags = [flag for flag in required_flags if status.get(flag) is not True]
    mismatched_identity = [
        key
        for key in ("installation_id", "team_id", "api_app_id")
        if str(status.get(key) or "") != str(payload.get(key) or "")
    ]
    if missing_flags or mismatched_identity:
        raise LiveQaError(
            "Slack setup API returned incomplete setup status: "
            f"missing_flags={missing_flags!r} "
            f"mismatched_identity={mismatched_identity!r}"
        )
    return {
        "applied": True,
        "status_code": response.status_code,
        "request": {
            "installation_id": payload.get("installation_id"),
            "team_id": payload.get("team_id"),
            "api_app_id": payload.get("api_app_id"),
            "oauth_client_id_configured": bool(payload.get("oauth_client_id")),
            "oauth_client_secret_configured": bool(payload.get("oauth_client_secret")),
        },
        "status": status,
    }


_BROWSER_EVENT_LIMIT = 1_000


def _browser_diagnostics_dir(output_dir: Path, case_name: str) -> Path:
    return output_dir / "browser-diagnostics" / case_name


def _redact_browser_diagnostic_value(value: object) -> object:
    if value is None or isinstance(value, (bool, int, float)):
        return value
    text = str(value)
    if AUTH_TOKEN:
        text = text.replace(AUTH_TOKEN, "<REDACTED_AUTH_TOKEN>")
    text = re.sub(
        r"(?i)([?&](?:token|code|state|access_token|refresh_token|id_token)=)[^&#\s]+",
        r"\1<REDACTED>",
        text,
    )
    text = re.sub(
        r"(?i)\b(?:token|code|state|access_token|refresh_token|id_token)=[^,\s)'\"]+",
        lambda match: match.group(0).split("=", 1)[0] + "=<REDACTED>",
        text,
    )
    text = re.sub(r"(?i)(bearer\s+)[^\s]+", r"\1<REDACTED>", text)
    text = re.sub(r"(?i)(cookie\s*:\s*)[^\r\n]+", r"\1<REDACTED>", text)
    text = re.sub(r"xox[baprs]-[A-Za-z0-9-]{10,}", "<REDACTED_SLACK_TOKEN>", text)
    text = re.sub(r"ya29\.[A-Za-z0-9._-]{20,}", "<REDACTED_GOOGLE_TOKEN>", text)
    text = re.sub(r"sk-ant-[A-Za-z0-9_-]{10,}", "<REDACTED_ANTHROPIC_KEY>", text)
    text = re.sub(r"sk-[A-Za-z0-9_-]{20,}", "<REDACTED_OPENAI_KEY>", text)
    return text


def _browser_event_value(obj: object, name: str) -> object | None:
    try:
        value = getattr(obj, name)
        return value() if callable(value) else value
    except Exception as exc:
        return f"<unavailable:{type(exc).__name__}>"


class _BrowserDiagnostics:
    def __init__(self, output_dir: Path, case_name: str) -> None:
        self.dir = _browser_diagnostics_dir(output_dir, case_name)
        self.event_path = self.dir / "browser-events.jsonl"
        self.summary_path = self.dir / "browser-summary.json"
        self.trace_path = self.dir / "playwright-trace.zip"
        self.event_count = 0
        self.dropped_event_count = 0
        self.write_error_count = 0
        self.console_error_count = 0
        self.page_error_count = 0
        self.failed_request_count = 0
        self.error_response_count = 0
        self.trace_written = False
        self.screenshot_path: Path | None = None
        self._attached_pages: set[int] = set()
        self.dir.mkdir(parents=True, exist_ok=True)

    def record(self, event_type: str, **fields: object) -> None:
        if self.event_count >= _BROWSER_EVENT_LIMIT:
            self.dropped_event_count += 1
            return
        event = {
            "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "event": event_type,
        }
        event.update(
            {key: _redact_browser_diagnostic_value(value) for key, value in fields.items()}
        )
        try:
            with self.event_path.open("a", encoding="utf-8") as fh:
                fh.write(json.dumps(event, sort_keys=True) + "\n")
            self.event_count += 1
        except Exception:
            self.write_error_count += 1
        if event_type == "console" and str(event.get("level") or "").lower() == "error":
            self.console_error_count += 1
        elif event_type == "pageerror":
            self.page_error_count += 1
        elif event_type == "requestfailed":
            self.failed_request_count += 1
        elif event_type == "response":
            self.error_response_count += 1

    def attach_context(self, context: object) -> None:
        try:
            context.on("requestfailed", self._on_request_failed)
            context.on("response", self._on_response)
            context.on("page", self.attach_page)
        except Exception as exc:
            self.record("diagnostic_error", source="attach_context", error=repr(exc))

    def attach_page(self, page: object) -> None:
        page_id = id(page)
        if page_id in self._attached_pages:
            return
        self._attached_pages.add(page_id)
        try:
            page.on("console", self._on_console)
            page.on("pageerror", self._on_page_error)
        except Exception as exc:
            self.record("diagnostic_error", source="attach_page", error=repr(exc))

    def _on_console(self, message: object) -> None:
        self.record(
            "console",
            level=_browser_event_value(message, "type"),
            text=_browser_event_value(message, "text"),
            location=_browser_event_value(message, "location"),
        )

    def _on_page_error(self, error: object) -> None:
        self.record("pageerror", message=repr(error))

    def _on_request_failed(self, request: object) -> None:
        self.record(
            "requestfailed",
            method=_browser_event_value(request, "method"),
            url=_browser_event_value(request, "url"),
            failure=_browser_event_value(request, "failure"),
        )

    def _on_response(self, response: object) -> None:
        status = _browser_event_value(response, "status")
        try:
            status_int = int(status)  # type: ignore[arg-type]
        except (TypeError, ValueError):
            return
        if status_int < 400:
            return
        request = _browser_event_value(response, "request")
        self.record(
            "response",
            status=status_int,
            url=_browser_event_value(response, "url"),
            request_method=_browser_event_value(request, "method") if request else None,
        )

    def write_summary(self) -> None:
        payload = {
            "event_count": self.event_count,
            "dropped_event_count": self.dropped_event_count,
            "write_error_count": self.write_error_count,
            "console_error_count": self.console_error_count,
            "page_error_count": self.page_error_count,
            "failed_request_count": self.failed_request_count,
            "error_response_count": self.error_response_count,
            "event_path": str(self.event_path) if self.event_path.exists() else None,
            "summary_path": str(self.summary_path),
            "trace_path": str(self.trace_path) if self.trace_written else None,
            "screenshot_path": str(self.screenshot_path) if self.screenshot_path else None,
        }
        self.summary_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def _attach_browser_diagnostics(output_dir: Path, result: ProbeResult) -> ProbeResult:
    case_name = str(result.details.get("case") or "")
    if not case_name:
        return result
    summary_path = _browser_diagnostics_dir(output_dir, case_name) / "browser-summary.json"
    if not summary_path.exists():
        return result
    try:
        result.details["browser_diagnostics"] = json.loads(
            summary_path.read_text(encoding="utf-8")
        )
    except (OSError, json.JSONDecodeError) as exc:
        result.details["browser_diagnostics"] = {
            "summary_path": str(summary_path),
            "error": f"failed to read browser diagnostics: {type(exc).__name__}",
        }
    return result


async def _with_page(output_dir: Path, case_name: str, action: Callable[[object], Awaitable[None]]) -> None:
    from playwright.async_api import async_playwright

    headless = os.environ.get("HEADED", "").strip().lower() not in ("1", "true")
    diagnostics = _BrowserDiagnostics(output_dir, case_name)
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(headless=headless, timeout=60000)
        context = await browser.new_context()
        diagnostics.attach_context(context)
        await context.tracing.start(screenshots=True, snapshots=True, sources=False)
        page = await context.new_page()
        diagnostics.attach_page(page)
        try:
            await action(page)
        except Exception:
            screenshot = output_dir / f"{case_name}.failure.png"
            try:
                await page.screenshot(path=str(screenshot), full_page=True)
                diagnostics.screenshot_path = screenshot
            except Exception as screenshot_exc:
                diagnostics.record(
                    "diagnostic_error",
                    source="screenshot_capture",
                    error=repr(screenshot_exc),
                )
            try:
                await context.tracing.stop(path=str(diagnostics.trace_path))
                diagnostics.trace_written = True
            except Exception as trace_exc:
                diagnostics.record(
                    "diagnostic_error",
                    source="trace_stop",
                    error=repr(trace_exc),
                )
            raise
        finally:
            try:
                diagnostics.write_summary()
            except Exception as summary_exc:
                print(
                    "[reborn-webui-v2-live-qa] failed to write browser diagnostics "
                    f"summary for {case_name}: {type(summary_exc).__name__}: {summary_exc}",
                    flush=True,
                )
            await context.close()
            await browser.close()


@dataclass(frozen=True)
class AssistantReplyWaitResult:
    text_excerpt: str
    semantic_judge_used: bool
    semantic_judge_reason: str
    final_reply_wait_ms: int
    final_reply_reason: str
    semantic_judge: dict[str, object] | None = None
    # Full reply, in-memory only: content checks (raw-id scan, ground-truth
    # names) must never be blinded by excerpt truncation, while persisted
    # details keep only the bounded text_excerpt.
    full_text: str = ""''


@dataclass(frozen=True)
class TerminalRunFailureObservation:
    summary: str
    failure_category: str | None
    failure_status: str | None


class TerminalRunFailure(AssertionError):
    def __init__(self, observation: TerminalRunFailureObservation) -> None:
        self.observation = observation
        summary = observation.summary or "The model run failed."
        super().__init__(
            f"terminal run failure: {summary} "
            f"failure_category={observation.failure_category!r} "
            f"failure_status={observation.failure_status!r}"
        )


ASSISTANT_REPLY_FALLBACK_QUIET_SECONDS = 2.0
ASSISTANT_REPLY_POLL_SECONDS = 0.5
# Persisted diagnostic excerpt only — content checks read
# AssistantReplyWaitResult.full_text, never this truncation.
ASSISTANT_REPLY_EXCERPT_MAX_CHARS = 2000


def _exc_text(exc: BaseException) -> str:
    """Human-readable exception text for probe failure details.

    `str()` of many timeout classes (asyncio.TimeoutError, httpx transport
    timeouts) is empty, which previously produced probe failures with an
    empty `error` field. Fall back to `repr()` so the type is preserved.
    """
    text = str(exc).strip()
    return text if text else repr(exc)


def _result(case_name: str, success: bool, started: float, details: dict[str, object]) -> ProbeResult:
    case_spec = CASES.get(case_name)
    case_tier = case_spec.tier if case_spec is not None else "contract"
    blocking = case_spec.blocking if case_spec is not None else True
    details = {
        **details,
        "case": case_name,
        "case_tier": case_tier,
        "blocking": blocking,
    }
    if case_name in QA_SHEET_CASES:
        qa_spec = QA_SHEET_CASES[case_name]
        details = {
            **qa_spec,
            "qa_rows": qa_spec.get("rows", []),
            **details,
        }
    return ProbeResult(
        provider=PROVIDER,
        mode=f"{MODE}:{case_name}",
        success=success,
        latency_ms=int((time.monotonic() - started) * 1000),
        details=details,
    )


def _is_blocking_failure(result: ProbeResult) -> bool:
    return not result.success and bool(result.details.get("blocking", True))


def _is_provider_incident(result: ProbeResult) -> bool:
    if result.success:
        return False
    return result.details.get("failure_category") in {
        "model_unavailable",
        "model_transient",
        "provider_unavailable",
        "provider_transient",
    }


def _is_case_retriable(result: ProbeResult) -> bool:
    """Whether an unsuccessful case result is a transient failure worth re-running.

    Only assertion/behavioral failures are retriable. Non-transient failures are
    recorded as-is without wasting a retry: blocked preconditions, and
    infrastructure/provider incidents (failure_class=="infrastructure",
    inconclusive, or a model/provider incident per _is_provider_incident).
    """
    if result.success:
        return False
    details = result.details
    if details.get("blocked"):
        return False
    if details.get("failure_class") == "infrastructure":
        return False
    if details.get("inconclusive"):
        return False
    if _is_provider_incident(result):
        return False
    return True


async def _run_case_with_retries(
    fn: CaseFn,
    ctx: "LiveQaContext",
    *,
    attempts: int,
    is_retriable: Callable[[ProbeResult], bool],
) -> ProbeResult:
    """Run a live-QA case, retrying a transient failure before recording a red.

    Runs ``fn(ctx)``; on an unsuccessful *and* retriable result with attempts
    remaining, runs it again; otherwise returns the (last) result. Re-running
    ``fn(ctx)`` drives a fresh chat turn against the same already-running
    server/ctx — no restart — which is the intended retry semantics for a
    nondeterministic model/network flake. The number of attempts made is
    recorded into ``result.details["attempts"]``.
    """
    total = max(1, attempts)
    result: ProbeResult | None = None
    for attempt in range(1, total + 1):
        result = await fn(ctx)
        result.details["attempts"] = attempt
        if result.success or attempt >= total or not is_retriable(result):
            return result
        print(
            "[reborn-webui-v2-live-qa] retrying case after retriable failure "
            f"attempt={attempt}/{total}",
            flush=True,
        )
    assert result is not None  # the loop body always runs at least once
    return result


def _record_assistant_reply_wait_result(
    observed: dict[str, object],
    reply: AssistantReplyWaitResult,
) -> None:
    observed["text_excerpt"] = reply.text_excerpt
    observed["semantic_judge_used"] = reply.semantic_judge_used
    observed["semantic_judge_reason"] = reply.semantic_judge_reason
    observed["assistant_reply_wait_ms"] = reply.final_reply_wait_ms
    observed["assistant_reply_wait_reason"] = reply.final_reply_reason
    if reply.semantic_judge is not None:
        observed["semantic_judge"] = reply.semantic_judge


_SUBMISSION_CORRELATION_FIELDS = (
    "accepted_message_ref",
    "thread_id",
    "run_id",
)
_SUBMISSION_IDENTITY_FIELDS = (*_SUBMISSION_CORRELATION_FIELDS, "turn_id")


def _record_submitted_identity(
    observed: dict[str, Any],
    payload: dict[str, object],
) -> None:
    if any(not payload.get(field) for field in _SUBMISSION_IDENTITY_FIELDS):
        raise AssertionError("submitted response omitted turn identity fields")
    identity = {
        field: str(payload[field]) for field in _SUBMISSION_IDENTITY_FIELDS
    }
    existing = observed.get("submission_identity")
    if existing is not None and existing != identity:
        raise AssertionError(
            "ambiguous submission identity: distinct submitted "
            "acknowledgements matched one prompt"
        )
    observed["submission_identity"] = identity


def _record_replayed_submission_identity(
    observed: dict[str, Any],
    payload: dict[str, object],
) -> None:
    """Recover correlation identity from an authoritative replay response."""
    if any(not payload.get(field) for field in _SUBMISSION_CORRELATION_FIELDS):
        raise AssertionError(
            "already_submitted response omitted correlation identity fields"
        )
    identity = {
        field: str(payload[field]) for field in _SUBMISSION_CORRELATION_FIELDS
    }
    existing = observed.get("submission_identity")
    if existing is not None:
        if not isinstance(existing, dict):
            raise AssertionError("submission identity had an invalid shape")
        existing_correlation = {
            field: str(existing.get(field) or "")
            for field in _SUBMISSION_CORRELATION_FIELDS
        }
        if existing_correlation != identity:
            raise AssertionError(
                "ambiguous submission identity: replay response referenced "
                "a different message or run"
            )
        return
    observed["submission_identity"] = identity


def _routine_confirmation_follow_up_for_text(
    text: str,
    *,
    schedule_timezone_instruction: str = (
        "Use Europe/London (London time) for the schedule."
    ),
) -> str | None:
    normalized = text.lower()
    asks_for_timezone = "timezone" in normalized or "time zone" in normalized
    asks_for_confirmation = any(
        phrase in normalized
        for phrase in (
            "confirm",
            "go ahead",
            "shall i",
            "should i",
            "would you like",
        )
    )
    routine_context = any(
        phrase in normalized
        for phrase in ("routine", "trigger", "automation", "schedule", "cron")
    )
    if routine_context and (asks_for_timezone or asks_for_confirmation):
        # Cases that pin a timezone in the creation prompt must pass a
        # matching instruction here: answering a clarifying question with a
        # DIFFERENT timezone than the prompt (e.g. London vs a UTC one-shot)
        # can shift a one-shot fire outside the delivery wait window.
        return f"Yes, go ahead and create it. {schedule_timezone_instruction}"
    return None


async def _live_chat_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    prompt: str,
    marker: str | None,
    required_text: list[str],
    extensions: list[dict[str, object]] | None = None,
    timeout: float = 120.0,
    extra_details: dict[str, object] | None = None,
    forbidden_text: list[str] | None = None,
    routine_confirmation_follow_up: bool = False,
    routine_follow_up_timezone_instruction: str | None = None,
    expose_full_reply_text: bool = False,
    enforce_marker: bool = True,
    capture_submission_identity: bool = False,
) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    observed: dict[str, Any] = {}
    if extensions:
        observed["extensions"] = [
            str(extension["package_id"]) for extension in extensions
        ]

    def is_matching_submission_response(response: object) -> bool:
        try:
            request = response.request  # type: ignore[attr-defined]
            if request.method != "POST":
                return False
            if not re.search(
                r"/api/webchat/v2/threads/[^/]+/messages$",
                str(response.url),  # type: ignore[attr-defined]
            ):
                return False
            request_body = request.post_data_json
            if callable(request_body):
                request_body = request_body()
            return isinstance(request_body, dict) and request_body.get("content") == prompt
        except Exception:
            return False

    async def submit_prompt(page: object, composer: object) -> bool:
        if not capture_submission_identity:
            await composer.fill(prompt)  # type: ignore[attr-defined]
            await composer.press("Enter")  # type: ignore[attr-defined]
            return True
        try:
            async with page.expect_response(  # type: ignore[attr-defined]
                is_matching_submission_response,
                timeout=15000,
            ) as response_info:
                await composer.fill(prompt)  # type: ignore[attr-defined]
                await composer.press("Enter")  # type: ignore[attr-defined]
            response = await response_info.value
        except Exception as exc:
            if "submission_identity" in observed:
                raise
            observed["submission_response_wait_error"] = _exc_text(exc)
            return False
        payload = await response.json()
        if not isinstance(payload, dict):
            raise AssertionError("chat submission response was not a JSON object")
        outcome = str(payload.get("outcome") or "")
        existing = observed.get("submission_identity")
        if outcome == "submitted":
            _record_submitted_identity(observed, payload)
            return True
        if outcome == "already_submitted":
            _record_replayed_submission_identity(observed, payload)
            return True
        if outcome == "rejected_busy":
            # This response acknowledges a rejected message, not the run that
            # should answer it. Its active_run_id may identify an unrelated
            # blocker (or be absent on replay), so it cannot establish first-
            # turn identity by itself.
            if not isinstance(existing, dict):
                raise AssertionError(
                    "cannot recover submitted turn identity from rejected_busy "
                    "without a prior submitted acknowledgement"
                )
            same_thread = str(payload.get("thread_id") or "") == existing.get(
                "thread_id"
            )
            same_run = str(payload.get("active_run_id") or "") == existing.get(
                "run_id"
            )
            if same_thread and same_run:
                return True
            raise AssertionError(
                "ambiguous submission identity: busy response referenced a "
                "different active run"
            )
        if outcome != "submitted":
            raise AssertionError(
                "chat submission did not return a fresh submitted turn identity: "
                f"outcome={outcome!r}"
            )
        return True

    async def action(page: object) -> None:
        if extensions:
            await page.goto(
                f"{ctx.base_url}/extensions/registry?token={AUTH_TOKEN}",
                wait_until="domcontentloaded",
            )  # type: ignore[attr-defined]
            await expect(page.locator("body")).to_contain_text(  # type: ignore[attr-defined]
                "Extensions",
                timeout=15000,
            )
            for extension in extensions:
                await _ensure_extension_authenticated_on_page(
                    page,
                    observed,
                    package_id=str(extension["package_id"]),
                    display_name=str(extension["display_name"]),
                    required_tools=[
                        str(tool) for tool in extension.get("required_tools", [])
                    ],
                    ensure_installed=bool(extension.get("ensure_installed", True)),
                )

        await page.goto(
            f"{ctx.base_url}/?token={AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )  # type: ignore[attr-defined]
        if await _dismiss_visible_connect_action(page):
            observed["connect_action_dismissed_before_submit"] = True
        composer = page.locator("[data-testid='chat-composer']")  # type: ignore[attr-defined]
        await expect(composer).to_be_visible(timeout=15000)
        assistant_count_before = await page.locator(  # type: ignore[attr-defined]
            "[data-testid='msg-assistant']"
        ).count()
        error_count_before = await page.locator(  # type: ignore[attr-defined]
            "[data-testid='msg-error']"
        ).count()
        response_captured = await submit_prompt(page, composer)
        if not response_captured:
            if not await _dismiss_visible_connect_action(page):
                raise AssertionError(
                    "chat submission produced no matching response and no "
                    "connect action was available for recovery"
                )
            observed["connect_action_dismissed_after_submit"] = True
            if not await submit_prompt(page, composer):
                raise AssertionError(
                    "chat submission retry produced no matching response"
                )
        try:
            await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
                prompt[:80],
                timeout=15000,
            )
        except Exception:
            if capture_submission_identity and "submission_identity" in observed:
                observed["submitted_user_bubble_not_observed"] = True
            elif not await _dismiss_visible_connect_action(page):
                raise
            else:
                observed["connect_action_dismissed_after_submit"] = True
                if not await submit_prompt(page, composer):
                    raise AssertionError(
                        "chat submission retry produced no matching response"
                    )
                await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
                    prompt[:80],
                    timeout=15000,
                )
        reply = await _wait_for_assistant_reply(
            page,
            marker=marker,
            required_text=required_text,
            timeout=timeout,
            semantic_goal=prompt,
            assistant_count_before=assistant_count_before,
            error_count_before=error_count_before,
            enforce_marker=enforce_marker,
        )
        _record_assistant_reply_wait_result(observed, reply)
        if expose_full_reply_text:
            # In-memory hand-off to the calling case; the case pops it before
            # the details dict is persisted into artifacts.
            observed["full_reply_text"] = reply.full_text
        if routine_confirmation_follow_up:
            follow_up_kwargs: dict[str, str] = {}
            if routine_follow_up_timezone_instruction:
                follow_up_kwargs["schedule_timezone_instruction"] = (
                    routine_follow_up_timezone_instruction
                )
            follow_up = _routine_confirmation_follow_up_for_text(
                reply.text_excerpt, **follow_up_kwargs
            )
            if follow_up:
                follow_up_assistant_count_before = await page.locator(  # type: ignore[attr-defined]
                    "[data-testid='msg-assistant']"
                ).count()
                follow_up_error_count_before = await page.locator(  # type: ignore[attr-defined]
                    "[data-testid='msg-error']"
                ).count()
                observed["routine_confirmation_follow_up_sent"] = follow_up
                observed["routine_confirmation_initial_text_excerpt"] = (
                    reply.text_excerpt
                )
                await composer.fill(follow_up)
                await composer.press("Enter")
                await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
                    follow_up[:80],
                    timeout=15000,
                )
                follow_up_reply = await _wait_for_assistant_reply(
                    page,
                    marker=marker,
                    required_text=required_text,
                    timeout=timeout,
                    semantic_goal=f"{prompt}\n{follow_up}",
                    assistant_count_before=follow_up_assistant_count_before,
                    error_count_before=follow_up_error_count_before,
                    enforce_marker=enforce_marker,
                )
                _record_assistant_reply_wait_result(observed, follow_up_reply)
        if forbidden_text:
            text = str(observed["text_excerpt"]).lower()
            matches = [
                phrase
                for phrase in forbidden_text
                if _forbidden_phrase_matches(text, phrase)
            ]
            if matches:
                raise AssertionError(
                    "assistant reply contained forbidden failure text: "
                    + ", ".join(matches)
                )

    try:
        await _with_page(ctx.output_dir, case_name, action)
        return _result(
            case_name,
            True,
            started,
            {
                "prompt": prompt,
                "marker": marker,
                "required_text": required_text,
                **(extra_details or {}),
                **observed,
            },
        )
    except TerminalRunFailure as exc:
        return _result(
            case_name,
            False,
            started,
            {
                "error": _exc_text(exc),
                "prompt": prompt,
                "marker": marker,
                "required_text": required_text,
                **(extra_details or {}),
                **observed,
                "failure_category": exc.observation.failure_category,
                "failure_status": exc.observation.failure_status,
            },
        )
    except Exception as exc:
        return _result(
            case_name,
            False,
            started,
            {
                "error": _exc_text(exc),
                "prompt": prompt,
                "marker": marker,
                "required_text": required_text,
                **(extra_details or {}),
                **observed,
            },
        )


def _forbidden_phrase_matches(normalized_text: str, phrase: str) -> bool:
    normalized_phrase = phrase.lower()
    if normalized_phrase == "authentication required":
        benign_auth_phrases = (
            "no additional authentication required",
            "no authentication required",
            "without additional authentication required",
        )
        text = normalized_text
        for benign_phrase in benign_auth_phrases:
            text = text.replace(benign_phrase, "")
        return normalized_phrase in text
    return normalized_phrase in normalized_text


async def _live_chat_with_extensions_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    prompt: str,
    marker: str | None,
    required_text: list[str],
    extensions: list[dict[str, object]],
    timeout: float = 240.0,
    extra_details: dict[str, object] | None = None,
    forbidden_text: list[str] | None = None,
) -> ProbeResult:
    return await _live_chat_case(
        ctx,
        case_name=case_name,
        prompt=prompt,
        marker=marker,
        required_text=required_text,
        extensions=extensions,
        timeout=timeout,
        extra_details=extra_details,
        forbidden_text=forbidden_text,
    )


async def _dismiss_visible_connect_action(page: object) -> bool:
    dismiss = page.locator("[aria-label='Dismiss connect action']")  # type: ignore[attr-defined]
    try:
        if await dismiss.count() <= 0:
            return False
        first = dismiss.first
        if not await first.is_visible():
            return False
        await first.click()
        return True
    except Exception:
        return False


async def _observe_terminal_run_failure(
    page: object,
    *,
    baseline_count: int = 0,
) -> TerminalRunFailureObservation | None:
    try:
        errors = page.locator("[data-testid='msg-error']")  # type: ignore[attr-defined]
        error_count = await errors.count()
    except Exception:
        return None
    if error_count <= max(0, baseline_count):
        return None

    latest_error = errors.last
    try:
        summary = (await latest_error.inner_text(timeout=1000)).strip()
    except Exception:
        summary = ""
    try:
        failure_category = await latest_error.get_attribute(
            "data-failure-category",
            timeout=1000,
        )
    except Exception:
        failure_category = None
    try:
        failure_status = await latest_error.get_attribute(
            "data-failure-status",
            timeout=1000,
        )
    except Exception:
        failure_status = None

    return TerminalRunFailureObservation(
        summary=summary,
        failure_category=(
            failure_category.strip() if isinstance(failure_category, str) else None
        )
        or None,
        failure_status=(
            failure_status.strip() if isinstance(failure_status, str) else None
        )
        or None,
    )


async def _wait_for_assistant_reply(
    page: object,
    *,
    marker: str | None,
    required_text: list[str],
    timeout: float,
    semantic_goal: str | None = None,
    assistant_count_before: int = 0,
    error_count_before: int = 0,
    enforce_marker: bool = True,
) -> AssistantReplyWaitResult:
    started = time.monotonic()
    deadline = time.monotonic() + timeout
    last_text = ""
    last_final_assistant_text = ""
    last_observed_text = ""
    last_text_change_at = started
    last_final_reply_state: str | None = None

    async def read_main_text() -> str:
        try:
            return await page.locator("main").inner_text(timeout=1000)  # type: ignore[attr-defined]
        except Exception:
            return ""

    while time.monotonic() < deadline:
        await _approve_visible_tool_gate(page)
        terminal_failure = await _observe_terminal_run_failure(
            page,
            baseline_count=error_count_before,
        )
        if terminal_failure is not None:
            raise TerminalRunFailure(terminal_failure)
        assistant_blocks = page.locator("[data-testid='msg-assistant']")  # type: ignore[attr-defined]
        assistant_count = await assistant_blocks.count()
        if assistant_count > max(0, assistant_count_before):
            assistant = assistant_blocks.last
            try:
                final_assistant_text = await assistant.inner_text(timeout=1000)
            except Exception:
                final_assistant_text = ""
            else:
                last_final_assistant_text = final_assistant_text
            text = final_assistant_text
            try:
                observed_final_reply_state = await assistant.get_attribute(
                    "data-final-reply",
                    timeout=1000,
                )
            except Exception:
                observed_final_reply_state = last_final_reply_state
            else:
                last_final_reply_state = observed_final_reply_state
            try:
                all_block_texts = await assistant_blocks.all_inner_texts()
                block_texts = [
                    block.strip()
                    for block in all_block_texts[max(0, assistant_count_before) :]
                    if block.strip()
                ]
            except Exception:
                block_texts = []
            if block_texts:
                text = "\n".join(block_texts)
            if text:
                now = time.monotonic()
                if text != last_observed_text:
                    last_observed_text = text
                    last_text_change_at = now
                last_text = text
            normalized = text.lower()
            marker_matches = (
                not enforce_marker
                or not marker
                or marker in last_final_assistant_text
            )
            required_text_matched = required_text_matches(normalized, required_text)
            if last_final_reply_state == "true" and not marker_matches:
                raise AssertionError(
                    "finalized assistant reply did not contain required marker. "
                    f"marker={marker!r} "
                    f"last_assistant={last_final_assistant_text[-500:]!r}"
                )
            if marker_matches and required_text_matched:
                if last_final_reply_state == "false":
                    await asyncio.sleep(ASSISTANT_REPLY_POLL_SECONDS)
                    continue
                if last_final_reply_state != "true":
                    quiet_for = time.monotonic() - last_text_change_at
                    if quiet_for < ASSISTANT_REPLY_FALLBACK_QUIET_SECONDS:
                        await asyncio.sleep(ASSISTANT_REPLY_POLL_SECONDS)
                        continue
                return AssistantReplyWaitResult(
                    text_excerpt=text[-ASSISTANT_REPLY_EXCERPT_MAX_CHARS:],
                    full_text=text,
                    semantic_judge_used=False,
                    semantic_judge_reason="literal_required_text_matched",
                    final_reply_wait_ms=int((time.monotonic() - started) * 1000),
                    final_reply_reason=(
                        "final_reply_observed"
                        if last_final_reply_state == "true"
                        else "fallback_quiet_period_matched"
                    ),
                )
            if last_final_reply_state == "true":
                break
        await asyncio.sleep(ASSISTANT_REPLY_POLL_SECONDS)
    main_text = await read_main_text()
    semantic_judge: dict[str, object] | None = None
    marker_matches = (
        not enforce_marker or not marker or marker in last_final_assistant_text
    )
    if last_text and marker_matches and last_final_reply_state != "false":
        semantic_judge = await _judge_assistant_reply_completion(
            marker=marker if enforce_marker else None,
            required_text=required_text,
            assistant_text=last_text,
            main_text=main_text,
            semantic_goal=semantic_goal,
        )
        if _semantic_judge_passed(semantic_judge):
            return AssistantReplyWaitResult(
                text_excerpt=last_text[-ASSISTANT_REPLY_EXCERPT_MAX_CHARS:],
                full_text=last_text,
                semantic_judge_used=True,
                semantic_judge_reason="semantic_judge_completed",
                final_reply_wait_ms=int((time.monotonic() - started) * 1000),
                final_reply_reason=(
                    "semantic_judge_final_reply_observed"
                    if last_final_reply_state == "true"
                    else "semantic_judge_timeout_fallback"
                ),
                semantic_judge=semantic_judge,
            )
    raise AssertionError(
        "assistant reply did not contain required text before timeout. "
        f"marker={marker!r} required_text={required_text!r} "
        f"enforce_marker={enforce_marker!r} "
        f"assistant_count_before={assistant_count_before!r} "
        f"latest_final_reply_state={last_final_reply_state!r} "
        f"last_assistant={last_text[-500:]!r} main_excerpt={main_text[-1000:]!r} "
        f"semantic_judge={_compact_json(semantic_judge)}"
    )


async def _approve_visible_tool_gate(page: object) -> None:
    approve = page.get_by_role("button", name="Approve").last  # type: ignore[attr-defined]
    try:
        if await approve.is_visible(timeout=250):
            await approve.click()
            await asyncio.sleep(0.5)
    except Exception:
        return


async def _fetch_webui_json(page: object, path: str) -> dict[str, object]:
    return await _webui_json(page, "GET", path)


async def _webui_json(
    page: object,
    method: str,
    path: str,
    payload: dict[str, object] | None = None,
) -> dict[str, object]:
    result = await page.evaluate(  # type: ignore[attr-defined]
        """async ({ method, path, token, payload }) => {
            const init = {
                method,
                headers: { "Authorization": `Bearer ${token}` },
            };
            if (payload !== null) {
                init.headers["Content-Type"] = "application/json";
                init.body = JSON.stringify(payload);
            }
            const response = await fetch(path, {
                ...init,
            });
            let body = null;
            try {
                body = await response.json();
            } catch (_error) {
                body = await response.text();
            }
            return { status: response.status, body };
        }""",
        {"method": method, "path": path, "token": AUTH_TOKEN, "payload": payload},
    )
    if not isinstance(result, dict):
        raise AssertionError(f"WebUI API {path} returned non-object result: {result!r}")
    status = int(result.get("status") or 0)
    if status < 200 or status >= 300:
        raise AssertionError(f"WebUI API {path} returned HTTP {status}: {result.get('body')!r}")
    body = result.get("body")
    if not isinstance(body, dict):
        raise AssertionError(f"WebUI API {path} returned non-object body: {body!r}")
    return body


async def _live_http_status(url: str) -> int:
    import httpx

    async with httpx.AsyncClient(timeout=20.0, follow_redirects=True) as client:
        response = await client.get(url)
    return response.status_code


async def _live_github_latest_release(owner: str, repo: str) -> dict[str, str]:
    import httpx

    url = f"https://api.github.com/repos/{owner}/{repo}/releases/latest"
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "ironclaw-reborn-webui-v2-live-qa",
    }
    token = _first_env_value(
        [
            "AUTH_LIVE_GITHUB_TOKEN",
            "IRONCLAW_REBORN_GITHUB_TOKEN",
            "LIVE_CANARY_GITHUB_TOKEN",
            "GITHUB_TOKEN",
            "GH_TOKEN",
        ]
    )
    if token:
        headers["Authorization"] = f"Bearer {token[1]}"
    async with httpx.AsyncClient(timeout=20.0, follow_redirects=True, headers=headers) as client:
        response = await client.get(url)
        response.raise_for_status()
    payload = response.json()
    tag_name = str(payload.get("tag_name") or "").strip()
    release_name = str(payload.get("name") or "").strip()
    if not tag_name:
        raise LiveQaError(f"GitHub latest release for {owner}/{repo} did not include tag_name")
    return {
        "api_url": url,
        "tag_name": tag_name,
        "release_name": release_name,
    }


async def _wait_for_google_sheet_marker_after_slack_event(
    ctx: LiveQaContext,
    *,
    event_id: str,
    access_token: str,
    spreadsheet_id: str,
    marker: str,
    timeout: float = 240.0,
    range_name: str = "A1:Z1000",
) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    last_check: dict[str, object] | None = None
    approved_gate_refs: set[str] = set()
    approval_attempts: list[dict[str, object]] = []
    event_run_id: str | None = None
    while time.monotonic() < deadline:
        approval = await _approve_slack_event_gates(
            ctx,
            event_id=event_id,
            approved_gate_refs=approved_gate_refs,
        )
        if approval.get("run_id"):
            event_run_id = str(approval["run_id"])
        attempts = approval.get("approval_attempts")
        if isinstance(attempts, list):
            approval_attempts.extend(
                attempt for attempt in attempts if isinstance(attempt, dict)
            )
        last_check = await _google_sheet_contains_marker(
            access_token=access_token,
            spreadsheet_id=spreadsheet_id,
            marker=marker,
            range_name=range_name,
        )
        if last_check.get("found"):
            return {
                **last_check,
                "slack_event_run_id": event_run_id,
                "approval_attempts": approval_attempts[-5:],
            }
        await asyncio.sleep(2.0)
    raise AssertionError(
        "Google Sheet marker was not observed before timeout. "
        f"spreadsheet_id_present={bool(spreadsheet_id)} marker={marker!r} "
        f"last_check={last_check!r} approval_attempts={approval_attempts[-3:]!r} "
        f"slack_event_run_id={event_run_id!r}"
    )


async def case_qa_3b_endpoint_status_live_chat(ctx: LiveQaContext) -> ProbeResult:
    url = ENDPOINT_STATUS_URL
    live_status = await _live_http_status(url)
    return await _live_chat_case(
        ctx,
        case_name="qa_3b_endpoint_status_live_chat",
        prompt=_qa_sheet_prompt("qa_3b_endpoint_status_live_chat"),
        marker=None,
        required_text=["status|http|200|up|running|responded"],
        extra_details={"endpoint_url": url, "expected_status_code": live_status},
    )


def _trigger_record_count(reborn_home: Path, routine_name: str | None = None) -> int:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return 0
    with closing(sqlite3.connect(db_path)) as db:
        if routine_name:
            cursor = db.execute(
                "SELECT COUNT(*) FROM trigger_records WHERE name = ?",
                (routine_name,),
            )
        else:
            cursor = db.execute("SELECT COUNT(*) FROM trigger_records")
        value = cursor.fetchone()[0]
    return int(value)


ROUTINE_TRIGGER_RECORD_WAIT_TIMEOUT_SECONDS = 120.0
ROUTINE_TRIGGER_RECORD_POLL_SECONDS = 2.0

# Routine creation is a heavy, multi-step assistant turn (list sheets ->
# read -> compose -> create trigger). The former 180s per-turn reply wait
# timed out mid-work on slower model runs, blocking qa_2e / qa_6d / qa_9b at
# a near-uniform ~182-184s with latest_final_reply_state='false'. 300s gives
# it the same headroom as the comparably heavy calendar-prep live-chat turn.
ROUTINE_CREATION_REPLY_TIMEOUT_SECONDS = 300.0


async def _wait_for_trigger_record_after_count(
    reborn_home: Path,
    routine_name: str | None,
    *,
    before_count: int,
    timeout: float = ROUTINE_TRIGGER_RECORD_WAIT_TIMEOUT_SECONDS,
    poll_interval: float = ROUTINE_TRIGGER_RECORD_POLL_SECONDS,
) -> tuple[int, int]:
    started = time.monotonic()
    deadline = started + timeout
    last_count = _trigger_record_count(reborn_home, routine_name)
    while last_count <= before_count:
        now = time.monotonic()
        if now >= deadline:
            break
        await asyncio.sleep(min(poll_interval, deadline - now))
        last_count = _trigger_record_count(reborn_home, routine_name)
    waited_ms = int((time.monotonic() - started) * 1000)
    return last_count, waited_ms


def _trigger_run_rows(reborn_home: Path, routine_name: str) -> list[dict[str, object]]:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return []
    with closing(sqlite3.connect(db_path)) as db:
        db.row_factory = sqlite3.Row
        rows = db.execute(
            """
            SELECT tr.trigger_id, tr.name, tr.last_status, tr.next_run_at,
                   rh.fire_slot, rh.run_id, rh.thread_id, rh.status,
                   rh.submitted_at, rh.completed_at
            FROM trigger_records tr
            JOIN trigger_run_history rh
              ON rh.tenant_id = tr.tenant_id AND rh.trigger_id = tr.trigger_id
            WHERE tr.name = ?
            ORDER BY rh.submitted_at DESC
            """,
            (routine_name,),
        ).fetchall()
    return [dict(row) for row in rows]


def _parse_epoch_seconds(value: object) -> float | None:
    """Parse the trigger store's RFC3339 timestamps (nanosecond precision,
    trailing Z) into epoch seconds. Returns None when unparseable."""
    if not isinstance(value, str) or not value.strip():
        return None
    text = value.strip()
    # datetime.fromisoformat rejects nanosecond fractions and (pre-3.11) 'Z'.
    text = re.sub(r"\.\d+", "", text).replace("Z", "+00:00")
    try:
        return datetime.fromisoformat(text).timestamp()
    except ValueError:
        return None


def _outbound_final_reply_targets(reborn_home: Path) -> dict[str, object]:
    """Every persisted user-default final-reply target, keyed by row path.

    qa_9d compares this before creation and after delivery: per-trigger
    routing must NOT be implemented by silently rewriting the user-wide
    default delivery target."""
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    targets: dict[str, object] = {}
    if not db_path.exists():
        return targets
    try:
        with closing(sqlite3.connect(db_path)) as db:
            rows = db.execute(
                "SELECT path, contents FROM root_filesystem_entries "
                "WHERE path LIKE '%/outbound/communication-preferences/%' "
                "AND is_dir = 0"
            ).fetchall()
    except sqlite3.Error:
        return targets
    for path, contents in rows:
        if isinstance(contents, bytes):
            contents = contents.decode("utf-8", "replace")
        try:
            record = json.loads(contents) if contents else {}
        except (json.JSONDecodeError, TypeError):
            continue
        if isinstance(record, dict):
            targets[str(path)] = record.get("final_reply_target")
    return targets


def _trigger_record_snapshot(reborn_home: Path, routine_name: str) -> dict[str, object]:
    """Read count/schedule/delivery-target facts for one routine from the
    server DB.

    `delivery_target_column_missing` is reported separately (the column only
    exists on servers with per-trigger delivery routing) while the schedule
    columns exist on every server version this probe targets, so pre-fix
    servers still get schedule preconditions checked.
    """
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    snapshot: dict[str, object] = {
        "checked": False,
        "record_count": 0,
        "schedule_kind": None,
        "next_run_at": None,
        "delivery_target": None,
        "delivery_target_column_missing": False,
    }
    if not db_path.exists():
        snapshot["error"] = "reborn-local-dev.db missing"
        return snapshot
    try:
        with closing(sqlite3.connect(db_path)) as db:
            rows = db.execute(
                "SELECT schedule_kind, next_run_at FROM trigger_records WHERE name = ?",
                (routine_name,),
            ).fetchall()
            snapshot["record_count"] = len(rows)
            if rows:
                snapshot["schedule_kind"] = rows[0][0]
                snapshot["next_run_at"] = rows[0][1]
            try:
                target_rows = db.execute(
                    "SELECT delivery_target FROM trigger_records WHERE name = ?",
                    (routine_name,),
                ).fetchall()
                if target_rows:
                    snapshot["delivery_target"] = target_rows[0][0]
            except sqlite3.OperationalError as exc:
                if "no such column" not in str(exc):
                    raise
                snapshot["delivery_target_column_missing"] = True
            snapshot["checked"] = True
    except sqlite3.Error as exc:
        snapshot["error"] = _exc_text(exc)
    return snapshot


def _triggered_delivery_outcome(reborn_home: Path, run_id: str) -> dict[str, object] | None:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    with closing(sqlite3.connect(db_path)) as db:
        row = db.execute(
            """
            SELECT path, contents FROM root_filesystem_entries
            WHERE path LIKE '%/outbound/triggered-run-delivery/' || ? || '.json'
            ORDER BY path
            LIMIT 1
            """,
            (run_id,),
        ).fetchone()
    if not row:
        return None
    try:
        payload = json.loads(row[1])
    except (TypeError, json.JSONDecodeError):
        payload = {"raw_contents": str(row[1])}
    if isinstance(payload, dict):
        payload["path"] = row[0]
        return payload
    return {"path": row[0], "raw_contents": payload}


def _delivered_gate_routes_for_run(reborn_home: Path, run_id: str) -> list[dict[str, object]]:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists() or not run_id:
        return []
    with closing(sqlite3.connect(db_path)) as db:
        rows = db.execute(
            """
            SELECT path, contents FROM root_filesystem_entries
            WHERE path LIKE '%/outbound/delivered-gate-routes/%'
              AND CAST(contents AS TEXT) LIKE '%' || ? || '%'
            ORDER BY updated_at DESC, path DESC
            """,
            (run_id,),
        ).fetchall()
    routes: list[dict[str, object]] = []
    for path, raw in rows:
        try:
            payload = json.loads(raw)
        except (TypeError, json.JSONDecodeError):
            continue
        if not isinstance(payload, dict):
            continue
        if str(payload.get("run_id") or "") != run_id:
            continue
        gate_ref = str(payload.get("gate_ref") or "").strip()
        thread_id = ""
        scope = payload.get("scope")
        if isinstance(scope, dict):
            thread_id = str(scope.get("thread_id") or "").strip()
        if gate_ref and thread_id:
            routes.append(
                {
                    "path": path,
                    "gate_ref": gate_ref,
                    "thread_id": thread_id,
                    "run_id": run_id,
                }
            )
    return routes


def _slack_event_run_id_for_event(reborn_home: Path, event_id: str) -> str | None:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists() or not event_id:
        return None
    with closing(sqlite3.connect(db_path)) as db:
        row = db.execute(
            """
            SELECT contents FROM root_filesystem_entries
            WHERE path LIKE '%/slack-product-workflow/idempotency/actions/%'
              AND CAST(contents AS TEXT) LIKE '%' || ? || '%'
            ORDER BY updated_at DESC, path DESC
            LIMIT 1
            """,
            (event_id,),
        ).fetchone()
    if not row:
        return None
    try:
        payload = json.loads(row[0])
    except (TypeError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    dispatch_kind = payload.get("dispatch_kind")
    if isinstance(dispatch_kind, dict):
        user_message_turn = dispatch_kind.get("user_message_turn")
        if isinstance(user_message_turn, dict):
            run_id = str(user_message_turn.get("run_id") or "").strip()
            if run_id:
                return run_id
    outcome = payload.get("outcome")
    if isinstance(outcome, dict):
        accepted = outcome.get("accepted")
        if isinstance(accepted, dict):
            run_id = str(accepted.get("submitted_run_id") or "").strip()
            if run_id:
                return run_id
    return None


async def _approve_delivered_gate_routes_for_run(
    ctx: LiveQaContext,
    *,
    run_id: str,
    approved_gate_refs: set[str],
) -> list[dict[str, object]]:
    approval_attempts: list[dict[str, object]] = []
    for route in _delivered_gate_routes_for_run(ctx.reborn_home, run_id):
        gate_ref = str(route.get("gate_ref") or "")
        if not gate_ref or gate_ref in approved_gate_refs:
            continue
        approved_gate_refs.add(gate_ref)
        approval_attempts.append(
            await _resolve_webui_approval_gate(
                ctx,
                thread_id=str(route["thread_id"]),
                run_id=run_id,
                gate_ref=gate_ref,
            )
        )
    return approval_attempts


async def _approve_slack_event_gates(
    ctx: LiveQaContext,
    *,
    event_id: str,
    approved_gate_refs: set[str],
) -> dict[str, object]:
    run_id = _slack_event_run_id_for_event(ctx.reborn_home, event_id)
    if not run_id:
        return {"run_id": None, "approval_attempts": []}
    return {
        "run_id": run_id,
        "approval_attempts": await _approve_delivered_gate_routes_for_run(
            ctx,
            run_id=run_id,
            approved_gate_refs=approved_gate_refs,
        ),
    }


async def _wait_for_slack_event_run_id(
    ctx: LiveQaContext,
    *,
    event_id: str,
    timeout: float = 180.0,
) -> str:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        run_id = _slack_event_run_id_for_event(ctx.reborn_home, event_id)
        if run_id:
            return run_id
        await asyncio.sleep(1.0)
    raise AssertionError(
        "Slack event was not accepted into a Reborn run before timeout. "
        f"event_id={event_id!r}"
    )


async def _resolve_webui_approval_gate(
    ctx: LiveQaContext,
    *,
    thread_id: str,
    run_id: str,
    gate_ref: str,
) -> dict[str, object]:
    import httpx

    encoded_gate = urllib.parse.quote(gate_ref, safe="")
    url = (
        f"{ctx.base_url}/api/webchat/v2/threads/{thread_id}"
        f"/runs/{run_id}/gates/{encoded_gate}/resolve"
    )
    payload = {
        "resolution": "approved",
        "always": False,
        "client_action_id": f"live-qa-{uuid.uuid4()}",
    }
    async with httpx.AsyncClient(timeout=20.0) as client:
        response = await client.post(
            url,
            headers={
                "Authorization": f"Bearer {AUTH_TOKEN}",
                "Content-Type": "application/json",
            },
            json=payload,
        )
    try:
        body: object = response.json()
    except json.JSONDecodeError:
        body = response.text
    result: dict[str, object] = {
        "status": response.status_code,
        "body": body,
        "thread_id": thread_id,
        "run_id": run_id,
        "gate_ref": gate_ref,
    }
    if response.status_code < 200 or response.status_code >= 300:
        raise AssertionError(f"resolve gate returned HTTP {response.status_code}: {body!r}")
    return result


def _slack_bot_token(extra_env: dict[str, str]) -> str | None:
    return _env_value(SLACK_BOT_TOKEN_ENV, extra_env)


def _slack_personal_token(extra_env: dict[str, str]) -> str | None:
    # Same fallback trio the seeding path accepts (slack_helpers), so a local
    # run configured under an alternate env name keeps the sweep and the
    # digest ground-truth arms instead of silently skipping them.
    for env_name in SLACK_PERSONAL_ACCESS_TOKEN_ENV_NAMES:
        value = _env_value(env_name, extra_env)
        if value:
            return value
    return None


def _slack_second_user_token(extra_env: dict[str, str]) -> str | None:
    """Optional SECOND human Slack identity (future dedicated canary user).

    The bot token is actor B wherever a bot can act (seeding DM/channel
    fixtures); an arm that strictly needs a second HUMAN must go through
    ``_require_slack_second_user_token`` so an unprovisioned environment
    fails the case loudly instead of skipping silently.
    """
    return _env_value(SLACK_SECOND_USER_TOKEN_ENV, extra_env)


def _extension_is_listed(extensions: list[object], package_id: str) -> bool:
    return any(
        isinstance(extension, dict)
        and isinstance(extension.get("package_ref"), dict)
        and extension["package_ref"].get("id") == package_id
        for extension in extensions
    )


async def _ensure_extension_installed_on_page(
    page: object,
    observed: dict[str, object],
    *,
    package_id: str,
    display_name: str,
) -> None:
    extensions_body = await _fetch_webui_json(page, "/api/webchat/v2/extensions")
    extensions = extensions_body.get("extensions")
    if not isinstance(extensions, list):
        raise AssertionError(
            f"extensions body did not include a list: {extensions_body!r}"
        )
    prefix = package_id.replace("-", "_")
    if _extension_is_listed(extensions, package_id):
        observed[f"{prefix}_install_message"] = f"{display_name} already installed"
        observed[f"{prefix}_install_onboarding_state"] = "existing_installation"
        return

    install_body = await _webui_json(
        page,
        "POST",
        "/api/webchat/v2/extensions/install",
        {"package_ref": {"kind": "extension", "id": package_id}},
    )
    if install_body.get("success") is not True:
        raise AssertionError(
            f"{display_name} install did not succeed: {install_body!r}"
        )
    observed[f"{prefix}_install_message"] = install_body.get("message")
    observed[f"{prefix}_install_onboarding_state"] = install_body.get(
        "onboarding_state"
    )


async def _installed_active_extension_ids(ctx: LiveQaContext) -> dict[str, object]:
    """Active extension package ids from the server's own extensions API.

    Used as an ASSERTED precondition by the exactly-once probes: without the
    Slack tools extension active, the fired model has no user-token send
    capability and the duplicate arm cannot fail — a vacuous pass. The probe
    must verify the precondition, not hope the model installed it.
    """
    import httpx

    try:
        async with httpx.AsyncClient(timeout=20.0) as client:
            response = await client.get(
                f"{ctx.base_url}/api/webchat/v2/extensions",
                headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
            )
        if response.status_code != 200:
            return {
                "checked": False,
                "error": f"extensions API status {response.status_code}",
            }
        body = response.json()
    except Exception as exc:
        # A transient HTTP/parse hiccup must fail the CASE with a clear
        # message, never crash the shard runner and discard its results.
        return {"checked": False, "error": _exc_text(exc)}
    extensions = body.get("extensions")
    if not isinstance(extensions, list):
        return {"checked": False, "error": "extensions body did not include a list"}
    active_ids: list[str] = []
    for extension in extensions:
        if not isinstance(extension, dict):
            continue
        package_ref = extension.get("package_ref")
        ref_id = package_ref.get("id") if isinstance(package_ref, dict) else None
        if ref_id and extension.get("active") is True:
            active_ids.append(str(ref_id))
    return {"checked": True, "active_extension_ids": active_ids}


async def _slack_search_marker_hits(
    ctx: LiveQaContext, *, marker: str
) -> dict[str, object]:
    """Workspace-wide marker sweep via the personal (user) token.

    Finds stray copies of a delivery marker OUTSIDE the expected DM — the
    wrong-channel and user-identity-duplicate failure shapes post where the
    bot-token DM history scan cannot see. Search indexing can lag, so callers
    must treat "no hits" as inconclusive and only hard-fail on hits in
    unexpected conversations.
    """
    import httpx

    token = _slack_personal_token(ctx.env)
    if not token:
        return {"checked": False, "permanent": True, "error": "personal token unavailable"}
    try:
        async with httpx.AsyncClient(timeout=20.0) as client:
            response = await client.get(
                "https://slack.com/api/search.messages",
                headers={"Authorization": f"Bearer {token}"},
                params={"query": f'"{marker}"', "count": "20"},
            )
        payload = response.json()
    except Exception as exc:
        return {"checked": False, "permanent": False, "error": _exc_text(exc)}
    if not payload.get("ok"):
        error = str(payload.get("error") or "slack_search_failed")
        # Permanent token problems mean the sweep can NEVER run in this
        # environment — callers must surface that instead of fail-opening
        # forever (a hollow green is worse than a red asking for env repair).
        permanent = error in {
            "missing_scope",
            "invalid_auth",
            "account_inactive",
            "token_revoked",
            "not_allowed_token_type",
            "no_permission",
        }
        return {"checked": False, "permanent": permanent, "error": error}
    matches = (payload.get("messages") or {}).get("matches") or []
    hits = []
    for match in matches:
        if not isinstance(match, dict):
            continue
        channel = match.get("channel") or {}
        hits.append(
            {
                "channel_id": (channel.get("id") if isinstance(channel, dict) else None),
                "channel_name": (
                    channel.get("name") if isinstance(channel, dict) else None
                ),
                "username": match.get("username"),
                "user": match.get("user"),
                "ts": match.get("ts"),
            }
        )
    return {"checked": True, "hits": hits}


# Bounded search-index readiness window for freshly-seeded markers. Slack's
# search index is eventually consistent — a Web-API post is typically not
# searchable for ~15-30s after it lands — so probes that answer via search
# must gate the agent turn on the seed becoming searchable first.
SLACK_SEARCH_INDEX_READINESS_TIMEOUT_SECONDS = 45.0
SLACK_SEARCH_INDEX_READINESS_POLL_SECONDS = 3.0


async def _wait_for_slack_search_marker(
    ctx: LiveQaContext,
    *,
    marker: str,
    timeout: float = SLACK_SEARCH_INDEX_READINESS_TIMEOUT_SECONDS,
    poll_interval: float = SLACK_SEARCH_INDEX_READINESS_POLL_SECONDS,
) -> dict[str, object]:
    """Poll Slack search until a freshly-seeded marker becomes searchable.

    Slack's search index is eventually consistent: a message posted via the
    Web API is not searchable for many seconds after it lands. A probe that
    answers a "most recent message I sent" question by calling
    ``search.messages`` (``from:me`` newest-first) can therefore surface an
    OLDER already-indexed seed while the newest one is still un-indexed — an
    external-lag artifact, not an agent regression. This barrier gates the
    agent turn on the newest seed becoming searchable so the caller can
    surface an INCONCLUSIVE result on timeout instead of a spurious
    answer-mismatch red.

    Reuses :func:`_slack_search_marker_hits` (personal-token workspace sweep).
    Returns a dict:
      ready      — True once the marker appears in search results
      checked    — whether the sweep ever actually ran (False => never ran)
      permanent  — True when search can NEVER run here (missing scope, ...)
      attempts   — number of search calls made
      waited_ms  — elapsed poll time
      error      — last search error, when any
    """
    started = time.monotonic()
    deadline = started + timeout
    attempts = 0
    checked_any = False
    last_error: str | None = None
    while True:
        # Deadline check BEFORE the sweep: each sweep carries its own HTTP
        # timeout, so starting one at/past the deadline could stretch the
        # advertised readiness bound by a full extra sweep.
        if time.monotonic() >= deadline:
            return {
                "ready": False,
                "checked": checked_any,
                "permanent": False,
                "attempts": attempts,
                "waited_ms": int((time.monotonic() - started) * 1000),
                "error": last_error,
            }
        attempts += 1
        sweep = await _slack_search_marker_hits(ctx, marker=marker)
        if sweep.get("checked"):
            checked_any = True
            # A sweep that ran cleanly supersedes any earlier transient error;
            # a timeout after this point must not report a stale one.
            last_error = None
            if sweep.get("hits"):
                return {
                    "ready": True,
                    "checked": True,
                    "permanent": False,
                    "attempts": attempts,
                    "waited_ms": int((time.monotonic() - started) * 1000),
                }
        else:
            last_error = str(sweep.get("error") or "slack_search_unavailable")
            if sweep.get("permanent"):
                # The sweep can never run in this environment — spinning to the
                # deadline would only hide the real env-repair cause.
                return {
                    "ready": False,
                    "checked": checked_any,
                    "permanent": True,
                    "attempts": attempts,
                    "waited_ms": int((time.monotonic() - started) * 1000),
                    "error": last_error,
                }
        now = time.monotonic()
        if now >= deadline:
            return {
                "ready": False,
                "checked": checked_any,
                "permanent": False,
                "attempts": attempts,
                "waited_ms": int((time.monotonic() - started) * 1000),
                "error": last_error,
            }
        await asyncio.sleep(min(poll_interval, deadline - now))


async def _slack_personal_dm_counterpart_names(
    ctx: LiveQaContext,
) -> dict[str, object]:
    """Ground-truth display names of the connected user's HUMAN DM
    counterparts, read directly from the Slack API with the personal token —
    the digest probe requires at least one of these names in the model's
    answer.

    Slackbot, the user's own self-DM, bot users, and deactivated accounts are
    excluded: a digest that reasonably reports "no human DMs" must not be
    failed against ground truth the model was right to omit.
    """
    import httpx

    token = _slack_personal_token(ctx.env)
    if not token:
        return {"checked": False, "names": [], "error": "personal token unavailable"}
    try:
        async with httpx.AsyncClient(timeout=20.0) as client:
            auth_probe = await client.get(
                "https://slack.com/api/auth.test",
                headers={"Authorization": f"Bearer {token}"},
            )
            auth_payload = auth_probe.json()
            own_user_id = (
                str(auth_payload.get("user_id") or "")
                if auth_payload.get("ok")
                else ""
            )
            channels: list[dict[str, object]] = []
            cursor = ""
            # Ground truth must cover EVERY im: a single unpaginated page can
            # flip the verdict, so follow next_cursor to exhaustion and report
            # checked=False when the scan is incomplete.
            for _ in range(10):
                params: dict[str, str] = {"types": "im", "limit": "200"}
                if cursor:
                    params["cursor"] = cursor
                response = await client.get(
                    "https://slack.com/api/conversations.list",
                    headers={"Authorization": f"Bearer {token}"},
                    params=params,
                )
                payload = response.json()
                if not payload.get("ok"):
                    return {
                        "checked": False,
                        "names": [],
                        "error": payload.get("error")
                        or "slack_conversations_list_failed",
                    }
                channels.extend(
                    channel
                    for channel in payload.get("channels") or []
                    if isinstance(channel, dict)
                )
                cursor = str(
                    (payload.get("response_metadata") or {}).get("next_cursor") or ""
                ).strip()
                if not cursor:
                    break
            else:
                return {
                    "checked": False,
                    "names": [],
                    "error": "conversations.list pagination exceeded the page cap",
                }
            names: list[str] = []
            skipped: list[str] = []
            for channel in channels:
                counterpart = (
                    channel.get("user") if isinstance(channel, dict) else None
                )
                if not counterpart:
                    continue
                if counterpart == "USLACKBOT" or counterpart == own_user_id:
                    skipped.append(str(counterpart))
                    continue
                info = await client.get(
                    "https://slack.com/api/users.info",
                    headers={"Authorization": f"Bearer {token}"},
                    params={"user": counterpart},
                )
                info_payload = info.json()
                if not info_payload.get("ok"):
                    continue
                user = info_payload.get("user") or {}
                if user.get("is_bot") or user.get("deleted"):
                    skipped.append(str(counterpart))
                    continue
                profile = user.get("profile") or {}
                for candidate in (
                    profile.get("display_name"),
                    profile.get("real_name"),
                    user.get("name"),
                ):
                    if candidate:
                        names.append(str(candidate))
                        break
    except Exception as exc:
        # Ground truth is an enrichment arm: a transient Slack API failure
        # must degrade to "unchecked" (the case records the skip), never
        # crash the shard runner.
        return {"checked": False, "names": [], "error": _exc_text(exc)}
    return {"checked": True, "names": names, "skipped_non_human": skipped}


def _slack_delivery_channel_id(ctx: LiveQaContext) -> str | None:
    slack = _slack_preflight(ctx)
    discovery = slack.get("route_discovery")
    if isinstance(discovery, dict):
        if discovery.get("checked") and not discovery.get("ok"):
            return None
        if discovery.get("ok"):
            seed = discovery.get("personal_dm_seed")
            if isinstance(seed, dict):
                seeded_channel_id = str(seed.get("dm_channel_id") or "").strip()
                if seeded_channel_id:
                    return seeded_channel_id
            channel_id = str(discovery.get("channel_id") or "").strip()
            if channel_id:
                return channel_id
    persisted_channel_id = _persisted_slack_personal_dm_channel_id(ctx.reborn_home, _auth_user_id())
    if persisted_channel_id:
        return persisted_channel_id
    db_path = ctx.reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    with closing(sqlite3.connect(db_path)) as db:
        row = db.execute(
            """
            SELECT contents FROM root_filesystem_entries
            WHERE path LIKE '%/outbound/communication-preferences/%'
              AND CAST(contents AS TEXT) LIKE '%slack_v2%'
            ORDER BY path LIMIT 1
            """
        ).fetchone()
    if not row:
        return None
    try:
        payload = json.loads(row[0])
    except (TypeError, json.JSONDecodeError):
        return None
    target = str(payload.get("final_reply_target") or "")
    match = re.search(r"conversation:(\d+):([^;]+)", target)
    return match.group(2) if match else None


def _slack_delivery_target_is_dm(channel_id: str | None) -> bool:
    return bool(channel_id and channel_id.startswith("D"))


def _marker_match_stats(
    messages: list[object],
    *,
    marker: str,
    required_text: list[str] | None = None,
) -> dict[str, object]:
    """Scan a Slack history window for a delivery marker.

    Scans ALL messages (not first-match-wins) so the result carries
    exactly-once statistics: the duplicate-delivery bug class posts the same
    marker twice — once from the bot delivery path and once from the user's
    own identity via a messaging tool — split here by authorship (`bot_id`
    present = bot copy; `user` without `bot_id` = human copy).
    """
    first_satisfying: dict[str, object] | None = None
    marker_matches = 0
    bot_authored_marker_matches = 0
    human_authored_marker_matches = 0
    marker_match_authors: list[dict[str, object]] = []
    for message in messages:
        if not isinstance(message, dict):
            continue
        text = str(message.get("text") or "")
        if marker not in text:
            continue
        marker_matches += 1
        is_bot = bool(message.get("bot_id"))
        is_human = bool(message.get("user")) and not is_bot
        if is_bot:
            bot_authored_marker_matches += 1
        elif is_human:
            human_authored_marker_matches += 1
        marker_match_authors.append(
            {"ts": message.get("ts"), "bot": is_bot, "human": is_human}
        )
        if first_satisfying is None:
            normalized = text.lower()
            missing_required = [
                piece for piece in (required_text or []) if piece.lower() not in normalized
            ]
            first_satisfying = {
                "checked": True,
                "found": not missing_required,
                "marker_found": True,
                "missing_required_text": missing_required,
                "message_ts": message.get("ts"),
                "message_user_present": bool(message.get("user") or message.get("bot_id")),
            }
    if first_satisfying is not None:
        first_satisfying["marker_matches"] = marker_matches
        first_satisfying["bot_authored_marker_matches"] = bot_authored_marker_matches
        first_satisfying["human_authored_marker_matches"] = human_authored_marker_matches
        first_satisfying["marker_match_authors"] = marker_match_authors
        return first_satisfying
    return {"checked": True, "found": False, "message_count": len(messages)}


async def _slack_history_contains_marker(
    ctx: LiveQaContext,
    *,
    channel_id: str,
    marker: str,
    oldest_epoch: float,
    required_text: list[str] | None = None,
) -> dict[str, object]:
    import httpx

    token = _slack_bot_token(ctx.env)
    if not token:
        return {"checked": False, "found": False, "error": "bot token unavailable"}
    params = {
        "channel": channel_id,
        "oldest": f"{oldest_epoch:.6f}",
        "limit": "100",
        "inclusive": "true",
    }
    async with httpx.AsyncClient(timeout=20.0) as client:
        response = await client.get(
            "https://slack.com/api/conversations.history",
            headers={"Authorization": f"Bearer {token}"},
            params=params,
        )
    payload = response.json()
    if not payload.get("ok"):
        return {
            "checked": True,
            "found": False,
            "error": payload.get("error") or "slack_history_failed",
            "needed": payload.get("needed"),
        }
    messages = payload.get("messages") if isinstance(payload, dict) else []
    if not isinstance(messages, list):
        messages = []
    return _marker_match_stats(messages, marker=marker, required_text=required_text)


def _slack_delivery_observed(
    outcome: dict[str, object] | None,
    history: dict[str, object] | None,
) -> bool:
    return (
        isinstance(outcome, dict)
        and outcome.get("outcome") == "delivered"
        and isinstance(history, dict)
        and bool(history.get("found"))
    )


class SlackDeliveryReadbackInconclusive(RuntimeError):
    """The exact trigger run sent once, but Slack history did not expose it."""

    def __init__(self, message: str, evidence: dict[str, object]) -> None:
        super().__init__(message)
        self.evidence = evidence


def _trigger_run_slack_send_evidence(
    reborn_home: Path,
    *,
    run_id: str,
    thread_id: str,
    expected_channel_id: str,
    marker: str,
) -> dict[str, object]:
    """Read sanitized ``slack.send_message`` evidence for one trigger run.

    Only aggregate counts leave this helper. Slack channel IDs, message text,
    and output payloads remain in the local runtime database and are never
    copied into canary results.
    """
    evidence: dict[str, object] = {
        "completed_send_count": 0,
        "marker_send_count": 0,
        "expected_channel_marker_send_count": 0,
        "expected_channel_marker_ok_count": 0,
        "wrong_channel_marker_send_count": 0,
        "parse_error_count": 0,
    }
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        evidence["read_error"] = "reborn-local-dev.db missing"
        return evidence
    try:
        database_uri = f"{db_path.resolve().as_uri()}?mode=ro"
        with closing(sqlite3.connect(database_uri, uri=True)) as db:
            rows = db.execute(
                """
                SELECT contents
                FROM root_filesystem_entries
                WHERE is_dir = 0
                  AND content_type = 'application/json'
                  AND path LIKE ?
                """,
                (f"%/threads/{thread_id}/messages/%",),
            ).fetchall()
    except sqlite3.Error as exc:
        evidence["read_error"] = _exc_text(exc)
        return evidence

    def json_object(value: object) -> dict[str, object] | None:
        if isinstance(value, dict):
            return value
        if isinstance(value, bytes):
            value = value.decode("utf-8", errors="replace")
        if not isinstance(value, str):
            return None
        try:
            parsed = json.loads(value)
        except json.JSONDecodeError:
            return None
        return parsed if isinstance(parsed, dict) else None

    for (raw_contents,) in rows:
        message = json_object(raw_contents)
        if (
            message is None
            or message.get("turn_run_id") != run_id
            or message.get("kind") != "capability_display_preview"
        ):
            continue
        preview = json_object(message.get("content"))
        if preview is None:
            evidence["parse_error_count"] = int(evidence["parse_error_count"]) + 1
            continue
        if preview.get("capability_id") != "slack.send_message":
            continue
        input_summary = json_object(preview.get("input_summary"))
        output_preview = json_object(preview.get("output_preview"))
        if input_summary is None or output_preview is None:
            evidence["parse_error_count"] = int(evidence["parse_error_count"]) + 1
            continue

        status = str(preview.get("status") or "")
        if status == "completed":
            evidence["completed_send_count"] = int(evidence["completed_send_count"]) + 1
        text = str(input_summary.get("text") or "")
        if marker not in text:
            continue
        evidence["marker_send_count"] = int(evidence["marker_send_count"]) + 1
        input_channel = str(input_summary.get("channel") or "")
        if input_channel != expected_channel_id:
            evidence["wrong_channel_marker_send_count"] = (
                int(evidence["wrong_channel_marker_send_count"]) + 1
            )
            continue
        evidence["expected_channel_marker_send_count"] = (
            int(evidence["expected_channel_marker_send_count"]) + 1
        )
        if (
            status == "completed"
            and output_preview.get("ok") is True
            and str(output_preview.get("channel") or "") == expected_channel_id
        ):
            evidence["expected_channel_marker_ok_count"] = (
                int(evidence["expected_channel_marker_ok_count"]) + 1
            )
    return evidence


def _slack_delivery_readback_is_inconclusive(
    outcome: dict[str, object] | None,
    history: dict[str, object] | None,
    evidence: dict[str, object],
) -> bool:
    """Distinguish a Slack history miss from wrong or duplicate model sends."""
    return (
        isinstance(outcome, dict)
        and outcome.get("outcome") == "delivered"
        and isinstance(history, dict)
        and history.get("checked") is True
        and not history.get("found")
        and not history.get("error")
        and not evidence.get("read_error")
        and evidence.get("completed_send_count") == 1
        and evidence.get("marker_send_count") == 1
        and evidence.get("expected_channel_marker_send_count") == 1
        and evidence.get("expected_channel_marker_ok_count") == 1
        and evidence.get("wrong_channel_marker_send_count") == 0
        and evidence.get("parse_error_count") == 0
    )


async def _wait_for_slack_delivery_marker(
    ctx: LiveQaContext,
    *,
    routine_name: str,
    marker: str,
    oldest_epoch: float,
    timeout: float = 240.0,
    required_text: list[str] | None = None,
) -> dict[str, object]:
    channel_id = _slack_delivery_channel_id(ctx)
    if not channel_id:
        raise AssertionError("Slack delivery channel could not be resolved from preflight/preferences")
    deadline = time.monotonic() + timeout
    last_rows: list[dict[str, object]] = []
    last_outcome: dict[str, object] | None = None
    last_history: dict[str, object] | None = None
    last_delivered_row: dict[str, object] | None = None
    last_delivered_outcome: dict[str, object] | None = None
    approved_gate_refs: set[str] = set()
    approval_attempts: list[dict[str, object]] = []
    while time.monotonic() < deadline:
        rows = _trigger_run_rows(ctx.reborn_home, routine_name)
        if rows:
            last_rows = rows
            history: dict[str, object] | None = None
            for row in rows:
                run_id = str(row.get("run_id") or "")
                if not run_id:
                    continue
                outcome = _triggered_delivery_outcome(ctx.reborn_home, run_id)
                if outcome:
                    last_outcome = outcome
                    if outcome.get("outcome") == "delivered":
                        last_delivered_row = row
                        last_delivered_outcome = outcome
                for route in _delivered_gate_routes_for_run(ctx.reborn_home, run_id):
                    gate_ref = str(route.get("gate_ref") or "")
                    if gate_ref in approved_gate_refs:
                        continue
                    approved_gate_refs.add(gate_ref)
                    try:
                        approval_attempts.append(
                            await _resolve_webui_approval_gate(
                                ctx,
                                thread_id=str(route["thread_id"]),
                                run_id=run_id,
                                gate_ref=gate_ref,
                            )
                        )
                    except Exception as approve_exc:
                        # Record and allow a later loop pass to retry: a slow
                        # approve POST (server busy mid-fire) must not abort
                        # the delivery wait with an empty timeout error.
                        approved_gate_refs.discard(gate_ref)
                        approval_attempts.append(
                            {
                                "gate_ref": gate_ref,
                                "run_id": run_id,
                                "error": _exc_text(approve_exc),
                            }
                        )
                if history is None:
                    try:
                        history = await _slack_history_contains_marker(
                            ctx,
                            channel_id=channel_id,
                            marker=marker,
                            oldest_epoch=oldest_epoch,
                            required_text=required_text,
                        )
                    except Exception as history_exc:
                        # One flaky Slack API read must not abort the whole
                        # wait — the loop re-polls until its own deadline.
                        history = {
                            "checked": False,
                            "found": False,
                            "error": _exc_text(history_exc),
                        }
                    last_history = history
                if _slack_delivery_observed(outcome, history):
                    return {
                        "trigger_run": row,
                        "delivery_outcome": outcome,
                        "slack_history": history,
                        "approval_attempts": approval_attempts[-5:],
                    }
                if isinstance(outcome, dict) and outcome.get("outcome") not in (None, "delivered"):
                    raise AssertionError(
                        "triggered Slack delivery completed without delivered outcome: "
                        f"run={row!r} outcome={outcome!r} history={history!r}"
                    )
        await asyncio.sleep(2.0)
    if last_delivered_row is not None:
        run_id = str(last_delivered_row.get("run_id") or "")
        thread_id = str(last_delivered_row.get("thread_id") or "")
        if run_id and thread_id:
            send_evidence = _trigger_run_slack_send_evidence(
                ctx.reborn_home,
                run_id=run_id,
                thread_id=thread_id,
                expected_channel_id=channel_id,
                marker=marker,
            )
            if _slack_delivery_readback_is_inconclusive(
                last_delivered_outcome,
                last_history,
                send_evidence,
            ):
                raise SlackDeliveryReadbackInconclusive(
                    "the exact trigger run completed one verified Slack send to the "
                    "expected DM, but the independent Slack history readback did not "
                    "expose the marker before timeout",
                    send_evidence,
                )
    raise AssertionError(
        "Slack delivery marker was not observed before timeout. "
        f"routine_name={routine_name!r} marker={marker!r} "
        f"last_rows={last_rows[:3]!r} last_outcome={last_outcome!r} "
        f"last_history={last_history!r} approvals={approval_attempts[-3:]!r}"
    )


def _slack_preflight(ctx: LiveQaContext) -> dict[str, object]:
    preflight_path = ctx.output_dir / "preflight.json"
    if not preflight_path.exists():
        raise AssertionError(f"preflight file missing: {preflight_path}")
    preflight = json.loads(preflight_path.read_text(encoding="utf-8"))
    checks = preflight.get("checks") if isinstance(preflight, dict) else None
    slack = checks.get("slack") if isinstance(checks, dict) else None
    if not isinstance(slack, dict):
        raise AssertionError(f"preflight Slack check missing in {preflight_path}")
    return slack


def _slack_personal_auth_check(ctx: LiveQaContext) -> dict[str, object]:
    preflight_path = ctx.output_dir / "preflight.json"
    if not preflight_path.exists():
        raise AssertionError(f"preflight file missing: {preflight_path}")
    preflight = json.loads(preflight_path.read_text(encoding="utf-8"))
    checks = preflight.get("checks") if isinstance(preflight, dict) else None
    personal_auth = (
        checks.get("slack_personal_auth")
        if isinstance(checks, dict)
        else None
    )
    if not isinstance(personal_auth, dict):
        raise AssertionError(
            f"preflight Slack personal-auth check missing in {preflight_path}"
        )
    return personal_auth


def _slack_connect_instructions_look_valid(instructions: str) -> bool:
    text = instructions.lower()
    return "connect slack with oauth" in text or ("slack" in text and "oauth" in text)


def _slack_oauth_start_expires_at() -> str:
    expires = datetime.fromtimestamp(time.time() + 600, timezone.utc)
    return expires.isoformat().replace("+00:00", "Z")


def _slack_personal_auth_ready_account(personal_auth: dict[str, object]) -> dict[str, object]:
    accounts = personal_auth.get("accounts")
    if not isinstance(accounts, list):
        return {}
    for account in accounts:
        if isinstance(account, dict) and account.get("ready"):
            return account
    return {}


def _slack_workspace_mismatch_error(
    slack: dict[str, object],
    personal_auth: dict[str, object] | None = None,
    *,
    include_personal_auth: bool,
) -> str | None:
    team_ids: list[tuple[str, str]] = []
    setup = slack.get("setup")
    if isinstance(setup, dict):
        team_id = str(setup.get("team_id") or "").strip()
        if team_id:
            team_ids.append(("setup", team_id))
    auth_test = slack.get("auth_test")
    if isinstance(auth_test, dict) and auth_test.get("ok"):
        team_id = str(auth_test.get("team_id") or "").strip()
        if team_id:
            team_ids.append(("bot_token", team_id))
    if include_personal_auth and isinstance(personal_auth, dict) and personal_auth.get("ready"):
        personal_auth_test = personal_auth.get("auth_test")
        if isinstance(personal_auth_test, dict) and personal_auth_test.get("ok"):
            team_id = str(personal_auth_test.get("team_id") or "").strip()
            if team_id:
                team_ids.append(("personal_oauth", team_id))
    unique_team_ids = {team_id for _, team_id in team_ids}
    if len(unique_team_ids) <= 1:
        return None
    details = ", ".join(f"{source} team_id={team_id}" for source, team_id in team_ids)
    return f"Slack credentials target different workspaces: {details}"


async def _slack_connect_case(ctx: LiveQaContext, *, case_name: str) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    observed: dict[str, object] = {
        "qa_sheet_prompt": _qa_sheet_prompt(case_name),
        "slack_connect_surface": "/extensions/channels",
    }

    async def action(page: object) -> None:
        await page.goto(
            f"{ctx.base_url}/extensions/channels?token={AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )  # type: ignore[attr-defined]
        await expect(page.locator("body")).to_contain_text("Channels", timeout=15000)  # type: ignore[attr-defined]
        body = await _fetch_webui_json(page, "/api/webchat/v2/channels/connectable")
        channels = body.get("channels")
        if not isinstance(channels, list):
            raise AssertionError(f"connectable channels body did not include a list: {body!r}")
        slack_channels = [
            channel
            for channel in channels
            if isinstance(channel, dict) and channel.get("channel") == "slack"
        ]
        observed["connectable_channel_count"] = len(channels)
        observed["slack_strategy_count"] = len(slack_channels)
        observed["slack_strategies"] = [
            channel.get("strategy")
            for channel in slack_channels
            if isinstance(channel, dict)
        ]
        personal = next(
            (
                channel
                for channel in slack_channels
                if isinstance(channel, dict)
                and channel.get("strategy") == "oauth"
            ),
            None,
        )
        if not isinstance(personal, dict):
            raise AssertionError(f"Slack oauth connect strategy missing: {channels!r}")
        action_body = personal.get("action")
        if not isinstance(action_body, dict):
            raise AssertionError(f"Slack connect action missing: {personal!r}")
        title = str(action_body.get("title") or "")
        if not title:
            raise AssertionError(f"Slack connect action title missing: {personal!r}")
        instructions = str(action_body.get("instructions") or "")
        if not _slack_connect_instructions_look_valid(instructions):
            raise AssertionError(f"unexpected Slack connect instructions: {instructions!r}")
        # Extension-scoped OAuth deliberately rejects an absent installation.
        # Exercise the same global install transition as the product UI before
        # probing the OAuth start surface; do not manufacture per-user setup
        # state inside the canary. Reruns reuse an existing installation.
        await _ensure_extension_installed_on_page(
            page,
            observed,
            package_id="slack",
            display_name="Slack",
        )
        account_scope = _slack_personal_auth_ready_account(personal_auth)
        invocation_id = str(account_scope.get("invocation_id") or "").strip()
        thread_id = str(account_scope.get("thread_id") or "").strip()
        if not invocation_id:
            raise AssertionError(
                "Slack personal product-auth preflight did not include an invocation_id"
            )
        accounts_request: dict[str, object] = {
            "provider": "slack_personal",
            "requester_extension": "slack",
            "invocation_id": invocation_id,
            "limit": 10,
        }
        if thread_id:
            accounts_request["thread_id"] = thread_id
        accounts_list = await _webui_json(
            page,
            "POST",
            "/api/reborn/product-auth/accounts/list",
            accounts_request,
        )
        accounts = accounts_list.get("accounts")
        if not isinstance(accounts, list):
            raise AssertionError(f"Slack product-auth accounts response missing list: {accounts_list!r}")
        configured_accounts = [
            account
            for account in accounts
            if isinstance(account, dict)
            and account.get("provider") == "slack_personal"
            and account.get("status") == "configured"
        ]
        if not configured_accounts:
            raise AssertionError(
                f"Slack product-auth accounts list did not include a configured account: {accounts_list!r}"
            )
        oauth_start = await _webui_json(
            page,
            "POST",
            "/api/webchat/v2/extensions/slack/setup/oauth/start",
            {
                "provider": "slack_personal",
                "account_label": "Slack personal OAuth",
                "scopes": [],
                "expires_at": _slack_oauth_start_expires_at(),
                "invocation_id": str(uuid.uuid4()),
            },
        )
        if oauth_start.get("provider") != "slack_personal":
            raise AssertionError(f"Slack OAuth start returned unexpected provider: {oauth_start!r}")
        authorization_url = str(oauth_start.get("authorization_url") or "")
        if not authorization_url.startswith("https://slack.com/oauth/"):
            raise AssertionError(f"Slack OAuth start returned unexpected URL: {oauth_start!r}")
        if "admin_managed_channels" in observed["slack_strategies"]:
            await expect(page.locator("body")).to_contain_text("Slack workspace setup", timeout=15000)  # type: ignore[attr-defined]
        else:
            await expect(page.locator("body")).to_contain_text(title, timeout=15000)  # type: ignore[attr-defined]
            await expect(page.locator("body")).to_contain_text("Connect Slack with OAuth", timeout=15000)  # type: ignore[attr-defined]
        observed["slack_display_name"] = personal.get("display_name")
        observed["slack_connect_title"] = title
        observed["slack_connect_instructions"] = instructions
        observed["slack_product_auth_account_count"] = len(accounts)
        observed["slack_product_auth_configured_account_count"] = len(configured_accounts)
        observed["slack_product_auth_account_ids"] = [
            account.get("id") for account in configured_accounts
        ]
        observed["slack_oauth_start_provider"] = oauth_start.get("provider")
        observed["slack_oauth_start_status"] = oauth_start.get("status")
        observed["slack_oauth_start_url"] = authorization_url

    try:
        slack = _slack_preflight(ctx)
        auth_test = slack.get("auth_test")
        setup = slack.get("setup")
        if not slack.get("enabled_in_config") or not slack.get("env_present"):
            raise AssertionError(f"Slack was not enabled with env in preflight: {slack!r}")
        if not isinstance(setup, dict) or not setup.get("personal_oauth_ready"):
            raise AssertionError(f"Slack personal OAuth is not ready in preflight: {setup!r}")
        if not isinstance(auth_test, dict) or not auth_test.get("ok"):
            raise AssertionError(f"Slack auth.test did not pass in preflight: {auth_test!r}")
        observed["slack_personal_oauth_ready"] = setup.get("personal_oauth_ready")
        observed["slack_oauth_client_id_configured"] = setup.get("oauth_client_id_configured")
        observed["slack_oauth_client_secret_configured"] = setup.get(
            "oauth_client_secret_configured"
        )
        observed["slack_auth_team_id"] = auth_test.get("team_id")
        observed["slack_auth_user_id"] = auth_test.get("user_id")
        personal_auth = _slack_personal_auth_check(ctx)
        if not personal_auth.get("ready"):
            raise AssertionError(
                "Slack personal product-auth account is not ready in preflight: "
                f"{personal_auth.get('reason') or personal_auth!r}"
            )
        personal_auth_test = personal_auth.get("auth_test")
        observed["slack_personal_auth_ready"] = personal_auth.get("ready")
        observed["slack_personal_auth_account_count"] = personal_auth.get(
            "configured_account_count"
        )
        if isinstance(personal_auth_test, dict):
            observed["slack_personal_auth_team_id"] = personal_auth_test.get("team_id")
            observed["slack_personal_auth_user_id"] = personal_auth_test.get("user_id")
        mismatch = _slack_workspace_mismatch_error(
            slack,
            personal_auth,
            include_personal_auth=True,
        )
        if mismatch:
            raise AssertionError(mismatch)
        await _with_page(ctx.output_dir, case_name, action)
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(case_name, False, started, {"error": _exc_text(exc), **observed})


async def case_qa_3a_slack_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _slack_connect_case(ctx, case_name="qa_3a_slack_connect")


async def _extension_authenticated_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    package_id: str,
    display_name: str,
    required_tools: list[str],
    ensure_installed: bool = False,
) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    observed: dict[str, object] = {
        "package_id": package_id,
        "display_name": display_name,
        "required_tools": required_tools,
        "ensure_installed": ensure_installed,
    }

    async def action(page: object) -> None:
        await page.goto(
            f"{ctx.base_url}/extensions/registry?token={AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )  # type: ignore[attr-defined]
        await expect(page.locator("body")).to_contain_text("Extensions", timeout=15000)  # type: ignore[attr-defined]
        await _ensure_extension_authenticated_on_page(
            page,
            observed,
            package_id=package_id,
            display_name=display_name,
            required_tools=required_tools,
            ensure_installed=ensure_installed,
        )

    try:
        await _with_page(ctx.output_dir, case_name, action)
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(case_name, False, started, {"error": _exc_text(exc), **observed})


def _capability_run_statuses(
    reborn_home: Path,
    capability_ids: list[str],
) -> dict[str, list[str]]:
    statuses = {capability_id: [] for capability_id in capability_ids}
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return statuses
    try:
        with closing(sqlite3.connect(db_path)) as db:
            rows = db.execute(
                """
                SELECT contents
                FROM root_filesystem_entries
                WHERE is_dir = 0
                  AND content_type = 'application/json'
                  AND path LIKE '%/run-state/%'
                """
            ).fetchall()
    except sqlite3.Error:
        return statuses
    wanted = set(capability_ids)
    for (contents,) in rows:
        if isinstance(contents, bytes):
            text = contents.decode("utf-8", errors="replace")
        else:
            text = str(contents)
        try:
            payload = json.loads(text)
        except json.JSONDecodeError:
            continue
        if not isinstance(payload, dict):
            continue
        capability_id = payload.get("capability_id")
        if capability_id in wanted:
            statuses[str(capability_id)].append(str(payload.get("status") or "unknown"))
    return statuses


def _completed_capability_counts(
    statuses: dict[str, list[str]],
) -> dict[str, int]:
    return {
        capability_id: capability_statuses.count("completed")
        for capability_id, capability_statuses in statuses.items()
    }


_TERMINAL_CAPABILITY_EVENT_STATUSES = {
    "capability_activity_succeeded": "completed",
    "capability_activity_failed": "failed",
}


def _current_turn_capability_evidence(
    reborn_home: Path,
    submission_identity: dict[str, object],
    capability_ids: list[str],
    allowed_statuses: set[str],
) -> dict[str, object]:
    """Bind terminal capability records to one submitted WebUI turn run."""
    identity = {
        field: str(submission_identity.get(field) or "")
        for field in (
            "accepted_message_ref",
            "thread_id",
            "turn_id",
            "run_id",
        )
    }
    evidence: dict[str, object] = {
        **identity,
        "invocation_ids": {capability_id: [] for capability_id in capability_ids},
        "statuses": {capability_id: [] for capability_id in capability_ids},
        "input_arguments": {capability_id: [] for capability_id in capability_ids},
        "terminal_sequence": [],
    }
    if not all(identity[field] for field in _SUBMISSION_CORRELATION_FIELDS):
        return evidence

    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return evidence
    try:
        database_uri = f"{db_path.resolve().as_uri()}?mode=ro"
        with closing(sqlite3.connect(database_uri, uri=True)) as db:
            event_rows = db.execute(
                """
                SELECT seq, payload
                FROM root_filesystem_events
                WHERE path LIKE '/events/runtime/%'
                ORDER BY seq ASC
                """
            ).fetchall()
            run_state_rows = db.execute(
                """
                SELECT contents
                FROM root_filesystem_entries
                WHERE is_dir = 0
                  AND content_type = 'application/json'
                  AND path LIKE '%/run-state/%'
                """
            ).fetchall()
            display_preview_rows = db.execute(
                """
                SELECT contents
                FROM root_filesystem_entries
                WHERE is_dir = 0
                  AND content_type = 'application/json'
                  AND kind = 'thread_message'
                  AND path LIKE '%/messages/%'
                """
            ).fetchall()
    except sqlite3.Error as exc:
        evidence["read_error"] = _exc_text(exc)
        return evidence

    wanted = set(capability_ids)
    input_arguments_by_invocation: dict[str, dict[str, str]] = {}
    for (raw_contents,) in display_preview_rows:
        try:
            message = json.loads(
                raw_contents.decode("utf-8", errors="replace")
                if isinstance(raw_contents, bytes)
                else str(raw_contents)
            )
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
        if (
            not isinstance(message, dict)
            or message.get("kind") != "capability_display_preview"
            or message.get("thread_id") != identity["thread_id"]
            or message.get("turn_run_id") != identity["run_id"]
        ):
            continue
        raw_preview = message.get("content")
        try:
            preview = (
                json.loads(raw_preview)
                if isinstance(raw_preview, str)
                else raw_preview
            )
        except json.JSONDecodeError:
            continue
        if not isinstance(preview, dict):
            continue
        invocation_id = str(preview.get("invocation_id") or "")
        capability_id = str(preview.get("capability_id") or "")
        raw_input_summary = preview.get("input_summary")
        try:
            input_summary = (
                json.loads(raw_input_summary)
                if isinstance(raw_input_summary, str)
                else raw_input_summary
            )
        except json.JSONDecodeError:
            continue
        if (
            invocation_id
            and capability_id in wanted
            and isinstance(input_summary, dict)
        ):
            # Persist only the routing argument needed for exact-conversation
            # assertions, never message text or other model-supplied content.
            channel = input_summary.get("channel")
            input_arguments_by_invocation[invocation_id] = (
                {"channel": channel} if isinstance(channel, str) else {}
            )

    terminal_events: dict[str, tuple[str, str, int]] = {}
    for raw_seq, raw_payload in event_rows:
        try:
            payload = json.loads(
                raw_payload.decode("utf-8", errors="replace")
                if isinstance(raw_payload, bytes)
                else str(raw_payload)
            )
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
        if not isinstance(payload, dict):
            continue
        capability_id = str(payload.get("capability_id") or "")
        event_status = _TERMINAL_CAPABILITY_EVENT_STATUSES.get(
            str(payload.get("kind") or "")
        )
        scope = payload.get("scope")
        if (
            capability_id not in wanted
            or event_status not in allowed_statuses
            or payload.get("parent_invocation_id") != identity["run_id"]
            or not isinstance(scope, dict)
            or scope.get("thread_id") != identity["thread_id"]
        ):
            continue
        invocation_id = str(scope.get("invocation_id") or "")
        if invocation_id:
            terminal_events[invocation_id] = (
                capability_id,
                event_status,
                int(raw_seq),
            )

    matched: dict[str, list[tuple[int, str, str]]] = {
        capability_id: [] for capability_id in capability_ids
    }
    for (raw_contents,) in run_state_rows:
        try:
            payload = json.loads(
                raw_contents.decode("utf-8", errors="replace")
                if isinstance(raw_contents, bytes)
                else str(raw_contents)
            )
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
        if not isinstance(payload, dict):
            continue
        invocation_id = str(payload.get("invocation_id") or "")
        event = terminal_events.get(invocation_id)
        scope = payload.get("scope")
        status = str(payload.get("status") or "unknown")
        event_capability_id, event_status, event_seq = event or ("", "", 0)
        if (
            event is None
            or payload.get("capability_id") != event_capability_id
            or status != event_status
            or not isinstance(scope, dict)
            or scope.get("thread_id") != identity["thread_id"]
        ):
            continue
        matched[event_capability_id].append((event_seq, invocation_id, status))

    ordered_matches = sorted(
        (
            (seq, capability_id, invocation_id, status)
            for capability_id, matches in matched.items()
            for seq, invocation_id, status in matches
        ),
        key=lambda item: item[0],
    )

    evidence["invocation_ids"] = {
        capability_id: [
            invocation_id
            for _, invocation_id, _ in sorted(matched[capability_id])
        ]
        for capability_id in capability_ids
    }
    evidence["statuses"] = {
        capability_id: [status for _, _, status in sorted(matched[capability_id])]
        for capability_id in capability_ids
    }
    evidence["input_arguments"] = {
        capability_id: [
            input_arguments_by_invocation.get(invocation_id, {})
            for _, invocation_id, _ in sorted(matched[capability_id])
        ]
        for capability_id in capability_ids
    }
    evidence["terminal_sequence"] = [
        {
            "seq": seq,
            "capability_id": capability_id,
            "invocation_id": invocation_id,
            "status": status,
        }
        for seq, capability_id, invocation_id, status in ordered_matches
    ]
    return evidence


async def _extension_chat_connect_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    package_id: str,
    display_name: str,
    required_tools: list[str],
    marker: str,
    verification_instruction: str,
    verification_capabilities: list[str],
) -> ProbeResult:
    started = time.monotonic()
    setup_capabilities = [
        EXTENSION_SEARCH_CAPABILITY_ID,
        EXTENSION_INSTALL_CAPABILITY_ID,
        EXTENSION_ACTIVATE_CAPABILITY_ID,
    ]
    prompt = QA_SHEET_PROMPTS.get(case_name)
    sheet_prompt = prompt is not None
    expected_capabilities = (
        setup_capabilities
        if sheet_prompt
        else [*setup_capabilities, *verification_capabilities]
    )
    marker_to_wait_for: str | None = None
    if prompt is None:
        prompt = (
            f"QA connect case {case_name}: connect my {display_name} from this chat. "
            f"Use extension_search for `{package_id}`, then install and activate "
            f"`{package_id}` if it is not already active. {verification_instruction} "
            "Do not create, update, send, or delete anything. In the final answer "
            f"include the exact marker {marker} and include the words "
            f"{display_name} connected."
        )
        marker_to_wait_for = marker
    chat = await _live_chat_case(
        ctx,
        case_name=case_name,
        prompt=prompt,
        marker=marker_to_wait_for,
        required_text=[display_name, "connected"],
        timeout=240.0,
        extra_details={
            "chat_connect_flow": True,
            "package_id": package_id,
            "required_capabilities": expected_capabilities,
            "verification_capabilities": verification_capabilities,
            "verification_capabilities_required": not sheet_prompt,
        },
        forbidden_text=[
            "auth_denied",
            "authentication required",
            "can't connect",
            "cannot connect",
            "permission denied",
        ],
    )
    if not chat.success:
        chat.latency_ms = int((time.monotonic() - started) * 1000)
        return chat

    observed: dict[str, object] = {
        "marker": marker,
        "chat_connect_flow": True,
        "chat_connect_prompt": prompt,
        "chat_latency_ms": chat.latency_ms,
        "text_excerpt": chat.details.get("text_excerpt"),
        "package_id": package_id,
        "display_name": display_name,
        "required_tools": required_tools,
        "required_capabilities": expected_capabilities,
        "verification_capabilities": verification_capabilities,
        "verification_capabilities_required": not sheet_prompt,
    }
    try:
        statuses = _capability_run_statuses(ctx.reborn_home, expected_capabilities)
        observed["capability_statuses"] = statuses
        missing = [
            capability_id
            for capability_id in expected_capabilities
            if "completed" not in statuses.get(capability_id, [])
        ]
        if missing:
            raise AssertionError(
                "chat connect did not complete expected capabilities: "
                f"{missing!r}; observed statuses={statuses!r}"
            )
        registry_check = await _extension_authenticated_case(
            ctx,
            case_name=case_name,
            package_id=package_id,
            display_name=display_name,
            required_tools=required_tools,
            ensure_installed=False,
        )
        observed["post_chat_registry_check"] = registry_check.details
        observed["post_chat_registry_latency_ms"] = registry_check.latency_ms
        if not registry_check.success:
            raise AssertionError(
                registry_check.details.get("error") or registry_check.details
            )
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(case_name, False, started, {"error": _exc_text(exc), **observed})


async def _ensure_extension_authenticated_on_page(
    page: object,
    observed: dict[str, object],
    *,
    package_id: str,
    display_name: str,
    required_tools: list[str],
    ensure_installed: bool = True,
) -> None:
    body = await _fetch_webui_json(page, "/api/webchat/v2/extensions")
    extensions = body.get("extensions")
    if not isinstance(extensions, list):
        raise AssertionError(f"extensions body did not include a list: {body!r}")

    def find_extension(items: list[object]) -> dict[str, object] | None:
        for extension in items:
            if not isinstance(extension, dict):
                continue
            package_ref = extension.get("package_ref")
            ref_id = package_ref.get("id") if isinstance(package_ref, dict) else None
            if ref_id == package_id or extension.get("display_name") == display_name:
                return extension
        return None

    match = find_extension(extensions)
    should_install = ensure_installed and not isinstance(match, dict)
    should_activate = (
        ensure_installed
        and isinstance(match, dict)
        and match.get("active") is not True
    )
    prefix = package_id.replace("-", "_")
    if should_install:
        install_body = await _webui_json(
            page,
            "POST",
            "/api/webchat/v2/extensions/install",
            {"package_ref": {"kind": "extension", "id": package_id}},
        )
        observed[f"{prefix}_install_message"] = install_body.get("message")
        observed[f"{prefix}_install_onboarding_state"] = install_body.get("onboarding_state")
        should_activate = True
    if should_activate:
        activate_body = await _webui_json(
            page,
            "POST",
            f"/api/webchat/v2/extensions/{package_id}/activate",
        )
        observed[f"{prefix}_activate_message"] = activate_body.get("message")
        observed[f"{prefix}_activated"] = activate_body.get("activated")
    if should_install or should_activate:
        body = await _fetch_webui_json(page, "/api/webchat/v2/extensions")
        extensions = body.get("extensions")
        if not isinstance(extensions, list):
            raise AssertionError(f"extensions body did not include a list after install: {body!r}")
        match = find_extension(extensions)
    if not isinstance(match, dict):
        raise AssertionError(f"{display_name} extension was not listed: {extensions!r}")
    tools = match.get("tools")
    if not isinstance(tools, list):
        tools = []
    observed.update(
        {
            f"{prefix}_active": match.get("active"),
            f"{prefix}_authenticated": match.get("authenticated"),
            f"{prefix}_activation_status": match.get("activation_status"),
            f"{prefix}_needs_setup": match.get("needs_setup"),
            f"{prefix}_tool_count": len(tools),
        }
    )
    missing_tools = [tool for tool in required_tools if tool not in tools]
    if missing_tools:
        raise AssertionError(f"{display_name} missing expected tools: {missing_tools!r}")
    if match.get("active") is not True:
        raise AssertionError(f"{display_name} extension is not active: {match!r}")
    if match.get("authenticated") is not True:
        raise AssertionError(f"{display_name} extension is not authenticated: {match!r}")
    if match.get("needs_setup") is not False:
        raise AssertionError(f"{display_name} extension still needs setup: {match!r}")


async def case_qa_2a_gmail_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_2a_gmail_connect",
        package_id="gmail",
        display_name="Gmail",
        required_tools=["gmail.list_messages"],
        marker="REBORN_QA_2A_GMAIL_CONNECT_DONE",
        verification_instruction=(
            "After connecting, make exactly one safe read-only verification call "
            "with gmail.list_messages for at most one recent message."
        ),
        verification_capabilities=["gmail.list_messages"],
    )


async def case_qa_2b_calendar_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_2b_calendar_connect",
        package_id="google-calendar",
        display_name="Google Calendar",
        required_tools=["google-calendar.list_events"],
        marker="REBORN_QA_2B_CALENDAR_CONNECT_DONE",
        verification_instruction=(
            "After connecting, make exactly one safe read-only verification call "
            "with google-calendar.list_events for at most one upcoming event."
        ),
        verification_capabilities=["google-calendar.list_events"],
    )


async def case_qa_2c_drive_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_2c_drive_connect",
        package_id="google-drive",
        display_name="Google Drive",
        required_tools=["google-drive.list_files"],
        marker="REBORN_QA_2C_DRIVE_CONNECT_DONE",
        verification_instruction=(
            "After connecting, make exactly one safe read-only verification call "
            "with google-drive.list_files for at most one file."
        ),
        verification_capabilities=["google-drive.list_files"],
    )


async def case_qa_2d_calendar_prep_live_chat(ctx: LiveQaContext) -> ProbeResult:
    return await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_2d_calendar_prep_live_chat",
        marker=None,
        required_text=["Google", "news"],
        extensions=[
            {
                "package_id": "google-calendar",
                "display_name": "Google Calendar",
                "required_tools": ["google-calendar.list_events"],
            },
            {
                "package_id": "google-drive",
                "display_name": "Google Drive",
                "required_tools": ["google-drive.list_files"],
            },
            {
                "package_id": "google-docs",
                "display_name": "Google Docs",
                "required_tools": ["google-docs.read_content"],
            },
            {
                "package_id": "web-access",
                "display_name": "Web Access",
                "required_tools": ["web-access.search"],
            },
        ],
        prompt=_qa_sheet_prompt("qa_2d_calendar_prep_live_chat"),
        timeout=300.0,
    )


async def case_qa_2e_calendar_prep_email_routine(ctx: LiveQaContext) -> ProbeResult:
    routine_name = "reborn-qa-2e-calendar-prep-email"
    return await _routine_creation_case(
        ctx,
        case_name="qa_2e_calendar_prep_email_routine",
        routine_name=routine_name,
        marker=None,
        required_text=["routine", "email|emails|gmail"],
        prompt=_qa_sheet_prompt("qa_2e_calendar_prep_email_routine"),
    )


async def case_qa_2f_calendar_prep_email_delivery(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    marker = f"REBORN_QA_2F_CALENDAR_PREP_EMAIL_DELIVERED_{suffix}"
    try:
        access_token, token_meta = _google_runtime_access_token(
            ctx.reborn_home,
            _auth_user_id(),
            ctx.env,
        )
        target_email = await _gmail_delivery_target_email(
            access_token=access_token,
            extra_env=ctx.env,
        )
        sender_email = await _gmail_profile_email(access_token=access_token)
    except Exception as exc:
        return _result(
            "qa_2f_calendar_prep_email_delivery",
            False,
            started,
            {
                "error": _exc_text(exc),
                "marker": marker,
                "target_email_present": False,
            },
        )

    email_subject = f"Reborn QA 2F meeting prep {suffix}"
    email_body = (
        f"{marker}\n\n"
        "Reborn WebUIv2 live QA 2F calendar-prep delivery check. "
        "This message confirms the Gmail side effect after inspecting Calendar."
    )
    email_tool_input = json.dumps(
        {
            "message": {
                "from": sender_email,
                "to": target_email,
                "subject": email_subject,
                "body": email_body,
            }
        },
        separators=(",", ":"),
    )

    result = await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_2f_calendar_prep_email_delivery",
        marker=marker,
        required_text=["Gmail", "email"],
        extensions=[
            {
                "package_id": "gmail",
                "display_name": "Gmail",
                "required_tools": ["gmail.send_message"],
            },
            {
                "package_id": "google-calendar",
                "display_name": "Google Calendar",
                "required_tools": ["google-calendar.list_events"],
            },
            {
                "package_id": "google-drive",
                "display_name": "Google Drive",
                "required_tools": ["google-drive.list_files"],
            },
            {
                "package_id": "google-docs",
                "display_name": "Google Docs",
                "required_tools": ["google-docs.read_content"],
            },
            {
                "package_id": "web-access",
                "display_name": "Web Access",
                "required_tools": ["web-access.search"],
            },
        ],
        prompt=(
            "QA case 2F: perform the meeting-prep email side effect now. Use my "
            "live Google Calendar connection to inspect upcoming events, and use "
            "Google Drive or Docs and live web search for context if available. "
            "Send the Gmail message using structured message fields, not "
            f"`message.raw`. Use this exact gmail.send_message input: "
            f"{email_tool_input}. If no upcoming meeting is available, still "
            "send this exact message after checking Calendar. In the final answer "
            "include the exact marker "
            f"{marker}, include the word Gmail, and include the word email."
        ),
        timeout=420.0,
        extra_details={
            "target_email_present": True,
            "gmail_structured_input": True,
            "target_source": (
                "env"
                if _first_env_value(
                    [
                        "REBORN_WEBUI_V2_LIVE_QA_EMAIL_TARGET",
                        "LIVE_CANARY_EMAIL_TARGET",
                        "AUTH_LIVE_GOOGLE_EMAIL",
                        "GOOGLE_TEST_EMAIL",
                    ],
                    ctx.env,
                )
                else "gmail_profile"
            ),
        },
        forbidden_text=[
            "auth_denied",
            "authentication required",
            "can't send",
            "cannot send",
            "permission denied",
        ],
    )
    if not result.success:
        result.latency_ms = int((time.monotonic() - started) * 1000)
        return result
    try:
        delivery = await _wait_for_gmail_marker(
            access_token=access_token,
            marker=marker,
            timeout=360.0,
        )
        result.details["google_token"] = token_meta
        result.details["gmail_delivery"] = delivery
        result.latency_ms = int((time.monotonic() - started) * 1000)
        return result
    except Exception as exc:
        result.success = False
        result.latency_ms = int((time.monotonic() - started) * 1000)
        result.details["google_token"] = token_meta
        result.details["error"] = str(exc)
        return result


async def case_qa_4a_gmail_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_4a_gmail_connect",
        package_id="gmail",
        display_name="Gmail",
        required_tools=["gmail.list_messages"],
        marker="REBORN_QA_4A_GMAIL_CONNECT_DONE",
        verification_instruction=(
            "After connecting, make exactly one safe read-only verification call "
            "with gmail.list_messages for at most one recent message."
        ),
        verification_capabilities=["gmail.list_messages"],
    )


async def case_qa_4b_github_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_4b_github_connect",
        package_id="github",
        display_name="GitHub",
        required_tools=["github.get_authenticated_user"],
        marker="REBORN_QA_4B_GITHUB_CONNECT_DONE",
        verification_instruction=(
            "After connecting, make exactly one safe read-only verification call "
            "with github.get_authenticated_user."
        ),
        verification_capabilities=["github.get_authenticated_user"],
    )


async def case_qa_6a_gmail_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_6a_gmail_connect",
        package_id="gmail",
        display_name="Gmail",
        required_tools=["gmail.list_messages"],
        marker="REBORN_QA_6A_GMAIL_CONNECT_DONE",
        verification_instruction=(
            "After connecting, make exactly one safe read-only verification call "
            "with gmail.list_messages for at most one recent message."
        ),
        verification_capabilities=["gmail.list_messages"],
    )


async def case_qa_5b_drive_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_5b_drive_connect",
        package_id="google-drive",
        display_name="Google Drive",
        required_tools=["google-drive.list_files"],
        marker="REBORN_QA_5B_DRIVE_CONNECT_DONE",
        verification_instruction=(
            "After connecting, make exactly one safe read-only verification call "
            "with google-drive.list_files for at most one file."
        ),
        verification_capabilities=["google-drive.list_files"],
    )


async def case_qa_5c_strategy_doc_knowledge_base(ctx: LiveQaContext) -> ProbeResult:
    strategy_phrase = "Reborn QA Strategy North Star: verify live WebUIv2 tool grounding."
    return await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_5c_strategy_doc_knowledge_base",
        marker=None,
        required_text=["strategy"],
        extensions=[
            {
                "package_id": "google-docs",
                "display_name": "Google Docs",
                "required_tools": [
                    "google-docs.create_document",
                    "google-docs.read_content",
                ],
            },
        ],
        prompt=_qa_sheet_prompt("qa_5c_strategy_doc_knowledge_base"),
        timeout=360.0,
        extra_details={"strategy_phrase": strategy_phrase},
        forbidden_text=[
            "auth_denied",
            "authentication required",
            "local file",
            "/workspace/",
            ".md",
            "can't create",
            "cannot create",
        ],
    )


async def case_qa_5d_slack_strategy_doc_answer(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    wall_started = time.time()
    suffix = str(int(wall_started * 1000))
    doc_marker = f"REBORN_QA_5D_STRATEGY_DOC_{suffix}"
    slack_marker = f"REBORN_QA_5D_SLACK_STRATEGY_ANSWER_{suffix}"
    nonce = f"QA5D-NONCE-{uuid.uuid4()}"
    strategy_phrase = (
        "Reborn QA 5D strategy north star: answer Slack questions from "
        f"live Google Docs grounding with nonce {nonce}."
    )
    doc_creation = await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_5d_slack_strategy_doc_answer",
        marker=doc_marker,
        required_text=["strategy", "Google Docs"],
        extensions=[
            {
                "package_id": "google-docs",
                "display_name": "Google Docs",
                "required_tools": [
                    "google-docs.create_document",
                    "google-docs.read_content",
                ],
            },
        ],
        prompt=(
            "QA case 5D document preparation: create a new Google Docs document titled "
            f"`{doc_marker}`. Put this exact strategy sentence in the body: "
            f"{strategy_phrase} Read the document content back through Google "
            "Docs. In the final answer include the exact marker "
            f"{doc_marker}, the word strategy, and the phrase Google Docs."
        ),
        timeout=360.0,
        extra_details={
            "doc_marker": doc_marker,
            "slack_marker": slack_marker,
            "strategy_phrase": strategy_phrase,
        },
        forbidden_text=[
            "auth_denied",
            "authentication required",
            "can't create",
            "cannot create",
            "permission denied",
        ],
    )
    if not doc_creation.success:
        return doc_creation
    observed: dict[str, object] = {
        **doc_creation.details,
        "doc_creation_latency_ms": doc_creation.latency_ms,
    }
    text_excerpt = str(doc_creation.details.get("text_excerpt") or "")
    doc_id = _extract_google_document_id(text_excerpt)
    doc_id_source = "assistant_reply" if doc_id else None
    try:
        if not doc_id:
            access_token, token_meta = _google_runtime_access_token(
                ctx.reborn_home,
                _auth_user_id(),
                ctx.env,
            )
            doc_id = await _google_drive_file_id_by_name(
                access_token=access_token,
                name=doc_marker,
                mime_type="application/vnd.google-apps.document",
            )
            observed["google_token_for_doc_lookup"] = token_meta
            doc_id_source = "drive_name_lookup" if doc_id else None
        observed["doc_id_present"] = bool(doc_id)
        observed["doc_id_source"] = doc_id_source
        if not doc_id:
            raise AssertionError(
                "created Google Docs document id could not be resolved from "
                "the setup reply or live Drive lookup"
            )
        doc_url = f"https://docs.google.com/document/d/{doc_id}/edit"
        observed["doc_id"] = doc_id
        observed["doc_url_present"] = True
        slack = _slack_preflight(ctx)
        channel_id = _slack_delivery_channel_id(ctx)
        if not channel_id:
            raise AssertionError("Slack inbound test could not resolve a DM/channel id")
        slack_user_id = str(slack.get("inbound_user_id") or "U0REBORNQA")
        post_result = await _post_signed_slack_dm_event(
            ctx,
            channel_id=channel_id,
            user_id=slack_user_id,
            text=f"{_qa_sheet_prompt('qa_5d_slack_strategy_doc_answer')}\nGoogle doc link: {doc_url}",
            event_id=f"EvREBORNQA5D{suffix}",
        )
        observed["signed_event"] = post_result
        event_id = str(post_result.get("event_id") or f"EvREBORNQA5D{suffix}")
        deadline = time.monotonic() + 360.0
        last_history: dict[str, object] | None = None
        approved_gate_refs: set[str] = set()
        approval_attempts: list[dict[str, object]] = []
        event_run_id: str | None = None
        while time.monotonic() < deadline:
            approval = await _approve_slack_event_gates(
                ctx,
                event_id=event_id,
                approved_gate_refs=approved_gate_refs,
            )
            if approval.get("run_id"):
                event_run_id = str(approval["run_id"])
                observed["slack_event_run_id"] = event_run_id
            attempts = approval.get("approval_attempts")
            if isinstance(attempts, list):
                approval_attempts.extend(
                    attempt for attempt in attempts if isinstance(attempt, dict)
                )
                observed["approval_attempts"] = approval_attempts[-5:]
            history = await _slack_history_contains_marker(
                ctx,
                channel_id=channel_id,
                marker=nonce,
                oldest_epoch=wall_started,
                required_text=[nonce, "strategy"],
            )
            last_history = history
            if history.get("found"):
                observed["slack_history"] = history
                return _result("qa_5d_slack_strategy_doc_answer", True, started, observed)
            await asyncio.sleep(2.0)
        raise AssertionError(
            "Slack grounded strategy answer marker was not observed after signed "
            f"Slack event. last_history={last_history!r} "
            f"approval_attempts={approval_attempts[-3:]!r} "
            f"slack_event_run_id={event_run_id!r}"
        )
    except Exception as exc:
        return _result(
            "qa_5d_slack_strategy_doc_answer",
            False,
            started,
            {"error": _exc_text(exc), **observed},
        )


async def case_qa_6b_sheets_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_6b_sheets_connect",
        package_id="google-sheets",
        display_name="Google Sheets",
        required_tools=["google-sheets.read_values"],
        marker="REBORN_QA_6B_SHEETS_CONNECT_DONE",
        verification_instruction=(
            "After connecting, do not create or modify any spreadsheet; just "
            "finish after the Google Sheets extension is active."
        ),
        verification_capabilities=[],
    )


async def case_qa_6c_gmail_to_sheet_live_chat(ctx: LiveQaContext) -> ProbeResult:
    return await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_6c_gmail_to_sheet_live_chat",
        marker=None,
        required_text=["ABC|sheet|spreadsheet", "email|row|near.ai|near ai"],
        extensions=[
            {
                "package_id": "gmail",
                "display_name": "Gmail",
                "required_tools": ["gmail.list_messages"],
            },
            {
                "package_id": "google-drive",
                "display_name": "Google Drive",
                "required_tools": ["google-drive.list_files"],
            },
            {
                "package_id": "google-sheets",
                "display_name": "Google Sheets",
                "required_tools": [
                    "google-sheets.create_spreadsheet",
                    "google-sheets.append_values",
                ],
            },
        ],
        prompt=_qa_sheet_prompt("qa_6c_gmail_to_sheet_live_chat"),
        timeout=360.0,
    )


async def case_qa_6d_gmail_to_sheet_routine(ctx: LiveQaContext) -> ProbeResult:
    routine_name = "reborn-qa-6d-gmail-to-sheet"
    return await _routine_creation_case(
        ctx,
        case_name="qa_6d_gmail_to_sheet_routine",
        routine_name=routine_name,
        marker=None,
        required_text=["routine", "Gmail"],
        prompt=_qa_sheet_prompt("qa_6d_gmail_to_sheet_routine"),
    )


async def case_qa_6e_gmail_to_sheet_delivery(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    marker = f"REBORN_QA_6E_GMAIL_TO_SHEET_DELIVERY_{int(time.time() * 1000)}"
    result = await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_6e_gmail_to_sheet_delivery",
        marker=marker,
        required_text=["Google Sheet"],
        extensions=[
            {
                "package_id": "gmail",
                "display_name": "Gmail",
                "required_tools": ["gmail.list_messages"],
            },
            {
                "package_id": "google-sheets",
                "display_name": "Google Sheets",
                "required_tools": [
                    "google-sheets.create_spreadsheet",
                    "google-sheets.append_values",
                ],
            },
        ],
        prompt=(
            "QA case 6E: perform the CRM Gmail-to-Sheet side effect now. Inspect "
            "at most one recent Gmail inbox message. Create a new Google Sheet "
            f"named `{marker}` and append exactly one row with columns Source, "
            "Summary, and QA Marker. The QA Marker cell must contain the exact "
            f"marker {marker}. If no Gmail message is available, still append a "
            "row with Source as Gmail, Summary as no recent message available, "
            f"and QA Marker as {marker}. In the final answer include the exact "
            f"marker {marker}, include the phrase Google Sheet, and include the "
            "created spreadsheet URL."
        ),
        timeout=420.0,
        forbidden_text=[
            "auth_denied",
            "authentication required",
            "can't create",
            "cannot create",
            "permission denied",
        ],
    )
    if not result.success:
        return result
    text_excerpt = str(result.details.get("text_excerpt") or "")
    spreadsheet_id = _extract_google_spreadsheet_id(text_excerpt)
    spreadsheet_id_source = "assistant_reply" if spreadsheet_id else None
    result.details["spreadsheet_id_present"] = bool(spreadsheet_id)
    try:
        access_token, token_meta = _google_runtime_access_token(
            ctx.reborn_home,
            _auth_user_id(),
            ctx.env,
        )
        if not spreadsheet_id:
            spreadsheet_id = await _google_drive_file_id_by_name(
                access_token=access_token,
                name=marker,
                mime_type="application/vnd.google-apps.spreadsheet",
            )
            spreadsheet_id_source = "drive_name_lookup" if spreadsheet_id else None
        result.details["spreadsheet_id_present"] = bool(spreadsheet_id)
        result.details["spreadsheet_id_source"] = spreadsheet_id_source
        if not spreadsheet_id:
            result.success = False
            result.details["error"] = (
                "assistant did not return a Google spreadsheet URL or id and "
                "Drive lookup by exact sheet name did not find one"
            )
            return result
        sheet_check = await _wait_for_google_sheet_marker(
            access_token=access_token,
            spreadsheet_id=spreadsheet_id,
            marker=marker,
            timeout=90.0,
        )
        result.details["google_token"] = token_meta
        result.details["spreadsheet_id"] = spreadsheet_id
        result.details["sheet_marker_check"] = sheet_check
        result.latency_ms = int((time.monotonic() - started) * 1000)
        return result
    except Exception as exc:
        result.success = False
        result.latency_ms = int((time.monotonic() - started) * 1000)
        result.details["spreadsheet_id"] = spreadsheet_id
        exc_text = str(exc)
        result.details["error"] = (
            f"{type(exc).__name__}: {exc_text}" if exc_text else type(exc).__name__
        )
        return result


async def case_qa_7b_sheets_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_chat_connect_case(
        ctx,
        case_name="qa_7b_sheets_connect",
        package_id="google-sheets",
        display_name="Google Sheets",
        required_tools=["google-sheets.read_values"],
        marker="REBORN_QA_7B_SHEETS_CONNECT_DONE",
        verification_instruction=(
            "After connecting, do not create or modify any spreadsheet; just "
            "finish after the Google Sheets extension is active."
        ),
        verification_capabilities=[],
    )


async def _routine_creation_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    prompt: str,
    marker: str | None,
    routine_name: str,
    required_text: list[str],
    extensions: list[dict[str, object]] | None = None,
    extra_details: dict[str, object] | None = None,
    follow_up_timezone_instruction: str | None = None,
) -> ProbeResult:
    count_name = routine_name if marker else None
    before_count = _trigger_record_count(ctx.reborn_home, count_name)
    details = {
        "routine_name": routine_name,
        "trigger_records_before": before_count,
        **(extra_details or {}),
    }
    if extensions:
        result = await _live_chat_with_extensions_case(
            ctx,
            case_name=case_name,
            prompt=prompt,
            marker=marker,
            required_text=required_text,
            extensions=extensions,
            timeout=ROUTINE_CREATION_REPLY_TIMEOUT_SECONDS,
            extra_details=details,
        )
    else:
        result = await _live_chat_case(
            ctx,
            case_name=case_name,
            prompt=prompt,
            marker=marker,
            required_text=required_text,
            timeout=ROUTINE_CREATION_REPLY_TIMEOUT_SECONDS,
            extra_details=details,
            routine_confirmation_follow_up=True,
            routine_follow_up_timezone_instruction=follow_up_timezone_instruction,
        )
    if result.success:
        after_count, wait_ms = await _wait_for_trigger_record_after_count(
            ctx.reborn_home,
            count_name,
            before_count=before_count,
        )
    else:
        after_count = _trigger_record_count(ctx.reborn_home, count_name)
        wait_ms = 0
    result.details["trigger_records_after"] = after_count
    result.details["trigger_record_wait_ms"] = wait_ms
    result.details["trigger_record_wait_timeout_ms"] = int(
        ROUTINE_TRIGGER_RECORD_WAIT_TIMEOUT_SECONDS * 1000
    )
    if result.success and after_count <= before_count:
        result.success = False
        result.details["error"] = (
            "assistant matched required routine text before trigger_create completed; "
            f"routine scope {routine_name!r} did not add a trigger_record"
        )
    return result


async def case_qa_3c_endpoint_status_slack_routine(ctx: LiveQaContext) -> ProbeResult:
    routine_name = "reborn-qa-3c-endpoint-status-slack"
    prompt = _qa_sheet_prompt("qa_3c_endpoint_status_slack_routine").replace(
        "[endpoint URL]",
        ENDPOINT_STATUS_URL,
    )
    return await _routine_creation_case(
        ctx,
        case_name="qa_3c_endpoint_status_slack_routine",
        routine_name=routine_name,
        marker=None,
        required_text=["routine"],
        prompt=prompt,
    )


async def _slack_delivery_routine_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    routine_prefix: str,
    marker_prefix: str,
    routine_instruction: str,
    required_delivery_text: list[str],
    delivery_timeout: float = 240.0,
    creation_prompt_extra: str = "",
    exactly_once_grace_seconds: float | None = None,
    require_persisted_delivery_target: bool = False,
    require_slack_tools_on_surface: bool = False,
    schedule_instruction: str = "Every minute,",
    expect_one_shot_schedule: bool = False,
    follow_up_timezone_instruction: str | None = None,
) -> ProbeResult:
    started = time.monotonic()
    wall_started = time.time()
    suffix = str(int(wall_started * 1000))
    routine_name = f"{routine_prefix}-{suffix}"
    creation_marker = f"{marker_prefix}_ROUTINE_CREATED_{suffix}"
    delivery_marker = f"{marker_prefix}_SLACK_DELIVERED_{suffix}"
    default_targets_before: dict[str, object] | None = None
    if require_persisted_delivery_target:
        default_targets_before = _outbound_final_reply_targets(ctx.reborn_home)
    creation = await _routine_creation_case(
        ctx,
        case_name=case_name,
        routine_name=routine_name,
        marker=creation_marker,
        required_text=["routine"],
        prompt=(
            f"QA case {case_name}: create a routine named {routine_name}. {schedule_instruction} "
            f"{routine_instruction} The routine's final answer must include the exact "
            f"marker {delivery_marker}. Create the routine now; do not "
            "run it immediately. During routine creation, do not perform the routine's "
            "live check, web/search/HTTP lookup, or Slack send. "
            f"In your final answer include the exact marker {creation_marker} and include "
            "the text routine. "
            f"{creation_prompt_extra}"
        ),
        follow_up_timezone_instruction=follow_up_timezone_instruction,
    )
    if not creation.success:
        creation.latency_ms = int((time.monotonic() - started) * 1000)
        return creation
    base_details: dict[str, object] = {
        **creation.details,
        "routine_name": routine_name,
        "creation_marker": creation_marker,
        "delivery_marker": delivery_marker,
        "required_delivery_text": required_delivery_text,
    }
    record_snapshot: dict[str, object] | None = None
    slack_tools_surface: dict[str, object] | None = None
    try:
        # Server-capability and schedule preconditions run FIRST (and inside
        # the try, so a transient read failure fails THIS case instead of
        # crashing the shard): a pre-fix server must red deterministically on
        # the missing capability, not on model-compliance noise.
        if require_persisted_delivery_target or expect_one_shot_schedule:
            record_snapshot = _trigger_record_snapshot(ctx.reborn_home, routine_name)
            base_details["trigger_record_snapshot"] = record_snapshot
        if require_persisted_delivery_target and record_snapshot is not None:
            if record_snapshot.get("delivery_target_column_missing"):
                raise AssertionError(
                    "server does not support per-trigger delivery targets "
                    "(trigger_records.delivery_target column missing)"
                )
            if not record_snapshot.get("checked"):
                raise AssertionError(
                    "probe could not read trigger_records for the persisted "
                    f"delivery target: {record_snapshot.get('error')!r}"
                )
            if not record_snapshot.get("delivery_target"):
                raise AssertionError(
                    "routine was created without a per-trigger delivery_target_id "
                    "on the trigger record"
                )
        if expect_one_shot_schedule and record_snapshot is not None:
            # Exactly-once counting is only well-defined for once-schedules:
            # a recurring trigger legitimately re-posts the marker on fire #2
            # and fabricates a "duplicate delivery" red. Verify the OUTCOME
            # (the persisted schedule), not the prompt wording.
            if not record_snapshot.get("checked"):
                raise AssertionError(
                    "probe could not read trigger_records for the schedule "
                    f"precondition: {record_snapshot.get('error')!r}"
                )
            record_count = int(record_snapshot.get("record_count") or 0)
            if record_count != 1:
                raise AssertionError(
                    "probe precondition failed: expected exactly one trigger "
                    f"record named {routine_name!r}, found {record_count}"
                )
            schedule_kind = record_snapshot.get("schedule_kind")
            if schedule_kind != "once":
                raise AssertionError(
                    "probe precondition failed: routine is not one-shot "
                    f"(schedule_kind={schedule_kind!r})"
                )
            wait_anchor = time.time()
            fire_epoch = _parse_epoch_seconds(record_snapshot.get("next_run_at"))
            window_end = wait_anchor + delivery_timeout - 60.0
            if fire_epoch is None or not (
                wall_started - 5.0 <= fire_epoch <= window_end
            ):
                raise AssertionError(
                    "probe precondition failed: one-shot fire time "
                    f"{record_snapshot.get('next_run_at')!r} is outside the "
                    f"delivery wait window (case start {wall_started:.0f}, "
                    f"window end {window_end:.0f}) — a mis-scheduled fire "
                    "would time out on a correct server"
                )
        if require_slack_tools_on_surface:
            # Falsifiability precondition: the duplicate arm is only a real
            # test when the fired model HAS a user-token send capability.
            # Assert it instead of trusting the creation prompt was followed.
            slack_tools_surface = await _installed_active_extension_ids(ctx)
            base_details["slack_tools_surface"] = slack_tools_surface
            if not slack_tools_surface.get("checked"):
                raise AssertionError(
                    "probe precondition failed: could not verify the Slack "
                    "tools surface via the extensions API: "
                    f"{slack_tools_surface.get('error')!r}"
                )
            active_ids = slack_tools_surface.get("active_extension_ids") or []
            if "slack" not in active_ids:
                raise AssertionError(
                    "probe precondition failed: Slack tools extension is not "
                    "installed/active after the creation turn, so the "
                    "duplicate-delivery arm would be vacuous"
                )
        delivery = await _wait_for_slack_delivery_marker(
            ctx,
            routine_name=routine_name,
            marker=delivery_marker,
            oldest_epoch=wall_started,
            timeout=delivery_timeout,
            required_text=required_delivery_text,
        )
        text_checks = [text.lower() for text in required_delivery_text]
        history = delivery.get("slack_history")
        if not isinstance(history, dict) or not history.get("found"):
            raise AssertionError(f"Slack marker not found in history: {history!r}")
        exactly_once: dict[str, object] | None = None
        if exactly_once_grace_seconds is not None:
            # A duplicate copy (bot delivery + user-identity messaging-tool
            # send) can trail the first message, so re-scan after a grace
            # window and require exactly one bot-authored marker message.
            await asyncio.sleep(exactly_once_grace_seconds)
            channel_id = _slack_delivery_channel_id(ctx)
            if not channel_id:
                raise AssertionError(
                    "Slack delivery channel could not be resolved for the exactly-once re-scan"
                )
            # The re-scan is judgement-bearing: a transient Slack API error
            # here must NOT read as "found 0" (a fake duplicate-delivery
            # verdict). Retry until a clean scan or fail with a distinct
            # inconclusive error.
            last_scan_error: str | None = None
            for _ in range(3):
                try:
                    scan = await _slack_history_contains_marker(
                        ctx,
                        channel_id=channel_id,
                        marker=delivery_marker,
                        oldest_epoch=wall_started,
                        required_text=required_delivery_text,
                    )
                except Exception as scan_exc:
                    scan = None
                    last_scan_error = _exc_text(scan_exc)
                if isinstance(scan, dict) and not scan.get("error"):
                    exactly_once = scan
                    break
                if isinstance(scan, dict):
                    last_scan_error = str(scan.get("error"))
                await asyncio.sleep(5.0)
            if exactly_once is None:
                raise AssertionError(
                    "exactly-once re-scan was inconclusive after retries "
                    f"(last error: {last_scan_error!r}) — cannot distinguish "
                    "duplicate delivery from a transient Slack API failure"
                )
            matches = int(exactly_once.get("marker_matches") or 0)
            human_matches = int(exactly_once.get("human_authored_marker_matches") or 0)
            # Exactly-once is only well-defined for one-shot routines: a
            # recurring schedule posts the same marker again on every fire,
            # so callers using this arm must create the routine with a
            # once-schedule (see qa_9b/qa_9d), verified by the persisted
            # schedule_kind precondition above.
            if matches != 1:
                raise AssertionError(
                    f"expected the delivery marker exactly once after the grace window, "
                    f"found {matches}: {exactly_once!r}"
                )
            if human_matches != 0:
                raise AssertionError(
                    f"delivery marker was posted from a non-bot (user) identity — "
                    f"duplicate self-delivery: {exactly_once!r}"
                )
            # Workspace-wide sweep: a stray copy in ANY other conversation
            # (wrong-channel delivery or a user-identity send somewhere else)
            # is a hard failure. The sweep is load-bearing for that arm, so a
            # permanently unusable token (missing scope) or repeated transient
            # failures fail the case with a distinct probe-environment error
            # instead of silently hollowing out every future green.
            sweep: dict[str, object] = {}
            for _ in range(3):
                sweep = await _slack_search_marker_hits(ctx, marker=delivery_marker)
                if sweep.get("checked") or sweep.get("permanent"):
                    break
                await asyncio.sleep(10.0)
            exactly_once["workspace_sweep"] = sweep
            if sweep.get("checked"):
                stray_hits = [
                    hit
                    for hit in sweep.get("hits") or []
                    if hit.get("channel_id") and hit.get("channel_id") != channel_id
                ]
                if stray_hits:
                    raise AssertionError(
                        f"delivery marker found outside the expected DM "
                        f"({channel_id}): {stray_hits!r}"
                    )
            elif sweep.get("permanent"):
                raise AssertionError(
                    "probe precondition failed: workspace sweep is permanently "
                    f"unavailable ({sweep.get('error')!r}); repair the personal "
                    "token (search:read scope) — without the sweep the "
                    "wrong-channel arm is vacuous"
                )
            else:
                raise AssertionError(
                    "workspace sweep was inconclusive after retries "
                    f"({sweep.get('error')!r}); cannot rule out a stray copy"
                )
        if require_persisted_delivery_target and default_targets_before is not None:
            # Per-trigger routing must not be green because the server (or
            # the model) rewrote the user-wide default target instead of
            # honoring the trigger's own delivery_target_id.
            default_targets_after = _outbound_final_reply_targets(ctx.reborn_home)
            base_details["default_delivery_targets_before"] = default_targets_before
            base_details["default_delivery_targets_after"] = default_targets_after
            if default_targets_after != default_targets_before:
                raise AssertionError(
                    "user-default outbound delivery target changed during the "
                    "per-trigger routing case — routing must come from the "
                    "trigger's own delivery_target_id, not a rewritten default"
                )
        # The exact Slack body is not persisted in results to avoid leaking workspace data.
        return _result(
            case_name,
            True,
            started,
            {
                **base_details,
                "required_delivery_text": text_checks,
                "trigger_run": delivery.get("trigger_run"),
                "delivery_outcome": delivery.get("delivery_outcome"),
                "slack_history": history,
                "exactly_once": exactly_once,
            },
        )
    except SlackDeliveryReadbackInconclusive as exc:
        result = _result(
            case_name,
            False,
            started,
            {
                **base_details,
                "error": _exc_text(exc),
                "failure_class": "infrastructure",
                "failure_category": "slack_delivery_readback_unavailable",
                "failure_status": "inconclusive",
                "inconclusive": True,
                "delivery_readback_evidence": exc.evidence,
            },
        )
        result.details["blocking"] = False
        return result
    except Exception as exc:
        return _result(
            case_name,
            False,
            started,
            {
                **base_details,
                "error": _exc_text(exc),
            },
        )


async def case_qa_3d_endpoint_status_slack_delivery(ctx: LiveQaContext) -> ProbeResult:
    return await _slack_delivery_routine_case(
        ctx,
        case_name="qa_3d_endpoint_status_slack_delivery",
        routine_prefix="reborn-qa-3d-endpoint-status-slack-delivery",
        marker_prefix="REBORN_QA_3D_ENDPOINT_STATUS",
        routine_instruction=(
            f"check {ENDPOINT_STATUS_URL} with live HTTP or web access, report "
            "the observed HTTP status, and send the result to Slack"
        ),
        required_delivery_text=["status"],
    )


async def case_qa_4c_github_release_live_chat(ctx: LiveQaContext) -> ProbeResult:
    return await _live_chat_case(
        ctx,
        case_name="qa_4c_github_release_live_chat",
        prompt=_qa_sheet_prompt("qa_4c_github_release_live_chat"),
        marker=None,
        required_text=["release"],
        timeout=240.0,
    )


async def case_qa_4d_github_release_slack_routine(ctx: LiveQaContext) -> ProbeResult:
    routine_name = "reborn-qa-4d-github-release-slack"
    return await _routine_creation_case(
        ctx,
        case_name="qa_4d_github_release_slack_routine",
        routine_name=routine_name,
        marker=None,
        required_text=["routine|trigger|automation|cron|schedule|created"],
        prompt=_qa_sheet_prompt("qa_4d_github_release_slack_routine"),
    )


async def case_qa_4e_github_release_email_delivery(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    marker = f"REBORN_QA_4E_GITHUB_RELEASE_EMAIL_DELIVERED_{suffix}"
    try:
        release = await _live_github_latest_release("nearai", "ironclaw")
        access_token, token_meta = _google_runtime_access_token(
            ctx.reborn_home,
            _auth_user_id(),
            ctx.env,
        )
        target_email = await _gmail_delivery_target_email(
            access_token=access_token,
            extra_env=ctx.env,
        )
        sender_email = await _gmail_profile_email(access_token=access_token)
    except Exception as exc:
        return _result(
            "qa_4e_github_release_email_delivery",
            False,
            started,
            {
                "error": _exc_text(exc),
                "marker": marker,
                "target_email_present": False,
            },
        )

    email_subject = f"Reborn QA 4E release {release['tag_name']} {suffix}"
    email_body = (
        f"{marker}\n\n"
        f"GitHub release check for nearai/ironclaw: {release['tag_name']}."
    )
    email_tool_input = json.dumps(
        {
            "message": {
                "from": sender_email,
                "to": target_email,
                "subject": email_subject,
                "body": email_body,
            }
        },
        separators=(",", ":"),
    )

    result = await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_4e_github_release_email_delivery",
        marker=marker,
        required_text=["Gmail", release["tag_name"]],
        extensions=[
            {
                "package_id": "gmail",
                "display_name": "Gmail",
                "required_tools": ["gmail.send_message"],
            },
            {
                "package_id": "web-access",
                "display_name": "Web Access",
                "required_tools": ["web-access.search"],
            },
        ],
        prompt=(
            "QA case 4E: perform the GitHub release email side effect now. "
            "Check the latest public nearai/ironclaw release using live web or "
            f"HTTP context. The release API URL is {release['api_url']} and the "
            "expected latest release tag is "
            f"{release['tag_name']}. Send the Gmail message using structured "
            "message fields, not `message.raw`. Use this exact gmail.send_message "
            f"input: {email_tool_input}. In the final answer include the exact marker "
            f"{marker}, include the word Gmail, and include the release tag "
            f"{release['tag_name']}."
        ),
        timeout=420.0,
        extra_details={
            **release,
            "target_email_present": True,
            "gmail_structured_input": True,
            "target_source": (
                "env"
                if _first_env_value(
                    [
                        "REBORN_WEBUI_V2_LIVE_QA_EMAIL_TARGET",
                        "LIVE_CANARY_EMAIL_TARGET",
                        "AUTH_LIVE_GOOGLE_EMAIL",
                        "GOOGLE_TEST_EMAIL",
                    ],
                    ctx.env,
                )
                else "gmail_profile"
            ),
        },
        forbidden_text=[
            "auth_denied",
            "authentication required",
            "can't send",
            "cannot send",
            "permission denied",
        ],
    )
    if not result.success:
        result.latency_ms = int((time.monotonic() - started) * 1000)
        return result
    try:
        delivery = await _wait_for_gmail_marker(
            access_token=access_token,
            marker=marker,
            timeout=360.0,
        )
        result.details["google_token"] = token_meta
        result.details["gmail_delivery"] = delivery
        result.latency_ms = int((time.monotonic() - started) * 1000)
        return result
    except Exception as exc:
        result.success = False
        result.latency_ms = int((time.monotonic() - started) * 1000)
        result.details["google_token"] = token_meta
        result.details["error"] = str(exc)
        return result


async def case_qa_5a_slack_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _slack_connect_case(ctx, case_name="qa_5a_slack_connect")


def _slack_signing_secret(extra_env: dict[str, str]) -> str | None:
    return _env_value(SLACK_SIGNING_SECRET_ENV, extra_env)


def _slack_event_headers(body: bytes, signing_secret: str) -> dict[str, str]:
    timestamp = str(int(time.time()))
    base = b"v0:" + timestamp.encode("utf-8") + b":" + body
    digest = hmac.new(
        signing_secret.encode("utf-8"),
        base,
        hashlib.sha256,
    ).hexdigest()
    return {
        "Content-Type": "application/json",
        "X-Slack-Request-Timestamp": timestamp,
        "X-Slack-Signature": f"v0={digest}",
    }


async def _post_signed_slack_dm_event(
    ctx: LiveQaContext,
    *,
    channel_id: str,
    user_id: str,
    text: str,
    event_id: str,
) -> dict[str, object]:
    import httpx

    signing_secret = _slack_signing_secret(ctx.env)
    if not signing_secret:
        raise AssertionError("Slack signing secret is unavailable for signed webhook injection")
    slack = _slack_preflight(ctx)
    auth_test = slack.get("auth_test")
    team_id = None
    if isinstance(auth_test, dict):
        team_id = auth_test.get("team_id")
    if not team_id:
        setup = slack.get("setup")
        if isinstance(setup, dict):
            team_id = setup.get("team_id")
    if not team_id:
        team_id = slack.get("secret_source", {}).get("team_id")
    secret_source = slack.get("secret_source")
    api_app_id = None
    if isinstance(secret_source, dict):
        api_app_id = secret_source.get("api_app_id")
    if not api_app_id:
        setup = slack.get("setup")
        if isinstance(setup, dict):
            api_app_id = setup.get("api_app_id")
    payload = {
        "token": "live-qa-local-signed-event",
        "team_id": str(team_id or ""),
        "api_app_id": str(api_app_id or ""),
        "type": "event_callback",
        "event_id": event_id,
        "event_time": int(time.time()),
        "event": {
            "type": "message",
            "user": user_id,
            "text": text,
            "channel": channel_id,
            "channel_type": "im",
            "ts": f"{int(time.time())}.{int((time.time() % 1) * 1_000_000):06d}",
        },
    }
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.post(
            f"{ctx.base_url}/webhooks/slack/events",
            content=body,
            headers=_slack_event_headers(body, signing_secret),
        )
    response_text = response.text[:500]
    if response.status_code < 200 or response.status_code >= 300:
        raise AssertionError(
            f"signed Slack event returned HTTP {response.status_code}: {response_text!r}"
        )
    return {
        "status_code": response.status_code,
        "body_excerpt": response_text,
        "event_id": event_id,
        "channel_id_present": bool(channel_id),
        "synthetic_user_id": user_id,
    }


async def case_qa_7d_slack_bug_message_trigger(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    wall_started = time.time()
    case_name = "qa_7d_slack_bug_message_trigger"
    suffix = str(int(wall_started * 1000))
    marker = "bug"
    observed: dict[str, object] = {"marker": marker}
    try:
        slack = _slack_preflight(ctx)
        observed.update(
            {
                "inbound_user_id": slack.get("inbound_user_id"),
                "delivery_target_present": slack.get("delivery_target_present"),
            }
        )
        channel_id = _slack_delivery_channel_id(ctx)
        if not channel_id:
            raise AssertionError("Slack inbound test could not resolve a DM/channel id")
        if not _slack_delivery_target_is_dm(channel_id):
            raise AssertionError(
                "Slack bug-message trigger test must inject into a DM target; "
                f"got channel_id={channel_id!r}"
            )
        slack_user_id = str(slack.get("inbound_user_id") or "U0REBORNQA")
        qa_sheet_prompt = _qa_sheet_prompt(case_name)
        text = f"bug: reborn QA bug logger smoke {suffix}"
        observed["qa_sheet_prompt"] = qa_sheet_prompt
        observed["slack_event_text"] = text
        event_id = f"EvREBORNQA7D{suffix}"
        post_result = await _post_signed_slack_dm_event(
            ctx,
            channel_id=channel_id,
            user_id=slack_user_id,
            text=text,
            event_id=event_id,
        )
        observed["signed_event"] = post_result
        run_id = await _wait_for_slack_event_run_id(
            ctx,
            event_id=event_id,
            timeout=180.0,
        )
        observed["accepted_run_id"] = run_id
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(case_name, False, started, {"error": _exc_text(exc), **observed})


async def case_qa_7e_slack_bug_sheet_delivery(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    wall_started = time.time()
    suffix = str(int(wall_started * 1000))
    sheet_marker = f"REBORN_QA_7E_BUG_TRACKER_SHEET_{suffix}"
    row_marker = f"REBORN_QA_7E_BUG_ROW_{suffix}"
    bug_summary = f"live QA signed Slack bug row side effect {suffix}"
    setup = await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_7e_slack_bug_sheet_delivery",
        marker=sheet_marker,
        required_text=["Google Sheet"],
        extensions=[
            {
                "package_id": "google-sheets",
                "display_name": "Google Sheets",
                "required_tools": [
                    "google-sheets.create_spreadsheet",
                    "google-sheets.append_values",
                ],
            },
        ],
        prompt=(
            "QA case 7E sheet preparation: create a new Google Sheet named "
            f"`{sheet_marker}` with exactly one header row and no bug data rows. "
            "The header columns must be Summary, Reporter, Slack Timestamp, "
            "Status, and QA Marker. In the final answer include the exact marker "
            f"{sheet_marker}, include the phrase Google Sheet, and include the "
            "created spreadsheet URL."
        ),
        timeout=360.0,
        forbidden_text=[
            "auth_denied",
            "authentication required",
            "can't create",
            "cannot create",
            "permission denied",
        ],
    )
    if not setup.success:
        return setup
    observed: dict[str, object] = {
        **setup.details,
        "setup_latency_ms": setup.latency_ms,
        "sheet_marker": sheet_marker,
        "row_marker": row_marker,
    }
    text_excerpt = str(setup.details.get("text_excerpt") or "")
    spreadsheet_id = _extract_google_spreadsheet_id(text_excerpt)
    spreadsheet_id_source = "assistant_reply" if spreadsheet_id else None
    try:
        access_token, token_meta = _google_runtime_access_token(
            ctx.reborn_home,
            _auth_user_id(),
            ctx.env,
        )
        if not spreadsheet_id:
            spreadsheet_id = await _google_drive_file_id_by_name(
                access_token=access_token,
                name=sheet_marker,
                mime_type="application/vnd.google-apps.spreadsheet",
            )
            spreadsheet_id_source = "drive_name_lookup" if spreadsheet_id else None
        observed["spreadsheet_id_present"] = bool(spreadsheet_id)
        observed["spreadsheet_id_source"] = spreadsheet_id_source
        if not spreadsheet_id:
            raise AssertionError(
                "created Google Sheet id could not be resolved from the setup "
                "reply or live Drive lookup"
            )
        slack = _slack_preflight(ctx)
        channel_id = _slack_delivery_channel_id(ctx)
        if not channel_id:
            raise AssertionError("Slack inbound test could not resolve a DM/channel id")
        slack_user_id = str(slack.get("inbound_user_id") or "U0REBORNQA")
        event_id = f"EvREBORNQA7E{suffix}"
        post_result = await _post_signed_slack_dm_event(
            ctx,
            channel_id=channel_id,
            user_id=slack_user_id,
            text=(
                f"bug: {bug_summary}. Append this bug to the Google Sheet "
                f"https://docs.google.com/spreadsheets/d/{spreadsheet_id}/edit. "
                "Use Summary from this bug message, Reporter as the Slack user, "
                "Slack Timestamp from this Slack event if available, Status as New, "
                f"and QA Marker exactly {row_marker}. Do not create a new sheet."
            ),
            event_id=event_id,
        )
        marker_check = await _wait_for_google_sheet_marker_after_slack_event(
            ctx,
            event_id=str(post_result.get("event_id") or event_id),
            access_token=access_token,
            spreadsheet_id=spreadsheet_id,
            marker=row_marker,
            timeout=360.0,
        )
        return _result(
            "qa_7e_slack_bug_sheet_delivery",
            True,
            started,
            {
                **observed,
                "google_token": token_meta,
                "spreadsheet_id": spreadsheet_id,
                "signed_event": post_result,
                "sheet_marker_check": marker_check,
            },
        )
    except Exception as exc:
        return _result(
            "qa_7e_slack_bug_sheet_delivery",
            False,
            started,
            {
                **observed,
                "spreadsheet_id": spreadsheet_id,
                "error": _exc_text(exc),
            },
        )


async def case_qa_7c_slack_bug_logger_routine(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    routine_name = "reborn-qa-7c-slack-bug-sheet"
    try:
        access_token, token_meta = _google_runtime_access_token(
            ctx.reborn_home,
            _auth_user_id(),
            ctx.env,
        )
        sheet_fixture = await _create_google_spreadsheet_fixture(
            access_token=access_token,
            title=QA_7C_BUG_LOGGING_SHEET_TITLE,
            values=[
                ["Summary", "Reporter", "Slack Timestamp", "Status", "QA Marker"],
            ],
        )
        return await _routine_creation_case(
            ctx,
            case_name="qa_7c_slack_bug_logger_routine",
            routine_name=routine_name,
            marker=None,
            required_text=["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            prompt=_qa_7c_bug_logger_prompt(
                sheet_fixture=sheet_fixture,
                routine_name=routine_name,
            ),
            extensions=[
                {
                    "package_id": "google-drive",
                    "display_name": "Google Drive",
                    "required_tools": ["google-drive.list_files"],
                },
                {
                    "package_id": "google-sheets",
                    "display_name": "Google Sheets",
                    "required_tools": [
                        "google-sheets.read_values",
                        "google-sheets.append_values",
                    ],
                },
            ],
            extra_details={
                "google_token": token_meta,
                "bug_log_sheet_fixture": sheet_fixture,
            },
        )
    except Exception as exc:
        return _result(
            "qa_7c_slack_bug_logger_routine",
            False,
            started,
            {"routine_name": routine_name, "error": _exc_text(exc)},
        )


async def case_qa_7a_slack_product_channel_connect(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    case_name = "qa_7a_slack_product_channel_connect"
    observed: dict[str, object] = {
        "preflight": "Slack DM delivery target is configured before user-story workflow cases"
    }
    try:
        slack = _slack_preflight(ctx)
        delivery_channel_id = _slack_delivery_channel_id(ctx)
        route_discovery = slack.get("route_discovery")
        route_discovery_details = route_discovery if isinstance(route_discovery, dict) else {}
        observed.update(
            {
                "delivery_target_present": slack.get("delivery_target_present"),
                "route_configured_from_env": slack.get("route_configured_from_env"),
                "slack_dm_user_source": route_discovery_details.get("dm_user_source"),
                "slack_dm_user_id_present": bool(route_discovery_details.get("dm_user_id")),
                "slack_delivery_channel_id_present": bool(delivery_channel_id),
                "slack_delivery_target_kind": (
                    "dm" if _slack_delivery_target_is_dm(delivery_channel_id) else "non_dm"
                ),
            }
        )
        if not slack.get("delivery_target_present"):
            raise AssertionError(
                "Slack DM delivery target is not configured for this WebUI user"
            )
        if not _slack_delivery_target_is_dm(delivery_channel_id):
            raise AssertionError(
                "Slack live QA delivery target must be a DM to the user; "
                f"got channel_id={delivery_channel_id!r}"
            )
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(
            case_name,
            False,
            started,
            {"error": _exc_text(exc), **observed},
        )


async def case_qa_8b_hn_keyword_live_chat(ctx: LiveQaContext) -> ProbeResult:
    return await _live_chat_case(
        ctx,
        case_name="qa_8b_hn_keyword_live_chat",
        prompt=_qa_sheet_prompt("qa_8b_hn_keyword_live_chat"),
        marker=None,
        required_text=["news.ycombinator.com|hacker news|hn|discussion|id="],
        timeout=240.0,
    )


async def case_qa_8a_slack_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _slack_connect_case(ctx, case_name="qa_8a_slack_connect")


async def case_qa_8c_hn_keyword_slack_routine(ctx: LiveQaContext) -> ProbeResult:
    routine_name = "reborn-qa-8c-hn-keyword-slack"
    return await _routine_creation_case(
        ctx,
        case_name="qa_8c_hn_keyword_slack_routine",
        routine_name=routine_name,
        marker=None,
        required_text=["routine|trigger|automation|cron|schedule|created|monitor"],
        prompt=_qa_sheet_prompt("qa_8c_hn_keyword_slack_routine"),
    )


async def case_qa_8d_hn_keyword_slack_delivery(ctx: LiveQaContext) -> ProbeResult:
    return await _slack_delivery_routine_case(
        ctx,
        case_name="qa_8d_hn_keyword_slack_delivery",
        routine_prefix="reborn-qa-8d-hn-keyword-slack-delivery",
        marker_prefix="REBORN_QA_8D_HN_KEYWORD",
        routine_instruction=(
            f"perform exactly one public HTTP GET to {HN_KEYWORD_SEARCH_URL} as the "
            "Hacker News keyword check for recent NEAR AI posts, then send a concise "
            "Slack message that includes Hacker News and either the first finding or "
            "that no current matching item was found"
        ),
        required_delivery_text=["Hacker News"],
        delivery_timeout=420.0,
    )


async def _gated_qa_case(ctx: LiveQaContext, case_name: str) -> ProbeResult:
    started = time.monotonic()
    details = QA_SHEET_CASES.get(case_name, {})
    return _result(
        case_name,
        False,
        started,
        {
            "blocked": True,
            "gate": details.get("gate", "requires additional live credentials"),
            "message": (
                "This QA row is represented in the Reborn WebUI v2 live lane, "
                "but it is not default-runnable in this environment because the "
                "required live integration credentials or side-effect verifier "
                "are unavailable."
            ),
        },
    )


def _gated_case(case_name: str) -> CaseFn:
    async def run_gated(ctx: LiveQaContext) -> ProbeResult:
        return await _gated_qa_case(ctx, case_name)

    return run_gated


RAW_SLACK_USER_ID_PATTERN = re.compile(
    r"\b[UW](?=[A-Z0-9]*[0-9])[A-Z0-9]{8,}\b"
)
ENCODED_RAW_SLACK_USER_ID_PATTERN = re.compile(
    r"<@([UW][A-Z0-9]{8,})(?:\|[^>]*)?>"
)


def _raw_slack_user_ids_in_text(text: str) -> list[str]:
    """Raw Slack user ids (U…/W…) leaked into user-facing text.

    Encoded mentions are unambiguous. Bare tokens require a digit to avoid
    classifying all-caps prose such as UNDERSTAND as a Slack identifier.
    """
    source = text or ""
    encoded = ENCODED_RAW_SLACK_USER_ID_PATTERN.findall(source)
    bare_source = ENCODED_RAW_SLACK_USER_ID_PATTERN.sub("", source)
    return encoded + RAW_SLACK_USER_ID_PATTERN.findall(bare_source)


def _redact_slack_user_ids_in_text(text: str) -> str:
    source = text or ""
    without_mentions = ENCODED_RAW_SLACK_USER_ID_PATTERN.sub(
        "U_REDACTED", source
    )
    return RAW_SLACK_USER_ID_PATTERN.sub("U_REDACTED", without_mentions)


# Slack conversation ids (C… channel / D… DM / G… group) in the
# second-char-digit form real workspaces mint (C0…, D0…, G1…): requiring the
# digit keeps all-caps prose words (DELIVERED, CHANNELS, GENERAL) from
# false-positiving while every real leaked id still matches.
RAW_SLACK_CONVERSATION_ID_PATTERN = re.compile(r"\b[CDG][0-9][A-Z0-9]{7,}\b")


def _raw_slack_conversation_ids_in_text(text: str) -> list[str]:
    """Raw Slack conversation ids (C…/D…/G…) leaked into user-facing text."""
    return RAW_SLACK_CONVERSATION_ID_PATTERN.findall(text or "")


def _redact_slack_entity_ids_in_artifact_details(
    details: dict[str, object],
) -> None:
    """Remove Slack entity ids from persisted assistant-response fields."""

    def redact(value: object) -> object:
        if isinstance(value, str):
            return RAW_SLACK_CONVERSATION_ID_PATTERN.sub(
                "C_REDACTED",
                _redact_slack_user_ids_in_text(value),
            )
        if isinstance(value, dict):
            return {key: redact(item) for key, item in value.items()}
        if isinstance(value, list):
            return [redact(item) for item in value]
        if isinstance(value, tuple):
            return tuple(redact(item) for item in value)
        return value

    for key in (
        "text_excerpt",
        "routine_confirmation_initial_text_excerpt",
        "error",
        "semantic_judge",
        "capability_evidence",
    ):
        if key in details:
            details[key] = redact(details[key])


# Encoded Slack mention markup as stored in raw message text. Both the bare
# (<@U123…>) and labelled (<@U123…|name>) forms actually notify the target;
# a literal "@Display Name" renders as inert text and notifies nobody.
ENCODED_SLACK_MENTION_PATTERN = re.compile(r"<@U[A-Z0-9]+(?:\|[^>]*)?>")


def _encoded_mention_targets_user(text: str, user_id: str) -> bool:
    """True when ``text`` carries an encoded mention of exactly ``user_id``
    (label suffix allowed).

    Any-mention matching would let a self-mention or an unrelated <@U…> pass
    the mention-encoding probe while the person the prompt named is never
    notified — the target must be pinned, not just the markup shape.
    """
    if not user_id:
        return False
    return bool(
        re.search(rf"<@{re.escape(user_id)}(?:\|[^>]*)?>", text or "")
    )


EMAIL_ADDRESS_PATTERN = re.compile(
    r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"
)


def _email_addresses_in_text(text: str) -> list[str]:
    """Email addresses appearing in user-facing text (hallucination guard)."""
    return EMAIL_ADDRESS_PATTERN.findall(text or "")


def _display_name_tokens(name: str) -> list[str]:
    """Display-name tokens (>=3 chars) for word-boundary person matching —
    the same token rule the qa_9c digest ground-truth check applies, so
    "Benjamin Kurrek" accepts "Ben Kurrek" / "benjamin" answer forms."""
    return [
        token
        for token in re.split(r"[^a-z0-9]+", (name or "").lower())
        if len(token) >= 3
    ]


def _name_token_in_text(text: str, name: str) -> bool:
    """True when any (>=3 char) token of ``name`` appears word-bounded in
    ``text`` — the computable "named the person" check."""
    lowered = (text or "").lower()
    return any(
        re.search(rf"\b{re.escape(token)}\b", lowered)
        for token in _display_name_tokens(name)
    )


# Hyphenated all-caps code tokens (e.g. OOO-CANARY-FIXTURE) embedded in a
# status text: each segment is >=2 caps/digits and at least one hyphen joins
# them, so prose words and lowercase echoes never match.
STATUS_CODE_TOKEN_PATTERN = re.compile(r"\b[A-Z0-9]{2,}(?:-[A-Z0-9]{2,})+\b")


def _status_code_tokens(text: str) -> list[str]:
    """Hyphenated code tokens in a status text — when the ground-truth
    status carries one (OOO-CANARY-FIXTURE), the reply must quote it
    verbatim; token-level matching alone would accept a paraphrase that
    drops the code."""
    return STATUS_CODE_TOKEN_PATTERN.findall(text or "")


def _channel_name_mentioned(text: str, channel_name: str) -> bool:
    """Word-boundary channel-name match that treats ``-`` as part of the name.

    Plain ``\\b`` matching would find non-member "general" inside a mention of
    member "general-updates" (hyphen is a regex non-word char) and false-red
    the membership probe; hyphen-aware boundaries keep both arms exact.
    """
    name = (channel_name or "").strip().lower()
    if not name:
        return False
    pattern = rf"(?<![a-z0-9_-]){re.escape(name)}(?![a-z0-9_-])"
    return re.search(pattern, (text or "").lower()) is not None


async def case_qa_9a_slack_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _slack_connect_case(ctx, case_name="qa_9a_slack_connect")


async def case_qa_9b_routine_dm_delivery_exactly_once(ctx: LiveQaContext) -> ProbeResult:
    """Wrong-channel/duplicate-delivery probe.

    Uses the historically bug-triggering phrasing ("… and send me the result
    in a Slack DM") so the stored routine prompt reads like a delivery
    instruction, then asserts the result reaches the requester's Slack DM
    EXACTLY once, from the bot identity only, after a grace window. Failure
    modes this catches live: result delivered to another conversation (marker
    never appears in the DM → timeout), duplicate delivery (marker appears
    twice), and user-identity self-delivery (marker from a non-bot author).
    """
    return await _slack_delivery_routine_case(
        ctx,
        case_name="qa_9b_routine_dm_delivery_exactly_once",
        routine_prefix="reborn-qa-9b-dm-exactly-once",
        marker_prefix="REBORN_QA_9B_DM_EXACTLY_ONCE",
        routine_instruction=(
            "report the current UTC time using the time capability and "
            "send me the result in a Slack DM."
        ),
        required_delivery_text=[],
        # One-shot: a recurring schedule would legitimately re-post the same
        # marker on the next fire and break exactly-once counting (observed
        # live: "found 2" ~60s after the first delivery).
        schedule_instruction=(
            "As a one-time routine scheduled about 90 seconds from now "
            "(compute the time with the time capability, use the UTC timezone),"
        ),
        # Production parity: users who hit the duplicate-delivery bug had the
        # Slack tools extension installed, so the fired model had a user-token
        # send capability available. Without this the duplicate arm is
        # structurally inert (nothing to self-send with).
        creation_prompt_extra=(
            "Before creating the routine, make sure the Slack tools extension "
            "is installed and activated (the Slack account is already "
            "connected); install and activate it if it is not."
        ),
        exactly_once_grace_seconds=60.0,
        require_slack_tools_on_surface=True,
        expect_one_shot_schedule=True,
        follow_up_timezone_instruction="Use the UTC timezone for the schedule.",
    )


async def case_qa_9c_slack_digest_names_not_ids(ctx: LiveQaContext) -> ProbeResult:
    """Raw-user-id probe: a Slack DM digest must name senders, never echo
    raw U… ids from the history tools."""
    case_name = "qa_9c_slack_digest_names_not_ids"
    started = time.monotonic()
    marker = f"REBORN_QA_9C_DIGEST_{int(time.time() * 1000)}"
    result = await _live_chat_case(
        ctx,
        case_name=case_name,
        prompt=(
            "Using my connected Slack account, give me a digest of my direct "
            "message conversations: list each person I have a DM with by their "
            "name and one line about the most recent message. "
            f"Include the exact marker {marker} in your answer."
        ),
        marker=marker,
        required_text=[],
        timeout=240.0,
        expose_full_reply_text=True,
    )
    # Preserve the full reply only in memory for leak detection, then sanitize
    # every persisted string before any success or failure return.
    reply_text = str(
        result.details.pop("full_reply_text", None)
        or result.details.get("text_excerpt")
        or ""
    )
    _redact_slack_entity_ids_in_artifact_details(result.details)
    if not result.success:
        return result
    # Scan the FULL reply (a raw id early in a long digest must not escape
    # via excerpt truncation). Persist leak counts only below.
    # Persist leak COUNTS only: echoing the leaked identifiers into the
    # artifact JSON would re-leak the very values this probe exists to keep
    # out of persisted artifacts (the redacted excerpt would be moot).
    leaked_ids = _raw_slack_user_ids_in_text(reply_text)
    result.details["leaked_raw_user_id_count"] = len(set(leaked_ids))
    if leaked_ids:
        return _result(
            case_name,
            False,
            started,
            {
                **result.details,
                "error": (
                    f"digest leaked {len(set(leaked_ids))} raw Slack user "
                    "id(s) instead of display names"
                ),
            },
        )
    # Channel-ID arm (companion to qa_10i): a digest that surfaces raw C…/D…/G…
    # conversation ids instead of conversation names has the same raw-entity
    # hygiene failure as raw user ids.
    leaked_conversation_ids = _raw_slack_conversation_ids_in_text(reply_text)
    result.details["leaked_raw_conversation_id_count"] = len(
        set(leaked_conversation_ids)
    )
    if leaked_conversation_ids:
        return _result(
            case_name,
            False,
            started,
            {
                **result.details,
                "error": (
                    f"digest leaked {len(set(leaked_conversation_ids))} raw "
                    "Slack conversation id(s) instead of names"
                ),
            },
        )
    # Positive arm: the digest must actually NAME someone. Ground truth comes
    # from the Slack API with the personal token, so a cop-out answer ("you
    # have no DMs") cannot pass vacuously when human DM history exists.
    ground_truth = await _slack_personal_dm_counterpart_names(ctx)
    result.details["dm_counterpart_ground_truth"] = ground_truth
    known_names = [str(name) for name in ground_truth.get("names") or [] if name]
    if known_names:
        reply_lower = reply_text.lower()
        # Token-level matching: "Benjamin Kurrek" in ground truth must accept
        # a digest that says "Ben Kurrek" or "benjamin" — whole-string
        # matching false-fails on display-form differences.
        named = [
            name
            for name in known_names
            if any(
                re.search(rf"\b{re.escape(token)}\b", reply_lower)
                for token in re.split(r"[^a-z0-9]+", name.lower())
                if len(token) >= 3
            )
        ]
        result.details["ground_truth_names_found"] = named
        if not named:
            return _result(
                case_name,
                False,
                started,
                {
                    **result.details,
                    "error": (
                        "digest named none of the user's actual DM counterparts "
                        f"(expected at least one of {known_names!r})"
                    ),
                },
            )
    else:
        # No usable ground truth (token unavailable, API error, or a
        # workspace with no human DM counterparts): the positive arm did not
        # run. Record that visibly instead of letting the green over-read.
        result.details["vacuous_arms"] = [
            "dm_counterpart_ground_truth: "
            + str(ground_truth.get("error") or "no human DM counterparts")
        ]
    return result


async def case_qa_9d_routine_per_trigger_delivery_target(ctx: LiveQaContext) -> ProbeResult:
    """Per-trigger routing probe: the routine must be created with its OWN
    delivery_target_id (persisted on the trigger record) and deliver through
    it — not by mutating the user-wide default delivery target."""
    return await _slack_delivery_routine_case(
        ctx,
        case_name="qa_9d_routine_per_trigger_delivery_target",
        routine_prefix="reborn-qa-9d-per-trigger-target",
        marker_prefix="REBORN_QA_9D_PER_TRIGGER_TARGET",
        routine_instruction=(
            "report the current UTC time using the time capability as this "
            "routine's result."
        ),
        required_delivery_text=[],
        schedule_instruction=(
            "As a one-time routine scheduled about 90 seconds from now "
            "(compute the time with the time capability, use the UTC timezone),"
        ),
        creation_prompt_extra=(
            "Before creating the routine, make sure the Slack tools extension "
            "is installed and activated (the Slack account is already "
            "connected); install and activate it if it is not. "
            "Route THIS routine's results to my Slack DM by listing my outbound "
            "delivery targets and passing the Slack DM target id as "
            "delivery_target_id when creating the trigger. Do not change my "
            "default outbound delivery target."
        ),
        exactly_once_grace_seconds=60.0,
        require_persisted_delivery_target=True,
        require_slack_tools_on_surface=True,
        expect_one_shot_schedule=True,
        follow_up_timezone_instruction="Use the UTC timezone for the schedule.",
    )


# --- QA 10 family: Slack tool-correctness probes -----------------------------
#
# Every case seeds computable ground truth through the Slack Web API with the
# harness tokens BEFORE the chat prompt (per-run ms-suffix nonces on every
# seeded message), asserts its preconditions INSIDE the case try with distinct
# "probe precondition failed: …" messages, and judges the FULL reply text —
# no fuzzy heuristics, no vacuous passes, no shard crashes.
#
# Seeded cases anchor on the PERSONAL↔BOT DM
# (_slack_personal_bot_dm_channel), never on the delivery DM
# (_slack_delivery_channel_id): the delivery DM pairs the bot with the bound
# webchat user, a DIFFERENT human than the personal (xoxp) token's owner, so
# personal-token seeding/reading there fails channel_not_found (live run
# 29062917993).


async def _slack_api_get(
    token: str, method: str, params: dict[str, str] | None = None
) -> dict[str, object]:
    """GET a Slack Web API method; never raises.

    Transport/parse failures come back as ``{"ok": False, "error": …}`` (via
    ``_exc_text``) so probe arms turn them into distinct case failures instead
    of crashing the shard runner (guarded-httpx rule).
    """
    import httpx

    try:
        async with httpx.AsyncClient(timeout=20.0) as client:
            response = await client.get(
                f"https://slack.com/api/{method}",
                headers={"Authorization": f"Bearer {token}"},
                params=params or {},
            )
        payload = response.json()
    except Exception as exc:
        return {"ok": False, "error": _exc_text(exc)}
    if not isinstance(payload, dict):
        return {"ok": False, "error": f"non-object Slack {method} response"}
    return payload


async def _slack_api_post(
    token: str, method: str, payload: dict[str, object]
) -> dict[str, object]:
    """POST a Slack Web API method (JSON body); never raises — same
    guarded-httpx contract as ``_slack_api_get``."""
    import httpx

    try:
        async with httpx.AsyncClient(timeout=20.0) as client:
            response = await client.post(
                f"https://slack.com/api/{method}",
                headers={"Authorization": f"Bearer {token}"},
                json=payload,
            )
        body = response.json()
    except Exception as exc:
        return {"ok": False, "error": _exc_text(exc)}
    if not isinstance(body, dict):
        return {"ok": False, "error": f"non-object Slack {method} response"}
    return body


async def _slack_post_as(
    token: str, channel: str, text: str, thread_ts: str | None = None
) -> dict[str, object]:
    """chat.postMessage as the identity behind ``token`` (personal token =
    the connected user, bot token = actor B)."""
    payload: dict[str, object] = {"channel": channel, "text": text}
    if thread_ts:
        payload["thread_ts"] = thread_ts
    return await _slack_api_post(token, "chat.postMessage", payload)


async def _slack_auth_identity(token: str) -> dict[str, object]:
    """auth.test identity for ``token``: {ok, user_id, team_id} or
    {ok: False, error}."""
    payload = await _slack_api_get(token, "auth.test")
    if not payload.get("ok"):
        return {
            "ok": False,
            "error": str(payload.get("error") or "slack_auth_test_failed"),
        }
    return {
        "ok": True,
        "user_id": str(payload.get("user_id") or ""),
        "team_id": str(payload.get("team_id") or ""),
    }


async def _slack_user_status_text(token: str, user_id: str) -> dict[str, object]:
    """profile.status_text for ``user_id`` via users.info — the qa_10b
    read-verify ground truth (the probe never writes a status)."""
    payload = await _slack_api_get(token, "users.info", {"user": user_id})
    if not payload.get("ok"):
        return {
            "ok": False,
            "error": str(payload.get("error") or "slack_users_info_failed"),
        }
    user = payload.get("user")
    user = user if isinstance(user, dict) else {}
    profile = user.get("profile")
    profile = profile if isinstance(profile, dict) else {}
    return {"ok": True, "status_text": str(profile.get("status_text") or "")}


async def _slack_display_name(token: str, user_id: str) -> dict[str, object]:
    """Best display name for ``user_id`` via users.info (display_name →
    real_name → name fallback, same precedence as the qa_9c ground truth)."""
    payload = await _slack_api_get(token, "users.info", {"user": user_id})
    if not payload.get("ok"):
        return {
            "ok": False,
            "error": str(payload.get("error") or "slack_users_info_failed"),
        }
    user = payload.get("user")
    user = user if isinstance(user, dict) else {}
    profile = user.get("profile")
    profile = profile if isinstance(profile, dict) else {}
    for candidate in (
        profile.get("display_name"),
        profile.get("real_name"),
        user.get("real_name"),
        user.get("name"),
    ):
        if candidate:
            return {"ok": True, "display_name": str(candidate)}
    return {"ok": False, "error": f"user {user_id} has no display name"}


async def _slack_membership_view(token: str) -> dict[str, object]:
    """Membership ground truth for the connected user.

    ``member_channel_ids``/``member_channels`` come from users.conversations
    (the user's OWN memberships) paginated to exhaustion — an incomplete
    member list would misclassify a real membership as a lie and false-red
    the membership probe, so hitting the page cap is a hard error. ``listed``
    is the workspace public-channel directory from conversations.list, first
    pages only: truncation there merely SHRINKS the non-member candidate
    pool, it can never flip a verdict.
    """
    member_channels: list[dict[str, str]] = []
    cursor = ""
    for _ in range(20):
        params = {
            "types": "public_channel",
            "exclude_archived": "true",
            "limit": "200",
        }
        if cursor:
            params["cursor"] = cursor
        payload = await _slack_api_get(token, "users.conversations", params)
        if not payload.get("ok"):
            return {
                "ok": False,
                "error": str(
                    payload.get("error") or "slack_users_conversations_failed"
                ),
            }
        for channel in payload.get("channels") or []:
            if not isinstance(channel, dict):
                continue
            channel_id = str(channel.get("id") or "")
            if channel_id:
                member_channels.append(
                    {"id": channel_id, "name": str(channel.get("name") or "")}
                )
        cursor = str(
            (payload.get("response_metadata") or {}).get("next_cursor") or ""
        ).strip()
        if not cursor:
            break
    else:
        return {
            "ok": False,
            "error": "users.conversations pagination exceeded the page cap",
        }
    listed: list[dict[str, object]] = []
    cursor = ""
    for _ in range(5):
        params = {
            "types": "public_channel",
            "exclude_archived": "true",
            "limit": "200",
        }
        if cursor:
            params["cursor"] = cursor
        payload = await _slack_api_get(token, "conversations.list", params)
        if not payload.get("ok"):
            return {
                "ok": False,
                "error": str(
                    payload.get("error") or "slack_conversations_list_failed"
                ),
            }
        for channel in payload.get("channels") or []:
            if not isinstance(channel, dict):
                continue
            channel_id = str(channel.get("id") or "")
            if not channel_id:
                continue
            entry: dict[str, object] = {
                "id": channel_id,
                "name": str(channel.get("name") or ""),
            }
            if "is_member" in channel:
                entry["is_member"] = bool(channel.get("is_member"))
            listed.append(entry)
        cursor = str(
            (payload.get("response_metadata") or {}).get("next_cursor") or ""
        ).strip()
        if not cursor:
            break
    return {
        "ok": True,
        "member_channel_ids": [channel["id"] for channel in member_channels],
        "member_channels": member_channels,
        "listed": listed,
    }


def _slack_non_member_public_channels(
    view: dict[str, object],
) -> list[dict[str, str]]:
    """Listed public channels the user is provably NOT a member of.

    Pure ground-truth diff for the membership probe. Belt and braces: a
    listing row flagged ``is_member=True`` is excluded even if the paginated
    users.conversations scan somehow missed it — the negative arm must never
    call a real membership a lie.
    """
    member_ids = {
        str(channel_id) for channel_id in view.get("member_channel_ids") or []
    }
    non_members: list[dict[str, str]] = []
    for channel in view.get("listed") or []:
        if not isinstance(channel, dict):
            continue
        channel_id = str(channel.get("id") or "")
        name = str(channel.get("name") or "")
        if not channel_id or not name:
            continue
        if channel_id in member_ids or channel.get("is_member") is True:
            continue
        non_members.append({"id": channel_id, "name": name})
    return non_members


async def _slack_dm_counterpart(token: str, channel_id: str) -> dict[str, object]:
    """The OTHER member of a DM, resolved deterministically.

    Uses auth.test + conversations.members rather than the im object's
    perspective-dependent ``user`` field, and rather than
    ``_slack_personal_dm_counterpart_names`` (which deliberately filters out
    bot users — the seeded personal↔bot DM's counterpart IS the app's bot
    user).
    """
    identity = await _slack_auth_identity(token)
    if not identity.get("ok"):
        return {"ok": False, "error": f"auth.test failed: {identity.get('error')}"}
    own_user_id = str(identity.get("user_id") or "")
    members_payload = await _slack_api_get(
        token, "conversations.members", {"channel": channel_id, "limit": "10"}
    )
    if not members_payload.get("ok"):
        return {
            "ok": False,
            "error": str(
                members_payload.get("error") or "slack_conversations_members_failed"
            ),
        }
    members = [str(member) for member in members_payload.get("members") or []]
    others = [member for member in members if member and member != own_user_id]
    if len(others) != 1:
        return {
            "ok": False,
            "error": f"expected exactly one DM counterpart, got {others!r}",
        }
    counterpart_id = others[0]
    name_lookup = await _slack_display_name(token, counterpart_id)
    if not name_lookup.get("ok"):
        return {
            "ok": False,
            "error": f"users.info failed for the counterpart: {name_lookup.get('error')}",
        }
    return {
        "ok": True,
        "user_id": counterpart_id,
        "display_name": str(name_lookup.get("display_name") or ""),
        "own_user_id": own_user_id,
    }


def _require_slack_personal_token(ctx: LiveQaContext) -> str:
    token = _slack_personal_token(ctx.env)
    if not token:
        raise AssertionError(
            "probe precondition failed: Slack personal token not provisioned "
            f"({SLACK_PERSONAL_ACCESS_TOKEN_ENV})"
        )
    return token


def _require_slack_bot_token(ctx: LiveQaContext) -> str:
    token = _slack_bot_token(ctx.env)
    if not token:
        raise AssertionError(
            "probe precondition failed: Slack bot token not provisioned "
            f"({SLACK_BOT_TOKEN_ENV})"
        )
    return token


def _find_im_channel_for_user(channels: list[object], user_id: str) -> str | None:
    """The im channel whose counterpart is exactly ``user_id`` from a
    conversations.list types=im page (pure selection logic)."""
    if not user_id:
        return None
    for channel in channels:
        if not isinstance(channel, dict):
            continue
        if str(channel.get("user") or "") != user_id:
            continue
        channel_id = str(channel.get("id") or "")
        if channel_id:
            return channel_id
    return None


async def _slack_personal_bot_dm_channel(ctx: LiveQaContext) -> dict[str, object]:
    """Resolve the DM between the PERSONAL token's user and the app bot —
    the seeding/read anchor for every seeded QA 10 case.

    ``_slack_delivery_channel_id`` is the bot↔bound-webchat-user DM; the
    personal (xoxp) token's human is a DIFFERENT user who is not a member of
    that conversation, so personal-token chat.postMessage/history there fails
    channel_not_found (live run 29062917993). Resolution: (1) bot user id via
    bot-token auth.test, (2) personal-token conversations.list types=im scan
    (guarded pagination, same shape as
    ``_slack_personal_dm_counterpart_names``), (3) personal-token
    conversations.open fallback. Guarded like every probe API arm: failures
    come back as ``{"ok": False, "error": …}``, never raised. A successful
    resolution is cached on ``ctx`` — every case in a run anchors on the same
    DM.
    """
    cached = getattr(ctx, "_personal_bot_dm_cache", None)
    if isinstance(cached, dict) and cached.get("ok"):
        return cached
    resolved = await _resolve_slack_personal_bot_dm_channel(ctx)
    if resolved.get("ok"):
        ctx._personal_bot_dm_cache = resolved
    return resolved


async def _resolve_slack_personal_bot_dm_channel(
    ctx: LiveQaContext,
) -> dict[str, object]:
    personal_token = _slack_personal_token(ctx.env)
    if not personal_token:
        return {
            "ok": False,
            "error": (
                "Slack personal token not provisioned "
                f"({SLACK_PERSONAL_ACCESS_TOKEN_ENV})"
            ),
        }
    bot_token = _slack_bot_token(ctx.env)
    if not bot_token:
        return {
            "ok": False,
            "error": f"Slack bot token not provisioned ({SLACK_BOT_TOKEN_ENV})",
        }
    bot_identity = await _slack_auth_identity(bot_token)
    if not bot_identity.get("ok"):
        return {
            "ok": False,
            "error": f"bot auth.test failed: {bot_identity.get('error')}",
        }
    bot_user_id = str(bot_identity.get("user_id") or "")
    if not bot_user_id:
        return {"ok": False, "error": "bot auth.test returned no user_id"}
    cursor = ""
    for _ in range(10):
        params: dict[str, str] = {"types": "im", "limit": "200"}
        if cursor:
            params["cursor"] = cursor
        payload = await _slack_api_get(
            personal_token, "conversations.list", params
        )
        if not payload.get("ok"):
            return {
                "ok": False,
                "error": str(
                    payload.get("error") or "slack_conversations_list_failed"
                ),
            }
        channels = payload.get("channels")
        channel_id = _find_im_channel_for_user(
            channels if isinstance(channels, list) else [], bot_user_id
        )
        if channel_id:
            return {
                "ok": True,
                "channel_id": channel_id,
                "bot_user_id": bot_user_id,
                "opened": False,
            }
        cursor = str(
            (payload.get("response_metadata") or {}).get("next_cursor") or ""
        ).strip()
        if not cursor:
            break
    else:
        return {
            "ok": False,
            "error": "conversations.list pagination exceeded the page cap",
        }
    # No existing im with the bot: open one. Fails cleanly (e.g. a token
    # without im:write) instead of raising.
    opened = await _slack_api_post(
        personal_token, "conversations.open", {"users": bot_user_id}
    )
    if not opened.get("ok"):
        return {
            "ok": False,
            "error": str(opened.get("error") or "slack_conversations_open_failed"),
        }
    channel = opened.get("channel")
    opened_channel_id = (
        str(channel.get("id") or "") if isinstance(channel, dict) else ""
    )
    if not opened_channel_id:
        return {"ok": False, "error": "conversations.open returned no channel id"}
    return {
        "ok": True,
        "channel_id": opened_channel_id,
        "bot_user_id": bot_user_id,
        "opened": True,
    }


async def _require_slack_personal_bot_dm_channel(ctx: LiveQaContext) -> str:
    """The personal↔bot DM channel id, or the distinct precondition failure
    every seeded QA 10 case must fail red with."""
    resolved = await _slack_personal_bot_dm_channel(ctx)
    if not resolved.get("ok"):
        raise AssertionError(
            "probe precondition failed: could not resolve the personal↔bot "
            f"DM: {resolved.get('error')}"
        )
    return str(resolved.get("channel_id") or "")


def _require_slack_second_user_token(ctx: LiveQaContext) -> str:
    """Assert the optional second-HUMAN identity is provisioned.

    Any future arm that strictly needs a second human (not the bot) MUST call
    this and fail red — silently skipping would hollow out the arm forever.
    """
    token = _slack_second_user_token(ctx.env)
    if not token:
        raise AssertionError(
            "probe precondition failed: second-identity token not provisioned "
            "(AUTH_LIVE_SLACK_SECOND_USER_TOKEN)"
        )
    return token


async def _seed_slack_fixture_message(
    token: str,
    channel_id: str,
    text: str,
    *,
    label: str,
    actor: str,
    thread_ts: str | None = None,
) -> str:
    """Seed one fixture message via the Slack Web API and return its ts.

    A rejected post raises a distinct "probe precondition failed" message: the
    chat arm must never run against an unseeded conversation (that would be a
    red with a fabricated cause, or worse a vacuous pass).
    """
    posted = await _slack_post_as(token, channel_id, text, thread_ts=thread_ts)
    if not posted.get("ok"):
        raise AssertionError(
            f"probe precondition failed: seeding {label} via the {actor} token "
            f"failed: {posted.get('error')!r}"
        )
    ts = str(posted.get("ts") or "")
    if not ts:
        raise AssertionError(
            f"probe precondition failed: seeding {label} via the {actor} token "
            "returned no message ts"
        )
    return ts


def _classify_encoded_mention_messages(
    messages: object,
    *,
    marker: str,
    author_user_id: str,
) -> dict[str, object]:
    """Classify marker messages for the mention-encoding probe (pure).

    Target selection is ENCODED-MENTION-FIRST: only marker messages whose
    RAW text carries an encoded ``<@U…>`` mention are authorship candidates —
    a marker echo without an encoded mention (e.g. an assistant reply
    delivered into the same DM) is never selected, so it can neither pass
    nor mask the real post.

    Authorship is judged by Slack's authoritative ``user`` field, which no
    other token can forge (``chat:write.customize`` changes display
    name/icon only — never ``user``). ``bot_id`` alone does NOT disqualify:
    new (granular) Slack apps stamp EVERY user-token post with the app's
    ``bot_id``/``bot_profile`` ("via app"), while ``user`` remains the
    human author — this workspace's own personal-token seeds all carry the
    stamp, and the legacy classic-app ``as_user`` escape hatch does not
    exist for new apps. A true bot-identity post (the serving bot, the
    host's reply delivery) has ``user`` = the BOT user id and still fails
    the author check. The stamp is surfaced as ``via_app`` for forensics.

    Returns a dict:
      found            — first encoded marker message with user == author
                          ({text, ts, via_app}) or None
      author_mismatch  — redacted {ts, bot, user_matches_author} entries for
                          encoded marker messages from other identities
      unencoded_author_marker_ts — ts of a marker message the connected user
                          posted WITHOUT an encoded mention (the literal-@
                          failure), else None
      unencoded_author_text — that message's raw text (for redacted
                          diagnostics), else None
    """
    found: dict[str, object] | None = None
    author_mismatch: list[dict[str, object]] = []
    unencoded_author_marker_ts: object = None
    unencoded_author_text: str | None = None
    for message in messages if isinstance(messages, list) else []:
        if not isinstance(message, dict):
            continue
        text = str(message.get("text") or "")
        if marker not in text:
            continue
        author_matches = (
            bool(author_user_id)
            and str(message.get("user") or "") == author_user_id
        )
        if not ENCODED_SLACK_MENTION_PATTERN.search(text):
            if author_matches and unencoded_author_marker_ts is None:
                unencoded_author_marker_ts = message.get("ts")
                unencoded_author_text = text
            continue
        if author_matches:
            if found is None:
                found = {
                    "text": text,
                    "ts": message.get("ts"),
                    "via_app": bool(message.get("bot_id")),
                }
            continue
        author_mismatch.append(
            {
                "ts": message.get("ts"),
                "bot": bool(message.get("bot_id")),
                "user_matches_author": False,
            }
        )
    return {
        "found": found,
        "author_mismatch": author_mismatch,
        "unencoded_author_marker_ts": unencoded_author_marker_ts,
        "unencoded_author_text": unencoded_author_text,
    }


async def _wait_for_authored_slack_message(
    token: str,
    *,
    channel_id: str,
    marker: str,
    author_user_id: str,
    oldest_epoch: float,
    timeout: float = 90.0,
) -> dict[str, object]:
    """Poll conversations.history for the ENCODED-mention marker message
    authored by the connected user and return its raw text (ground truth
    for the mention-encoding probe). Selection/authorship semantics live in
    :func:`_classify_encoded_mention_messages`.

    Guarded like every probe API arm: transient errors keep the poll alive
    and only the last error is reported — never raised, never a shard crash.
    Encoded marker messages from OTHER identities are reported as
    ``author_mismatch`` so a wrong-identity post fails with its real cause,
    and a marker post by the connected user WITHOUT an encoded mention is
    reported as ``unencoded_author_marker`` (the literal-@ failure).
    """
    deadline = time.monotonic() + timeout
    last_error: str | None = None
    verdict: dict[str, object] = {}
    while True:
        payload = await _slack_api_get(
            token,
            "conversations.history",
            {
                "channel": channel_id,
                "oldest": f"{oldest_epoch:.6f}",
                "limit": "100",
                "inclusive": "true",
            },
        )
        if payload.get("ok"):
            verdict = _classify_encoded_mention_messages(
                payload.get("messages"),
                marker=marker,
                author_user_id=author_user_id,
            )
            found = verdict.get("found")
            if isinstance(found, dict):
                return {"found": True, **found}
        else:
            last_error = str(payload.get("error") or "slack_history_failed")
        if time.monotonic() >= deadline:
            result: dict[str, object] = {"found": False, "error": last_error}
            if verdict.get("author_mismatch"):
                result["author_mismatch"] = verdict["author_mismatch"]
            if verdict.get("unencoded_author_marker_ts") is not None:
                result["unencoded_author_marker_ts"] = verdict[
                    "unencoded_author_marker_ts"
                ]
                result["unencoded_author_text"] = verdict.get(
                    "unencoded_author_text"
                )
            return result
        await asyncio.sleep(5.0)


async def _slack_correctness_chat_reply(
    ctx: LiveQaContext,
    *,
    case_name: str,
    started: float,
    prompt: str,
    answer_marker: str,
    extra_details: dict[str, object],
    expected_capability: str | None = None,
    accept_any_capability: tuple[str, ...] = (),
    expected_capability_statuses: tuple[str, ...] = ("completed",),
    expected_capability_sequence: tuple[str, ...] = (),
    expected_capability_arguments: dict[str, dict[str, str]] | None = None,
    timeout: float = 240.0,
) -> tuple[ProbeResult, str]:
    """Chat arm shared by the QA-10 Slack tool-correctness probes.

    Runs the WebUI chat turn, waits for its structural terminal state, and
    hands back the FULL in-memory reply text for seeded content assertions
    (excerpt truncation must never blind a marker/leak check). The synthetic
    answer marker remains prompt context but is not a liveness condition. The
    full text is stripped from persisted details on both paths; a failed chat
    result is ready to return as-is with latency re-anchored to the case start.

    `expected_capability` (and the sequence/argument variants) assert TOOL
    IDENTITY — the model must terminally use that exact capability. Use
    `accept_any_capability` instead when the case asserts an OUTCOME that any
    of several capabilities can satisfy: the arm passes as long as at least
    ONE of the accept-any set produced current-turn terminal evidence, rather
    than pinning a single tool id. The two are composable — accept-any members
    form an OR-group while every plain `expected_capability` stays required.
    """
    expected_capabilities = list(
        dict.fromkeys(
            [
                *(
                    [expected_capability]
                    if expected_capability is not None
                    else []
                ),
                *expected_capability_sequence,
                *(expected_capability_arguments or {}),
                *accept_any_capability,
            ]
        )
    )
    chat = await _live_chat_case(
        ctx,
        case_name=case_name,
        prompt=prompt,
        marker=answer_marker,
        required_text=[],
        extensions=[SLACK_EXTENSION_REQUIREMENT],
        timeout=timeout,
        extra_details=extra_details,
        expose_full_reply_text=True,
        enforce_marker=False,
        capture_submission_identity=bool(expected_capabilities),
    )
    reply_text = str(
        chat.details.pop("full_reply_text", None)
        or chat.details.get("text_excerpt")
        or ""
    )
    _redact_slack_entity_ids_in_artifact_details(chat.details)
    if not chat.success:
        failure_category = str(chat.details.get("failure_category") or "")
        if failure_category.endswith(("_unavailable", "_transient")):
            chat.details["failure_class"] = "infrastructure"
        chat.latency_ms = int((time.monotonic() - started) * 1000)
        return chat, reply_text

    if expected_capabilities:
        submission_identity = chat.details.get("submission_identity")
        evidence = _current_turn_capability_evidence(
            ctx.reborn_home,
            submission_identity if isinstance(submission_identity, dict) else {},
            expected_capabilities,
            set(expected_capability_statuses),
        )
        chat.details.update(
            {
                "expected_capabilities": expected_capabilities,
                "expected_capability_statuses": list(expected_capability_statuses),
                "expected_capability_sequence": list(expected_capability_sequence),
                "expected_capability_argument_fields": {
                    capability_id: list(arguments)
                    for capability_id, arguments in (
                        expected_capability_arguments or {}
                    ).items()
                },
                "capability_evidence": evidence,
            }
        )
        statuses = evidence.get("statuses")
        accept_any_set = set(accept_any_capability)
        if isinstance(statuses, dict):
            # Plain expected capabilities stay individually required; the
            # accept-any set is an OR-group that only counts as missing when
            # NONE of its members produced current-turn terminal evidence.
            missing_capabilities = [
                capability_id
                for capability_id in expected_capabilities
                if capability_id not in accept_any_set
                and not statuses.get(capability_id, [])
            ]
            if accept_any_capability and not any(
                statuses.get(capability_id, [])
                for capability_id in accept_any_capability
            ):
                missing_capabilities.append(
                    "any-of:" + "|".join(accept_any_capability)
                )
        else:
            missing_capabilities = list(expected_capabilities)
        evidence_read_error = evidence.get("read_error")
        observed_arguments = evidence.get("input_arguments")
        argument_mismatches = {
            capability_id: list(expected_arguments)
            for capability_id, expected_arguments in (
                expected_capability_arguments or {}
            ).items()
            if not isinstance(observed_arguments, dict)
            or not isinstance(observed_arguments.get(capability_id), list)
            or not observed_arguments[capability_id]
            or any(
                not isinstance(arguments, dict)
                or any(
                    arguments.get(field) != expected_value
                    for field, expected_value in expected_arguments.items()
                )
                for arguments in observed_arguments[capability_id]
            )
        }
        if evidence_read_error:
            chat.success = False
            chat.details.update(
                {
                    "error": (
                        "Slack capability evidence could not be read: "
                        f"{evidence_read_error}"
                    ),
                    "failure_class": "infrastructure",
                    "failure_category": "capability_evidence_unavailable",
                    "failure_status": "inconclusive",
                    "inconclusive": True,
                    "blocking": False,
                }
            )
        elif missing_capabilities:
            chat.success = False
            chat.details.update(
                {
                    "error": (
                        "Slack correctness reply did not produce current-turn "
                        "terminal evidence for the expected capabilities: "
                        f"{missing_capabilities!r}"
                    ),
                    "failure_class": "model_quality",
                    "failure_category": "missing_expected_capability",
                    "failure_status": "failed",
                }
            )
        elif argument_mismatches:
            chat.success = False
            chat.details.update(
                {
                    "error": (
                        "Slack correctness reply used unexpected arguments for "
                        "the exact capability calls; mismatched fields: "
                        f"{argument_mismatches!r}"
                    ),
                    "failure_class": "model_quality",
                    "failure_category": "unexpected_capability_arguments",
                    "failure_status": "failed",
                }
            )
        elif expected_capability_sequence:
            terminal_sequence = evidence.get("terminal_sequence")
            observed_sequence = [
                str(item.get("capability_id") or "")
                for item in (
                    terminal_sequence if isinstance(terminal_sequence, list) else []
                )
                if isinstance(item, dict)
            ]
            first_positions = [
                observed_sequence.index(capability_id)
                for capability_id in expected_capability_sequence
                if capability_id in observed_sequence
            ]
            if (
                len(first_positions) != len(expected_capability_sequence)
                or first_positions != sorted(first_positions)
                or len(set(first_positions)) != len(first_positions)
            ):
                chat.success = False
                chat.details.update(
                    {
                        "error": (
                            "Slack correctness reply used capabilities in the "
                            "wrong order: expected "
                            f"{list(expected_capability_sequence)!r}, observed "
                            f"{observed_sequence!r}"
                        ),
                        "failure_class": "model_quality",
                        "failure_category": "unexpected_capability_order",
                        "failure_status": "failed",
                    }
                )
    _redact_slack_entity_ids_in_artifact_details(chat.details)
    chat.latency_ms = int((time.monotonic() - started) * 1000)
    return chat, reply_text


def _slack_correctness_failure_result(
    case_name: str,
    started: float,
    details: dict[str, object],
    exc: BaseException,
) -> ProbeResult:
    error = _exc_text(exc)
    precondition = error.startswith("probe precondition failed:")
    artifact_details = {
        **details,
        "error": error,
        "failure_class": "precondition" if precondition else "product",
        "failure_category": (
            "invalid_fixture" if precondition else "answer_mismatch"
        ),
        "failure_status": "failed",
    }
    _redact_slack_entity_ids_in_artifact_details(artifact_details)
    return _result(
        case_name,
        False,
        started,
        artifact_details,
    )


async def case_qa_10a_slack_self_attribution(ctx: LiveQaContext) -> ProbeResult:
    """Self-attribution probe: reading a DM, the agent must identify which
    messages the CONNECTED USER sent and never claim other-author messages
    as the user's own.

    Pins the no-self-identity gap (Firat audit part 1): the host never
    surfaces auth.test self-identity to the model, so "messages I sent"
    cannot be computed from history authorship. Seeds two personal-token
    messages (mine) and two bot-token messages (not mine), all carrying the
    per-run nonce, then requires the reply to list exactly the personal
    markers. Note: a bot actor is still distinguishable via ``bot_id`` even
    without self-identity, so this arm under-detects relative to a second
    human actor — the strict variant needs AUTH_LIVE_SLACK_SECOND_USER_TOKEN
    (see ``_require_slack_second_user_token``) plus a shared channel once a
    dedicated canary user exists.
    """
    case_name = "qa_10a_slack_self_attribution"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10A_SELF_ATTRIBUTION_{suffix}"
    self_markers = [f"SELFMSG_A_{suffix}", f"SELFMSG_B_{suffix}"]
    other_markers = [f"OTHERMSG_C_{suffix}", f"OTHERMSG_D_{suffix}"]
    details: dict[str, object] = {
        "self_markers": self_markers,
        "other_markers": other_markers,
    }
    try:
        personal_token = _require_slack_personal_token(ctx)
        bot_token = _require_slack_bot_token(ctx)
        channel_id = await _require_slack_personal_bot_dm_channel(ctx)
        for marker_text in self_markers:
            await _seed_slack_fixture_message(
                personal_token,
                channel_id,
                marker_text,
                label=marker_text,
                actor="personal",
            )
        for marker_text in other_markers:
            await _seed_slack_fixture_message(
                bot_token,
                channel_id,
                marker_text,
                label=marker_text,
                actor="bot",
            )
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "Using my connected Slack account, read the recent messages in "
                f"the Slack conversation with ID {channel_id} and list the "
                "exact marker codes of only the messages that I myself sent — "
                "do not include or mention any code from messages sent by "
                f"anyone else. Include the exact marker {answer_marker} in "
                "your answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
            expected_capability="slack.get_conversation_history",
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        missing_self = [m for m in self_markers if m not in reply_text]
        details["missing_self_markers"] = missing_self
        misattributed = [m for m in other_markers if m in reply_text]
        details["misattributed_other_markers"] = misattributed
        if missing_self:
            raise AssertionError(
                "agent could not attribute the connected user's own messages "
                f"(no-self-identity gap): reply omitted {missing_self}"
            )
        if misattributed:
            raise AssertionError(
                "agent misattributed other-author messages as the user's own "
                f"(no-self-identity gap): reply included {misattributed}"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10b_slack_ooo_status(ctx: LiveQaContext) -> ProbeResult:
    """Own-status readback probe (read-verify mode): the QA account carries
    a manually-set permanent OOO canary status; "what does my current status
    say" must surface it.

    Pins dropped status fields + self-identity (Firat audit part 2): the
    host's user lookup strips status_text/status_emoji and the agent cannot
    resolve which user is "me", so it fabricates or refuses. Ground truth is
    READ via personal-token auth.test + users.info before the prompt — the
    probe never writes to the live account (no users.profile.set seeding,
    nothing to restore). The reply must carry a word-boundary token of the
    ground-truth status text, and when the status embeds a hyphenated code
    token (e.g. OOO-CANARY-FIXTURE) that exact substring must be quoted
    verbatim — token matching alone would accept a paraphrase that drops the
    code.
    """
    case_name = "qa_10b_slack_ooo_status"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10B_STATUS_{suffix}"
    details: dict[str, object] = {}
    try:
        personal_token = _require_slack_personal_token(ctx)
        identity = await _slack_auth_identity(personal_token)
        if not identity.get("ok"):
            raise AssertionError(
                "probe precondition failed: auth.test could not resolve the "
                f"connected user's identity: {identity.get('error')!r}"
            )
        own_user_id = str(identity.get("user_id") or "")
        status = await _slack_user_status_text(personal_token, own_user_id)
        if not status.get("ok"):
            raise AssertionError(
                "probe precondition failed: users.info could not read the "
                f"QA account's status: {status.get('error')!r}"
            )
        status_text = str(status.get("status_text") or "").strip()
        details["status_text"] = status_text
        if not status_text:
            raise AssertionError(
                "probe precondition failed: no status is set on the QA account "
                "— restore the OOO canary status fixture"
            )
        code_tokens = _status_code_tokens(status_text)
        details["status_code_tokens"] = code_tokens
        if not _display_name_tokens(status_text):
            raise AssertionError(
                "probe precondition failed: the QA account status text "
                f"{status_text!r} has no matchable token (>=3 chars) for the "
                "readback arm"
            )
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "Check Slack and tell me exactly what my current Slack status "
                f"says, word for word. Include the exact marker {answer_marker} "
                "in your answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
            expected_capability="slack.get_user_info",
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        missing_codes = [code for code in code_tokens if code not in reply_text]
        details["missing_status_code_tokens"] = missing_codes
        if missing_codes:
            raise AssertionError(
                "agent did not report the user's own current Slack status "
                "(dropped status fields / self-identity): reply lacked the "
                f"exact status code token(s) {missing_codes}"
            )
        token_matched = _name_token_in_text(reply_text, status_text)
        details["status_token_matched"] = token_matched
        if not token_matched:
            raise AssertionError(
                "agent did not report the user's own current Slack status "
                "(dropped status fields / self-identity): reply contained no "
                "word-boundary token (>=3 chars) of the ground-truth status "
                "text"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10c_slack_thread_replies(ctx: LiveQaContext) -> ProbeResult:
    """Thread-visibility probe: listing a conversation "including thread
    replies" must surface the replies seeded under a thread root.

    Asserts the OUTCOME — the seeded thread replies appear in the answer —
    not the identity of the tool that fetched them. Thread visibility is
    satisfied whether the model reaches the replies through the dedicated
    `slack.get_thread_replies` capability or through indexed
    `slack.search_messages` (whose threaded hits carry reply text), so the
    capability arm accepts either (accept-any). Conversation history is NOT a
    member: the shipped manifest documents that history returns thread
    parents only, never replies. The real regression this guards is the
    agent being UNABLE to see thread replies at all: THREADROOT/TOPLEVEL
    presence is the control proving plain history reads worked, and the
    missing-REPLY_* check stays red when the replies are not surfaced.
    """
    case_name = "qa_10c_slack_thread_replies"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10C_THREADS_{suffix}"
    root_marker = f"THREADROOT_{suffix}"
    reply_markers = [
        f"REPLY_ONE_{suffix}",
        f"REPLY_TWO_{suffix}",
        f"REPLY_THREE_{suffix}",
    ]
    top_level_marker = f"TOPLEVEL_{suffix}"
    details: dict[str, object] = {
        "root_marker": root_marker,
        "reply_markers": reply_markers,
        "top_level_marker": top_level_marker,
    }
    try:
        personal_token = _require_slack_personal_token(ctx)
        bot_token = _require_slack_bot_token(ctx)
        channel_id = await _require_slack_personal_bot_dm_channel(ctx)
        root_ts = await _seed_slack_fixture_message(
            personal_token,
            channel_id,
            root_marker,
            label=root_marker,
            actor="personal",
        )
        for reply_marker in reply_markers:
            await _seed_slack_fixture_message(
                bot_token,
                channel_id,
                reply_marker,
                label=reply_marker,
                actor="bot",
                thread_ts=root_ts,
            )
        await _seed_slack_fixture_message(
            personal_token,
            channel_id,
            top_level_marker,
            label=top_level_marker,
            actor="personal",
        )
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "Using my connected Slack account, read the Slack conversation "
                f"with ID {channel_id} and list the exact text of every "
                "message from the last ten minutes, including every reply in "
                f"the thread under the message containing {root_marker}. "
                f"Include the exact marker {answer_marker} in your answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
            # Outcome, not tool identity: any capability that can genuinely
            # retrieve thread-reply content satisfies the arm. Per the shipped
            # manifest, conversation history NEVER returns replies (only
            # thread parents), so it is deliberately NOT in this set; indexed
            # search does surface reply text (threaded hits), so a model
            # reaching the replies via search passes. The missing-REPLY_*
            # assert below is what actually pins thread visibility.
            accept_any_capability=(
                "slack.get_thread_replies",
                "slack.search_messages",
            ),
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        missing_control = [
            marker
            for marker in (root_marker, top_level_marker)
            if marker not in reply_text
        ]
        details["missing_control_markers"] = missing_control
        if missing_control:
            raise AssertionError(
                "control arm failed: plain conversation history did not "
                f"surface the seeded top-level messages {missing_control} — "
                "cannot judge the thread-replies arm"
            )
        missing_replies = [
            marker for marker in reply_markers if marker not in reply_text
        ]
        details["missing_thread_reply_markers"] = missing_replies
        if missing_replies:
            raise AssertionError(
                "thread replies are invisible to the agent (missing "
                f"thread-replies capability): reply omitted {missing_replies}"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10d_slack_channel_membership(ctx: LiveQaContext) -> ProbeResult:
    """Membership-honesty probe: "which channels am I a member of" must
    match users.conversations ground truth.

    Pins the channel-membership lie: without a membership-scoped view the
    agent lists conversations.list results (every public channel in the
    workspace) as if the user were a member of them all. Ground truth is
    API-computed before the prompt: the reply must name at least one true
    member channel and must not name ANY channel from the provably
    non-member pool (hyphen-aware word-boundary matching).
    """
    case_name = "qa_10d_slack_channel_membership"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10D_MEMBERSHIP_{suffix}"
    details: dict[str, object] = {}
    try:
        personal_token = _require_slack_personal_token(ctx)
        view = await _slack_membership_view(personal_token)
        details["membership_view_ok"] = bool(view.get("ok"))
        if not view.get("ok"):
            raise AssertionError(
                "probe precondition failed: could not read Slack membership "
                f"ground truth: {view.get('error')!r}"
            )
        member_names = [
            str(channel.get("name"))
            for channel in view.get("member_channels") or []
            if isinstance(channel, dict) and channel.get("name")
        ]
        non_member_channels = _slack_non_member_public_channels(view)
        details["member_channel_count"] = len(member_names)
        details["non_member_channel_count"] = len(non_member_channels)
        if not member_names:
            raise AssertionError(
                "probe precondition failed: the connected user is not a "
                "member of any named public channel — the positive membership "
                "arm has no ground truth (env must provide one)"
            )
        if not non_member_channels:
            raise AssertionError(
                "probe precondition failed: no non-member public channel "
                "available in the workspace listing — the membership-lie arm "
                "has nothing to catch (env must provide one)"
            )
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "Using my connected Slack account, list the names of the "
                "Slack channels I am a member of — channel names only. "
                f"Include the exact marker {answer_marker} in your answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
            expected_capability="slack.list_conversations",
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        claimed_non_members = sorted(
            {
                channel["name"]
                for channel in non_member_channels
                if _channel_name_mentioned(reply_text, channel["name"])
            }
        )
        details["non_member_channels_claimed"] = claimed_non_members
        named_members = sorted(
            {
                name
                for name in member_names
                if _channel_name_mentioned(reply_text, name)
            }
        )
        details["member_channels_named"] = named_members
        if claimed_non_members:
            raise AssertionError(
                "membership lie: the reply named public channels the user is "
                f"NOT a member of: {claimed_non_members}"
            )
        if not named_members:
            raise AssertionError(
                "reply named none of the user's actual member channels "
                f"(expected at least one of {sorted(set(member_names))!r})"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10e_slack_error_honesty(ctx: LiveQaContext) -> ProbeResult:
    """Error-honesty probe: a failing Slack read must surface the exact
    Slack error code, not a paraphrase.

    Pins host error-code erasure: the Slack tool host collapses Slack API
    error codes (here ``channel_not_found`` for the guaranteed-nonexistent
    conversation C0CANARYNOPE) into a generic failure string, so neither the
    agent nor the user ever sees the real cause. Red until the
    structured-error fix lands; no seeding required.
    """
    case_name = "qa_10e_slack_error_honesty"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10E_ERROR_HONESTY_{suffix}"
    details: dict[str, object] = {"expected_error_code": "channel_not_found"}
    try:
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "Try to read the message history of the Slack conversation "
                "with ID C0CANARYNOPE and tell me the exact error code the "
                "Slack tool reported, verbatim. Include the exact marker "
                f"{answer_marker} in your answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
            expected_capability="slack.get_conversation_history",
            expected_capability_statuses=("completed", "failed"),
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        if "channel_not_found" not in reply_text.lower():
            raise AssertionError(
                "the exact Slack error code was erased before reaching the "
                "user: reply did not contain channel_not_found"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10f_slack_mention_encoding(ctx: LiveQaContext) -> ProbeResult:
    """Mention-encoding probe: a posted @-mention must be <@U…>-encoded in
    the message's RAW text so the target is actually notified.

    The prompt supplies an exact DM conversation ID, so a completed
    slack.get_conversation_info lookup is required before the verified write.

    Pins literal-@ mention posting: the model writes "@Display Name" as
    plain text, which renders inert and notifies nobody. Ground truth is
    API-computed after the turn — the marker message authored by the
    connected user is fetched from conversations.history and its raw text
    must match the encoded-mention pattern.
    """
    case_name = "qa_10f_slack_mention_encoding"
    started = time.monotonic()
    wall_started = time.time()
    suffix = str(int(wall_started * 1000))
    answer_marker = f"REBORN_QA_10F_MENTION_ANSWER_{suffix}"
    mention_marker = f"MENTION_{suffix}"
    details: dict[str, object] = {"mention_marker": mention_marker}
    try:
        personal_token = _require_slack_personal_token(ctx)
        channel_id = await _require_slack_personal_bot_dm_channel(ctx)
        counterpart = await _slack_dm_counterpart(personal_token, channel_id)
        if not counterpart.get("ok"):
            raise AssertionError(
                "probe precondition failed: could not resolve the seeded DM "
                f"counterpart: {counterpart.get('error')!r}"
            )
        display_name = str(counterpart.get("display_name") or "")
        own_user_id = str(counterpart.get("own_user_id") or "")
        target_user_id = str(counterpart.get("user_id") or "")
        details["counterpart_display_name"] = display_name
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "Using my connected Slack account, post one message in the "
                f"Slack conversation with ID {channel_id} that @-mentions "
                f"{display_name} so they actually get notified. The message "
                f"text must include the exact marker {mention_marker}. Then "
                "confirm you posted it and include the exact marker "
                f"{answer_marker} in your reply."
            ),
            answer_marker=answer_marker,
            extra_details=details,
            expected_capability="slack.get_conversation_info",
            expected_capability_sequence=(
                "slack.get_conversation_info",
                "slack.send_message",
            ),
            expected_capability_arguments={
                "slack.get_conversation_info": {"channel": channel_id}
            },
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        excerpt = str(details.get("text_excerpt") or "")
        if excerpt:
            # The model often echoes the encoded mention it posted — keep raw
            # user ids out of persisted artifacts, same as qa_9c/qa_10i.
            details["text_excerpt"] = _redact_slack_user_ids_in_text(excerpt)
        posted = await _wait_for_authored_slack_message(
            personal_token,
            channel_id=channel_id,
            marker=mention_marker,
            author_user_id=own_user_id,
            oldest_epoch=wall_started - 5.0,
        )
        details["posted_message_found"] = bool(posted.get("found"))
        if not posted.get("found"):
            # Selection is encoded-mention-first, so a marker post by the
            # connected user WITHOUT an encoded mention is the literal-@
            # failure — the original pin this probe exists for.
            if posted.get("unencoded_author_marker_ts") is not None:
                details["mention_encoded"] = False
                details["posted_text_redacted"] = _redact_slack_user_ids_in_text(
                    str(posted.get("unencoded_author_text") or "")
                )
                raise AssertionError(
                    "posted mention is NOT <@U…>-encoded in the raw message "
                    "text — a literal @-name notifies nobody"
                )
            if posted.get("author_mismatch"):
                raise AssertionError(
                    "the mention message was posted from the wrong identity "
                    "(expected the connected user, found "
                    f"{posted.get('author_mismatch')!r})"
                )
            raise AssertionError(
                "the agent did not post the mention message to the DM "
                f"({posted.get('error') or 'marker never appeared in history'})"
            )
        raw_text = str(posted.get("text") or "")
        # Selection guarantees the encoded-mention shape; record the granular
        # "via app" stamp (bot_id on a user-token post) for forensics.
        details["mention_encoded"] = True
        details["mention_via_app"] = bool(posted.get("via_app"))
        # Encoded is not enough: the mention must target the counterpart the
        # prompt named — a self-mention or an unrelated <@U…> notifies the
        # wrong person and must fail.
        targets_counterpart = _encoded_mention_targets_user(
            raw_text, target_user_id
        )
        details["mention_targets_counterpart"] = targets_counterpart
        if not targets_counterpart:
            details["posted_text_redacted"] = _redact_slack_user_ids_in_text(
                raw_text
            )
            raise AssertionError(
                "posted mention is encoded but does not target the requested "
                "counterpart — the person the prompt named is never notified"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10g_slack_last_message_sent(ctx: LiveQaContext) -> ProbeResult:
    """Conversation-scoped last-sent contract over seeded history.

    The prompt names the seeded conversation, requires a fresh
    slack.get_conversation_history call, and verifies the exact per-run nonce.
    """
    case_name = "qa_10g_slack_last_message_sent"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10G_LAST_SENT_{suffix}"
    last_sent_marker = f"LASTSENT_{suffix}"
    details: dict[str, object] = {"last_sent_marker": last_sent_marker}
    try:
        personal_token = _require_slack_personal_token(ctx)
        channel_id = await _require_slack_personal_bot_dm_channel(ctx)
        await _seed_slack_fixture_message(
            personal_token,
            channel_id,
            last_sent_marker,
            label=last_sent_marker,
            actor="personal",
        )
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "What is the exact text of the most recent message I sent in "
                f"the Slack conversation with ID {channel_id}? Include the "
                f"exact marker {answer_marker} in your answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
            expected_capability="slack.get_conversation_history",
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        if last_sent_marker not in reply_text:
            raise AssertionError(
                "agent did not surface the user's most recent sent message "
                f"(search-lag/self-identity class): reply lacked {last_sent_marker}"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10g_slack_last_message_sent_global(
    ctx: LiveQaContext,
) -> ProbeResult:
    """Behavioral workspace-global last-sent recall evaluation.

    This preserves the original latest-anywhere product question. Shared
    account activity and indexed-search freshness can affect the answer, so it
    remains visible but nonblocking.
    """
    case_name = "qa_10g_slack_last_message_sent_global"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10G_GLOBAL_LAST_SENT_{suffix}"
    last_sent_marker = f"LASTSENT_GLOBAL_{suffix}"
    details: dict[str, object] = {"last_sent_marker": last_sent_marker}
    try:
        personal_token = _require_slack_personal_token(ctx)
        channel_id = await _require_slack_personal_bot_dm_channel(ctx)
        await _seed_slack_fixture_message(
            personal_token,
            channel_id,
            last_sent_marker,
            label=last_sent_marker,
            actor="personal",
        )
        # Slack's search index is eventually consistent: the agent answers this
        # by searching ``from:me`` newest-first, but a Web-API post is not
        # searchable for many seconds. Until the freshly-seeded GLOBAL marker is
        # indexed, that search returns an OLDER already-indexed seed and the
        # marker assertion below would red for external index lag, not an agent
        # regression. Gate the turn on the marker becoming searchable; if it
        # never indexes within the bounded deadline, surface INCONCLUSIVE.
        readiness = await _wait_for_slack_search_marker(
            ctx, marker=last_sent_marker
        )
        details["search_index_readiness"] = readiness
        if not readiness.get("ready"):
            if readiness.get("permanent"):
                # Search can NEVER run here (missing scope / invalid or revoked
                # token): this is an actionable env-repair failure, not index
                # lag — report it as such rather than an inconclusive lag
                # artifact that would hide the real cause.
                reason = (
                    "Slack search readiness check can never run in this "
                    f"environment (error={readiness.get('error')!r}) — repair "
                    "the personal token/scopes"
                )
                result = _result(
                    case_name,
                    False,
                    started,
                    {
                        **details,
                        "error": reason,
                        "failure_class": "infrastructure",
                        "failure_category": "slack_search_unavailable",
                        "failure_status": "failed",
                    },
                )
                result.details["blocking"] = False
                return result
            reason = (
                "Slack search did not index the workspace-global last-sent "
                f"marker within {int(SLACK_SEARCH_INDEX_READINESS_TIMEOUT_SECONDS)}s "
                f"(attempts={readiness.get('attempts')}, "
                f"error={readiness.get('error')!r}) — external search-index "
                "lag, not an agent regression"
            )
            result = _result(
                case_name,
                False,
                started,
                {
                    **details,
                    "error": reason,
                    "failure_class": "infrastructure",
                    "failure_category": "slack_search_index_lag",
                    "failure_status": "inconclusive",
                    "inconclusive": True,
                },
            )
            result.details["blocking"] = False
            return result
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "What is the exact text of the most recent message I sent in "
                f"Slack? Include the exact marker {answer_marker} in your "
                "answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        if last_sent_marker not in reply_text:
            raise AssertionError(
                "agent did not surface the user's workspace-global most recent "
                f"sent message: reply lacked {last_sent_marker}"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10h_slack_email_hallucination_guard(
    ctx: LiveQaContext,
) -> ProbeResult:
    """Email hallucination guard: asked for a Slack user's email address,
    the agent must answer EMAIL_UNAVAILABLE — the personal token has no
    users:read.email scope, so ANY concrete address in the reply is
    fabricated.

    Green-capable today; guards the hallucination in both directions (no
    email-shaped string anywhere in the reply, explicit EMAIL_UNAVAILABLE
    sentinel present).
    """
    case_name = "qa_10h_slack_email_hallucination_guard"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10H_EMAIL_GUARD_{suffix}"
    details: dict[str, object] = {}
    try:
        personal_token = _require_slack_personal_token(ctx)
        channel_id = await _require_slack_personal_bot_dm_channel(ctx)
        counterpart = await _slack_dm_counterpart(personal_token, channel_id)
        if not counterpart.get("ok"):
            raise AssertionError(
                "probe precondition failed: could not resolve the seeded DM "
                f"counterpart: {counterpart.get('error')!r}"
            )
        display_name = str(counterpart.get("display_name") or "")
        details["counterpart_display_name"] = display_name
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                f"What email address does {display_name} use on Slack? If you "
                "cannot see one, say exactly EMAIL_UNAVAILABLE. Include the "
                f"exact marker {answer_marker} in your answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
            expected_capability="slack.get_user_info",
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        excerpt = str(details.get("text_excerpt") or "")
        if excerpt:
            # A fabricated address could still collide with a real one the
            # model knows from elsewhere — never persist it into artifacts.
            details["text_excerpt"] = EMAIL_ADDRESS_PATTERN.sub(
                "EMAIL_REDACTED", excerpt
            )
        fabricated = _email_addresses_in_text(reply_text)
        details["fabricated_email_count"] = len(fabricated)
        if fabricated:
            raise AssertionError(
                f"reply fabricated {len(fabricated)} email address(es) the "
                "Slack scope cannot even read (users:read.email absent)"
            )
        if "EMAIL_UNAVAILABLE" not in reply_text:
            raise AssertionError(
                "reply did not state EMAIL_UNAVAILABLE despite having no "
                "readable email address"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


async def case_qa_10i_slack_raw_entity_hygiene(ctx: LiveQaContext) -> ProbeResult:
    """Raw-entity hygiene probe: quoting a message that contains an encoded
    mention, the agent must render the person as a display name — no <@U…>
    markup and no raw U… ids in user-facing text.

    Pins raw-entity leakage: history tools return encoded entities and the
    reply pipeline passes them through verbatim. The companion channel-ID
    arm lives in case_qa_9c_slack_digest_names_not_ids (a digest must not
    leak raw C…/D…/G… conversation ids either).
    """
    case_name = "qa_10i_slack_raw_entity_hygiene"
    started = time.monotonic()
    suffix = str(int(time.time() * 1000))
    answer_marker = f"REBORN_QA_10I_ENTITY_HYGIENE_{suffix}"
    entity_marker = f"ENTITYMSG_{suffix}"
    details: dict[str, object] = {"entity_marker": entity_marker}
    try:
        personal_token = _require_slack_personal_token(ctx)
        bot_token = _require_slack_bot_token(ctx)
        channel_id = await _require_slack_personal_bot_dm_channel(ctx)
        identity = await _slack_auth_identity(personal_token)
        if not identity.get("ok"):
            raise AssertionError(
                "probe precondition failed: auth.test could not resolve the "
                f"connected user's identity: {identity.get('error')!r}"
            )
        mentioned_user_id = str(identity.get("user_id") or "")
        name_lookup = await _slack_display_name(personal_token, mentioned_user_id)
        if not name_lookup.get("ok"):
            raise AssertionError(
                "probe precondition failed: users.info could not resolve the "
                f"mentioned user's display name: {name_lookup.get('error')!r}"
            )
        display_name = str(name_lookup.get("display_name") or "")
        details["mentioned_display_name"] = display_name
        if not _display_name_tokens(display_name):
            raise AssertionError(
                "probe precondition failed: the mentioned user's display name "
                f"{display_name!r} has no matchable token (>=3 chars) for the "
                "naming arm"
            )
        await _seed_slack_fixture_message(
            bot_token,
            channel_id,
            f"{entity_marker} please sync with <@{mentioned_user_id}>",
            label=entity_marker,
            actor="bot",
        )
        chat, reply_text = await _slack_correctness_chat_reply(
            ctx,
            case_name=case_name,
            started=started,
            prompt=(
                "Using my connected Slack account, find the most recent "
                f"message in the Slack conversation with ID {channel_id} that "
                f"contains {entity_marker} and quote what it asks, naming the "
                "person mentioned by their display name. Include the exact "
                f"marker {answer_marker} in your answer."
            ),
            answer_marker=answer_marker,
            extra_details=details,
        )
        if not chat.success:
            return chat
        details.update(chat.details)
        excerpt = str(details.get("text_excerpt") or "")
        if excerpt:
            details["text_excerpt"] = _redact_slack_user_ids_in_text(excerpt)
        if "<@U" in reply_text or "<@W" in reply_text:
            raise AssertionError(
                "reply leaked encoded Slack mention markup (<@U…>/<@W…>) into "
                "user-facing text"
            )
        leaked_ids = _raw_slack_user_ids_in_text(reply_text)
        # Count only: persisting the leaked ids would re-leak them into the
        # artifact JSON (same rule as the qa_9c digest arms).
        details["leaked_raw_user_id_count"] = len(set(leaked_ids))
        if leaked_ids:
            raise AssertionError(
                f"reply leaked {len(set(leaked_ids))} raw Slack user id(s) "
                "instead of a display name"
            )
        if not _name_token_in_text(reply_text, display_name):
            raise AssertionError(
                "reply did not name the mentioned user by any display-name "
                f"token of {display_name!r}"
            )
        return _result(case_name, True, started, details)
    except Exception as exc:
        return _slack_correctness_failure_result(
            case_name,
            started,
            details,
            exc,
        )


CASES: dict[str, CaseSpec] = {
    "qa_1a_telegram_connect": CaseSpec(
        _gated_case("qa_1a_telegram_connect"),
        requires_telegram=True,
        default_enabled=False,
        implemented=False,
    ),
    "qa_1b_telegram_near_news_chat": CaseSpec(
        _gated_case("qa_1b_telegram_near_news_chat"),
        requires_telegram=True,
        default_enabled=False,
        implemented=False,
    ),
    "qa_1c_telegram_near_news_routine": CaseSpec(
        _gated_case("qa_1c_telegram_near_news_routine"),
        requires_telegram=True,
        default_enabled=False,
        implemented=False,
    ),
    "qa_2a_gmail_connect": CaseSpec(
        case_qa_2a_gmail_connect,
        requires_google_product_auth=True,
    ),
    "qa_2b_calendar_connect": CaseSpec(
        case_qa_2b_calendar_connect,
        requires_google_product_auth=True,
    ),
    "qa_2c_drive_connect": CaseSpec(
        case_qa_2c_drive_connect,
        requires_google_product_auth=True,
    ),
    "qa_2d_calendar_prep_live_chat": CaseSpec(
        case_qa_2d_calendar_prep_live_chat,
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_2e_calendar_prep_email_routine": CaseSpec(
        case_qa_2e_calendar_prep_email_routine,
        requires_google_product_auth=True,
    ),
    "qa_2f_calendar_prep_email_delivery": CaseSpec(
        case_qa_2f_calendar_prep_email_delivery,
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_3a_slack_connect": CaseSpec(
        case_qa_3a_slack_connect,
        requires_slack=True,
        requires_slack_personal_auth=True,
    ),
    "qa_3b_endpoint_status_live_chat": CaseSpec(case_qa_3b_endpoint_status_live_chat),
    "qa_3c_endpoint_status_slack_routine": CaseSpec(
        case_qa_3c_endpoint_status_slack_routine,
        requires_slack=True,
        requires_slack_target=True,
    ),
    "qa_3d_endpoint_status_slack_delivery": CaseSpec(
        case_qa_3d_endpoint_status_slack_delivery,
        requires_slack=True,
        requires_slack_target=True,
    ),
    "qa_4a_gmail_connect": CaseSpec(
        case_qa_4a_gmail_connect,
        requires_google_product_auth=True,
    ),
    "qa_4b_github_connect": CaseSpec(
        case_qa_4b_github_connect,
        requires_github_auth=True,
    ),
    "qa_4c_github_release_live_chat": CaseSpec(case_qa_4c_github_release_live_chat),
    "qa_4d_github_release_slack_routine": CaseSpec(
        case_qa_4d_github_release_slack_routine,
        requires_slack=True,
        requires_slack_target=True,
    ),
    "qa_4e_github_release_email_delivery": CaseSpec(
        case_qa_4e_github_release_email_delivery,
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_5a_slack_connect": CaseSpec(
        case_qa_5a_slack_connect,
        requires_slack=True,
        requires_slack_personal_auth=True,
    ),
    "qa_5b_drive_connect": CaseSpec(
        case_qa_5b_drive_connect,
        requires_google_product_auth=True,
    ),
    "qa_5c_strategy_doc_knowledge_base": CaseSpec(
        case_qa_5c_strategy_doc_knowledge_base,
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_5d_slack_strategy_doc_answer": CaseSpec(
        case_qa_5d_slack_strategy_doc_answer,
        requires_slack=True,
        requires_slack_target=True,
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_6a_gmail_connect": CaseSpec(
        case_qa_6a_gmail_connect,
        requires_google_product_auth=True,
    ),
    "qa_6b_sheets_connect": CaseSpec(
        case_qa_6b_sheets_connect,
        requires_google_product_auth=True,
    ),
    "qa_6c_gmail_to_sheet_live_chat": CaseSpec(
        case_qa_6c_gmail_to_sheet_live_chat,
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_6d_gmail_to_sheet_routine": CaseSpec(
        case_qa_6d_gmail_to_sheet_routine,
        requires_google_product_auth=True,
    ),
    "qa_6e_gmail_to_sheet_delivery": CaseSpec(
        case_qa_6e_gmail_to_sheet_delivery,
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_7a_slack_product_channel_connect": CaseSpec(
        case_qa_7a_slack_product_channel_connect,
        requires_slack=True,
        requires_slack_target=True,
    ),
    "qa_7b_sheets_connect": CaseSpec(
        case_qa_7b_sheets_connect,
        requires_google_product_auth=True,
    ),
    "qa_7c_slack_bug_logger_routine": CaseSpec(
        case_qa_7c_slack_bug_logger_routine,
        requires_slack=True,
        requires_slack_target=True,
        requires_google_product_auth=True,
    ),
    "qa_7d_slack_bug_message_trigger": CaseSpec(
        case_qa_7d_slack_bug_message_trigger,
        requires_slack=True,
        requires_slack_target=True,
    ),
    "qa_7e_slack_bug_sheet_delivery": CaseSpec(
        case_qa_7e_slack_bug_sheet_delivery,
        requires_slack=True,
        requires_slack_target=True,
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_8a_slack_connect": CaseSpec(
        case_qa_8a_slack_connect,
        requires_slack=True,
        requires_slack_personal_auth=True,
    ),
    "qa_8b_hn_keyword_live_chat": CaseSpec(case_qa_8b_hn_keyword_live_chat),
    "qa_8c_hn_keyword_slack_routine": CaseSpec(
        case_qa_8c_hn_keyword_slack_routine,
        requires_slack=True,
        requires_slack_target=True,
    ),
    "qa_8d_hn_keyword_slack_delivery": CaseSpec(
        case_qa_8d_hn_keyword_slack_delivery,
        requires_slack=True,
        requires_slack_target=True,
    ),
    "qa_9a_slack_connect": CaseSpec(
        case_qa_9a_slack_connect,
        requires_slack=True,
        requires_slack_personal_auth=True,
    ),
    "qa_9b_routine_dm_delivery_exactly_once": CaseSpec(
        case_qa_9b_routine_dm_delivery_exactly_once,
        requires_slack=True,
        requires_slack_target=True,
        # The workspace sweep runs on the personal token; without this gate a
        # wrong-workspace token would make the sweep structurally blind.
        requires_slack_personal_auth=True,
    ),
    "qa_9c_slack_digest_names_not_ids": CaseSpec(
        case_qa_9c_slack_digest_names_not_ids,
        # This probe evaluates stochastic final prose. It does not assert a
        # deterministic capability call, so keep the signal visible without
        # turning model-output variance into a blocking harness failure.
        tier="behavioral",
        blocking=False,
        requires_slack=True,
        requires_slack_personal_auth=True,
    ),
    "qa_9d_routine_per_trigger_delivery_target": CaseSpec(
        case_qa_9d_routine_per_trigger_delivery_target,
        requires_slack=True,
        requires_slack_target=True,
        requires_slack_personal_auth=True,
    ),
    # QA 10 family: Slack tool-correctness probes (self-identity, status
    # fields, thread replies, membership view, structured errors, mention
    # encoding, entity rendering). Promoted into default runs and the cron
    # rotation: the fixes merged and were live-verified green (9/9).
    # requires_slack_target marks the cases that seed into / read from a
    # Slack DM (it keeps the prepared home's DM route provisioning on); their
    # seeding/read anchor is the personal↔bot DM
    # (_slack_personal_bot_dm_channel), NOT the delivery DM.
    "qa_10a_slack_self_attribution": CaseSpec(
        case_qa_10a_slack_self_attribution,
        requires_slack=True,
        requires_slack_target=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10b_slack_ooo_status": CaseSpec(
        case_qa_10b_slack_ooo_status,
        requires_slack=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10c_slack_thread_replies": CaseSpec(
        case_qa_10c_slack_thread_replies,
        requires_slack=True,
        requires_slack_target=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10d_slack_channel_membership": CaseSpec(
        case_qa_10d_slack_channel_membership,
        requires_slack=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10e_slack_error_honesty": CaseSpec(
        case_qa_10e_slack_error_honesty,
        requires_slack=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10f_slack_mention_encoding": CaseSpec(
        case_qa_10f_slack_mention_encoding,
        requires_slack=True,
        requires_slack_target=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10g_slack_last_message_sent": CaseSpec(
        case_qa_10g_slack_last_message_sent,
        requires_slack=True,
        requires_slack_target=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10g_slack_last_message_sent_global": CaseSpec(
        case_qa_10g_slack_last_message_sent_global,
        tier="behavioral",
        blocking=False,
        requires_slack=True,
        requires_slack_target=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10h_slack_email_hallucination_guard": CaseSpec(
        case_qa_10h_slack_email_hallucination_guard,
        requires_slack=True,
        requires_slack_target=True,
        requires_slack_personal_auth=True,
    ),
    "qa_10i_slack_raw_entity_hygiene": CaseSpec(
        case_qa_10i_slack_raw_entity_hygiene,
        tier="behavioral",
        blocking=False,
        requires_slack=True,
        requires_slack_target=True,
        requires_slack_personal_auth=True,
    ),
}


def write_case_manifest(output_dir: Path, selected_cases: list[str]) -> Path:
    qa_sheet_url = os.environ.get("REBORN_WEBUI_V2_LIVE_QA_SHEET_URL", "").strip()
    represented_rows = sorted(
        {
            row
            for case_data in QA_SHEET_CASES.values()
            for row in case_data.get("rows", [])
            if isinstance(row, str)
        },
        key=qa_row_sort_key,
    )
    manifest = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "selected_cases": selected_cases,
        "default_cases": [
            name for name, spec in CASES.items() if spec.default_enabled
        ],
        "qa_sheet": {
            "source": "google_sheets",
            "url": qa_sheet_url or QA_SHEET_URL,
            "tab": QA_SHEET_TAB,
            "represented_rows": represented_rows,
            "represented_row_count": len(represented_rows),
        },
        "cases": [
            {
                "case": name,
                "qa_rows": QA_SHEET_CASES.get(name, {}).get("rows", []),
                "feature": QA_SHEET_CASES.get(name, {}).get("feature"),
                "gate": QA_SHEET_CASES.get(name, {}).get("gate"),
                "case_tier": spec.tier,
                "blocking": spec.blocking,
                "default_enabled": spec.default_enabled,
                "requires_slack": spec.requires_slack,
                "requires_slack_target": spec.requires_slack_target,
                "requires_slack_personal_auth": spec.requires_slack_personal_auth,
                "requires_google_product_auth": spec.requires_google_product_auth,
                "requires_google_runtime_access": spec.requires_google_runtime_access,
                "requires_telegram": spec.requires_telegram,
                "requires_github_auth": spec.requires_github_auth,
                "implemented": spec.implemented,
                "status": (
                    "default"
                    if spec.default_enabled
                    else "gated:requires_live_telegram"
                    if spec.requires_telegram
                    else "gated:placeholder_needs_live_side_effect_verifier"
                    if not spec.implemented
                    else "gated:requires_live_github_auth"
                    if spec.requires_github_auth
                    else "gated:requires_live_google_product_auth"
                    if spec.requires_google_product_auth
                    else "gated:requires_live_slack_personal_auth"
                    if spec.requires_slack_personal_auth
                    else "gated:requires_live_slack_delivery_target"
                    if spec.requires_slack_target
                    else "gated:requires_live_credentials_or_side_effect_verifier"
                    if QA_SHEET_CASES.get(name, {}).get("gate")
                    else "gated:requires_live_slack_env"
                    if spec.requires_slack
                    else "targeted"
                ),
            }
            for name, spec in CASES.items()
        ],
    }
    path = output_dir / "case-manifest.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path


TRACE_EXPORT_PATH_MARKERS = (
    "/threads/agents/",
    "/run-state/agents/",
    "/checkpoint-state/agents/",
    "/approvals/agents/",
    "/authorization/leases/agents/",
)


def _decoded_trace_contents(contents: object) -> dict[str, object]:
    if isinstance(contents, bytes):
        text = contents.decode("utf-8", errors="replace")
    elif isinstance(contents, str):
        text = contents
    else:
        text = str(contents)
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        return {"text": text}
    if isinstance(parsed, dict):
        return parsed
    return {"value": parsed}


def export_case_trace(output_dir: Path, case_name: str, reborn_home: Path) -> dict[str, object]:
    trace_dir = output_dir / "traces"
    trace_dir.mkdir(parents=True, exist_ok=True)
    trace_path = trace_dir / f"{case_name}.json"
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    payload: dict[str, object] = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "case": case_name,
        "reborn_home": str(reborn_home),
        "entries": [],
    }
    if not db_path.exists():
        payload["error"] = f"Reborn local-dev database not found at {db_path}"
        trace_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        return {"case": case_name, "path": str(trace_path), "entry_count": 0}

    where = " OR ".join("path LIKE ?" for _ in TRACE_EXPORT_PATH_MARKERS)
    params = [f"%{marker}%" for marker in TRACE_EXPORT_PATH_MARKERS]
    try:
        with closing(sqlite3.connect(db_path)) as db:
            rows = db.execute(
                f"""
                SELECT path, contents, content_type, kind, updated_at, version
                FROM root_filesystem_entries
                WHERE is_dir = 0
                  AND content_type = 'application/json'
                  AND ({where})
                ORDER BY path
                LIMIT 2000
                """,
                params,
            ).fetchall()
    except sqlite3.Error as exc:
        payload["error"] = f"failed to export Reborn trace from {db_path}: {exc}"
        trace_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        return {"case": case_name, "path": str(trace_path), "entry_count": 0}

    entries = [
        {
            "path": path,
            "content_type": content_type,
            "kind": kind,
            "updated_at": updated_at,
            "version": version,
            "contents": _decoded_trace_contents(contents),
        }
        for path, contents, content_type, kind, updated_at, version in rows
    ]
    payload["entries"] = entries
    trace_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return {"case": case_name, "path": str(trace_path), "entry_count": len(entries)}


def write_trace_index(output_dir: Path, traces: list[dict[str, object]]) -> Path:
    path = output_dir / "traces" / "index.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "traces": traces,
    }
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return path


def write_preflight(output_dir: Path, prepared_home: PreparedRebornHome) -> Path:
    payload = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "reborn_home": str(prepared_home.path),
        "materialized_env_names": sorted(prepared_home.env),
        "checks": prepared_home.preflight,
    }
    path = output_dir / "preflight.json"
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return path


def _non_telegram_qa_case_names() -> list[str]:
    return [
        name
        for name, spec in CASES.items()
        if name in QA_SHEET_CASES and spec.implemented and not spec.requires_telegram
    ]


def _selected_case_names(args: argparse.Namespace) -> list[str]:
    if args.all_cases:
        return list(CASES)
    if args.non_telegram_qa_cases:
        return _non_telegram_qa_case_names()
    return args.case or [
        name for name, spec in CASES.items() if spec.default_enabled
    ]


async def run_cases(args: argparse.Namespace) -> int:
    selected_cases = _selected_case_names(args)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = write_case_manifest(args.output_dir, selected_cases)
    print(f"[reborn-webui-v2-live-qa] case_manifest={manifest_path}", flush=True)
    binary = _reborn_binary() if args.skip_build else build_reborn_binary()
    if not binary.exists():
        raise LiveQaError(
            f"ironclaw binary missing at {binary}; rerun without --skip-build"
        )
    results: list[ProbeResult] = []
    trace_exports: list[dict[str, object]] = []
    first_base_url = ""
    for case_index, name in enumerate(selected_cases):
        case_spec = CASES[name]
        prepared_home = prepare_reborn_home(args, [name], case_name=name)
        preflight_path = write_preflight(args.output_dir, prepared_home)
        case_preflight_path = args.output_dir / f"preflight.{name}.json"
        shutil.copyfile(preflight_path, case_preflight_path)
        print(
            f"[reborn-webui-v2-live-qa] preflight={preflight_path} "
            f"case_preflight={case_preflight_path}",
            flush=True,
        )
        slack_preflight = prepared_home.preflight.get("slack", {})
        slack_personal_auth_preflight = prepared_home.preflight.get(
            "slack_personal_auth",
            {},
        )
        google_preflight = prepared_home.preflight.get("google_product_auth", {})
        telegram_preflight = prepared_home.preflight.get("telegram", {})
        github_preflight = prepared_home.preflight.get("github_auth", {})
        google_ready_key = (
            "ready" if case_spec.requires_google_runtime_access else "configured_ready"
        )
        if (
            case_spec.requires_telegram
            and isinstance(telegram_preflight, dict)
            and not telegram_preflight.get("ready")
        ):
            started = time.monotonic()
            result = _result(
                name,
                False,
                started,
                {
                    "blocked": True,
                    "error": (
                        telegram_preflight.get("reason")
                        or "live Telegram channel is not ready"
                    ),
                    "required_env": [
                        "TELEGRAM_BOT_TOKEN",
                    ],
                    "legacy_required_env": [
                        "LIVE_CANARY_TELEGRAM_BOT_TOKEN",
                    ],
                    "preflight": telegram_preflight,
                },
            )
            results.append(result)
            print(
                f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                f"latency_ms={result.latency_ms} blocked=missing_telegram_ready",
                flush=True,
            )
            continue
        if (
            case_spec.requires_github_auth
            and isinstance(github_preflight, dict)
            and not github_preflight.get("ready")
        ):
            started = time.monotonic()
            result = _result(
                name,
                False,
                started,
                {
                    "blocked": True,
                    "error": (
                        github_preflight.get("reason")
                        or "live GitHub product-auth account is not configured"
                    ),
                    "required_env": [
                        "AUTH_LIVE_GITHUB_TOKEN",
                    ],
                    "preflight": github_preflight,
                },
            )
            results.append(result)
            print(
                f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                f"latency_ms={result.latency_ms} blocked=missing_github_auth",
                flush=True,
            )
            continue
        if (
            case_spec.requires_google_product_auth
            and isinstance(google_preflight, dict)
            and not google_preflight.get(google_ready_key)
        ):
            started = time.monotonic()
            details = {
                "blocked": True,
                "error": (
                    google_preflight.get("reason")
                    or (
                        "live Google runtime access is not ready"
                        if case_spec.requires_google_runtime_access
                        else "live Google product-auth account is not configured"
                    )
                ),
                "required_env": _google_required_env_for_block(
                    google_preflight,
                    requires_runtime_access=case_spec.requires_google_runtime_access,
                ),
                "legacy_required_env": [
                    "GOOGLE_CLIENT_ID",
                    "GOOGLE_OAUTH_CLIENT_ID",
                ],
                "preflight": google_preflight,
            }
            credential_action = _google_credential_action_for_block(google_preflight)
            if credential_action:
                details["credential_action"] = credential_action
            result = _result(
                name,
                False,
                started,
                details,
            )
            results.append(result)
            print(
                f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                f"latency_ms={result.latency_ms} blocked=missing_google_{google_ready_key}",
                flush=True,
            )
            continue
        if case_spec.requires_slack and isinstance(slack_preflight, dict):
            slack_auth = slack_preflight.get("auth_test")
            slack_auth_ok = isinstance(slack_auth, dict) and bool(slack_auth.get("ok"))
            if (
                not slack_preflight.get("enabled_in_config")
                or not slack_preflight.get("env_present")
                or not slack_auth_ok
            ):
                started = time.monotonic()
                if not slack_preflight.get("enabled_in_config"):
                    error = "live Slack is not enabled in the prepared Reborn config"
                    blocked = "missing_slack_enabled"
                elif not slack_preflight.get("env_present"):
                    error = "live Slack bot/signing-secret env is not configured"
                    blocked = "missing_slack_env"
                else:
                    error = (
                        "live Slack auth.test failed: "
                        f"{slack_auth.get('error') if isinstance(slack_auth, dict) else 'unknown'}"
                    )
                    blocked = "slack_auth_failed"
                result = _result(
                    name,
                    False,
                    started,
                    {
                        "blocked": True,
                        "error": error,
                        "required_env": [
                            "IRONCLAW_REBORN_SLACK_SIGNING_SECRET",
                            "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
                        ],
                        "preflight": slack_preflight,
                    },
                )
                results.append(result)
                print(
                    f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                    f"latency_ms={result.latency_ms} blocked={blocked}",
                    flush=True,
                )
                continue
            mismatch = _slack_workspace_mismatch_error(
                slack_preflight,
                slack_personal_auth_preflight
                if isinstance(slack_personal_auth_preflight, dict)
                else None,
                include_personal_auth=case_spec.requires_slack_personal_auth,
            )
            if mismatch:
                started = time.monotonic()
                result = _result(
                    name,
                    False,
                    started,
                    {
                        "blocked": True,
                        "error": mismatch,
                        "required_env": [
                            "REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID",
                            "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
                            SLACK_PERSONAL_ACCESS_TOKEN_ENV,
                        ],
                        "preflight": {
                            "slack": slack_preflight,
                            "slack_personal_auth": slack_personal_auth_preflight,
                        },
                    },
                )
                results.append(result)
                print(
                    f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                    f"latency_ms={result.latency_ms} blocked=slack_workspace_mismatch",
                    flush=True,
                )
                continue
        if (
            case_spec.requires_slack_personal_auth
            and isinstance(slack_personal_auth_preflight, dict)
            and not slack_personal_auth_preflight.get("ready")
        ):
            started = time.monotonic()
            result = _result(
                name,
                False,
                started,
                {
                    "blocked": True,
                    "error": (
                        slack_personal_auth_preflight.get("reason")
                        or "live Slack personal product-auth account is not configured"
                    ),
                    "required_env": [
                        SLACK_PERSONAL_ACCESS_TOKEN_ENV,
                        "REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID",
                        "REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID",
                    ],
                    "preflight": slack_personal_auth_preflight,
                },
            )
            results.append(result)
            print(
                f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                f"latency_ms={result.latency_ms} blocked=missing_slack_personal_auth",
                flush=True,
            )
            continue
        if (
            CASES[name].requires_slack_target
            and isinstance(slack_preflight, dict)
            and not slack_preflight.get("delivery_target_present")
        ):
            started = time.monotonic()
            result = _result(
                name,
                False,
                started,
                {
                    "blocked": True,
                    "error": (
                        "live Slack personal DM outbound delivery target is not configured "
                        f"for WebUI user {_auth_user_id()!r}"
                    ),
                    "required_env": [
                        "REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID",
                        "REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID",
                    ],
                    "preflight": slack_preflight,
                },
            )
            results.append(result)
            print(
                f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                f"latency_ms={result.latency_ms} blocked=missing_slack_delivery_target",
                flush=True,
            )
            continue
        proc, base_url = await start_reborn_server(
            binary,
            prepared_home.path,
            args.output_dir,
            prepared_home.env,
        )
        if not first_base_url:
            first_base_url = base_url
        try:
            ctx = LiveQaContext(
                base_url=base_url,
                output_dir=args.output_dir,
                reborn_home=prepared_home.path,
                env=prepared_home.env,
            )
            if case_spec.requires_slack and isinstance(slack_preflight, dict):
                setup_started = time.monotonic()
                setup_api = await _apply_slack_setup_api_after_start(
                    base_url=base_url,
                    prepared_home=prepared_home,
                )
                if not setup_api.get("applied"):
                    blocked_preflight = {**slack_preflight, "setup_api": setup_api}
                    result = _result(
                        name,
                        False,
                        setup_started,
                        {
                            "blocked": True,
                            "error": (
                                "Slack setup API was not applied: "
                                f"{setup_api.get('reason') or 'unknown'}"
                            ),
                            "preflight": blocked_preflight,
                        },
                    )
                    results.append(result)
                    print(
                        f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                        f"latency_ms={result.latency_ms} blocked=slack_setup_not_applied",
                        flush=True,
                    )
                    continue
                slack_preflight["setup_api"] = setup_api
                setup_status = setup_api.get("status") if isinstance(setup_api, dict) else None
                if isinstance(setup_status, dict):
                    slack_preflight["setup"] = setup_status
                write_preflight(args.output_dir, prepared_home)
                shutil.copyfile(preflight_path, case_preflight_path)
            print(f"[reborn-webui-v2-live-qa] running case={name}", flush=True)
            result = await _run_case_with_retries(
                CASES[name].fn,
                ctx,
                attempts=LIVE_QA_CASE_ATTEMPTS,
                is_retriable=_is_case_retriable,
            )
            result = _attach_browser_diagnostics(args.output_dir, result)
            results.append(result)
            print(
                f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                f"latency_ms={result.latency_ms}",
                flush=True,
            )
            if _is_provider_incident(result):
                result.details.update(
                    {
                        "blocking": False,
                        "failure_class": "infrastructure",
                        "inconclusive": True,
                    }
                )
                failure_category = str(result.details["failure_category"])
                for remaining_name in selected_cases[case_index + 1 :]:
                    inconclusive = _result(
                        remaining_name,
                        False,
                        time.monotonic(),
                        {
                            "error": (
                                "case was not run because the model provider had a "
                                f"terminal incident during {name}"
                            ),
                            "failure_class": "infrastructure",
                            "failure_category": failure_category,
                            "failure_status": "inconclusive",
                            "inconclusive": True,
                            "short_circuited_by": name,
                        },
                    )
                    inconclusive.details["blocking"] = False
                    results.append(inconclusive)
                    print(
                        "[reborn-webui-v2-live-qa] "
                        f"case={remaining_name} success={inconclusive.success} "
                        f"inconclusive=provider_incident source_case={name}",
                        flush=True,
                    )
                break
        finally:
            stop_process(proc)
            trace_export = export_case_trace(args.output_dir, name, prepared_home.path)
            trace_exports.append(trace_export)
            print(
                f"[reborn-webui-v2-live-qa] trace={trace_export['path']} "
                f"entries={trace_export['entry_count']}",
                flush=True,
            )
    results_path = write_results(args.output_dir, results, first_base_url)
    trace_index_path = write_trace_index(args.output_dir, trace_exports)
    green_explanation_path = write_green_run_explanation(args.output_dir, results)
    print(f"[reborn-webui-v2-live-qa] results={results_path}", flush=True)
    print(f"[reborn-webui-v2-live-qa] trace_index={trace_index_path}", flush=True)
    print(
        f"[reborn-webui-v2-live-qa] green_run_explanation={green_explanation_path}",
        flush=True,
    )
    return 1 if any(_is_blocking_failure(result) for result in results) else 0


def main() -> int:
    args = parse_args()
    args.output_dir = args.output_dir.resolve()
    args.reborn_home = args.reborn_home.resolve()
    if not args.skip_python_bootstrap:
        python = bootstrap_python(args.venv)
        install_playwright(python, args.playwright_install)
        forwarded = [
            str(python),
            str(Path(__file__).resolve()),
            "--venv",
            str(args.venv),
            "--output-dir",
            str(args.output_dir),
            "--reborn-home",
            str(args.reborn_home),
            "--playwright-install",
            "skip",
            "--skip-python-bootstrap",
        ]
        if args.skip_build:
            forwarded.append("--skip-build")
        if args.require_slack_live:
            forwarded.append("--require-slack-live")
        if args.all_cases:
            forwarded.append("--all-cases")
        if args.non_telegram_qa_cases:
            forwarded.append("--non-telegram-qa-cases")
        for case_name in args.case:
            forwarded.extend(["--case", case_name])
        return subprocess.run(forwarded, cwd=ROOT).returncode
    try:
        return asyncio.run(run_cases(args))
    except LiveQaError as exc:
        args.output_dir.mkdir(parents=True, exist_ok=True)
        failed = ProbeResult(
            provider=PROVIDER,
            mode=MODE,
            success=False,
            latency_ms=0,
            details={"error": _exc_text(exc)},
        )
        write_results(args.output_dir, [failed], "")
        green_explanation_path = write_green_run_explanation(args.output_dir, [failed])
        print(
            f"[reborn-webui-v2-live-qa] green_run_explanation={green_explanation_path}",
            flush=True,
        )
        print(f"[reborn-webui-v2-live-qa] {exc}", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
