"""Live QA runner for Reborn WebUI v2.

This lane intentionally starts the standalone ``ironclaw-reborn serve`` binary
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
from scripts.reborn_webui_v2_live_qa.semantic_judge import (  # noqa: E402
    _compact_json,
    _judge_assistant_reply_completion,
    _semantic_judge_passed,
)
from scripts.reborn_webui_v2_live_qa.slack_helpers import (  # noqa: E402
    _append_slack_channel_route,
    _append_slack_channel_route_if_configured,
    _configure_slack_legacy_actor_if_needed,
    _disable_slack_in_config,
    _discover_slack_dm_route_channel,
    _has_live_slack_env,
    _has_slack_delivery_target,
    _materialize_slack_env_from_reborn_home,
    _set_slack_section_key,
    _slack_auth_test,
    _slack_config_value,
    _slack_enabled,
    _slack_team_id_from_bot_token_env,
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
    "qa_3c_endpoint_status_slack_routine": """In WebUI, ask IronClaw, "Every 5 minutes, ping [endpoint URL] checking if it returns a 200 status and send result in a DM in slack"
Expected result: Routine created""",
    "qa_4a_gmail_connect": """In WebUI, ask IronClaw "connect to Gmail." Go through the flow w/ Gmail.
Expected result: Gmail is connected""",
    "qa_4b_github_connect": """In WebUI, ask IronClaw "connect to GitHub." Go through the auth flow.
Expected result: GitHub is connected""",
    "qa_4c_github_release_live_chat": """In WebUI, ask IronClaw "summarize the latest release from https://github.com/nearai/ironclaw."
Expected result: summary of the most recent release""",
    "qa_4d_github_release_slack_routine": """In WebUI, ask IronClaw, "Every 5 minutes, check https://github.com/nearai/ironclaw for latest releases and send me a Slack message summarizing any new ones."
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
    "qa_7a_slack_product_channel_connect": """In WebUI, ask IronClaw "connect to Slack, using channel #product." Go through the flow
Expected result: Slack channel is connected""",
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
    "qa_8c_hn_keyword_slack_routine": """In WebUI, ask IronClaw, "Every hour, check Hacker News for new posts mentioning 'IronClaw' or 'NEAR AI' and send a summary to Slack."
Expected result: Routine created""",
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
HN_KEYWORD_SEARCH_URL = (
    "https://hn.algolia.com/api/v1/search_by_date"
    "?query=NEAR%20AI&tags=story&hitsPerPage=1"
)
EXTENSION_SEARCH_CAPABILITY_ID = "builtin.extension_search"
EXTENSION_INSTALL_CAPABILITY_ID = "builtin.extension_install"
EXTENSION_ACTIVATE_CAPABILITY_ID = "builtin.extension_activate"
OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID = "builtin.outbound_delivery_targets_list"
QA_7A_CHAT_CONNECT_CAPABILITY_IDS = [
    EXTENSION_SEARCH_CAPABILITY_ID,
    EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_ACTIVATE_CAPABILITY_ID,
]
QA_7C_BUG_LOGGING_SHEET_TITLE = "bug logging Google Sheet"


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
    return _cargo_target_dir() / "debug" / "ironclaw-reborn"


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
            "ironclaw-reborn",
        ],
        cwd=ROOT,
        env=build_env,
    )
    binary = _reborn_binary()
    if not binary.exists():
        raise LiveQaError(f"ironclaw-reborn binary was not produced at {binary}")
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
        slack_installation_id = _non_empty_env(
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID",
            "local-dev-installation",
        )
        slack_signing_secret_env = _non_empty_env(
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_SIGNING_SECRET_ENV",
            "IRONCLAW_REBORN_SLACK_SIGNING_SECRET",
        )
        slack_bot_token_env = _non_empty_env(
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_BOT_TOKEN_ENV",
            "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
        )
        slack_team_id = _non_empty_env(
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID",
            _slack_team_id_from_bot_token_env(slack_bot_token_env) or "local-dev-team",
        )
        slack_api_app_id = _non_empty_env(
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID",
            "local-dev-app-id",
        )
        slack_lines = [
            "[slack]",
            "enabled = true",
            f'installation_id = "{slack_installation_id}"',
            f'team_id = "{slack_team_id}"',
            f'api_app_id = "{slack_api_app_id}"',
            f'signing_secret_env = "{slack_signing_secret_env}"',
            f'bot_token_env = "{slack_bot_token_env}"',
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
    with sqlite3.connect(db_path) as db:
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
    route_configured_from_env = _append_slack_channel_route_if_configured(
        config_path,
        auth_user_id,
    )
    legacy_actor_configured, legacy_actor_user_id = _configure_slack_legacy_actor_if_needed(
        config_path,
        selected_cases,
    )
    config = _config_text(config_path)
    secret_env: dict[str, str] = {}
    secret_preflight: dict[str, object] = {"materialized": False}
    google_env, google_env_preflight = _materialize_google_oauth_env_for_reborn(
        prepared_home,
    )
    telegram_env, telegram_env_preflight = _materialize_telegram_env_for_reborn()

    if _slack_enabled(config) and not _has_live_slack_env(config):
        secret_env, secret_preflight = _materialize_slack_env_from_reborn_home(
            prepared_home,
            config,
        )
        if secret_preflight.get("materialized"):
            for key in ("installation_id", "team_id", "api_app_id"):
                value = str(secret_preflight.get(key) or "").strip()
                if value:
                    _set_slack_section_key(config_path, key, value)
            config = _config_text(config_path)
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
        and _has_live_slack_env(config, process_env)
        and not _has_slack_delivery_target(config, prepared_home, auth_user_id)
    ):
        slack_route_discovery = _discover_slack_dm_route_channel(config, process_env)
        channel_id = str(slack_route_discovery.get("channel_id") or "").strip()
        if channel_id:
            slack_route_discovery["configured_route"] = _append_slack_channel_route(
                config_path,
                subject_user_id=auth_user_id,
                channel_id=channel_id,
            )
            config = _config_text(config_path)

    missing = sorted(name for name in _referenced_env_names(config) if not _env_present(name, process_env))
    missing = [name for name in missing if not name.startswith("IRONCLAW_REBORN_SLACK_")]
    if missing:
        raise LiveQaError(
            "Reborn config references unset live env vars: " + ", ".join(missing)
        )

    slack_enabled = _slack_enabled(config)
    slack_target_present = _has_slack_delivery_target(config, prepared_home, auth_user_id)
    slack_auth = (
        _slack_auth_test(config, process_env)
        if slack_enabled and _has_live_slack_env(config, process_env)
        else {"checked": False, "ok": False, "error": "Slack env unavailable"}
    )
    if args.require_slack_live and needs_slack and not slack_enabled:
        raise LiveQaError(
            "selected cases require live Slack, but [slack].enabled is not true "
            "in the prepared Reborn config."
        )
    if slack_enabled and not _has_live_slack_env(config, process_env):
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
    return PreparedRebornHome(
        path=prepared_home,
        env=process_env,
        preflight={
            "slack": {
                "enabled_in_config": slack_enabled,
                "env_present": _has_live_slack_env(config, process_env),
                "requires_slack": needs_slack,
                "requires_delivery_target": needs_slack_target,
                "delivery_target_present": slack_target_present,
                "route_configured_from_env": route_configured_from_env,
                "route_discovery": slack_route_discovery,
                "legacy_actor_configured": legacy_actor_configured,
                "legacy_actor_user_id": legacy_actor_user_id,
                "auth_user_id": auth_user_id,
                "config_installation_id": _slack_config_value(config, "installation_id"),
                "config_team_id": _slack_config_value(config, "team_id"),
                "config_api_app_id": _slack_config_value(config, "api_app_id"),
                "auth_test": slack_auth,
                "secret_source": secret_preflight,
                "path_secret_env_names": sorted(path_secret_env),
            },
            "google_product_auth": google_preflight,
            "telegram": telegram_preflight,
            "github_auth": github_preflight,
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
                "ironclaw=warn,ironclaw_reborn=warn,ironclaw_reborn_webui_ingress=info",
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
    stdout_path = output_dir / "ironclaw-reborn-serve.stdout.log"
    stderr_path = output_dir / "ironclaw-reborn-serve.stderr.log"
    workspace_dir = output_dir / "workspace"
    workspace_dir.mkdir(parents=True, exist_ok=True)
    out = stdout_path.open("a", encoding="utf-8")
    err = stderr_path.open("a", encoding="utf-8")
    separator = f"\n--- ironclaw-reborn serve start {time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} ---\n"
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
            f"ironclaw-reborn serve did not become healthy at {base_url}: {exc}\n{tail}"
        ) from exc
    return proc, base_url


async def _with_page(output_dir: Path, case_name: str, action: Callable[[object], Awaitable[None]]) -> None:
    from playwright.async_api import async_playwright

    headless = os.environ.get("HEADED", "").strip().lower() not in ("1", "true")
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(headless=headless, timeout=60000)
        context = await browser.new_context()
        page = await context.new_page()
        try:
            await action(page)
        except Exception:
            screenshot = output_dir / f"{case_name}.failure.png"
            await page.screenshot(path=str(screenshot), full_page=True)
            raise
        finally:
            await context.close()
            await browser.close()


def _result(case_name: str, success: bool, started: float, details: dict[str, object]) -> ProbeResult:
    details = {"case": case_name, **details}
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


async def _live_chat_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    prompt: str,
    marker: str | None,
    required_text: list[str],
    timeout: float = 120.0,
    extra_details: dict[str, object] | None = None,
    forbidden_text: list[str] | None = None,
) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    observed: dict[str, Any] = {}

    async def action(page: object) -> None:
        await page.goto(
            f"{ctx.base_url}/v2/?token={AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )  # type: ignore[attr-defined]
        if await _dismiss_visible_connect_action(page):
            observed["connect_action_dismissed_before_submit"] = True
        composer = page.locator("[data-testid='chat-composer']")  # type: ignore[attr-defined]
        await expect(composer).to_be_visible(timeout=15000)
        await composer.fill(prompt)
        await composer.press("Enter")
        try:
            await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
                prompt[:80],
                timeout=15000,
            )
        except Exception:
            if not await _dismiss_visible_connect_action(page):
                raise
            observed["connect_action_dismissed_after_submit"] = True
            await composer.fill(prompt)
            await composer.press("Enter")
            await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
                prompt[:80],
                timeout=15000,
            )
        observed["text_excerpt"] = await _wait_for_assistant_reply(
            page,
            marker=marker,
            required_text=required_text,
            timeout=timeout,
            semantic_goal=prompt,
        )
        if forbidden_text:
            text = str(observed["text_excerpt"]).lower()
            matches = [phrase for phrase in forbidden_text if phrase.lower() in text]
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
    except Exception as exc:
        return _result(
            case_name,
            False,
            started,
            {
                "error": str(exc),
                "prompt": prompt,
                "marker": marker,
                "required_text": required_text,
                **(extra_details or {}),
                **observed,
            },
        )


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
    from playwright.async_api import expect

    started = time.monotonic()
    observed: dict[str, object] = {
        "marker": marker,
        "prompt": prompt,
        "required_text": required_text,
        "extensions": [extension["package_id"] for extension in extensions],
        **(extra_details or {}),
    }

    async def action(page: object) -> None:
        await page.goto(
            f"{ctx.base_url}/v2/extensions/registry?token={AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )  # type: ignore[attr-defined]
        await expect(page.locator("body")).to_contain_text("Extensions", timeout=15000)  # type: ignore[attr-defined]
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
            f"{ctx.base_url}/v2/?token={AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )  # type: ignore[attr-defined]
        if await _dismiss_visible_connect_action(page):
            observed["connect_action_dismissed_before_submit"] = True
        composer = page.locator("[data-testid='chat-composer']")  # type: ignore[attr-defined]
        await expect(composer).to_be_visible(timeout=15000)
        await composer.fill(prompt)
        await composer.press("Enter")
        try:
            await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
                prompt[:80],
                timeout=15000,
            )
        except Exception:
            if not await _dismiss_visible_connect_action(page):
                raise
            observed["connect_action_dismissed_after_submit"] = True
            await composer.fill(prompt)
            await composer.press("Enter")
            await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
                prompt[:80],
                timeout=15000,
            )
        observed["text_excerpt"] = await _wait_for_assistant_reply(
            page,
            marker=marker,
            required_text=required_text,
            timeout=timeout,
            semantic_goal=prompt,
        )

    try:
        await _with_page(ctx.output_dir, case_name, action)
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(case_name, False, started, {"error": str(exc), **observed})


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


async def _wait_for_assistant_reply(
    page: object,
    *,
    marker: str | None,
    required_text: list[str],
    timeout: float,
    semantic_goal: str | None = None,
) -> str:
    deadline = time.monotonic() + timeout
    assistant = page.locator("[data-testid='msg-assistant']").last  # type: ignore[attr-defined]
    last_text = ""
    while time.monotonic() < deadline:
        await _approve_visible_tool_gate(page)
        assistant_blocks = page.locator("[data-testid='msg-assistant']")  # type: ignore[attr-defined]
        if await assistant_blocks.count() > 0:
            try:
                text = await assistant.inner_text(timeout=1000)
            except Exception:
                text = ""
            try:
                block_texts = [
                    block.strip()
                    for block in await assistant_blocks.all_inner_texts()
                    if block.strip()
                ]
            except Exception:
                block_texts = []
            if block_texts:
                text = "\n".join(block_texts)
            if text:
                last_text = text
            normalized = text.lower()
            marker_matches = not marker or marker in text
            if marker_matches and _required_text_matches(normalized, required_text):
                return text[-2000:]
        await asyncio.sleep(0.5)
    main_text = ""
    try:
        main_text = await page.locator("main").inner_text(timeout=1000)  # type: ignore[attr-defined]
    except Exception:
        pass
    semantic_judge: dict[str, object] | None = None
    if last_text and (not marker or marker in last_text):
        semantic_judge = await _judge_assistant_reply_completion(
            marker=marker,
            required_text=required_text,
            assistant_text=last_text,
            main_text=main_text,
            semantic_goal=semantic_goal,
        )
        if _semantic_judge_passed(semantic_judge):
            return last_text[-2000:]
    raise AssertionError(
        "assistant reply did not contain required text before timeout. "
        f"marker={marker!r} required_text={required_text!r} "
        f"last_assistant={last_text[-500:]!r} main_excerpt={main_text[-1000:]!r} "
        f"semantic_judge={_compact_json(semantic_judge)}"
    )


def _required_text_matches(text: str, required_text: list[str]) -> bool:
    normalized_text = text.lower()
    return all(
        any(_required_option_matches(normalized_text, option) for option in piece.split("|"))
        for piece in required_text
    )


def _required_option_matches(normalized_text: str, option: str) -> bool:
    normalized_option = option.strip().lower()
    if not normalized_option:
        return False
    if re.fullmatch(r"\w+", normalized_option):
        return re.search(rf"\b{re.escape(normalized_option)}\b", normalized_text) is not None
    return normalized_option in normalized_text


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
    with sqlite3.connect(db_path) as db:
        if routine_name:
            cursor = db.execute(
                "SELECT COUNT(*) FROM trigger_records WHERE name = ?",
                (routine_name,),
            )
        else:
            cursor = db.execute("SELECT COUNT(*) FROM trigger_records")
        value = cursor.fetchone()[0]
    return int(value)


def _trigger_run_rows(reborn_home: Path, routine_name: str) -> list[dict[str, object]]:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return []
    with sqlite3.connect(db_path) as db:
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


def _triggered_delivery_outcome(reborn_home: Path, run_id: str) -> dict[str, object] | None:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    with sqlite3.connect(db_path) as db:
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
    with sqlite3.connect(db_path) as db:
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
    with sqlite3.connect(db_path) as db:
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


def _slack_bot_token(config_text: str, extra_env: dict[str, str]) -> str | None:
    bot_env = _section_env_name(
        config_text,
        "bot_token_env",
        "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
    )
    return _env_value(bot_env, extra_env)


def _slack_delivery_channel_id(ctx: LiveQaContext) -> str | None:
    slack = _slack_preflight(ctx)
    discovery = slack.get("route_discovery")
    if isinstance(discovery, dict):
        channel_id = str(discovery.get("channel_id") or "").strip()
        if channel_id:
            return channel_id
    env_channel = os.environ.get("REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_CHANNEL_ID", "").strip()
    if env_channel:
        return env_channel
    db_path = ctx.reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    with sqlite3.connect(db_path) as db:
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


async def _slack_history_contains_marker(
    ctx: LiveQaContext,
    *,
    channel_id: str,
    marker: str,
    oldest_epoch: float,
    required_text: list[str] | None = None,
) -> dict[str, object]:
    import httpx

    token = _slack_bot_token(_config_text(ctx.reborn_home / "config.toml"), ctx.env)
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
    for message in messages:
        if not isinstance(message, dict):
            continue
        text = str(message.get("text") or "")
        if marker in text:
            normalized = text.lower()
            missing_required = [
                piece for piece in (required_text or []) if piece.lower() not in normalized
            ]
            return {
                "checked": True,
                "found": not missing_required,
                "marker_found": True,
                "missing_required_text": missing_required,
                "message_ts": message.get("ts"),
                "message_user_present": bool(message.get("user") or message.get("bot_id")),
            }
    return {"checked": True, "found": False, "message_count": len(messages)}


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
                for route in _delivered_gate_routes_for_run(ctx.reborn_home, run_id):
                    gate_ref = str(route.get("gate_ref") or "")
                    if gate_ref in approved_gate_refs:
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
                if history is None:
                    history = await _slack_history_contains_marker(
                        ctx,
                        channel_id=channel_id,
                        marker=marker,
                        oldest_epoch=oldest_epoch,
                        required_text=required_text,
                    )
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


async def _slack_connect_case(ctx: LiveQaContext, *, case_name: str) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    observed: dict[str, object] = {
        "qa_sheet_prompt": _qa_sheet_prompt(case_name),
        "slack_connect_surface": "/v2/extensions/channels",
    }

    async def action(page: object) -> None:
        await page.goto(
            f"{ctx.base_url}/v2/extensions/channels?token={AUTH_TOKEN}",
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
                and channel.get("strategy") == "inbound_proof_code"
            ),
            None,
        )
        if not isinstance(personal, dict):
            raise AssertionError(f"Slack inbound_proof_code connect strategy missing: {channels!r}")
        action_body = personal.get("action")
        if not isinstance(action_body, dict):
            raise AssertionError(f"Slack connect action missing: {personal!r}")
        title = str(action_body.get("title") or "")
        if not title:
            raise AssertionError(f"Slack connect action title missing: {personal!r}")
        instructions = str(action_body.get("instructions") or "")
        if "Message the Slack app" not in instructions:
            raise AssertionError(f"unexpected Slack connect instructions: {instructions!r}")
        await expect(page.locator("body")).to_contain_text(title, timeout=15000)  # type: ignore[attr-defined]
        await expect(page.locator("body")).to_contain_text("Message the Slack app", timeout=15000)  # type: ignore[attr-defined]
        observed["slack_display_name"] = personal.get("display_name")
        observed["slack_connect_title"] = title
        observed["slack_connect_instructions"] = instructions

    try:
        slack = _slack_preflight(ctx)
        auth_test = slack.get("auth_test")
        if not slack.get("enabled_in_config") or not slack.get("env_present"):
            raise AssertionError(f"Slack was not enabled with env in preflight: {slack!r}")
        if not isinstance(auth_test, dict) or not auth_test.get("ok"):
            raise AssertionError(f"Slack auth.test did not pass in preflight: {auth_test!r}")
        observed["slack_auth_team_id"] = auth_test.get("team_id")
        observed["slack_auth_user_id"] = auth_test.get("user_id")
        await _with_page(ctx.output_dir, case_name, action)
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(case_name, False, started, {"error": str(exc), **observed})


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
            f"{ctx.base_url}/v2/extensions/registry?token={AUTH_TOKEN}",
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
        return _result(case_name, False, started, {"error": str(exc), **observed})


def _capability_run_statuses(
    reborn_home: Path,
    capability_ids: list[str],
) -> dict[str, list[str]]:
    statuses = {capability_id: [] for capability_id in capability_ids}
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return statuses
    try:
        with sqlite3.connect(db_path) as db:
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
        return _result(case_name, False, started, {"error": str(exc), **observed})


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
                "error": str(exc),
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
        slack_user_id = str(slack.get("legacy_actor_user_id") or "U0REBORNQA")
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
            {"error": str(exc), **observed},
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
        sheet_check = await _google_sheet_contains_marker(
            access_token=access_token,
            spreadsheet_id=spreadsheet_id,
            marker=marker,
        )
        result.details["google_token"] = token_meta
        result.details["spreadsheet_id"] = spreadsheet_id
        result.details["sheet_marker_check"] = sheet_check
        if not sheet_check.get("found"):
            result.success = False
            result.details["error"] = "Google Sheet did not contain the QA marker row"
        result.latency_ms = int((time.monotonic() - started) * 1000)
        return result
    except Exception as exc:
        result.success = False
        result.latency_ms = int((time.monotonic() - started) * 1000)
        result.details["spreadsheet_id"] = spreadsheet_id
        result.details["error"] = str(exc)
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
            timeout=180.0,
            extra_details=details,
        )
    else:
        result = await _live_chat_case(
            ctx,
            case_name=case_name,
            prompt=prompt,
            marker=marker,
            required_text=required_text,
            timeout=180.0,
            extra_details=details,
        )
    after_count = _trigger_record_count(ctx.reborn_home, count_name)
    result.details["trigger_records_after"] = after_count
    if result.success and after_count <= before_count:
        result.success = False
        result.details["error"] = (
            f"assistant returned success but routine scope {routine_name!r} "
            "did not add a trigger_record"
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
) -> ProbeResult:
    started = time.monotonic()
    wall_started = time.time()
    suffix = str(int(wall_started * 1000))
    routine_name = f"{routine_prefix}-{suffix}"
    creation_marker = f"{marker_prefix}_ROUTINE_CREATED_{suffix}"
    delivery_marker = f"{marker_prefix}_SLACK_DELIVERED_{suffix}"
    creation = await _routine_creation_case(
        ctx,
        case_name=case_name,
        routine_name=routine_name,
        marker=creation_marker,
        required_text=["routine"],
        prompt=(
            f"QA case {case_name}: create a routine named {routine_name}. Every minute, "
            f"{routine_instruction} The routine's final answer and Slack message must "
            f"include the exact marker {delivery_marker}. Create the routine now; do not "
            "run it immediately. During routine creation, do not perform the routine's "
            "live check, web/search/HTTP lookup, or Slack send. Before calling trigger_create, "
            "call builtin__outbound_delivery_targets_list, then call "
            "builtin__outbound_delivery_target_set with the Slack target id returned by the "
            "list tool; do not only mention the target in text. Then create the routine "
            "definition. "
            f"In your final answer include the exact marker {creation_marker} and include "
            "the text routine."
        ),
    )
    if not creation.success:
        creation.latency_ms = int((time.monotonic() - started) * 1000)
        return creation
    try:
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
        # The exact Slack body is not persisted in results to avoid leaking workspace data.
        return _result(
            case_name,
            True,
            started,
            {
                **creation.details,
                "routine_name": routine_name,
                "creation_marker": creation_marker,
                "delivery_marker": delivery_marker,
                "required_delivery_text": text_checks,
                "trigger_run": delivery.get("trigger_run"),
                "delivery_outcome": delivery.get("delivery_outcome"),
                "slack_history": history,
            },
        )
    except Exception as exc:
        return _result(
            case_name,
            False,
            started,
            {
                **creation.details,
                "error": str(exc),
                "routine_name": routine_name,
                "creation_marker": creation_marker,
                "delivery_marker": delivery_marker,
                "required_delivery_text": required_delivery_text,
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
                "error": str(exc),
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


def _slack_signing_secret(config_text: str, extra_env: dict[str, str]) -> str | None:
    signing_env = _section_env_name(
        config_text,
        "signing_secret_env",
        "IRONCLAW_REBORN_SLACK_SIGNING_SECRET",
    )
    return _env_value(signing_env, extra_env)


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

    config_text = _config_text(ctx.reborn_home / "config.toml")
    signing_secret = _slack_signing_secret(config_text, ctx.env)
    if not signing_secret:
        raise AssertionError("Slack signing secret is unavailable for signed webhook injection")
    slack = _slack_preflight(ctx)
    auth_test = slack.get("auth_test")
    team_id = None
    if isinstance(auth_test, dict):
        team_id = auth_test.get("team_id")
    if not team_id:
        team_id = slack.get("team_id") or slack.get("secret_source", {}).get("team_id")
    secret_source = slack.get("secret_source")
    api_app_id = None
    if isinstance(secret_source, dict):
        api_app_id = secret_source.get("api_app_id")
    if not api_app_id:
        api_app_id = slack.get("config_api_app_id")
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
                "legacy_actor_configured": slack.get("legacy_actor_configured"),
                "legacy_actor_user_id": slack.get("legacy_actor_user_id"),
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
        slack_user_id = str(slack.get("legacy_actor_user_id") or "U0REBORNQA")
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
        return _result(case_name, False, started, {"error": str(exc), **observed})


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
        slack_user_id = str(slack.get("legacy_actor_user_id") or "U0REBORNQA")
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
                "error": str(exc),
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
            prompt=_qa_sheet_prompt("qa_7c_slack_bug_logger_routine"),
            extensions=[
                {
                    "package_id": "slack",
                    "display_name": "Slack",
                    "required_tools": [],
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
            {"routine_name": routine_name, "error": str(exc)},
        )


async def case_qa_7a_slack_product_channel_connect(ctx: LiveQaContext) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    case_name = "qa_7a_slack_product_channel_connect"
    prompt = _qa_sheet_prompt(case_name)
    observed: dict[str, object] = {"chat_connect_prompt": prompt}
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

        async def action(page: object) -> None:
            capability_ids = QA_7A_CHAT_CONNECT_CAPABILITY_IDS
            baseline_statuses = _capability_run_statuses(
                ctx.reborn_home,
                capability_ids,
            )
            baseline_completed = _completed_capability_counts(baseline_statuses)
            observed["baseline_capability_statuses"] = baseline_statuses
            await page.goto(
                f"{ctx.base_url}/v2/?token={AUTH_TOKEN}",
                wait_until="domcontentloaded",
            )  # type: ignore[attr-defined]
            composer = page.locator("[data-testid='chat-composer']")  # type: ignore[attr-defined]
            await expect(composer).to_be_visible(timeout=15000)
            await composer.fill(prompt)
            await composer.press("Enter")
            await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
                prompt[:80],
                timeout=15000,
            )
            observed["text_excerpt"] = await _wait_for_assistant_reply(
                page,
                marker=None,
                required_text=["slack"],
                timeout=180.0,
            )
            deadline = time.monotonic() + 180.0
            while time.monotonic() < deadline:
                await _approve_visible_tool_gate(page)
                statuses = _capability_run_statuses(ctx.reborn_home, capability_ids)
                observed["capability_statuses"] = statuses
                completed = _completed_capability_counts(statuses)
                missing = [
                    capability_id
                    for capability_id in capability_ids
                    if completed.get(capability_id, 0)
                    <= baseline_completed.get(capability_id, 0)
                ]
                if not missing:
                    return
                await asyncio.sleep(1.0)
            raise AssertionError(
                "Slack DM connect prompt did not complete expected capabilities: "
                f"{capability_ids!r}; observed statuses={observed.get('capability_statuses')!r}"
            )

        await _with_page(ctx.output_dir, case_name, action)
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(
            case_name,
            False,
            started,
            {"error": str(exc), **observed},
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
        required_text=["routine"],
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
                "default_enabled": spec.default_enabled,
                "requires_slack": spec.requires_slack,
                "requires_slack_target": spec.requires_slack_target,
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
        with sqlite3.connect(db_path) as db:
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
            f"ironclaw-reborn binary missing at {binary}; rerun without --skip-build"
        )
    results: list[ProbeResult] = []
    trace_exports: list[dict[str, object]] = []
    first_base_url = ""
    for name in selected_cases:
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
                        "live Slack outbound delivery target is not configured "
                        f"for WebUI user {_auth_user_id()!r}"
                    ),
                    "required_env": [
                        "REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_CHANNEL_ID",
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
            print(f"[reborn-webui-v2-live-qa] running case={name}", flush=True)
            result = await CASES[name].fn(ctx)
            results.append(result)
            print(
                f"[reborn-webui-v2-live-qa] case={name} success={result.success} "
                f"latency_ms={result.latency_ms}",
                flush=True,
            )
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
    print(f"[reborn-webui-v2-live-qa] results={results_path}", flush=True)
    print(f"[reborn-webui-v2-live-qa] trace_index={trace_index_path}", flush=True)
    return 0 if all(result.success for result in results) else 1


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
            details={"error": str(exc)},
        )
        write_results(args.output_dir, [failed], "")
        print(f"[reborn-webui-v2-live-qa] {exc}", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
