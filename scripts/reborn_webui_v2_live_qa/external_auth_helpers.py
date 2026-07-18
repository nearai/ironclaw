"""Telegram and GitHub auth helpers for Reborn WebUI v2 live QA."""

from __future__ import annotations

import hashlib
import json
import os
import sqlite3
import uuid
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path

from scripts.reborn_webui_v2_live_qa.env_helpers import (
    _env_present,
    _first_env_value,
)
from scripts.reborn_webui_v2_live_qa.root_filesystem import (
    _encrypt_filesystem_secret,
    _put_root_filesystem_json,
    _root_filesystem_create_table,
    _write_new_secret_file_0600,
)

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
    """Live prerequisites for the reborn `telegram` channel extension.

    The shipped model is admin setup via
    ``PUT /api/webchat/v2/channels/telegram/setup`` with the bot token as the
    only secret (the webhook shared secret is minted server-side per save
    revision and ``setWebhook`` registers
    ``/webhooks/extensions/telegram/updates``), then per-user pairing via
    ``/api/webchat/v2/channels/telegram/pairing``. The bot token env is
    therefore the credential prerequisite; the v1 monolith WASM channel
    artifacts (``channels-src/telegram``) are no longer part of the reborn
    lane. ``TELEGRAM_WEBHOOK_SECRET`` presence is reported for diagnostics
    only — the reborn host never consumes an operator-supplied webhook
    secret.
    """
    copied_home_mentions = False
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if db_path.exists():
        with closing(sqlite3.connect(db_path)) as db:
            row = db.execute(
                "SELECT COUNT(*) FROM root_filesystem_entries "
                "WHERE LOWER(path) LIKE '%telegram%' OR LOWER(CAST(contents AS TEXT)) LIKE '%telegram%'"
            ).fetchone()
        copied_home_mentions = bool(row and int(row[0]) > 0)
    bot_token_present = _env_present("TELEGRAM_BOT_TOKEN", extra_env)
    ready = bool(bot_token_present)
    reason = None
    if not ready:
        reason = "missing Telegram live prerequisites: TELEGRAM_BOT_TOKEN"
    return {
        "requires_telegram": requires_telegram,
        "ready": ready,
        "reason": reason,
        "bot_token_present": bot_token_present,
        "webhook_secret_present": _env_present("TELEGRAM_WEBHOOK_SECRET", extra_env),
        "chat_id_present": _env_present("REBORN_WEBUI_V2_LIVE_QA_TELEGRAM_CHAT_ID", extra_env),
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
        with closing(sqlite3.connect(db_path)) as db:
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
        _write_new_secret_file_0600(master_key_path, master_key)

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
