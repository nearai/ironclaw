#!/usr/bin/env python3
"""
Insert encrypted secrets into IronClaw's libSQL database.

Replicates exactly what `ironclaw tool setup` would do:
  1. HKDF-SHA256 to derive a per-secret key from the master key + random salt
  2. AES-256-GCM encrypt the secret value
  3. Insert into the secrets table

Usage:
  python3 insert_secret.py <name> <value> [--user-id default]

Example:
  python3 insert_secret.py gotify_app_token "Axxxxxxxxxx"
  python3 insert_secret.py gotify_url "https://gotify.darkc.sobe.world"
"""

import sys
import os
import uuid
import sqlite3
from datetime import datetime, timezone

# pip install cryptography
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives import hashes

# Constants matching ironclaw src/secrets/crypto.rs
KEY_SIZE = 32
NONCE_SIZE = 12
SALT_SIZE = 32
HKDF_INFO = b"near-agent-secrets-v1"

DB_PATH = os.path.expanduser("~/.ironclaw/ironclaw.db")


def get_master_key():
    """Get master key from environment or prompt."""
    key = os.environ.get("IRONCLAW_MASTER_KEY")
    if not key:
        # Try to read from keychain via secret-tool
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
        # Try other common keychain attribute patterns
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
    """HKDF-SHA256 key derivation, matching ironclaw's derive_key()."""
    hkdf = HKDF(
        algorithm=hashes.SHA256(),
        length=KEY_SIZE,
        salt=salt,
        info=HKDF_INFO,
    )
    return hkdf.derive(master_key)


def encrypt(master_key: bytes, plaintext: bytes) -> tuple[bytes, bytes]:
    """
    Encrypt a secret value.
    Returns (encrypted_value, salt) where:
      encrypted_value = nonce (12) || ciphertext || tag (16)
    Matches ironclaw SecretsCrypto::encrypt()
    """
    salt = os.urandom(SALT_SIZE)
    derived_key = derive_key(master_key, salt)

    aesgcm = AESGCM(derived_key)
    nonce = os.urandom(NONCE_SIZE)

    # AES-GCM encrypt (ciphertext includes the 16-byte tag)
    ciphertext = aesgcm.encrypt(nonce, plaintext, None)

    # Combine: nonce || ciphertext (with tag)
    encrypted = nonce + ciphertext

    return encrypted, salt


def insert_secret(name: str, value: str, user_id: str = "default"):
    """Insert an encrypted secret into the database."""
    master_key_str = get_master_key()
    master_key = master_key_str.encode("utf-8")

    plaintext = value.encode("utf-8")
    encrypted_value, key_salt = encrypt(master_key, plaintext)

    secret_id = str(uuid.uuid4())
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"

    conn = sqlite3.connect(DB_PATH)
    cursor = conn.cursor()

    # Upsert: insert or update if exists
    cursor.execute(
        """
        INSERT INTO secrets (id, user_id, name, encrypted_value, key_salt, provider, expires_at, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, NULL, NULL, ?, ?)
        ON CONFLICT (user_id, name) DO UPDATE SET
            encrypted_value = excluded.encrypted_value,
            key_salt = excluded.key_salt,
            updated_at = excluded.updated_at
        """,
        (secret_id, user_id, name.lower(), encrypted_value, key_salt, now, now),
    )

    conn.commit()
    conn.close()

    print(f"OK: secret '{name}' stored for user '{user_id}'")
    print(f"    id={secret_id}")
    print(f"    encrypted_value={len(encrypted_value)} bytes")
    print(f"    key_salt={len(key_salt)} bytes")


def list_secrets(user_id: str = "default"):
    """List existing secrets (names only, no values)."""
    conn = sqlite3.connect(DB_PATH)
    cursor = conn.cursor()
    cursor.execute(
        "SELECT name, provider, created_at, usage_count FROM secrets WHERE user_id = ? ORDER BY name",
        (user_id,),
    )
    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(f"No secrets found for user '{user_id}'")
        return

    print(f"Secrets for user '{user_id}':")
    for name, provider, created_at, usage_count in rows:
        provider_str = f" (provider: {provider})" if provider else ""
        print(f"  - {name}{provider_str}  created={created_at}  used={usage_count}x")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage:")
        print("  python3 insert_secret.py <name> <value> [--user-id default]")
        print("  python3 insert_secret.py --list [--user-id default]")
        sys.exit(1)

    user_id = "default"
    if "--user-id" in sys.argv:
        idx = sys.argv.index("--user-id")
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
