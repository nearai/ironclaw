"""Google product-auth helpers for Reborn WebUI v2 live QA."""

from __future__ import annotations

import hashlib
import json
import os
import sqlite3
import urllib.parse
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from scripts.live_canary.common import env_secret
from scripts.reborn_webui_v2_live_qa.env_helpers import (
    _env_present,
    _first_env_value,
    _non_empty_env,
    _section_env_name,
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

DEFAULT_USER_ID = "reborn-webui-v2-live-qa-user"


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


def _google_refresh_probe_error(preflight: dict[str, object]) -> str | None:
    accounts = preflight.get("accounts")
    if not isinstance(accounts, list):
        return None
    for account in accounts:
        if not isinstance(account, dict):
            continue
        refresh_probe = account.get("refresh_probe")
        if not isinstance(refresh_probe, dict) or refresh_probe.get("ok"):
            continue
        error = refresh_probe.get("oauth_error_code") or refresh_probe.get("error")
        if error:
            return str(error)
    return None


def _google_refresh_probe_missing_client_secret(preflight: dict[str, object]) -> bool:
    accounts = preflight.get("accounts")
    if not isinstance(accounts, list):
        return False
    for account in accounts:
        if not isinstance(account, dict):
            continue
        refresh_probe = account.get("refresh_probe")
        if not isinstance(refresh_probe, dict) or refresh_probe.get("ok"):
            continue
        if refresh_probe.get("client_secret_present") is False:
            return True
    return False


def _google_credential_action_for_block(preflight: dict[str, object]) -> str | None:
    if _google_refresh_probe_error(preflight) == "invalid_grant":
        return (
            "Rotate AUTH_LIVE_GOOGLE_ACCESS_TOKEN and AUTH_LIVE_GOOGLE_REFRESH_TOKEN "
            "from the live Google QA account using the same OAuth client configured "
            "by IRONCLAW_REBORN_GOOGLE_CLIENT_ID and "
            "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET."
        )
    if (
        preflight.get("missing_google_client_secret")
        or _google_refresh_probe_missing_client_secret(preflight)
    ):
        return (
            "Set IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET to the Google Cloud OAuth "
            "client secret matching IRONCLAW_REBORN_GOOGLE_CLIENT_ID."
        )
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


def _google_oauth_client_pair(
    extra_env: dict[str, str] | None = None,
) -> tuple[tuple[str, str] | None, tuple[str, str] | None]:
    client_id = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
            "GOOGLE_CLIENT_ID",
            "GOOGLE_OAUTH_CLIENT_ID",
        ],
        extra_env,
    )
    client_secret = _first_env_value(
        [
            "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET",
            "GOOGLE_CLIENT_SECRET",
            "GOOGLE_OAUTH_CLIENT_SECRET",
        ],
        extra_env,
    )
    return client_id, client_secret


def _google_runtime_access_token(
    reborn_home: Path,
    user_id: str,
    extra_env: dict[str, str] | None = None,
) -> tuple[str, dict[str, object]]:
    env_access_token = _first_env_value(
        [
            "AUTH_LIVE_GOOGLE_ACCESS_TOKEN",
            "IRONCLAW_REBORN_GOOGLE_ACCESS_TOKEN",
        ],
        extra_env,
    )

    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    master_key_path = reborn_home / "local-dev" / ".reborn-local-dev-secrets-master-key"
    if not db_path.exists() or not master_key_path.exists():
        if env_access_token:
            return env_access_token[1], {
                "source": env_access_token[0],
                "refreshed": False,
                "account_id": None,
            }
        raise LiveQaError("Google runtime token unavailable: Reborn DB or secret key missing")

    account_pattern = (
        f"/tenants/reborn-cli/users/{user_id}/secrets/agents/reborn-cli-agent/"
        "product-auth/%/accounts/%.json"
    )
    with sqlite3.connect(db_path) as db:
        account_rows = db.execute(
            "SELECT path, contents FROM root_filesystem_entries WHERE path LIKE ?",
            (account_pattern,),
        ).fetchall()
    now = datetime.now(timezone.utc)
    master_key = master_key_path.read_text(encoding="utf-8").strip()
    client_id, client_secret = _google_oauth_client_pair(extra_env)
    if not client_id:
        stored_client_id = _stored_google_oauth_client_id_from_reborn_home(reborn_home)
        if stored_client_id:
            client_id = stored_client_id

    errors: list[str] = []
    for _path, raw in account_rows:
        try:
            account = json.loads(raw)
        except (TypeError, json.JSONDecodeError):
            continue
        if account.get("provider") != "google" or account.get("status") != "configured":
            continue
        account_id = str(account.get("id") or "")
        access_handle = str(
            account.get("access_secret") or account.get("access_secret_handle") or ""
        )
        refresh_handle = str(
            account.get("refresh_secret") or account.get("refresh_secret_handle") or ""
        )
        access_secret = _root_filesystem_secret_metadata_by_handle(db_path, access_handle)
        expires_at = access_secret.get("expires_at") if access_secret else None
        expires_dt = _parse_rfc3339(expires_at)
        if access_secret and (expires_dt is None or expires_dt > now):
            return _decrypt_filesystem_secret(
                master_key,
                _root_filesystem_secret_by_handle(db_path, access_handle),
            ), {
                "source": "reborn_product_auth_access_secret",
                "refreshed": False,
                "account_id": account_id,
                "access_secret_expired": False,
            }
        if not refresh_handle:
            errors.append(f"account {account_id or '<unknown>'} has no refresh secret")
            continue
        if not client_id or not client_secret:
            raise LiveQaError(
                "Google runtime token unavailable: copied access token is expired "
                "and Google OAuth client id/secret env is incomplete"
            )
        refresh_token = _decrypt_filesystem_secret(
            master_key,
            _root_filesystem_secret_by_handle(db_path, refresh_handle),
        )
        try:
            import httpx

            response = httpx.post(
                "https://oauth2.googleapis.com/token",
                data={
                    "client_id": client_id[1],
                    "client_secret": client_secret[1],
                    "grant_type": "refresh_token",
                    "refresh_token": refresh_token,
                },
                timeout=20.0,
            )
            payload = response.json()
        except Exception as exc:
            errors.append(
                f"account {account_id or '<unknown>'} refresh request failed: {type(exc).__name__}"
            )
            continue
        access_token = str(payload.get("access_token") or "")
        if response.status_code < 400 and access_token:
            return access_token, {
                "source": "reborn_product_auth_refresh_secret",
                "refreshed": True,
                "account_id": account_id,
                "expires_in_seconds": payload.get("expires_in"),
                "scope_count": len(str(payload.get("scope") or "").split()),
            }
        errors.append(
            "account "
            f"{account_id or '<unknown>'} refresh failed with "
            f"{payload.get('error') or response.status_code}"
        )

    if env_access_token and not errors:
        return env_access_token[1], {
            "source": env_access_token[0],
            "refreshed": False,
            "account_id": None,
        }

    raise LiveQaError(
        "Google runtime token unavailable: "
        + ("; ".join(errors) if errors else "no configured Google account found")
    )


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
            if not probe.get("client_secret_present"):
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
        _write_new_secret_file_0600(master_key_path, master_key)

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
