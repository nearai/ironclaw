#!/usr/bin/env python3
"""
Insert encrypted secrets into IronClaw's PostgreSQL database.

Changes vs old version:
- Handles newer Postgres schemas where secrets.user_id is UUID
- Resolves "default" to a real user UUID when possible
- Gives clearer errors instead of silently assuming text user IDs
"""

import sys
import os
import uuid
from datetime import datetime, timezone

# pip install cryptography psycopg2-binary
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives import hashes

KEY_SIZE = 32
NONCE_SIZE = 12
SALT_SIZE = 32
HKDF_INFO = b"near-agent-secrets-v1"

DATABASE_URL = os.environ.get(
    "DATABASE_URL",
    "postgresql://ironclaw@127.0.0.1:5432/ironclaw"
)


def get_connection():
    import psycopg2
    return psycopg2.connect(DATABASE_URL)


def get_master_key():
    key = os.environ.get("IRONCLAW_MASTER_KEY")
    if not key:
        import subprocess
        try:
            result = subprocess.run(
                ["secret-tool", "lookup", "service", "ironclaw"],
                capture_output=True, text=True, timeout=5
            )
            if result.returncode == 0 and result.stdout.strip():
                key = result.stdout.strip()
        except Exception:
            pass

    if not key:
        for attrs in [
            ["application", "ironclaw"],
            ["service", "ironclaw", "type", "master-key"],
            ["service", "ironclaw-secrets"],
        ]:
            try:
                result = subprocess.run(
                    ["secret-tool", "lookup"] + attrs,
                    capture_output=True, text=True, timeout=5
                )
                if result.returncode == 0 and result.stdout.strip():
                    key = result.stdout.strip()
                    break
            except Exception:
                continue

    if not key:
        import getpass
        key = getpass.getpass("Master key (from keychain): ")

    if len(key) < KEY_SIZE:
        print(f"ERROR: Master key must be at least {KEY_SIZE} bytes, got {len(key)}")
        sys.exit(1)

    return key


def derive_key(master_key: bytes, salt: bytes) -> bytes:
    hkdf = HKDF(
        algorithm=hashes.SHA256(),
        length=KEY_SIZE,
        salt=salt,
        info=HKDF_INFO,
    )
    return hkdf.derive(master_key)


def encrypt(master_key: bytes, plaintext: bytes) -> tuple[bytes, bytes]:
    salt = os.urandom(SALT_SIZE)
    derived_key = derive_key(master_key, salt)

    aesgcm = AESGCM(derived_key)
    nonce = os.urandom(NONCE_SIZE)
    ciphertext = aesgcm.encrypt(nonce, plaintext, None)
    encrypted = nonce + ciphertext
    return encrypted, salt


def get_column_type(cursor, table_name: str, column_name: str) -> str | None:
    cursor.execute(
        """
        SELECT data_type
        FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = %s
          AND column_name = %s
        """,
        (table_name, column_name),
    )
    row = cursor.fetchone()
    return row[0] if row else None


def resolve_user_id(cursor, requested_user_id: str) -> str:
    """
    Handles both legacy TEXT user_id schemas and newer UUID user_id schemas.
    """
    user_id_type = get_column_type(cursor, "secrets", "user_id")

    # Legacy text schema or unknown: preserve old behavior
    if user_id_type != "uuid":
        return requested_user_id

    # New UUID schema
    env_user_id = os.environ.get("IRONCLAW_USER_ID")
    if requested_user_id == "default" and env_user_id:
        requested_user_id = env_user_id

    # If caller supplied a UUID explicitly, accept it
    if requested_user_id != "default":
        try:
            return str(uuid.UUID(requested_user_id))
        except ValueError:
            raise SystemExit(
                f"ERROR: secrets.user_id is UUID, but --user-id is not a valid UUID: {requested_user_id}"
            )

    # Try to resolve automatically from users table
    cursor.execute(
        """
        SELECT id::text
        FROM users
        ORDER BY created_at NULLS LAST, id
        LIMIT 2
        """
    )
    rows = cursor.fetchall()

    if len(rows) == 1:
        return rows[0][0]

    if len(rows) == 0:
        raise SystemExit(
            "ERROR: secrets.user_id is UUID, but no users were found in the users table. "
            "Pass --user-id <uuid> explicitly."
        )

    raise SystemExit(
        "ERROR: secrets.user_id is UUID and multiple users exist. "
        "Pass --user-id <uuid> explicitly, or set IRONCLAW_USER_ID."
    )


def insert_secret(name: str, value: str, user_id: str = "default"):
    master_key_str = get_master_key()
    master_key = master_key_str.encode("utf-8")

    plaintext = value.encode("utf-8")
    encrypted_value, key_salt = encrypt(master_key, plaintext)

    secret_id = str(uuid.uuid4())
    now = datetime.now(timezone.utc)

    conn = get_connection()
    cursor = conn.cursor()

    resolved_user_id = resolve_user_id(cursor, user_id)

    cursor.execute(
        """
        INSERT INTO secrets (
            id, user_id, name, encrypted_value, key_salt,
            provider, expires_at, created_at, updated_at
        )
        VALUES (%s, %s, %s, %s, %s, NULL, NULL, %s, %s)
        ON CONFLICT (user_id, name) DO UPDATE SET
            encrypted_value = EXCLUDED.encrypted_value,
            key_salt = EXCLUDED.key_salt,
            updated_at = EXCLUDED.updated_at
        """,
        (
            secret_id,
            resolved_user_id,
            name.lower(),
            encrypted_value,
            key_salt,
            now,
            now,
        ),
    )

    conn.commit()
    cursor.close()
    conn.close()

    print(f"OK: secret '{name}' stored for user '{resolved_user_id}'")
    print(f"    id={secret_id}")
    print(f"    encrypted_value={len(encrypted_value)} bytes")
    print(f"    key_salt={len(key_salt)} bytes")


def list_secrets(user_id: str = "default"):
    conn = get_connection()
    cursor = conn.cursor()

    resolved_user_id = resolve_user_id(cursor, user_id)

    cursor.execute(
        """
        SELECT name, provider, created_at, usage_count
        FROM secrets
        WHERE user_id = %s
        ORDER BY name
        """,
        (resolved_user_id,),
    )
    rows = cursor.fetchall()

    cursor.close()
    conn.close()

    if not rows:
        print(f"No secrets found for user '{resolved_user_id}'")
        return

    print(f"Secrets for user '{resolved_user_id}':")
    for name, provider, created_at, usage_count in rows:
        provider_str = f" (provider: {provider})" if provider else ""
        print(f"  - {name}{provider_str}  created={created_at}  used={usage_count}x")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage:")
        print("  python3 insert_secret_pg.py <name> <value> [--user-id <uuid|legacy_text>]")
        print("  python3 insert_secret_pg.py --list [--user-id <uuid|legacy_text>]")
        sys.exit(1)

    user_id = "default"
    if "--user-id" in sys.argv:
        idx = sys.argv.index("--user-id")
        if idx + 1 >= len(sys.argv):
            print("ERROR: --user-id requires a value")
            sys.exit(1)
        user_id = sys.argv[idx + 1]
        sys.argv.pop(idx)
        sys.argv.pop(idx)

    if sys.argv[1] == "--list":
        list_secrets(user_id)
    elif len(sys.argv) >= 3:
        insert_secret(sys.argv[1], sys.argv[2], user_id)
    else:
        print("ERROR: need both <name> and <value>")
        sys.exit(1)