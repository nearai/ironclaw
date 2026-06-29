"""Slack setup helpers for the Reborn WebUI v2 live QA runner."""

from __future__ import annotations

import json
import os
import re
import sqlite3
from pathlib import Path

from scripts.live_canary.common import env_secret
from scripts.reborn_webui_v2_live_qa.env_helpers import (
    _env_present,
    _env_value,
    _section_env_name,
)
from scripts.reborn_webui_v2_live_qa.errors import LiveQaError
from scripts.reborn_webui_v2_live_qa.root_filesystem import (
    _decrypt_filesystem_secret,
    _root_filesystem_json,
    _root_filesystem_secret_by_handle,
)


def _toml_string(value: str) -> str:
    return json.dumps(value)


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
    if _config_has_slack_channel_route(
        config,
        subject_user_id=route_subject_user_id,
        channel_id=channel_id,
    ):
        return True
    with config_path.open("a", encoding="utf-8") as fh:
        fh.write(
            "\n[[slack.channel_routes]]\n"
            f"channel_id = {_toml_string(channel_id)}\n"
            f"subject_user_id = {_toml_string(route_subject_user_id)}\n"
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
            rendered = f"{key} = {_toml_string(value)}"
            if line.strip() == rendered:
                return False
            lines[index] = rendered
            config_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
            return True
    if slack_header_index is None:
        return False
    if insert_index is None:
        insert_index = len(lines)
    lines.insert(insert_index, f"{key} = {_toml_string(value)}")
    config_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return True


def _configure_slack_legacy_actor_if_needed(
    config_path: Path, selected_cases: list[str]
) -> tuple[bool, str | None]:
    signed_slack_event_cases = {
        "qa_5d_slack_strategy_doc_answer",
        "qa_7d_slack_bug_message_trigger",
        "qa_7e_slack_bug_sheet_delivery",
    }
    if not signed_slack_event_cases.intersection(selected_cases):
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


def _slack_channel_routes(config_text: str) -> list[dict[str, str]]:
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
    return routes


def _config_has_slack_channel_route(
    config_text: str,
    *,
    subject_user_id: str,
    channel_id: str,
) -> bool:
    return any(
        route.get("subject_user_id") == subject_user_id
        and route.get("channel_id") == channel_id
        for route in _slack_channel_routes(config_text)
    )


def _config_has_slack_channel_route_for_user(config_text: str, user_id: str) -> bool:
    return any(
        route.get("subject_user_id") == user_id and bool(route.get("channel_id"))
        for route in _slack_channel_routes(config_text)
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
