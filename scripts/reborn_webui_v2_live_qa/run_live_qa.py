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

DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "reborn-webui-v2-live-qa"
DEFAULT_REBORN_HOME = Path("/tmp/ironclaw-reborn-real-slack")
AUTH_TOKEN = "reborn-webui-v2-live-qa-token-0123456789abcdef"
DEFAULT_USER_ID = "reborn-webui-v2-live-qa-user"
PROVIDER = "reborn-webui-v2"
MODE = "live"

CaseFn = Callable[["LiveQaContext"], Awaitable[ProbeResult]]


class LiveQaError(RuntimeError):
    pass


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


QA_SHEET_CASES: dict[str, dict[str, object]] = {
    "qa_1a_telegram_connect": {
        "rows": ["1A"],
        "feature": "Telegram connection flow",
        "gate": "requires live Telegram bot/user credentials and OAuth/pairing automation",
    },
    "qa_1b_telegram_near_news_chat": {
        "rows": ["1B"],
        "feature": "Telegram NEAR AI news summary delivery",
        "gate": "requires live Telegram connection and live Twitter/X or web search access",
    },
    "qa_1c_telegram_near_news_routine": {
        "rows": ["1C"],
        "feature": "Scheduled Telegram NEAR AI news digest routine",
        "gate": "requires live Telegram connection and routine delivery verification",
    },
    "qa_2a_gmail_connect": {
        "rows": ["2A"],
        "feature": "Gmail connection flow",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_2b_calendar_connect": {
        "rows": ["2B"],
        "feature": "Google Calendar connection flow",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_2c_drive_connect": {
        "rows": ["2C"],
        "feature": "Google Drive connection flow",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_2d_calendar_prep_live_chat": {
        "rows": ["2D"],
        "feature": "Calendar prep assistant using Google Docs and live news",
        "gate": (
            "requires a live Google OAuth account authorized for Calendar, Drive, "
            "Docs, and web/search runtime execution, plus Google OAuth refresh "
            "env when the copied access token is expired"
        ),
    },
    "qa_2e_calendar_prep_email_routine": {
        "rows": ["2E"],
        "feature": "Scheduled meeting-prep email routine",
        "gate": "requires live Gmail, Calendar, Drive, Docs, and routine verification",
    },
    "qa_2f_calendar_prep_email_delivery": {
        "rows": ["2F"],
        "feature": "Meeting-prep email side-effect delivery",
        "gate": "requires live Gmail inbox delivery verification",
    },
    "qa_3a_slack_connect": {
        "rows": ["3A"],
        "feature": "Slack connection flow",
        "gate": "requires live Slack OAuth or host-beta Slack bot/signing-secret env",
    },
    "qa_3b_endpoint_status_live_chat": {
        "rows": ["3B"],
        "feature": "Deployment health watcher endpoint status check",
    },
    "qa_3c_endpoint_status_slack_routine": {
        "rows": ["3C"],
        "feature": "Deployment health watcher Slack routine creation",
        "gate": "requires live Slack host-beta bot/signing-secret env",
    },
    "qa_3d_endpoint_status_slack_delivery": {
        "rows": ["3D"],
        "feature": "Deployment health watcher Slack delivery",
        "gate": "requires live Slack message delivery verification",
    },
    "qa_4a_gmail_connect": {
        "rows": ["4A"],
        "feature": "Gmail connection flow for release tracker",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_4b_github_connect": {
        "rows": ["4B"],
        "feature": "GitHub connection flow",
        "gate": "requires live GitHub PAT/auth state",
    },
    "qa_4c_github_release_live_chat": {
        "rows": ["4C"],
        "feature": "GitHub release tracker summary",
    },
    "qa_4d_github_release_slack_routine": {
        "rows": ["4D"],
        "feature": "Scheduled GitHub release summary routine",
        "gate": "requires live Slack delivery target and routine verification",
    },
    "qa_4e_github_release_email_delivery": {
        "rows": ["4E"],
        "feature": "GitHub release summary email delivery",
        "gate": "requires live Gmail delivery verification and a new release/change trigger",
    },
    "qa_5a_slack_connect": {
        "rows": ["5A"],
        "feature": "Slack connection flow for AMA",
        "gate": "requires live Slack OAuth or host-beta Slack bot/signing-secret env",
    },
    "qa_5b_drive_connect": {
        "rows": ["5B"],
        "feature": "Google Drive connection flow for AMA",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_5c_strategy_doc_knowledge_base": {
        "rows": ["5C"],
        "feature": "Google Drive strategy document grounding",
        "gate": (
            "requires a live Google OAuth account authorized for Google Docs/Drive "
            "runtime execution, plus Google OAuth refresh env when the copied "
            "access token is expired"
        ),
    },
    "qa_5d_slack_strategy_doc_answer": {
        "rows": ["5D"],
        "feature": "Slack AMA answer grounded in Google Drive document",
        "gate": "requires live Slack and Google Drive side-effect verification",
    },
    "qa_6a_gmail_connect": {
        "rows": ["6A"],
        "feature": "Gmail connection flow for CRM tracker",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_6b_sheets_connect": {
        "rows": ["6B"],
        "feature": "Google Sheets connection flow",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_6c_gmail_to_sheet_live_chat": {
        "rows": ["6C"],
        "feature": "CRM inbound email extraction to Google Sheet",
        "gate": (
            "requires a live Google OAuth account authorized for Gmail and Google "
            "Sheets runtime execution plus test data, and Google OAuth refresh "
            "env when the copied access token is expired"
        ),
    },
    "qa_6d_gmail_to_sheet_routine": {
        "rows": ["6D"],
        "feature": "Scheduled CRM inbound email tracker routine",
        "gate": "requires live Gmail and Google Sheets routine verification",
    },
    "qa_6e_gmail_to_sheet_delivery": {
        "rows": ["6E"],
        "feature": "CRM inbound email row side effect",
        "gate": "requires live Gmail inbox and Google Sheets row verification",
    },
    "qa_7a_slack_product_channel_connect": {
        "rows": ["7A"],
        "feature": "Slack product channel connection flow",
        "gate": "requires live Slack OAuth/channel setup",
    },
    "qa_7b_sheets_connect": {
        "rows": ["7B"],
        "feature": "Google Sheets connection flow for bug logger",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_7c_slack_bug_logger_routine": {
        "rows": ["7C"],
        "feature": "Slack bug-message to Google Sheet routine creation",
        "gate": "requires live Slack and Google Sheets routine verification",
    },
    "qa_7d_slack_bug_message_trigger": {
        "rows": ["7D"],
        "feature": "Slack bug-message trigger",
        "gate": "requires live Slack message injection",
    },
    "qa_7e_slack_bug_sheet_delivery": {
        "rows": ["7E"],
        "feature": "Slack bug-message row side effect",
        "gate": "requires live Slack and Google Sheets row verification",
    },
    "qa_8a_slack_connect": {
        "rows": ["8A"],
        "feature": "Slack connection flow for HN monitor",
        "gate": "requires live Slack OAuth or host-beta Slack bot/signing-secret env",
    },
    "qa_8b_hn_keyword_live_chat": {
        "rows": ["8B"],
        "feature": "Hacker News keyword monitor search",
    },
    "qa_8c_hn_keyword_slack_routine": {
        "rows": ["8C"],
        "feature": "Hacker News keyword monitor Slack routine creation",
        "gate": "requires live Slack host-beta bot/signing-secret env",
    },
    "qa_8d_hn_keyword_slack_delivery": {
        "rows": ["8D"],
        "feature": "Hacker News keyword monitor Slack delivery",
        "gate": "requires live Slack message delivery verification",
    },
}


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
    if args.all_cases and args.case:
        parser.error("--all-cases cannot be combined with --case")
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


def _env_value(name: str, extra_env: dict[str, str] | None = None) -> str | None:
    if extra_env and extra_env.get(name):
        return extra_env[name]
    return env_secret(name)


def _env_present(name: str, extra_env: dict[str, str] | None = None) -> bool:
    return bool(_env_value(name, extra_env))


def _non_empty_env(name: str, default: str) -> str:
    value = os.environ.get(name)
    if value and value.strip():
        return value.strip()
    return default


def _slack_enabled(config_text: str) -> bool:
    in_slack = False
    for raw_line in config_text.splitlines():
        line = raw_line.strip()
        if line.startswith("[") and line.endswith("]"):
            in_slack = line == "[slack]"
            continue
        if in_slack and re.match(r"enabled\s*=\s*true\b", line):
            return True
    return False


def _has_live_slack_env(config_text: str, extra_env: dict[str, str] | None = None) -> bool:
    signing_env = _section_env_name(
        config_text,
        "signing_secret_env",
        "IRONCLAW_REBORN_SLACK_SIGNING_SECRET",
    )
    bot_env = _section_env_name(
        config_text,
        "bot_token_env",
        "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
    )
    return _env_present(signing_env, extra_env) and _env_present(bot_env, extra_env)


def _section_env_name(config_text: str, key: str, default: str) -> str:
    match = re.search(rf"^\s*{key}\s*=\s*\"([A-Za-z_][A-Za-z0-9_]*)\"", config_text, re.MULTILINE)
    return match.group(1) if match else default


def _slack_config_value(config_text: str, key: str) -> str | None:
    in_slack = False
    for raw_line in config_text.splitlines():
        line = raw_line.strip()
        if line.startswith("[") and line.endswith("]"):
            in_slack = line == "[slack]"
            continue
        if not in_slack:
            continue
        match = re.match(rf"{re.escape(key)}\s*=\s*\"([^\"]*)\"", line)
        if match:
            value = match.group(1).strip()
            return value or None
    return None


def _disable_slack_in_config(config_path: Path) -> None:
    lines = config_path.read_text(encoding="utf-8").splitlines()
    in_slack = False
    changed = False
    rewritten: list[str] = []
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            in_slack = stripped == "[slack]"
        if in_slack and re.match(r"^(\s*)enabled\s*=\s*true\b", line):
            indent = re.match(r"^(\s*)", line).group(1)  # type: ignore[union-attr]
            rewritten.append(f"{indent}enabled = false")
            changed = True
        else:
            rewritten.append(line)
    if changed:
        config_path.write_text("\n".join(rewritten) + "\n", encoding="utf-8")


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


def _append_slack_channel_route(
    config_path: Path,
    *,
    subject_user_id: str,
    channel_id: str,
) -> bool:
    channel_id = channel_id.strip()
    if not channel_id:
        return False
    route_subject_user_id = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_SUBJECT_USER_ID",
        subject_user_id,
    ).strip()
    if not route_subject_user_id:
        route_subject_user_id = subject_user_id
    config = config_path.read_text(encoding="utf-8")
    if channel_id in config and route_subject_user_id in config:
        return True
    with config_path.open("a", encoding="utf-8") as fh:
        fh.write(
            "\n[[slack.channel_routes]]\n"
            f'channel_id = "{channel_id}"\n'
            f'subject_user_id = "{route_subject_user_id}"\n'
        )
    return True


def _append_slack_channel_route_if_configured(config_path: Path, subject_user_id: str) -> bool:
    channel_id = os.environ.get("REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_CHANNEL_ID", "").strip()
    return _append_slack_channel_route(
        config_path,
        subject_user_id=subject_user_id,
        channel_id=channel_id,
    )


def _set_slack_section_key(config_path: Path, key: str, value: str) -> bool:
    if not value.strip():
        return False
    lines = config_path.read_text(encoding="utf-8").splitlines()
    in_slack = False
    slack_header_index: int | None = None
    insert_index: int | None = None
    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[slack]":
            in_slack = True
            slack_header_index = index
            insert_index = index + 1
            continue
        if in_slack and stripped.startswith("[") and stripped.endswith("]"):
            insert_index = index
            break
        if in_slack and stripped.startswith(f"{key} ="):
            if line.strip() == f'{key} = "{value}"':
                return False
            lines[index] = f'{key} = "{value}"'
            config_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
            return True
    if slack_header_index is None:
        return False
    if insert_index is None:
        insert_index = len(lines)
    lines.insert(insert_index, f'{key} = "{value}"')
    config_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return True


def _configure_slack_legacy_actor_if_needed(
    config_path: Path, selected_cases: list[str]
) -> tuple[bool, str | None]:
    if "qa_7d_slack_bug_message_trigger" not in selected_cases:
        return False, None
    slack_user_id = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID",
        "U0REBORNQA",
    ).strip()
    if not slack_user_id:
        return False, None
    changed = _set_slack_section_key(config_path, "slack_user_id", slack_user_id)
    return changed, slack_user_id


def _discover_slack_dm_route_channel(
    config_text: str,
    extra_env: dict[str, str],
) -> dict[str, object]:
    bot_env = _section_env_name(
        config_text,
        "bot_token_env",
        "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
    )
    token = _env_value(bot_env, extra_env)
    if not token:
        return {"checked": False, "ok": False, "error": "bot token env unavailable"}
    try:
        import httpx

        response = httpx.post(
            "https://slack.com/api/conversations.open",
            headers={"Authorization": f"Bearer {token}"},
            data={"users": "USLACKBOT"},
            timeout=20.0,
        )
        payload = response.json()
    except Exception as exc:
        return {
            "checked": True,
            "ok": False,
            "error": "slack_conversations_open_failed",
            "error_type": type(exc).__name__,
        }
    result: dict[str, object] = {
        "checked": True,
        "ok": bool(payload.get("ok")),
    }
    channel = payload.get("channel")
    if isinstance(channel, dict):
        channel_id = str(channel.get("id") or "").strip()
        if channel_id:
            result["channel_id"] = channel_id
        result["channel_is_im"] = channel.get("is_im")
    if not payload.get("ok"):
        result["error"] = payload.get("error")
        result["needed"] = payload.get("needed")
    return result


def _config_has_slack_channel_route_for_user(config_text: str, user_id: str) -> bool:
    in_route = False
    route: dict[str, str] = {}
    routes: list[dict[str, str]] = []
    for raw_line in config_text.splitlines():
        line = raw_line.strip()
        if line == "[[slack.channel_routes]]":
            if route:
                routes.append(route)
            route = {}
            in_route = True
            continue
        if line.startswith("[") and line.endswith("]") and line != "[[slack.channel_routes]]":
            if route:
                routes.append(route)
            route = {}
            in_route = False
            continue
        if in_route and "=" in line:
            key, _, value = line.partition("=")
            route[key.strip()] = value.strip().strip('"')
    if route:
        routes.append(route)
    return any(
        route.get("subject_user_id") == user_id and bool(route.get("channel_id"))
        for route in routes
    )


def _has_persisted_slack_personal_dm_target(reborn_home: Path, user_id: str) -> bool:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return False
    with sqlite3.connect(db_path) as db:
        row = db.execute(
            "SELECT COUNT(*) FROM root_filesystem_entries "
            "WHERE path LIKE '%slack-personal-binding/dm-targets%' "
            "AND CAST(contents AS TEXT) LIKE ?",
            (f"%{user_id}%",),
        ).fetchone()
    return bool(row and int(row[0]) > 0)


def _has_slack_delivery_target(config_text: str, reborn_home: Path, user_id: str) -> bool:
    return _config_has_slack_channel_route_for_user(
        config_text,
        user_id,
    ) or _has_persisted_slack_personal_dm_target(reborn_home, user_id)


def _google_product_auth_env_status(
    extra_env: dict[str, str] | None = None,
) -> dict[str, object]:
    client_id_names = [
        "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
        "GOOGLE_CLIENT_ID",
        "GOOGLE_OAUTH_CLIENT_ID",
    ]
    redirect_names = [
        "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI",
        "GOOGLE_OAUTH_REDIRECT_URI",
    ]
    optional_names = [
        "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET",
        "IRONCLAW_REBORN_GOOGLE_HOSTED_DOMAIN_HINT",
        "GOOGLE_CLIENT_SECRET",
        "GOOGLE_ALLOWED_HD",
        "GOOGLE_OAUTH_CLIENT_SECRET",
    ]
    present = {
        name: _env_present(name, extra_env)
        for name in [*client_id_names, *redirect_names, *optional_names]
    }
    client_id_ready = any(present[name] for name in client_id_names)
    redirect_ready = any(present[name] for name in redirect_names)
    return {
        "ready": client_id_ready,
        "client_id_ready": client_id_ready,
        "redirect_uri_ready": redirect_ready,
        "redirect_uri_source": "env" if redirect_ready else "dynamic_serve_port",
        "present": present,
        "required_sets": [
            ["IRONCLAW_REBORN_GOOGLE_CLIENT_ID"],
            ["GOOGLE_CLIENT_ID"],
            ["GOOGLE_OAUTH_CLIENT_ID"],
        ],
    }


def _materialize_telegram_env_for_reborn(
    extra_env: dict[str, str] | None = None,
) -> tuple[dict[str, str], dict[str, object]]:
    materialized: dict[str, str] = {}
    bot_token = _first_env_value(
        [
            "TELEGRAM_BOT_TOKEN",
            "IRONCLAW_REBORN_TELEGRAM_BOT_TOKEN",
            "LIVE_CANARY_TELEGRAM_BOT_TOKEN",
        ],
        extra_env,
    )
    webhook_secret = _first_env_value(
        [
            "TELEGRAM_WEBHOOK_SECRET",
            "IRONCLAW_REBORN_TELEGRAM_WEBHOOK_SECRET",
            "LIVE_CANARY_TELEGRAM_WEBHOOK_SECRET",
        ],
        extra_env,
    )
    chat_id = _first_env_value(
        [
            "REBORN_WEBUI_V2_LIVE_QA_TELEGRAM_CHAT_ID",
            "LIVE_CANARY_TELEGRAM_CHAT_ID",
        ],
        extra_env,
    )
    if bot_token:
        materialized["TELEGRAM_BOT_TOKEN"] = bot_token[1]
    if webhook_secret:
        materialized["TELEGRAM_WEBHOOK_SECRET"] = webhook_secret[1]
    if chat_id:
        materialized["REBORN_WEBUI_V2_LIVE_QA_TELEGRAM_CHAT_ID"] = chat_id[1]
    return materialized, {
        "materialized": bool(materialized),
        "env_names": sorted(materialized),
        "bot_token_present": bot_token is not None,
        "bot_token_source": bot_token[0] if bot_token else None,
        "webhook_secret_present": webhook_secret is not None,
        "webhook_secret_source": webhook_secret[0] if webhook_secret else None,
        "chat_id_present": chat_id is not None,
        "chat_id_source": chat_id[0] if chat_id else None,
    }


def _telegram_preflight(
    reborn_home: Path,
    extra_env: dict[str, str],
    env_materialization: dict[str, object],
    *,
    requires_telegram: bool,
) -> dict[str, object]:
    channels_src = ROOT / "channels-src" / "telegram"
    built_component = channels_src / "telegram.wasm"
    wasip2_release_component = (
        channels_src
        / "target"
        / "wasm32-wasip2"
        / "release"
        / "telegram_channel.wasm"
    )
    wasip1_release_component = (
        channels_src
        / "target"
        / "wasm32-wasip1"
        / "release"
        / "telegram_channel.wasm"
    )
    component_candidates = [
        built_component,
        wasip2_release_component,
        wasip1_release_component,
    ]
    capabilities = channels_src / "telegram.capabilities.json"
    copied_home_mentions = False
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if db_path.exists():
        with sqlite3.connect(db_path) as db:
            row = db.execute(
                "SELECT COUNT(*) FROM root_filesystem_entries "
                "WHERE LOWER(path) LIKE '%telegram%' OR LOWER(CAST(contents AS TEXT)) LIKE '%telegram%'"
            ).fetchone()
        copied_home_mentions = bool(row and int(row[0]) > 0)
    bot_token_present = _env_present("TELEGRAM_BOT_TOKEN", extra_env)
    component_present = any(path.exists() for path in component_candidates)
    ready = bool(bot_token_present and capabilities.exists() and component_present)
    reason = None
    if not ready:
        missing: list[str] = []
        if not bot_token_present:
            missing.append("TELEGRAM_BOT_TOKEN")
        if not capabilities.exists():
            missing.append("channels-src/telegram/telegram.capabilities.json")
        if not component_present:
            missing.append("built telegram WASM component")
        reason = "missing Telegram live prerequisites: " + ", ".join(missing)
    return {
        "requires_telegram": requires_telegram,
        "ready": ready,
        "reason": reason,
        "bot_token_present": bot_token_present,
        "webhook_secret_present": _env_present("TELEGRAM_WEBHOOK_SECRET", extra_env),
        "chat_id_present": _env_present("REBORN_WEBUI_V2_LIVE_QA_TELEGRAM_CHAT_ID", extra_env),
        "capabilities_present": capabilities.exists(),
        "built_component_present": component_present,
        "built_component_candidates": [
            str(path)
            for path in component_candidates
            if path.exists()
        ],
        "copied_reborn_home_mentions_telegram": copied_home_mentions,
        "env_materialization": env_materialization,
    }


def _github_auth_preflight(
    reborn_home: Path,
    extra_env: dict[str, str],
    *,
    requires_github_auth: bool,
) -> dict[str, object]:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    token_names = [
        "AUTH_LIVE_GITHUB_TOKEN",
        "IRONCLAW_REBORN_GITHUB_TOKEN",
        "GITHUB_TOKEN",
        "GH_TOKEN",
    ]
    token_present = any(_env_present(name, extra_env) for name in token_names)
    configured_accounts: list[str] = []
    if db_path.exists():
        with sqlite3.connect(db_path) as db:
            try:
                rows = db.execute(
                    """
                    SELECT path, contents FROM root_filesystem_entries
                    WHERE path LIKE '%product-auth/callback/accounts/%.json'
                    ORDER BY path
                    """
                ).fetchall()
            except sqlite3.Error:
                rows = []
        for path, raw in rows:
            try:
                payload = json.loads(raw)
            except (TypeError, json.JSONDecodeError):
                continue
            if (
                isinstance(payload, dict)
                and payload.get("provider") == "github"
                and payload.get("status") == "configured"
                and (payload.get("access_secret") or payload.get("access_secret_handle"))
            ):
                configured_accounts.append(str(path))
    ready = bool(configured_accounts)
    reason = None
    if requires_github_auth and not ready:
        reason = (
            "missing GitHub live prerequisites: configured GitHub product-auth "
            "account or PAT-seeded Reborn home"
        )
    return {
        "requires_github_auth": requires_github_auth,
        "ready": ready,
        "reason": reason,
        "db_present": db_path.exists(),
        "configured_account_count": len(configured_accounts),
        "configured_account_paths": configured_accounts,
        "token_env_present": token_present,
        "token_env_names": token_names,
    }


def _seed_generated_github_product_auth_if_configured(reborn_home: Path, user_id: str) -> dict[str, object]:
    token_names = [
        "AUTH_LIVE_GITHUB_TOKEN",
        "IRONCLAW_REBORN_GITHUB_TOKEN",
        "LIVE_CANARY_GITHUB_TOKEN",
        "GITHUB_TOKEN",
        "GH_TOKEN",
    ]
    selected = _first_env_value(token_names)
    preflight: dict[str, object] = {
        "checked": True,
        "seeded": False,
        "token_env_present": selected is not None,
        "token_env_names": token_names,
        "token_env_source": selected[0] if selected else None,
    }
    if not selected:
        return preflight

    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    master_key_path = reborn_home / "local-dev" / ".reborn-local-dev-secrets-master-key"
    master_key_path.parent.mkdir(parents=True, exist_ok=True)
    if master_key_path.exists():
        master_key = master_key_path.read_text(encoding="utf-8").strip()
    else:
        master_key = hashlib.sha256(os.urandom(32)).hexdigest()
        master_key_path.write_text(master_key, encoding="utf-8")
        master_key_path.chmod(0o600)

    _root_filesystem_create_table(db_path)
    account_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/github/{user_id}",
        )
    )
    invocation_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/github-invocation/{user_id}",
        )
    )
    thread_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/github-thread/{user_id}",
        )
    )
    now_s = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    resource = {
        "tenant_id": "reborn-cli",
        "user_id": user_id,
        "agent_id": "reborn-cli-agent",
        "project_id": None,
        "thread_id": thread_id,
        "invocation_id": invocation_id,
        "mission_id": None,
    }
    secret_scope = dict(resource)
    access_handle = f"product-auth-manual-{account_id}-{account_id}"
    secret_root = (
        f"/tenants/reborn-cli/users/{user_id}/secrets/agents/reborn-cli-agent/secrets"
    )
    encrypted_value, key_salt = _encrypt_filesystem_secret(
        master_key=master_key,
        scope=secret_scope,
        handle=access_handle,
        plaintext=selected[1],
    )
    _put_root_filesystem_json(
        db_path,
        f"{secret_root}/{access_handle}.json",
        {
            "handle": access_handle,
            "scope": secret_scope,
            "encrypted_value": encrypted_value,
            "key_salt": key_salt,
            "expires_at": None,
            "created_at": now_s,
            "updated_at": now_s,
        },
    )

    account_path = (
        f"/tenants/reborn-cli/users/{user_id}/secrets/agents/reborn-cli-agent/"
        f"product-auth/callback/accounts/{account_id}.json"
    )
    _put_root_filesystem_json(
        db_path,
        account_path,
        {
            "id": account_id,
            "provider": "github",
            "label": "github",
            "status": "configured",
            "ownership": "user_reusable",
            "owner_extension": None,
            "granted_extensions": [],
            "scope": {
                "resource": resource,
                "surface": "callback",
            },
            "scopes": [],
            "access_secret": access_handle,
            "refresh_secret": None,
            "created_at": now_s,
            "updated_at": now_s,
        },
    )
    preflight.update(
        {
            "seeded": True,
            "account_id": account_id,
            "account_path": account_path,
        }
    )
    return preflight


def _google_required_env_for_block(
    preflight: dict[str, object],
    *,
    requires_runtime_access: bool,
) -> list[str]:
    required = ["IRONCLAW_REBORN_GOOGLE_CLIENT_ID"]
    if preflight.get("missing_google_client_secret"):
        required.append("IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET")
    if requires_runtime_access or preflight.get("refresh_probe_failed"):
        for name in (
            "AUTH_LIVE_GOOGLE_ACCESS_TOKEN",
            "AUTH_LIVE_GOOGLE_REFRESH_TOKEN",
        ):
            if name not in required:
                required.append(name)
        if "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET" not in required:
            required.append("IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET")
    return required


def _first_env_value(
    names: list[str],
    extra_env: dict[str, str] | None = None,
) -> tuple[str, str] | None:
    for name in names:
        value = _env_value(name, extra_env)
        if value:
            return name, value
    return None


def _stored_google_oauth_client_id_from_reborn_home(reborn_home: Path) -> tuple[str, str] | None:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    with sqlite3.connect(db_path) as db:
        rows = db.execute(
            "SELECT path, contents FROM root_filesystem_entries "
            "WHERE path LIKE '%product-auth/callback/flows/%.json' "
            "AND CAST(contents AS TEXT) LIKE '%accounts.google.com%' "
            "ORDER BY path"
        ).fetchall()
    for path, contents in rows:
        try:
            payload = json.loads(contents)
        except (TypeError, json.JSONDecodeError):
            continue
        challenge = payload.get("challenge") if isinstance(payload, dict) else None
        if not isinstance(challenge, dict):
            continue
        authorization_url = str(challenge.get("authorization_url") or "")
        if not authorization_url:
            continue
        parsed = urllib.parse.urlparse(authorization_url)
        client_ids = urllib.parse.parse_qs(parsed.query).get("client_id") or []
        client_id = str(client_ids[0]).strip() if client_ids else ""
        if client_id:
            return (f"stored_flow:{path}", client_id)
    return None


def _materialize_google_oauth_env_for_reborn(
    reborn_home: Path | None = None,
    extra_env: dict[str, str] | None = None,
) -> tuple[dict[str, str], dict[str, object]]:
    materialized: dict[str, str] = {}

    client_id = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
            "GOOGLE_CLIENT_ID",
            "GOOGLE_OAUTH_CLIENT_ID",
        ],
        extra_env,
    )
    if not client_id and reborn_home is not None:
        client_id = _stored_google_oauth_client_id_from_reborn_home(reborn_home)
    if client_id:
        materialized["IRONCLAW_REBORN_GOOGLE_CLIENT_ID"] = client_id[1]

    client_secret = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET",
            "GOOGLE_CLIENT_SECRET",
            "GOOGLE_OAUTH_CLIENT_SECRET",
        ],
        extra_env,
    )
    if client_secret:
        materialized["IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET"] = client_secret[1]

    redirect_uri = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI",
            "GOOGLE_OAUTH_REDIRECT_URI",
        ],
        extra_env,
    )
    if redirect_uri:
        materialized["IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI"] = redirect_uri[1]

    hosted_domain = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_HOSTED_DOMAIN_HINT",
            "GOOGLE_ALLOWED_HD",
        ],
        extra_env,
    )
    if hosted_domain:
        materialized["IRONCLAW_REBORN_GOOGLE_HOSTED_DOMAIN_HINT"] = hosted_domain[1]

    return materialized, {
        "materialized": bool(materialized),
        "env_names": sorted(materialized),
        "client_id_source": client_id[0] if client_id else None,
        "client_id_from_stored_flow": bool(
            client_id and str(client_id[0]).startswith("stored_flow:")
        ),
        "client_secret_present": client_secret is not None,
        "redirect_uri_source": redirect_uri[0] if redirect_uri else "dynamic_serve_port",
        "hosted_domain_source": hosted_domain[0] if hosted_domain else None,
    }


def _parse_rfc3339(value: object) -> datetime | None:
    if not isinstance(value, str) or not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def _root_filesystem_secret_metadata_by_handle(
    db_path: Path,
    handle: str,
) -> dict[str, object] | None:
    if not handle:
        return None
    suffix = f"/{handle}.json"
    with sqlite3.connect(db_path) as db:
        rows = db.execute(
            "SELECT contents FROM root_filesystem_entries WHERE path LIKE ?",
            (f"%{suffix}",),
        ).fetchall()
    if len(rows) != 1:
        return None
    try:
        stored = json.loads(rows[0][0])
    except (TypeError, json.JSONDecodeError):
        return None
    return {
        "handle": stored.get("handle"),
        "expires_at": stored.get("expires_at"),
        "created_at": stored.get("created_at"),
        "updated_at": stored.get("updated_at"),
    }


def _google_oauth_refresh_probe(
    reborn_home: Path,
    db_path: Path,
    refresh_handle: str,
    extra_env: dict[str, str] | None,
) -> dict[str, object]:
    """Validate that the copied refresh token matches the configured client."""

    if os.environ.get("REBORN_WEBUI_V2_LIVE_QA_SKIP_GOOGLE_REFRESH_PROBE"):
        return {"checked": False, "skipped": True, "reason": "disabled_by_env"}

    client_id = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
            "GOOGLE_CLIENT_ID",
            "GOOGLE_OAUTH_CLIENT_ID",
        ],
        extra_env,
    )
    if not client_id:
        return {"checked": False, "ok": False, "error": "google_client_id_missing"}

    client_secret = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET",
            "GOOGLE_CLIENT_SECRET",
            "GOOGLE_OAUTH_CLIENT_SECRET",
        ],
        extra_env,
    )
    master_key_path = reborn_home / "local-dev" / ".reborn-local-dev-secrets-master-key"
    if not master_key_path.exists():
        return {
            "checked": True,
            "ok": False,
            "error": "reborn_secret_master_key_missing",
            "client_id_source": client_id[0],
            "client_secret_present": client_secret is not None,
        }

    try:
        refresh_token = _decrypt_filesystem_secret(
            master_key_path.read_text(encoding="utf-8").strip(),
            _root_filesystem_secret_by_handle(db_path, refresh_handle),
        )
    except Exception as exc:
        return {
            "checked": True,
            "ok": False,
            "error": "refresh_secret_unavailable",
            "error_type": type(exc).__name__,
            "client_id_source": client_id[0],
            "client_secret_present": client_secret is not None,
        }

    try:
        import httpx

        data = {
            "client_id": client_id[1],
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }
        if client_secret:
            data["client_secret"] = client_secret[1]
        response = httpx.post(
            "https://oauth2.googleapis.com/token",
            data=data,
            timeout=20.0,
        )
        try:
            payload = response.json()
        except ValueError:
            payload = {}
    except Exception as exc:
        return {
            "checked": True,
            "ok": False,
            "error": "google_oauth_refresh_request_failed",
            "error_type": type(exc).__name__,
            "client_id_source": client_id[0],
            "client_secret_present": client_secret is not None,
        }

    ok = (
        response.status_code < 400
        and isinstance(payload, dict)
        and bool(payload.get("access_token"))
    )
    result: dict[str, object] = {
        "checked": True,
        "ok": ok,
        "status_code": response.status_code,
        "client_id_source": client_id[0],
        "client_secret_present": client_secret is not None,
    }
    if isinstance(payload, dict):
        if payload.get("error"):
            result["oauth_error_code"] = payload.get("error")
        if ok:
            result["expires_in_seconds"] = payload.get("expires_in")
            result["scope_count"] = len(str(payload.get("scope") or "").split())
    if not ok and "oauth_error_code" not in result:
        result["error"] = "google_oauth_refresh_failed"
    return result


def _google_product_auth_preflight(
    reborn_home: Path,
    user_id: str,
    extra_env: dict[str, str] | None = None,
) -> dict[str, object]:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    env_status = _google_product_auth_env_status(extra_env)
    preflight: dict[str, object] = {
        "requires_google_product_auth": False,
        "db_present": db_path.exists(),
        "auth_user_id": user_id,
        "provider_env": env_status,
        "accounts": [],
        "ready": False,
    }
    if not db_path.exists():
        preflight["reason"] = "reborn local-dev db missing"
        return preflight
    account_pattern = (
        f"/tenants/reborn-cli/users/{user_id}/secrets/agents/reborn-cli-agent/"
        "product-auth/%/accounts/%.json"
    )
    with sqlite3.connect(db_path) as db:
        rows = db.execute(
            "SELECT path, contents FROM root_filesystem_entries WHERE path LIKE ?",
            (account_pattern,),
        ).fetchall()
    now = datetime.now(timezone.utc)
    accounts: list[dict[str, object]] = []
    for path, contents in rows:
        try:
            account = json.loads(contents)
        except (TypeError, json.JSONDecodeError):
            continue
        if account.get("provider") != "google":
            continue
        access_handle = str(
            account.get("access_secret") or account.get("access_secret_handle") or ""
        )
        refresh_handle = str(
            account.get("refresh_secret") or account.get("refresh_secret_handle") or ""
        )
        access_secret = _root_filesystem_secret_metadata_by_handle(db_path, access_handle)
        refresh_secret = _root_filesystem_secret_metadata_by_handle(db_path, refresh_handle)
        expires_at = access_secret.get("expires_at") if access_secret else None
        expires_dt = _parse_rfc3339(expires_at)
        expired = expires_dt is not None and expires_dt <= now
        scopes = account.get("scopes")
        if not isinstance(scopes, list):
            scopes = []
        has_usable_access_secret = access_secret is not None and not expired
        account_ready = (
            account.get("status") == "configured"
            and has_usable_access_secret
        )
        refresh_probe: dict[str, object] | None = None
        needs_refresh_probe = (
            account.get("status") == "configured"
            and refresh_secret is not None
            and bool(env_status["ready"])
            and not has_usable_access_secret
        )
        if needs_refresh_probe:
            refresh_probe = _google_oauth_refresh_probe(
                reborn_home,
                db_path,
                refresh_handle,
                extra_env,
            )
            account_ready = (
                bool(refresh_probe.get("ok"))
                or bool(refresh_probe.get("skipped"))
            )
        account_preflight = {
            "path": path,
            "id": account.get("id"),
            "status": account.get("status"),
            "ownership": account.get("ownership"),
            "surface": (
                account.get("scope", {}).get("surface")
                if isinstance(account.get("scope"), dict)
                else None
            ),
            "scope_count": len(scopes),
            "scopes": sorted(str(scope) for scope in scopes),
            "access_secret_present": access_secret is not None,
            "access_secret_expires_at": expires_at,
            "access_secret_expired": expired,
            "refresh_secret_present": refresh_secret is not None,
            "provider_env_ready": bool(env_status["ready"]),
            "ready_for_current_run": account_ready,
        }
        if refresh_probe is not None:
            account_preflight["refresh_probe"] = refresh_probe
        accounts.append(account_preflight)
    preflight["accounts"] = accounts
    configured_accounts = [
        account for account in accounts if account.get("status") == "configured"
    ]
    preflight["configured_account_count"] = len(configured_accounts)
    preflight["configured_ready"] = bool(configured_accounts)
    preflight["ready"] = any(
        account.get("ready_for_current_run") for account in configured_accounts
    )
    preflight["stable_refresh_ready"] = bool(env_status["ready"])
    if not configured_accounts:
        preflight["reason"] = "no configured Google product-auth account for WebUI user"
    elif not preflight["ready"]:
        expired = any(
            account.get("access_secret_expired") for account in configured_accounts
        )
        refresh_missing = any(
            not account.get("refresh_secret_present") for account in configured_accounts
        )
        refresh_probe_failures = [
            account.get("refresh_probe")
            for account in configured_accounts
            if isinstance(account.get("refresh_probe"), dict)
            and not account.get("refresh_probe", {}).get("ok")
        ]
        if refresh_probe_failures:
            probe = refresh_probe_failures[0]
            error = probe.get("oauth_error_code") or probe.get("error") or "unknown"
            if error == "invalid_request" and not probe.get("client_secret_present"):
                preflight["reason"] = (
                    "Google OAuth refresh client secret is missing for the copied "
                    "expired access token"
                )
                preflight["missing_google_client_secret"] = True
            else:
                preflight["reason"] = f"Google OAuth refresh probe failed: {error}"
            preflight["refresh_probe_failed"] = True
        elif expired and not env_status["ready"]:
            preflight["reason"] = (
                "configured Google account access token is expired and Google "
                "OAuth client id env is missing"
            )
        elif refresh_missing:
            preflight["reason"] = "configured Google account is missing refresh secret"
        else:
            preflight["reason"] = "configured Google product-auth account is not ready"
    return preflight


def _build_aad(domain: bytes, parts: list[bytes]) -> bytes:
    aad = bytearray(domain)
    for part in parts:
        aad.extend(len(part).to_bytes(8, "big"))
        aad.extend(part)
    return bytes(aad)


def _filesystem_secret_aad(scope: dict[str, object], handle: str) -> bytes:
    return _build_aad(
        b"reborn/v1/fs_secret_record",
        [
            str(scope.get("tenant_id") or "").encode(),
            str(scope.get("user_id") or "").encode(),
            str(scope.get("agent_id") or "").encode(),
            str(scope.get("project_id") or "").encode(),
            handle.encode(),
        ],
    )


def _root_filesystem_json(db_path: Path, path: str) -> dict[str, object]:
    with sqlite3.connect(db_path) as db:
        row = db.execute(
            "SELECT contents FROM root_filesystem_entries WHERE path = ?",
            (path,),
        ).fetchone()
    if not row:
        raise LiveQaError(f"expected Reborn root filesystem entry is missing: {path}")
    return json.loads(row[0])


def _root_filesystem_secret_by_handle(db_path: Path, handle: str) -> dict[str, object]:
    suffix = f"/{handle}.json"
    with sqlite3.connect(db_path) as db:
        rows = db.execute(
            "SELECT path, contents FROM root_filesystem_entries WHERE path LIKE ?",
            (f"%{suffix}",),
        ).fetchall()
    if len(rows) != 1:
        raise LiveQaError(
            f"expected exactly one Reborn secret record for handle {handle!r}, found {len(rows)}"
        )
    return json.loads(rows[0][1])


def _decrypt_filesystem_secret(master_key: str, stored: dict[str, object]) -> str:
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        from cryptography.hazmat.primitives.hashes import SHA256
        from cryptography.hazmat.primitives.kdf.hkdf import HKDF
    except ModuleNotFoundError as exc:
        raise LiveQaError(
            "Decrypting Slack secrets from the Reborn home requires the e2e "
            "Python dependency `cryptography`; rerun without SKIP_PYTHON_BOOTSTRAP "
            "or install tests/e2e dependencies."
        ) from exc

    handle = str(stored["handle"])
    scope = stored["scope"]
    if not isinstance(scope, dict):
        raise LiveQaError(f"secret record {handle!r} has invalid scope")
    encrypted_value = bytes(stored["encrypted_value"])  # type: ignore[arg-type]
    key_salt = bytes(stored["key_salt"])  # type: ignore[arg-type]
    if len(encrypted_value) < 28:
        raise LiveQaError(f"secret record {handle!r} is too short to decrypt")
    key = HKDF(
        algorithm=SHA256(),
        length=32,
        salt=key_salt,
        info=b"near-agent-secrets-v1",
    ).derive(master_key.encode())
    nonce = encrypted_value[:12]
    ciphertext = encrypted_value[12:]
    aad = _filesystem_secret_aad(scope, handle)
    plaintext = AESGCM(key).decrypt(nonce, ciphertext, aad)
    return plaintext.decode("utf-8")


def _encrypt_filesystem_secret(
    *,
    master_key: str,
    scope: dict[str, object],
    handle: str,
    plaintext: str,
) -> tuple[list[int], list[int]]:
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        from cryptography.hazmat.primitives.hashes import SHA256
        from cryptography.hazmat.primitives.kdf.hkdf import HKDF
    except ModuleNotFoundError as exc:
        raise LiveQaError(
            "Seeding Google OAuth secrets into a generated Reborn home requires "
            "the e2e Python dependency `cryptography`; rerun without "
            "SKIP_PYTHON_BOOTSTRAP or install tests/e2e dependencies."
        ) from exc

    key_salt = os.urandom(32)
    key = HKDF(
        algorithm=SHA256(),
        length=32,
        salt=key_salt,
        info=b"near-agent-secrets-v1",
    ).derive(master_key.encode())
    nonce = os.urandom(12)
    aad = _filesystem_secret_aad(scope, handle)
    ciphertext = AESGCM(key).encrypt(nonce, plaintext.encode("utf-8"), aad)
    return list(nonce + ciphertext), list(key_salt)


def _root_filesystem_create_table(db_path: Path) -> None:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(db_path) as db:
        db.execute(
            """
            CREATE TABLE IF NOT EXISTS root_filesystem_entries (
                path TEXT PRIMARY KEY,
                contents BLOB NOT NULL DEFAULT X'',
                is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1)),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
                kind TEXT,
                indexed TEXT NOT NULL DEFAULT '{}',
                version INTEGER NOT NULL DEFAULT 0
            )
            """
        )
        db.commit()


def _put_root_filesystem_json(db_path: Path, path: str, payload: dict[str, object]) -> None:
    now = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    contents = json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")
    with sqlite3.connect(db_path) as db:
        db.execute(
            """
            INSERT INTO root_filesystem_entries
                (path, contents, is_dir, created_at, updated_at, content_type, kind, indexed, version)
            VALUES
                (?, ?, 0, ?, ?, 'application/json', NULL, '{}', 0)
            ON CONFLICT(path) DO UPDATE SET
                contents = excluded.contents,
                updated_at = excluded.updated_at,
                content_type = excluded.content_type,
                version = root_filesystem_entries.version + 1
            """,
            (path, contents, now, now),
        )
        db.commit()


def _seed_generated_google_product_auth_if_configured(reborn_home: Path, user_id: str) -> dict[str, object]:
    access_token = env_secret("AUTH_LIVE_GOOGLE_ACCESS_TOKEN")
    refresh_token = env_secret("AUTH_LIVE_GOOGLE_REFRESH_TOKEN")
    client_id = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
            "GOOGLE_CLIENT_ID",
            "GOOGLE_OAUTH_CLIENT_ID",
        ],
        None,
    )
    preflight: dict[str, object] = {
        "checked": True,
        "seeded": False,
        "access_token_present": access_token is not None,
        "refresh_token_present": refresh_token is not None,
        "client_id_present": client_id is not None,
    }
    if not access_token or not refresh_token or not client_id:
        return preflight

    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    master_key_path = reborn_home / "local-dev" / ".reborn-local-dev-secrets-master-key"
    master_key_path.parent.mkdir(parents=True, exist_ok=True)
    if master_key_path.exists():
        master_key = master_key_path.read_text(encoding="utf-8").strip()
    else:
        master_key = hashlib.sha256(os.urandom(32)).hexdigest()
        master_key_path.write_text(master_key, encoding="utf-8")
        master_key_path.chmod(0o600)

    _root_filesystem_create_table(db_path)
    account_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/google/{user_id}",
        )
    )
    invocation_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/invocation/{user_id}",
        )
    )
    thread_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/thread/{user_id}",
        )
    )
    now = datetime.now(timezone.utc)
    now_s = now.isoformat().replace("+00:00", "Z")
    expired_s = (now.replace(microsecond=0)).isoformat().replace("+00:00", "Z")
    resource = {
        "tenant_id": "reborn-cli",
        "user_id": user_id,
        "agent_id": "reborn-cli-agent",
        "project_id": None,
        "thread_id": thread_id,
        "invocation_id": invocation_id,
        "mission_id": None,
    }
    secret_scope = {
        "tenant_id": "reborn-cli",
        "user_id": user_id,
        "agent_id": "reborn-cli-agent",
        "project_id": None,
        "thread_id": thread_id,
        "invocation_id": invocation_id,
        "mission_id": None,
    }
    access_handle = f"google-oauth-access-{account_id}-{invocation_id}"
    refresh_handle = f"google-oauth-refresh-{account_id}-{invocation_id}"
    secret_root = (
        f"/tenants/reborn-cli/users/{user_id}/secrets/agents/reborn-cli-agent/secrets"
    )
    for handle, token, expires_at in (
        (access_handle, access_token, expired_s),
        (refresh_handle, refresh_token, None),
    ):
        encrypted_value, key_salt = _encrypt_filesystem_secret(
            master_key=master_key,
            scope=secret_scope,
            handle=handle,
            plaintext=token,
        )
        _put_root_filesystem_json(
            db_path,
            f"{secret_root}/{handle}.json",
            {
                "handle": handle,
                "scope": secret_scope,
                "encrypted_value": encrypted_value,
                "key_salt": key_salt,
                "expires_at": expires_at,
                "created_at": now_s,
                "updated_at": now_s,
            },
        )

    scopes = [
        "https://www.googleapis.com/auth/calendar.events",
        "https://www.googleapis.com/auth/calendar.readonly",
        "https://www.googleapis.com/auth/documents",
        "https://www.googleapis.com/auth/documents.readonly",
        "https://www.googleapis.com/auth/drive",
        "https://www.googleapis.com/auth/drive.readonly",
        "https://www.googleapis.com/auth/gmail.modify",
        "https://www.googleapis.com/auth/gmail.readonly",
        "https://www.googleapis.com/auth/gmail.send",
        "https://www.googleapis.com/auth/presentations",
        "https://www.googleapis.com/auth/presentations.readonly",
        "https://www.googleapis.com/auth/spreadsheets",
        "https://www.googleapis.com/auth/spreadsheets.readonly",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
        "openid",
    ]
    account_path = (
        f"/tenants/reborn-cli/users/{user_id}/secrets/agents/reborn-cli-agent/"
        f"product-auth/callback/accounts/{account_id}.json"
    )
    _put_root_filesystem_json(
        db_path,
        account_path,
        {
            "id": account_id,
            "provider": "google",
            "label": "google",
            "status": "configured",
            "ownership": "user_reusable",
            "owner_extension": None,
            "granted_extensions": [],
            "scope": {
                "resource": resource,
                "surface": "callback",
            },
            "scopes": scopes,
            "access_secret": access_handle,
            "refresh_secret": refresh_handle,
            "created_at": now_s,
            "updated_at": now_s,
        },
    )
    preflight.update(
        {
            "seeded": True,
            "account_id": account_id,
            "scope_count": len(scopes),
            "account_path": account_path,
        }
    )
    return preflight


def _materialize_slack_env_from_reborn_home(
    reborn_home: Path,
    config_text: str,
) -> tuple[dict[str, str], dict[str, object]]:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    master_key_path = reborn_home / "local-dev" / ".reborn-local-dev-secrets-master-key"
    preflight: dict[str, object] = {
        "source": "reborn_home",
        "db_present": db_path.exists(),
        "master_key_present": master_key_path.exists(),
        "materialized": False,
    }
    if not db_path.exists() or not master_key_path.exists():
        return {}, preflight
    installation_path = "/tenants/reborn-cli/shared/slack-setup/installation.json"
    try:
        installation = _root_filesystem_json(db_path, installation_path)
    except LiveQaError:
        preflight["installation_present"] = False
        return {}, preflight
    preflight["installation_present"] = True
    bot_handle = str(installation.get("bot_token_handle") or "")
    signing_handle = str(installation.get("signing_secret_handle") or "")
    if not bot_handle or not signing_handle:
        preflight["handles_present"] = False
        return {}, preflight
    preflight["handles_present"] = True
    master_key = master_key_path.read_text(encoding="utf-8").strip()
    signing_secret = _decrypt_filesystem_secret(
        master_key,
        _root_filesystem_secret_by_handle(db_path, signing_handle),
    )
    bot_token = _decrypt_filesystem_secret(
        master_key,
        _root_filesystem_secret_by_handle(db_path, bot_handle),
    )
    signing_env = _section_env_name(
        config_text,
        "signing_secret_env",
        "IRONCLAW_REBORN_SLACK_SIGNING_SECRET",
    )
    bot_env = _section_env_name(
        config_text,
        "bot_token_env",
        "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
    )
    materialized = {
        signing_env: signing_secret,
        bot_env: bot_token,
    }
    preflight.update(
        {
            "materialized": True,
            "env_names": sorted(materialized),
            "installation_id": installation.get("installation_id"),
            "team_id": installation.get("team_id"),
            "api_app_id": installation.get("api_app_id"),
        }
    )
    return materialized, preflight


def _slack_auth_test(config_text: str, extra_env: dict[str, str]) -> dict[str, object]:
    bot_env = _section_env_name(
        config_text,
        "bot_token_env",
        "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
    )
    token = _env_value(bot_env, extra_env)
    if not token:
        return {
            "checked": False,
            "ok": False,
            "error": "bot token env unavailable",
            "bot_token_env": bot_env,
        }
    try:
        import httpx

        response = httpx.post(
            "https://slack.com/api/auth.test",
            headers={"Authorization": f"Bearer {token}"},
            timeout=20.0,
        )
        payload = response.json()
    except Exception as exc:
        return {
            "checked": True,
            "ok": False,
            "error": type(exc).__name__,
            "bot_token_env": bot_env,
        }
    result: dict[str, object] = {
        "checked": True,
        "ok": bool(payload.get("ok")),
        "bot_token_env": bot_env,
        "team_id": payload.get("team_id"),
        "user_id": payload.get("user_id"),
    }
    if not payload.get("ok"):
        result["error"] = payload.get("error")
        result["needed"] = payload.get("needed")
    return result


def _slack_team_id_from_bot_token_env(bot_token_env: str) -> str | None:
    token = env_secret(bot_token_env)
    if not token:
        return None
    try:
        import httpx

        response = httpx.post(
            "https://slack.com/api/auth.test",
            headers={"Authorization": f"Bearer {token}"},
            timeout=20.0,
        )
        payload = response.json()
    except Exception:
        return None
    if not payload.get("ok"):
        return None
    team_id = str(payload.get("team_id") or "").strip()
    return team_id or None


def prepare_reborn_home(args: argparse.Namespace, selected_cases: list[str]) -> PreparedRebornHome:
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
    if prepared_home.exists():
        shutil.rmtree(prepared_home)

    def _ignore(_dir: str, names: list[str]) -> set[str]:
        return {name for name in names if name.endswith(".lock")}

    shutil.copytree(source_home, prepared_home, ignore=_ignore)
    config_path = prepared_home / "config.toml"
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
    api_key_env = os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV",
        "NEARAI_API_KEY" if os.environ.get("NEARAI_API_KEY") else "LIVE_OPENAI_COMPATIBLE_API_KEY",
    )
    api_key = env_secret(api_key_env)
    if not api_key:
        raise LiveQaError(
            f"Reborn home does not exist and {api_key_env} is unset; "
            "set REBORN_WEBUI_V2_LIVE_QA_HOME or provide live LLM env."
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
    path.mkdir(parents=True, exist_ok=True)
    (path / "config.toml").write_text(
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
    google_seed = _seed_generated_google_product_auth_if_configured(path, _auth_user_id())
    github_seed = _seed_generated_github_product_auth_if_configured(path, _auth_user_id())
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


async def case_webui_auth_gate(ctx: LiveQaContext) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    case_name = "webui_auth_gate"

    async def action(page: object) -> None:
        await page.goto(f"{ctx.base_url}/v2/", wait_until="domcontentloaded")  # type: ignore[attr-defined]
        await expect(page.locator("#v2-token")).to_be_visible(timeout=15000)  # type: ignore[attr-defined]

    try:
        await _with_page(ctx.output_dir, case_name, action)
        return _result(case_name, True, started, {"checked": "/v2/ without token"})
    except Exception as exc:
        return _result(case_name, False, started, {"error": str(exc)})


async def case_webui_live_llm_chat(ctx: LiveQaContext) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    case_name = "webui_live_llm_chat"
    marker = "REBORN_WEBUI_V2_LIVE_QA_OK"

    async def action(page: object) -> None:
        await page.goto(
            f"{ctx.base_url}/v2/?token={AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )  # type: ignore[attr-defined]
        composer = page.locator("[data-testid='chat-composer']")  # type: ignore[attr-defined]
        await expect(composer).to_be_visible(timeout=15000)
        prompt = (
            "Live QA verification. Reply with exactly this token and no extra words: "
            f"{marker}"
        )
        await composer.fill(prompt)
        await composer.press("Enter")
        await expect(page.locator("[data-testid='msg-user']").last).to_contain_text(  # type: ignore[attr-defined]
            "Live QA verification",
            timeout=15000,
        )
        await expect(page.locator("[data-testid='msg-assistant']").last).to_contain_text(  # type: ignore[attr-defined]
            marker,
            timeout=120000,
        )

    try:
        await _with_page(ctx.output_dir, case_name, action)
        return _result(case_name, True, started, {"marker": marker})
    except Exception as exc:
        return _result(case_name, False, started, {"error": str(exc), "marker": marker})


async def case_webui_core_routes(ctx: LiveQaContext) -> ProbeResult:
    from playwright.async_api import expect

    started = time.monotonic()
    case_name = "webui_core_routes"
    routes = [
        ("/v2/workspace", "Workspace"),
        ("/v2/automations", "Automations"),
        ("/v2/extensions/registry", "Extensions"),
        ("/v2/settings/inference", "Settings"),
    ]

    async def action(page: object) -> None:
        for path, expected_text in routes:
            await page.goto(
                f"{ctx.base_url}{path}?token={AUTH_TOKEN}",
                wait_until="domcontentloaded",
            )  # type: ignore[attr-defined]
            await expect(page.locator("body")).to_contain_text(  # type: ignore[attr-defined]
                expected_text,
                timeout=15000,
            )

    try:
        await _with_page(ctx.output_dir, case_name, action)
        return _result(
            case_name,
            True,
            started,
            {"routes": [path for path, _ in routes]},
        )
    except Exception as exc:
        return _result(case_name, False, started, {"error": str(exc)})


async def _live_chat_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    prompt: str,
    marker: str,
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
            marker=marker,
            required_text=required_text,
            timeout=timeout,
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
    marker: str,
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
            marker=marker,
            required_text=required_text,
            timeout=timeout,
        )

    try:
        await _with_page(ctx.output_dir, case_name, action)
        return _result(case_name, True, started, observed)
    except Exception as exc:
        return _result(case_name, False, started, {"error": str(exc), **observed})


async def _wait_for_assistant_reply(
    page: object,
    *,
    marker: str,
    required_text: list[str],
    timeout: float,
) -> str:
    deadline = time.monotonic() + timeout
    assistant = page.locator("[data-testid='msg-assistant']").last  # type: ignore[attr-defined]
    last_text = ""
    while time.monotonic() < deadline:
        await _approve_visible_tool_gate(page)
        if await assistant.count() > 0:
            try:
                text = await assistant.inner_text(timeout=1000)
            except Exception:
                text = ""
            if text:
                last_text = text
            normalized = text.lower()
            if marker in text and all(piece.lower() in normalized for piece in required_text):
                return text[-2000:]
        await asyncio.sleep(0.5)
    main_text = ""
    try:
        main_text = await page.locator("main").inner_text(timeout=1000)  # type: ignore[attr-defined]
    except Exception:
        pass
    raise AssertionError(
        "assistant reply did not contain required text before timeout. "
        f"marker={marker!r} required_text={required_text!r} "
        f"last_assistant={last_text[-500:]!r} main_excerpt={main_text[-1000:]!r}"
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


async def case_qa_3b_endpoint_status_live_chat(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_3B_ENDPOINT_STATUS_DONE"
    url = "https://cloud-api.near.ai"
    live_status = await _live_http_status(url)
    return await _live_chat_case(
        ctx,
        case_name="qa_3b_endpoint_status_live_chat",
        prompt=(
            f"QA case 3B: check the current HTTP status for {url}. Use live HTTP "
            "or web capabilities if available. If the endpoint does not return 200, "
            "report the actual status code. In the final answer include the exact "
            f"marker {marker} and include the text status."
        ),
        marker=marker,
        required_text=["status", str(live_status)],
        extra_details={"endpoint_url": url, "expected_status_code": live_status},
    )


def _trigger_record_count(reborn_home: Path, routine_name: str) -> int:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return 0
    with sqlite3.connect(db_path) as db:
        cursor = db.execute(
            "SELECT COUNT(*) FROM trigger_records WHERE name = ?",
            (routine_name,),
        )
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
    observed: dict[str, object] = {}

    async def action(page: object) -> None:
        await page.goto(
            f"{ctx.base_url}/v2/extensions/registry?token={AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )  # type: ignore[attr-defined]
        await expect(page.locator("body")).to_contain_text("Extensions", timeout=15000)  # type: ignore[attr-defined]
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
        instructions = str(action_body.get("instructions") or "")
        if "Message the Slack app" not in instructions:
            raise AssertionError(f"unexpected Slack connect instructions: {instructions!r}")
        observed["slack_display_name"] = personal.get("display_name")
        observed["slack_connect_title"] = action_body.get("title")
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
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_2a_gmail_connect",
        package_id="gmail",
        display_name="Gmail",
        required_tools=["gmail.list_messages"],
        ensure_installed=True,
    )


async def case_qa_2b_calendar_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_2b_calendar_connect",
        package_id="google-calendar",
        display_name="Google Calendar",
        required_tools=["google-calendar.list_events"],
        ensure_installed=True,
    )


async def case_qa_2c_drive_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_2c_drive_connect",
        package_id="google-drive",
        display_name="Google Drive",
        required_tools=["google-drive.list_files"],
        ensure_installed=True,
    )


async def case_qa_2d_calendar_prep_live_chat(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_2D_CALENDAR_PREP_DONE"
    return await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_2d_calendar_prep_live_chat",
        marker=marker,
        required_text=["Calendar", "news"],
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
        prompt=(
            "QA case 2D: act as a meeting prep assistant. Use my live Google "
            "Calendar connection to inspect upcoming events, and use live web "
            "search for current NEAR AI news that could be useful context. If "
            "there are no upcoming events, say that directly. Do not create, "
            "update, or delete calendar events. In the final answer include the "
            f"exact marker {marker}, include the word Calendar, and include the "
            "word news."
        ),
        timeout=300.0,
    )


async def case_qa_2e_calendar_prep_email_routine(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_2E_CALENDAR_EMAIL_ROUTINE_DONE"
    routine_name = "reborn-qa-2e-calendar-prep-email"
    return await _routine_creation_case(
        ctx,
        case_name="qa_2e_calendar_prep_email_routine",
        routine_name=routine_name,
        marker=marker,
        required_text=["routine", "email"],
        prompt=(
            f"QA case 2E: create a routine named {routine_name}. Every weekday "
            "morning, inspect my connected Google Calendar for upcoming meetings, "
            "use connected Google Drive or Docs for relevant context when available, "
            "include current NEAR AI news if useful, and send the meeting-prep "
            "summary by Gmail email. Create the routine now; do not run it yet. "
            "Do not call Google, Gmail, Calendar, Drive, Docs, or auth tools now; "
            "only create the scheduled routine from these instructions. "
            f"In the final answer include the exact marker {marker} and include "
            "the words routine and email."
        ),
    )


async def case_qa_4a_gmail_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_4a_gmail_connect",
        package_id="gmail",
        display_name="Gmail",
        required_tools=["gmail.list_messages"],
        ensure_installed=True,
    )


async def case_qa_4b_github_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_4b_github_connect",
        package_id="github",
        display_name="GitHub",
        required_tools=["github.get_authenticated_user"],
        ensure_installed=True,
    )


async def case_qa_6a_gmail_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_6a_gmail_connect",
        package_id="gmail",
        display_name="Gmail",
        required_tools=["gmail.list_messages"],
        ensure_installed=True,
    )


async def case_qa_5b_drive_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_5b_drive_connect",
        package_id="google-drive",
        display_name="Google Drive",
        required_tools=["google-drive.list_files"],
        ensure_installed=True,
    )


async def case_qa_5c_strategy_doc_knowledge_base(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_5C_STRATEGY_DOC_DONE"
    strategy_phrase = "Reborn QA Strategy North Star: verify live WebUIv2 tool grounding."
    return await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_5c_strategy_doc_knowledge_base",
        marker=marker,
        required_text=["strategy", "WebUIv2", "grounding"],
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
            "QA case 5C: create a new Google Docs document titled "
            f"`{marker}` with this exact strategy sentence in the body: "
            f"{strategy_phrase} Then read the document content back through "
            "Google Docs and answer what the strategy north star is. In the "
            f"final answer include the exact marker {marker}, the word strategy, "
            "the word WebUIv2, and the word grounding."
        ),
        timeout=360.0,
        extra_details={"strategy_phrase": strategy_phrase},
        forbidden_text=[
            "auth_denied",
            "auth_required",
            "authentication required",
            "local file",
            "/workspace/",
            ".md",
            "can't create",
            "cannot create",
        ],
    )


async def case_qa_6b_sheets_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_6b_sheets_connect",
        package_id="google-sheets",
        display_name="Google Sheets",
        required_tools=["google-sheets.read_values"],
        ensure_installed=True,
    )


async def case_qa_6c_gmail_to_sheet_live_chat(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_6C_GMAIL_TO_SHEET_DONE"
    return await _live_chat_with_extensions_case(
        ctx,
        case_name="qa_6c_gmail_to_sheet_live_chat",
        marker=marker,
        required_text=["Gmail", "Google Sheet"],
        extensions=[
            {
                "package_id": "gmail",
                "display_name": "Gmail",
                "required_tools": ["gmail.list_messages"],
                "ensure_installed": False,
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
            "QA case 6C: use Gmail to inspect at most one recent inbox message, "
            "then create a new Google Sheet named "
            f"`{marker}` and write one row with columns Source, Summary, and "
            "QA Marker. Use the Gmail result if one is available; if no message "
            "is available, write Source as Gmail and Summary as no recent message "
            "available. In the final answer include the exact marker "
            f"{marker}, include the word Gmail, and include the phrase Google Sheet."
        ),
        timeout=360.0,
    )


async def case_qa_6d_gmail_to_sheet_routine(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_6D_GMAIL_TO_SHEET_ROUTINE_DONE"
    routine_name = "reborn-qa-6d-gmail-to-sheet"
    return await _routine_creation_case(
        ctx,
        case_name="qa_6d_gmail_to_sheet_routine",
        routine_name=routine_name,
        marker=marker,
        required_text=["routine", "Gmail"],
        prompt=(
            f"QA case 6D dry-run routine-definition test: create a routine named {routine_name}. "
            "When the routine runs later, it should check Gmail for new CRM or lead emails, "
            "extract the sender, company or account name if present, summary, and received time, "
            "then append one row to a Google Sheet CRM tracker. Create only the scheduled "
            "routine definition now; do not run it, inspect accounts, verify connections, "
            "or call Gmail, Google Sheets, Google auth, connector auth, or get_authenticated_user "
            "tools now. In the final answer include the "
            f"exact marker {marker} and include the words routine and Gmail."
        ),
    )


async def case_qa_7b_sheets_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _extension_authenticated_case(
        ctx,
        case_name="qa_7b_sheets_connect",
        package_id="google-sheets",
        display_name="Google Sheets",
        required_tools=["google-sheets.read_values"],
        ensure_installed=True,
    )


async def _routine_creation_case(
    ctx: LiveQaContext,
    *,
    case_name: str,
    prompt: str,
    marker: str,
    routine_name: str,
    required_text: list[str],
) -> ProbeResult:
    before_count = _trigger_record_count(ctx.reborn_home, routine_name)
    result = await _live_chat_case(
        ctx,
        case_name=case_name,
        prompt=prompt,
        marker=marker,
        required_text=required_text,
        timeout=180.0,
        extra_details={
            "routine_name": routine_name,
            "trigger_records_before": before_count,
        },
    )
    after_count = _trigger_record_count(ctx.reborn_home, routine_name)
    result.details["trigger_records_after"] = after_count
    if result.success and after_count <= before_count:
        result.success = False
        result.details["error"] = (
            f"assistant returned success marker but routine {routine_name!r} "
            "was not added to trigger_records"
        )
    return result


async def case_qa_3c_endpoint_status_slack_routine(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_3C_ENDPOINT_STATUS_ROUTINE_DONE"
    routine_name = "reborn-qa-3c-endpoint-status-slack"
    return await _routine_creation_case(
        ctx,
        case_name="qa_3c_endpoint_status_slack_routine",
        routine_name=routine_name,
        marker=marker,
        required_text=["routine"],
        prompt=(
            f"QA case 3C: create a routine named {routine_name}. Every 5 minutes, "
            "ping https://cloud-api.near.ai, check whether it returns HTTP 200, "
            "and send the result in a Slack DM. Create the routine now; do not run "
            "the check immediately. In the final answer include the exact marker "
            f"{marker} and include the text routine."
        ),
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
            f"run it immediately. In your final answer include the exact marker "
            f"{creation_marker} and include the text routine."
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
            "check https://cloud-api.near.ai with live HTTP or web access, report "
            "the observed HTTP status, and send the result to Slack"
        ),
        required_delivery_text=["status"],
    )


async def case_qa_4c_github_release_live_chat(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_4C_GITHUB_RELEASE_DONE"
    release = await _live_github_latest_release("nearai", "ironclaw")
    api_url = release["api_url"]
    return await _live_chat_case(
        ctx,
        case_name="qa_4c_github_release_live_chat",
        prompt=(
            "QA case 4C: perform exactly one public HTTP GET to "
            f"{api_url}. Do not use an authenticated GitHub connector, GitHub auth "
            "flow, save/download tools, or any other URL. Confirm that the live "
            f"response tag_name is {release['tag_name']}, then immediately final-answer "
            f"with the exact marker {marker}, the text GitHub, and the release tag "
            f"{release['tag_name']}."
        ),
        marker=marker,
        required_text=["GitHub", release["tag_name"]],
        timeout=240.0,
        extra_details=release,
    )


async def case_qa_4d_github_release_slack_routine(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_4D_GITHUB_RELEASE_SLACK_ROUTINE_DONE"
    routine_name = "reborn-qa-4d-github-release-slack"
    return await _routine_creation_case(
        ctx,
        case_name="qa_4d_github_release_slack_routine",
        routine_name=routine_name,
        marker=marker,
        required_text=["routine"],
        prompt=(
            f"QA case 4D: create a routine named {routine_name}. Every 5 minutes, "
            "check https://github.com/nearai/ironclaw for the latest releases and "
            "send a Slack message summarizing any new release. Create the routine "
            "now; do not run the check immediately. Do not call GitHub tools, "
            "GitHub auth, or connector auth tools now; only create the scheduled "
            "routine from these instructions. In the final answer include "
            f"the exact marker {marker} and include the text routine."
        ),
    )


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
    marker = f"REBORN_QA_7D_SLACK_BUG_TRIGGER_{suffix}"
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
        slack_user_id = str(slack.get("legacy_actor_user_id") or "U0REBORNQA")
        text = (
            f"bug: live QA signed Slack inbound test {marker}. "
            "This is a plain direct-message reply test; do not call tools, do not "
            "configure channels, and do not change delivery settings. "
            f"Answer directly with the exact marker {marker} and the word bug."
        )
        post_result = await _post_signed_slack_dm_event(
            ctx,
            channel_id=channel_id,
            user_id=slack_user_id,
            text=text,
            event_id=f"EvREBORNQA7D{suffix}",
        )
        observed["signed_event"] = post_result
        deadline = time.monotonic() + 180.0
        last_history: dict[str, object] | None = None
        while time.monotonic() < deadline:
            history = await _slack_history_contains_marker(
                ctx,
                channel_id=channel_id,
                marker=marker,
                oldest_epoch=wall_started,
                required_text=["bug"],
            )
            last_history = history
            if history.get("found"):
                observed["slack_history"] = history
                return _result(case_name, True, started, observed)
            await asyncio.sleep(2.0)
        raise AssertionError(
            "Slack reply marker was not observed after signed bug: event. "
            f"last_history={last_history!r}"
        )
    except Exception as exc:
        return _result(case_name, False, started, {"error": str(exc), **observed})


async def case_qa_7c_slack_bug_logger_routine(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_7C_SLACK_BUG_SHEET_ROUTINE_DONE"
    routine_name = "reborn-qa-7c-slack-bug-sheet"
    return await _routine_creation_case(
        ctx,
        case_name="qa_7c_slack_bug_logger_routine",
        routine_name=routine_name,
        marker=marker,
        required_text=["routine", "bug"],
        prompt=(
            f"QA case 7C: create a routine named {routine_name}. When a Slack "
            "message in my product channel starts with `bug:`, extract the bug "
            "summary, reporter, Slack timestamp, and current status, then append "
            "one row to my connected Google Sheet for product bug tracking. Create "
            "the routine now; do not trigger or run it yet. Do not call Slack, "
            "Google Sheets, Google auth, or connector auth tools now; only create "
            "the scheduled routine from these instructions. In the final answer "
            f"include the exact marker {marker} and include the words routine and bug."
        ),
    )


async def case_qa_7a_slack_product_channel_connect(ctx: LiveQaContext) -> ProbeResult:
    started = time.monotonic()
    observed: dict[str, object] = {}
    try:
        slack = _slack_preflight(ctx)
        observed.update(
            {
                "delivery_target_present": slack.get("delivery_target_present"),
                "route_configured_from_env": slack.get("route_configured_from_env"),
            }
        )
        if not slack.get("delivery_target_present"):
            raise AssertionError(
                "Slack product-channel route is not configured for this WebUI user"
            )
        connect_result = await _slack_connect_case(
            ctx,
            case_name="qa_7a_slack_product_channel_connect",
        )
        observed.update(connect_result.details)
        if not connect_result.success:
            raise AssertionError(str(connect_result.details.get("error") or connect_result.details))
        return _result("qa_7a_slack_product_channel_connect", True, started, observed)
    except Exception as exc:
        return _result(
            "qa_7a_slack_product_channel_connect",
            False,
            started,
            {"error": str(exc), **observed},
        )


async def case_qa_8b_hn_keyword_live_chat(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN8BHNSEARCHDONE"
    return await _live_chat_case(
        ctx,
        case_name="qa_8b_hn_keyword_live_chat",
        prompt=(
            "Task 8B: search Hacker News for recent posts mentioning IronClaw "
            "or NEAR AI. Use live web/search capabilities if available. In the final "
            f"answer copy this exact marker without changing it: {marker}. "
            "Also include the text Hacker News."
        ),
        marker=marker,
        required_text=["Hacker News"],
        timeout=240.0,
    )


async def case_qa_8a_slack_connect(ctx: LiveQaContext) -> ProbeResult:
    return await _slack_connect_case(ctx, case_name="qa_8a_slack_connect")


async def case_qa_8c_hn_keyword_slack_routine(ctx: LiveQaContext) -> ProbeResult:
    marker = "REBORN_QA_8C_HN_SLACK_ROUTINE_DONE"
    routine_name = "reborn-qa-8c-hn-keyword-slack"
    return await _routine_creation_case(
        ctx,
        case_name="qa_8c_hn_keyword_slack_routine",
        routine_name=routine_name,
        marker=marker,
        required_text=["routine"],
        prompt=(
            f"QA case 8C: create a routine named {routine_name}. Every hour, "
            "check Hacker News for new posts mentioning IronClaw or NEAR AI and "
            "send a summary to Slack. Create the routine now; do not run the "
            "search immediately. Do not call Slack delivery or auth tools now; "
            "only create the scheduled routine from these instructions. In the "
            "final answer include the exact marker "
            f"{marker} and include the text routine."
        ),
    )


async def case_qa_8d_hn_keyword_slack_delivery(ctx: LiveQaContext) -> ProbeResult:
    return await _slack_delivery_routine_case(
        ctx,
        case_name="qa_8d_hn_keyword_slack_delivery",
        routine_prefix="reborn-qa-8d-hn-keyword-slack-delivery",
        marker_prefix="REBORN_QA_8D_HN_KEYWORD",
        routine_instruction=(
            "perform one quick live Hacker News or public web check for IronClaw "
            "or NEAR AI mentions, then send a concise Slack message that includes "
            "Hacker News and either the finding or that no current matching item "
            "was found"
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


def _qa_row_sort_key(row_id: str) -> tuple[int, str]:
    match = re.match(r"^(\d+)([A-Z]+)$", row_id)
    if not match:
        return (9999, row_id)
    return (int(match.group(1)), match.group(2))


class CaseSpec:
    def __init__(
        self,
        fn: CaseFn,
        *,
        requires_slack: bool = False,
        requires_slack_target: bool = False,
        requires_google_product_auth: bool = False,
        requires_google_runtime_access: bool = False,
        requires_telegram: bool = False,
        requires_github_auth: bool = False,
        default_enabled: bool = True,
    ) -> None:
        self.fn = fn
        self.requires_slack = requires_slack
        self.requires_slack_target = requires_slack_target
        self.requires_google_product_auth = requires_google_product_auth
        self.requires_google_runtime_access = requires_google_runtime_access
        self.requires_telegram = requires_telegram
        self.requires_github_auth = requires_github_auth
        self.default_enabled = default_enabled


CASES: dict[str, CaseSpec] = {
    "qa_1a_telegram_connect": CaseSpec(
        _gated_case("qa_1a_telegram_connect"),
        requires_telegram=True,
        default_enabled=False,
    ),
    "qa_1b_telegram_near_news_chat": CaseSpec(
        _gated_case("qa_1b_telegram_near_news_chat"),
        requires_telegram=True,
        default_enabled=False,
    ),
    "qa_1c_telegram_near_news_routine": CaseSpec(
        _gated_case("qa_1c_telegram_near_news_routine"),
        requires_telegram=True,
        default_enabled=False,
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
        _gated_case("qa_2f_calendar_prep_email_delivery"),
        requires_google_product_auth=True,
        requires_google_runtime_access=True,
        default_enabled=False,
    ),
    "qa_3a_slack_connect": CaseSpec(
        case_qa_3a_slack_connect,
        requires_slack=True,
    ),
    "webui_auth_gate": CaseSpec(case_webui_auth_gate),
    "webui_live_llm_chat": CaseSpec(case_webui_live_llm_chat),
    "webui_core_routes": CaseSpec(case_webui_core_routes),
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
        _gated_case("qa_4e_github_release_email_delivery"),
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
        _gated_case("qa_5d_slack_strategy_doc_answer"),
        requires_slack=True,
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
        _gated_case("qa_6e_gmail_to_sheet_delivery"),
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
        _gated_case("qa_7e_slack_bug_sheet_delivery"),
        requires_slack=True,
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
    represented_rows = sorted(
        {
            row
            for case_data in QA_SHEET_CASES.values()
            for row in case_data.get("rows", [])
            if isinstance(row, str)
        },
        key=_qa_row_sort_key,
    )
    manifest = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "selected_cases": selected_cases,
        "default_cases": [
            name for name, spec in CASES.items() if spec.default_enabled
        ],
        "qa_sheet": {
            "url": "https://docs.google.com/spreadsheets/d/1IpioaRFnDw8cW4fj9vxg1pBRWN7swVQLRq1FqVlJAls/edit?gid=0#gid=0",
            "sheet": "Automated",
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
                "status": (
                    "default"
                    if spec.default_enabled
                    else "gated:requires_live_telegram"
                    if spec.requires_telegram
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


async def run_cases(args: argparse.Namespace) -> int:
    if args.all_cases:
        selected_cases = list(CASES)
    else:
        selected_cases = args.case or [
            name for name, spec in CASES.items() if spec.default_enabled
        ]
    args.output_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = write_case_manifest(args.output_dir, selected_cases)
    print(f"[reborn-webui-v2-live-qa] case_manifest={manifest_path}", flush=True)
    binary = _reborn_binary() if args.skip_build else build_reborn_binary()
    if not binary.exists():
        raise LiveQaError(
            f"ironclaw-reborn binary missing at {binary}; rerun without --skip-build"
        )
    prepared_home = prepare_reborn_home(args, selected_cases)
    preflight_path = write_preflight(args.output_dir, prepared_home)
    print(f"[reborn-webui-v2-live-qa] preflight={preflight_path}", flush=True)
    results: list[ProbeResult] = []
    first_base_url = ""
    for name in selected_cases:
        case_spec = CASES[name]
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
            result = _result(
                name,
                False,
                started,
                {
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
                },
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
    results_path = write_results(args.output_dir, results, first_base_url)
    print(f"[reborn-webui-v2-live-qa] results={results_path}", flush=True)
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
