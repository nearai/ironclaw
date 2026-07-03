"""Slack setup helpers for the Reborn WebUI v2 live QA runner."""

from __future__ import annotations

import json
import os
import re
import sqlite3
from pathlib import Path
from datetime import datetime, timezone

from scripts.live_canary.common import env_secret
from scripts.reborn_webui_v2_live_qa.env_helpers import (
    _env_present,
    _env_value,
    _section_env_name,
)
from scripts.reborn_webui_v2_live_qa.errors import LiveQaError
from scripts.reborn_webui_v2_live_qa.root_filesystem import (
    _decrypt_filesystem_secret,
    _put_root_filesystem_json,
    _root_filesystem_create_table,
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
    dm_user_id = _slack_dm_route_user_id()
    if not dm_user_id:
        return {
            "checked": False,
            "ok": False,
            "error": "missing_slack_route_user_id",
            "required_env": [
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID",
            ],
        }
    try:
        import httpx

        response = httpx.post(
            "https://slack.com/api/conversations.open",
            headers={"Authorization": f"Bearer {token}"},
            data={"users": dm_user_id},
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
        "dm_user_id": dm_user_id,
        "dm_user_source": "env",
    }
    channel = payload.get("channel")
    if isinstance(channel, dict):
        channel_id = str(channel.get("id") or "").strip()
        if channel_id:
            result["channel_id"] = channel_id
        result["channel_is_im"] = channel.get("is_im")
    channel_id = str(result.get("channel_id") or "")
    if payload.get("ok") and not channel_id.startswith("D"):
        result["ok"] = False
        result["error"] = "slack_conversations_open_returned_non_dm_channel"
    if payload.get("ok") and result.get("channel_is_im") is False:
        result["ok"] = False
        result["error"] = "slack_conversations_open_returned_non_im_channel"
    if not payload.get("ok"):
        result["error"] = payload.get("error")
        result["needed"] = payload.get("needed")
    return result


def _slack_dm_route_user_id() -> str | None:
    for name in (
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID",
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID",
    ):
        value = (env_secret(name) or "").strip()
        if value and value != "U0REBORNQA":
            return value
    return None


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


def _remove_dm_slack_channel_routes(config_path: Path) -> dict[str, object]:
    """Remove stale DM routes from legacy shared-channel route config."""
    if not config_path.exists():
        return {"changed": False, "removed": 0}

    lines = config_path.read_text(encoding="utf-8").splitlines(keepends=True)
    rewritten: list[str] = []
    removed = 0
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.strip() != "[[slack.channel_routes]]":
            rewritten.append(line)
            index += 1
            continue

        block = [line]
        index += 1
        while index < len(lines):
            next_line = lines[index]
            stripped = next_line.strip()
            if stripped.startswith("[") and stripped.endswith("]"):
                break
            block.append(next_line)
            index += 1

        if _slack_channel_route_block_has_dm_channel(block):
            removed += 1
        else:
            rewritten.extend(block)

    if not removed:
        return {"changed": False, "removed": 0}
    config_path.write_text("".join(rewritten), encoding="utf-8")
    return {"changed": True, "removed": removed}


def _slack_channel_route_block_has_dm_channel(block: list[str]) -> bool:
    for line in block:
        match = re.match(r"\s*channel_id\s*=\s*['\"]([^'\"]*)['\"]", line)
        if match and match.group(1).strip().startswith("D"):
            return True
    return False


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
    return _persisted_slack_personal_dm_payload(reborn_home, user_id) is not None


def _persisted_slack_personal_dm_payload(reborn_home: Path, user_id: str) -> dict[str, object] | None:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    with sqlite3.connect(db_path) as db:
        rows = db.execute(
            "SELECT contents FROM root_filesystem_entries "
            "WHERE path LIKE '%/slack-personal-binding/dm-targets/%' "
            "ORDER BY path"
        ).fetchall()
    for row in rows:
        try:
            payload = json.loads(row[0])
        except (TypeError, json.JSONDecodeError):
            continue
        if isinstance(payload, dict) and payload.get("user_id") == user_id:
            return payload
    return None


def _persisted_slack_personal_dm_channel_id(reborn_home: Path, user_id: str) -> str | None:
    payload = _persisted_slack_personal_dm_payload(reborn_home, user_id)
    if payload is None:
        return None
    channel_id = str(payload.get("dm_channel_id") or "").strip()
    return channel_id or None


def _seed_slack_personal_dm_target(
    reborn_home: Path,
    config_text: str,
    *,
    auth_user_id: str,
    slack_user_id: str,
    dm_channel_id: str,
) -> dict[str, object]:
    installation_id = _slack_config_value(config_text, "installation_id")
    team_id = _slack_config_value(config_text, "team_id")
    if not installation_id or not team_id:
        return {
            "seeded": False,
            "error": "missing_slack_installation_or_team_id",
            "installation_id_present": bool(installation_id),
            "team_id_present": bool(team_id),
        }
    if not dm_channel_id.startswith("D"):
        return {
            "seeded": False,
            "error": "slack_dm_channel_id_must_start_with_d",
            "dm_channel_id": dm_channel_id,
        }
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    _root_filesystem_create_table(db_path)
    now = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    path = (
        "/tenant-shared/slack-personal-binding/dm-targets/"
        f"{installation_id}/{team_id}/{auth_user_id}.json"
    )
    _put_root_filesystem_json(
        db_path,
        path,
        {
            "tenant_id": "reborn-cli",
            "installation_id": installation_id,
            "team_id": team_id,
            "user_id": auth_user_id,
            "slack_user_id": slack_user_id,
            "dm_channel_id": dm_channel_id,
            "created_at": now,
            "updated_at": now,
        },
    )
    return {
        "seeded": True,
        "path": path,
        "installation_id": installation_id,
        "team_id": team_id,
        "user_id": auth_user_id,
        "slack_user_id": slack_user_id,
        "dm_channel_id": dm_channel_id,
    }


def _has_slack_delivery_target(config_text: str, reborn_home: Path, user_id: str) -> bool:
    return _has_persisted_slack_personal_dm_target(reborn_home, user_id)


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
