"""Slack setup helpers for the Reborn WebUI v2 live QA runner."""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import sqlite3
import uuid
from contextlib import closing
from pathlib import Path
from datetime import datetime, timezone

from scripts.live_canary.common import env_secret
from scripts.reborn_webui_v2_live_qa.env_helpers import (
    _env_present,
    _env_value,
    _first_env_value,
)
from scripts.reborn_webui_v2_live_qa.errors import LiveQaError
from scripts.reborn_webui_v2_live_qa.root_filesystem import (
    _decrypt_filesystem_secret,
    _encrypt_filesystem_secret,
    _put_root_filesystem_json,
    _root_filesystem_create_table,
    _root_filesystem_json,
    _root_filesystem_secret_by_handle,
    _write_new_secret_file_0600,
)


SLACK_INSTALLATION_SETUP_PATH = "/tenants/reborn-cli/shared/slack-setup/installation.json"
SLACK_SIGNING_SECRET_ENV = "IRONCLAW_REBORN_SLACK_SIGNING_SECRET"
SLACK_BOT_TOKEN_ENV = "IRONCLAW_REBORN_SLACK_BOT_TOKEN"
SLACK_OAUTH_CLIENT_ID_ENV = "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_ID"
SLACK_OAUTH_CLIENT_SECRET_ENV = "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_SECRET"
SLACK_PERSONAL_ACCESS_TOKEN_ENV = "AUTH_LIVE_SLACK_ACCESS_TOKEN"
SLACK_PERSONAL_ACCESS_TOKEN_ENV_NAMES = [
    SLACK_PERSONAL_ACCESS_TOKEN_ENV,
    "AUTH_LIVE_SLACK_USER_TOKEN",
    "REBORN_WEBUI_V2_LIVE_QA_SLACK_USER_TOKEN",
]
# Optional SECOND human identity (a dedicated canary user, distinct from the
# connected personal account AND from the bot). Arms that strictly need a
# second HUMAN actor must assert this env and fail loudly when it is absent —
# never silently skip. Today no default-wired case hard-requires it; the bot
# token is actor B wherever a bot can act.
SLACK_SECOND_USER_TOKEN_ENV = "AUTH_LIVE_SLACK_SECOND_USER_TOKEN"
SLACK_PERSONAL_OAUTH_SCOPES = [
    "search:read",
    "channels:history",
    "groups:history",
    "im:history",
    "mpim:history",
    "channels:read",
    "groups:read",
    "im:read",
    "mpim:read",
    "users:read",
    "chat:write",
]
SIGNED_SLACK_EVENT_CASES = {
    "qa_5d_slack_strategy_doc_answer",
    "qa_7d_slack_bug_message_trigger",
    "qa_7e_slack_bug_sheet_delivery",
}
LEGACY_SLACK_SETUP_KEYS = {
    "installation_id",
    "team_id",
    "api_app_id",
    "slack_user_id",
    "user_id",
    "shared_subject_user_id",
    "signing_secret_env",
    "bot_token_env",
}


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


def _has_live_slack_env(extra_env: dict[str, str] | None = None) -> bool:
    return _env_present(SLACK_SIGNING_SECRET_ENV, extra_env) and _env_present(
        SLACK_BOT_TOKEN_ENV,
        extra_env,
    )


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


def _slack_inbound_user_id_for_cases(selected_cases: list[str]) -> str | None:
    if not SIGNED_SLACK_EVENT_CASES.intersection(selected_cases):
        return None
    slack_user_id = (
        _slack_dm_route_user_id()
        or os.environ.get(
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID",
            "U0REBORNQA",
        ).strip()
    )
    if not slack_user_id:
        return None
    return slack_user_id


def _discover_slack_dm_route_channel(extra_env: dict[str, str]) -> dict[str, object]:
    token = _env_value(SLACK_BOT_TOKEN_ENV, extra_env)
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


def _remove_legacy_slack_setup_fields(config_path: Path) -> dict[str, object]:
    if not config_path.exists():
        return {"changed": False, "removed_fields": [], "removed_channel_routes": 0}

    lines = config_path.read_text(encoding="utf-8").splitlines(keepends=True)
    rewritten: list[str] = []
    removed_fields: list[str] = []
    removed_channel_routes = 0
    in_slack = False
    index = 0
    while index < len(lines):
        line = lines[index]
        stripped = line.strip()
        if stripped == "[[slack.channel_routes]]":
            removed_channel_routes += 1
            index += 1
            while index < len(lines):
                next_line = lines[index]
                next_stripped = next_line.strip()
                if next_stripped.startswith("[") and next_stripped.endswith("]"):
                    break
                index += 1
            continue
        if stripped.startswith("[") and stripped.endswith("]"):
            in_slack = stripped == "[slack]"
            rewritten.append(line)
            index += 1
            continue
        if in_slack and "=" in line:
            key = line.split("=", 1)[0].strip()
            if key in LEGACY_SLACK_SETUP_KEYS:
                removed_fields.append(key)
                index += 1
                continue
        rewritten.append(line)
        index += 1

    changed = bool(removed_fields or removed_channel_routes)
    if changed:
        config_path.write_text("".join(rewritten), encoding="utf-8")
    return {
        "changed": changed,
        "removed_fields": sorted(set(removed_fields)),
        "removed_channel_routes": removed_channel_routes,
    }


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
        route.get("subject_user_id") == user_id
        and bool(route.get("channel_id"))
        and not str(route.get("channel_id") or "").startswith("D")
        for route in _slack_channel_routes(config_text)
    )


def _has_persisted_slack_personal_dm_target(reborn_home: Path, user_id: str) -> bool:
    return _persisted_slack_personal_dm_payload(reborn_home, user_id) is not None


def _persisted_slack_personal_dm_payload(reborn_home: Path, user_id: str) -> dict[str, object] | None:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    with closing(sqlite3.connect(db_path)) as db:
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


def _hash_scoped_part(hasher: "hashlib._Hash", value: str) -> None:
    encoded = value.encode("utf-8")
    hasher.update(len(encoded).to_bytes(8, "big"))
    hasher.update(encoded)


def _communication_preference_path(tenant_id: str, user_id: str) -> str:
    hasher = hashlib.sha256()
    hasher.update(b"v2:")
    for part in ("personal", tenant_id, user_id):
        _hash_scoped_part(hasher, part)
    return f"/tenants/{tenant_id}/users/{user_id}/outbound/communication-preferences/{hasher.hexdigest()}.json"


def _binding_segment(name: str, value: str) -> str:
    return f"{name}:{len(value.encode('utf-8'))}:{value};"


def _slack_host_state_path_segment(value: str) -> str:
    return base64.urlsafe_b64encode(value.encode("utf-8")).decode("ascii").rstrip("=")


def _slack_setup_from_reborn_home(reborn_home: Path) -> dict[str, object] | None:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return None
    try:
        return _root_filesystem_json(db_path, SLACK_INSTALLATION_SETUP_PATH)
    except LiveQaError:
        return None


def _slack_setup_field(
    setup: dict[str, object],
    config_text: str,
    field: str,
    env_name: str,
    default: str = "",
) -> str | None:
    value = str(setup.get(field) or "").strip()
    if value:
        return value
    value = (env_secret(env_name) or "").strip()
    if value:
        return value
    value = _slack_config_value(config_text, field)
    if value:
        return value
    return default or None


def _slack_setup_preflight(
    reborn_home: Path,
    config_text: str,
    extra_env: dict[str, str] | None = None,
) -> dict[str, object]:
    setup = _slack_setup_from_reborn_home(reborn_home) or {}
    installation_id = _slack_setup_field(
        setup,
        config_text,
        "installation_id",
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID",
    )
    team_id = _slack_setup_field(
        setup,
        config_text,
        "team_id",
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID",
    )
    api_app_id = _slack_setup_field(
        setup,
        config_text,
        "api_app_id",
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID",
    )
    stored_oauth_client_id = str(setup.get("oauth_client_id") or "").strip()
    env_oauth_client_id = (_env_value(SLACK_OAUTH_CLIENT_ID_ENV, extra_env) or "").strip()
    oauth_client_id = env_oauth_client_id or stored_oauth_client_id or None
    return {
        "source": "reborn_home" if setup else "env",
        "configured": bool(
            installation_id
            and team_id
            and api_app_id
            and _env_present(SLACK_BOT_TOKEN_ENV, extra_env)
            and _env_present(SLACK_SIGNING_SECRET_ENV, extra_env)
        ),
        "installation_id": installation_id,
        "team_id": team_id,
        "api_app_id": api_app_id,
        "oauth_client_id": oauth_client_id,
        "oauth_client_id_configured": bool(oauth_client_id),
        "oauth_client_secret_configured": _env_present(
            SLACK_OAUTH_CLIENT_SECRET_ENV,
            extra_env,
        ),
        "personal_oauth_ready": bool(
            oauth_client_id and _env_present(SLACK_OAUTH_CLIENT_SECRET_ENV, extra_env)
        ),
    }


def _slack_setup_payload(
    reborn_home: Path,
    config_text: str,
    extra_env: dict[str, str],
) -> tuple[dict[str, object] | None, dict[str, object]]:
    preflight = _slack_setup_preflight(reborn_home, config_text, extra_env)
    bot_token = _env_value(SLACK_BOT_TOKEN_ENV, extra_env)
    signing_secret = _env_value(SLACK_SIGNING_SECRET_ENV, extra_env)
    required = {
        "installation_id": preflight.get("installation_id"),
        "team_id": preflight.get("team_id"),
        "api_app_id": preflight.get("api_app_id"),
        "bot_token": bot_token,
        "signing_secret": signing_secret,
    }
    missing = [key for key, value in required.items() if not str(value or "").strip()]
    if missing:
        return None, {**preflight, "ready_for_api": False, "missing": missing}
    payload: dict[str, object] = {
        "installation_id": str(required["installation_id"]),
        "team_id": str(required["team_id"]),
        "api_app_id": str(required["api_app_id"]),
        "bot_token": bot_token,
        "signing_secret": signing_secret,
    }
    oauth_client_id = str(preflight.get("oauth_client_id") or "").strip()
    oauth_client_secret = _env_value(SLACK_OAUTH_CLIENT_SECRET_ENV, extra_env)
    if oauth_client_id:
        payload["oauth_client_id"] = oauth_client_id
    if oauth_client_secret:
        payload["oauth_client_secret"] = oauth_client_secret
    return payload, {**preflight, "ready_for_api": True, "missing": []}


def _seed_slack_personal_identity_binding(
    db_path: Path,
    *,
    installation_id: str,
    user_id: str,
    slack_user_id: str,
    now: str,
) -> dict[str, object]:
    provider = "slack"
    provider_user_id = f"{installation_id}:{slack_user_id}"
    identity_path = (
        "/tenants/reborn-cli/shared/slack-personal-binding/identities/"
        f"{_slack_host_state_path_segment(provider)}/"
        f"{_slack_host_state_path_segment(provider_user_id)}.json"
    )
    index_path = (
        "/tenants/reborn-cli/shared/slack-personal-binding/identities-by-user/"
        f"{_slack_host_state_path_segment(provider)}/"
        f"{_slack_host_state_path_segment(user_id)}/"
        f"{_slack_host_state_path_segment(provider_user_id)}.json"
    )
    _put_root_filesystem_json(
        db_path,
        identity_path,
        {
            "provider": provider,
            "provider_user_id": provider_user_id,
            "user_id": user_id,
            "created_at": now,
            "updated_at": now,
        },
    )
    _put_root_filesystem_json(
        db_path,
        index_path,
        {"provider_user_id": provider_user_id},
    )
    return {
        "identity_path": identity_path,
        "index_path": index_path,
        "provider_user_id": provider_user_id,
    }


def _slack_personal_dm_reply_target(
    *,
    installation_id: str,
    team_id: str,
    user_id: str,
    slack_user_id: str,
    dm_channel_id: str,
) -> str:
    return "reply:" + "".join(
        [
            _binding_segment("adapter", "slack_v2"),
            _binding_segment("installation", installation_id),
            _binding_segment("agent", "reborn-cli-agent"),
            _binding_segment("project", ""),
            _binding_segment("space", team_id),
            _binding_segment("conversation", dm_channel_id),
            _binding_segment("topic", ""),
            _binding_segment("actor_kind", "slack_user"),
            _binding_segment("actor", slack_user_id),
        ]
    )


def _seed_slack_outbound_default_target(
    db_path: Path,
    *,
    installation_id: str,
    team_id: str,
    user_id: str,
    slack_user_id: str,
    dm_channel_id: str,
    now: str,
) -> dict[str, object]:
    tenant_id = "reborn-cli"
    path = _communication_preference_path(tenant_id, user_id)
    final_reply_target = _slack_personal_dm_reply_target(
        installation_id=installation_id,
        team_id=team_id,
        user_id=user_id,
        slack_user_id=slack_user_id,
        dm_channel_id=dm_channel_id,
    )
    _put_root_filesystem_json(
        db_path,
        path,
        {
            "scope": {
                "kind": "personal",
                "tenant_id": tenant_id,
                "user_id": user_id,
            },
            "final_reply_target": final_reply_target,
            "progress_target": None,
            "approval_prompt_target": None,
            "auth_prompt_target": None,
            "default_modality": None,
            "updated_at": now,
            "updated_by": user_id,
        },
    )
    return {
        "path": path,
        "final_reply_target": final_reply_target,
    }


def _seed_slack_personal_dm_target(
    reborn_home: Path,
    config_text: str,
    *,
    auth_user_id: str,
    slack_user_id: str,
    dm_channel_id: str,
) -> dict[str, object]:
    setup = _slack_setup_from_reborn_home(reborn_home) or {}
    installation_id = _slack_setup_field(
        setup,
        config_text,
        "installation_id",
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID",
    )
    team_id = _slack_setup_field(
        setup,
        config_text,
        "team_id",
        "REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID",
    )
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
        "/tenants/reborn-cli/shared/slack-personal-binding/dm-targets/"
        f"{_slack_host_state_path_segment(installation_id)}/"
        f"{_slack_host_state_path_segment(team_id)}/"
        f"{_slack_host_state_path_segment(auth_user_id)}.json"
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
    identity_binding = _seed_slack_personal_identity_binding(
        db_path,
        installation_id=installation_id,
        user_id=auth_user_id,
        slack_user_id=slack_user_id,
        now=now,
    )
    outbound_default = _seed_slack_outbound_default_target(
        db_path,
        installation_id=installation_id,
        team_id=team_id,
        user_id=auth_user_id,
        slack_user_id=slack_user_id,
        dm_channel_id=dm_channel_id,
        now=now,
    )
    return {
        "seeded": True,
        "path": path,
        "identity_binding": identity_binding,
        "outbound_default": outbound_default,
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
    try:
        installation = _root_filesystem_json(db_path, SLACK_INSTALLATION_SETUP_PATH)
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
    materialized = {
        SLACK_SIGNING_SECRET_ENV: signing_secret,
        SLACK_BOT_TOKEN_ENV: bot_token,
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
    token = _env_value(SLACK_BOT_TOKEN_ENV, extra_env)
    if not token:
        return {
            "checked": False,
            "ok": False,
            "error": "bot token env unavailable",
            "bot_token_env": SLACK_BOT_TOKEN_ENV,
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
            "bot_token_env": SLACK_BOT_TOKEN_ENV,
        }
    result: dict[str, object] = {
        "checked": True,
        "ok": bool(payload.get("ok")),
        "bot_token_env": SLACK_BOT_TOKEN_ENV,
        "team_id": payload.get("team_id"),
        "user_id": payload.get("user_id"),
    }
    if not payload.get("ok"):
        result["error"] = payload.get("error")
        result["needed"] = payload.get("needed")
    return result


def _slack_user_token_auth_test(
    token: str,
    *,
    token_source: str,
) -> dict[str, object]:
    if not token:
        return {
            "checked": False,
            "ok": False,
            "error": "Slack user token unavailable",
            "token_source": token_source,
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
            "token_source": token_source,
        }
    result: dict[str, object] = {
        "checked": True,
        "ok": bool(payload.get("ok")),
        "token_source": token_source,
        "team_id": payload.get("team_id"),
        "user_id": payload.get("user_id"),
        "url": payload.get("url"),
    }
    if not payload.get("ok"):
        result["error"] = payload.get("error")
        result["needed"] = payload.get("needed")
    return result


def _slack_env_field(
    names: list[str],
    extra_env: dict[str, str] | None = None,
) -> tuple[str, str] | None:
    return _first_env_value(names, extra_env)


def _slack_personal_auth_preflight(
    reborn_home: Path,
    user_id: str,
    extra_env: dict[str, str] | None = None,
    *,
    requires_slack_personal_auth: bool,
) -> dict[str, object]:
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    master_key_path = reborn_home / "local-dev" / ".reborn-local-dev-secrets-master-key"
    token_env = _slack_env_field(SLACK_PERSONAL_ACCESS_TOKEN_ENV_NAMES, extra_env)
    preflight: dict[str, object] = {
        "requires_slack_personal_auth": requires_slack_personal_auth,
        "ready": False,
        "db_present": db_path.exists(),
        "master_key_present": master_key_path.exists(),
        "token_env_present": token_env is not None,
        "token_env_names": SLACK_PERSONAL_ACCESS_TOKEN_ENV_NAMES,
        "token_env_source": token_env[0] if token_env else None,
        "configured_account_count": 0,
        "accounts": [],
    }
    if not db_path.exists():
        if requires_slack_personal_auth:
            preflight["reason"] = "no Slack personal product-auth DB is present"
        return preflight

    account_path_prefix = (
        f"/tenants/reborn-cli/users/{user_id}/secrets/agents/reborn-cli-agent/"
        "product-auth/"
    )
    account_pattern = f"{account_path_prefix}%/accounts/%.json"
    with closing(sqlite3.connect(db_path)) as db:
        try:
            rows = db.execute(
                """
                SELECT path, contents FROM root_filesystem_entries
                WHERE path LIKE ?
                ORDER BY path
                """,
                (account_pattern,),
            ).fetchall()
        except sqlite3.Error:
            rows = []

    master_key = None
    if master_key_path.exists():
        master_key = master_key_path.read_text(encoding="utf-8").strip()

    accounts: list[dict[str, object]] = []
    for path, raw in rows:
        if not str(path).startswith(account_path_prefix):
            continue
        try:
            account = json.loads(raw)
        except (TypeError, json.JSONDecodeError):
            continue
        if account.get("provider") != "slack_personal" or account.get("status") != "configured":
            continue
        scope = account.get("scope")
        resource = scope.get("resource") if isinstance(scope, dict) else None
        account_user_id = (
            str(resource.get("user_id") or "").strip()
            if isinstance(resource, dict)
            else ""
        )
        if account_user_id != user_id:
            continue
        access_handle = str(
            account.get("access_secret") or account.get("access_secret_handle") or ""
        ).strip()
        account_preflight: dict[str, object] = {
            "path": str(path),
            "id": account.get("id"),
            "status": account.get("status"),
            "user_id": account_user_id or None,
            "thread_id": (
                str(resource.get("thread_id") or "").strip()
                if isinstance(resource, dict)
                else ""
            )
            or None,
            "invocation_id": (
                str(resource.get("invocation_id") or "").strip()
                if isinstance(resource, dict)
                else ""
            )
            or None,
            "access_secret_present": bool(access_handle),
            "ready": False,
        }
        if access_handle and master_key:
            try:
                stored = _root_filesystem_secret_by_handle(db_path, access_handle)
                token = _decrypt_filesystem_secret(master_key, stored)
                auth_test = _slack_user_token_auth_test(
                    token,
                    token_source=f"product_auth:{access_handle}",
                )
                account_preflight["auth_test"] = auth_test
                account_preflight["ready"] = bool(auth_test.get("ok"))
            except Exception as exc:
                account_preflight["auth_test"] = {
                    "checked": True,
                    "ok": False,
                    "error": type(exc).__name__,
                }
        elif access_handle:
            account_preflight["auth_test"] = {
                "checked": False,
                "ok": False,
                "error": "Slack personal secret master key unavailable",
            }
        else:
            account_preflight["auth_test"] = {
                "checked": False,
                "ok": False,
                "error": "Slack personal account has no access secret",
            }
        accounts.append(account_preflight)

    ready_accounts = [account for account in accounts if account.get("ready")]
    preflight["accounts"] = accounts
    preflight["configured_account_count"] = len(accounts)
    preflight["ready"] = bool(ready_accounts)
    if ready_accounts:
        first_auth = ready_accounts[0].get("auth_test")
        if isinstance(first_auth, dict):
            preflight["auth_test"] = first_auth
    elif requires_slack_personal_auth:
        if not accounts:
            preflight["reason"] = "no configured Slack personal product-auth account"
        else:
            first_auth = accounts[0].get("auth_test")
            reason = (
                first_auth.get("error")
                if isinstance(first_auth, dict)
                else "Slack personal product-auth account is not ready"
            )
            preflight["reason"] = f"Slack personal product-auth auth.test failed: {reason}"
    return preflight


def _seed_generated_slack_product_auth_if_configured(
    reborn_home: Path,
    user_id: str,
    extra_env: dict[str, str] | None = None,
) -> dict[str, object]:
    selected = _slack_env_field(SLACK_PERSONAL_ACCESS_TOKEN_ENV_NAMES, extra_env)
    preflight: dict[str, object] = {
        "checked": True,
        "seeded": False,
        "token_env_present": selected is not None,
        "token_env_names": SLACK_PERSONAL_ACCESS_TOKEN_ENV_NAMES,
        "token_env_source": selected[0] if selected else None,
    }
    if not selected:
        return preflight

    auth_test = _slack_user_token_auth_test(selected[1], token_source=selected[0])
    preflight["auth_test"] = auth_test
    if not auth_test.get("ok"):
        preflight["reason"] = (
            "Slack personal user token auth.test failed: "
            f"{auth_test.get('error') or 'unknown Slack auth error'}"
        )
        return preflight

    team_id = str(auth_test.get("team_id") or "").strip()
    slack_user_id = str(auth_test.get("user_id") or "").strip()
    installation = _slack_env_field(
        [
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID",
            "IRONCLAW_REBORN_SLACK_INSTALLATION_ID",
        ],
        extra_env,
    )
    api_app = _slack_env_field(
        [
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID",
            "IRONCLAW_REBORN_SLACK_APP_ID",
            "IRONCLAW_REBORN_SLACK_API_APP_ID",
        ],
        extra_env,
    )
    if not team_id or not slack_user_id or not installation or not api_app:
        missing = []
        if not team_id:
            missing.append("team_id_from_auth_test")
        if not slack_user_id:
            missing.append("user_id_from_auth_test")
        if not installation:
            missing.append("REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID")
        if not api_app:
            missing.append("REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID")
        preflight["reason"] = "missing Slack personal seed fields: " + ", ".join(missing)
        preflight["missing"] = missing
        return preflight

    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    master_key_path = reborn_home / "local-dev" / ".reborn-local-dev-secrets-master-key"
    master_key_path.parent.mkdir(parents=True, exist_ok=True)
    if master_key_path.exists():
        master_key = master_key_path.read_text(encoding="utf-8").strip()
    else:
        master_key = hashlib.sha256(os.urandom(32)).hexdigest()
        _write_new_secret_file_0600(master_key_path, master_key)

    _root_filesystem_create_table(db_path)
    account_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/slack_personal/{user_id}",
        )
    )
    invocation_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/slack-personal-invocation/{user_id}",
        )
    )
    thread_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"ironclaw-reborn-webui-v2-live-qa/slack-personal-thread/{user_id}",
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
    access_handle = f"slack-personal-oauth-access-{account_id}-{invocation_id}"
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
            "provider": "slack_personal",
            "label": "slack_personal",
            "status": "configured",
            "ownership": "user_reusable",
            "owner_extension": None,
            "granted_extensions": [],
            "scope": {
                "resource": resource,
                "surface": "callback",
            },
            "scopes": SLACK_PERSONAL_OAUTH_SCOPES,
            "access_secret": access_handle,
            "refresh_secret": None,
            "provider_identity": {
                "subject": slack_user_id,
                "team_id": team_id,
                "enterprise_id": None,
                "app_id": api_app[1],
            },
            "created_at": now_s,
            "updated_at": now_s,
        },
    )
    identity_binding = _seed_slack_personal_identity_binding(
        db_path,
        installation_id=installation[1],
        user_id=user_id,
        slack_user_id=slack_user_id,
        now=now_s,
    )
    preflight.update(
        {
            "seeded": True,
            "account_id": account_id,
            "account_path": account_path,
            "identity_binding": identity_binding,
            "thread_id": thread_id,
            "invocation_id": invocation_id,
            "scope_count": len(SLACK_PERSONAL_OAUTH_SCOPES),
            "team_id": team_id,
            "slack_user_id": slack_user_id,
        }
    )
    return preflight


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
