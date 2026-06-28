"""Helpers for Reborn local root-filesystem rows used by live QA fixtures."""

from __future__ import annotations

import json
import os
import sqlite3
from datetime import datetime, timezone
from pathlib import Path

from scripts.reborn_webui_v2_live_qa.errors import LiveQaError


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


def _write_new_secret_file_0600(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    fd = os.open(path, flags, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as fh:
        fh.write(value)


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
