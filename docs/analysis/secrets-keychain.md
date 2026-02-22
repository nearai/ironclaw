# IronClaw Codebase Analysis — Secrets Management & Keychain

> Updated: 2026-02-22 | Version: v0.9.0

## 1. Overview

IronClaw implements a multi-layer secrets management system designed around a single core principle: **secrets are never exposed to WASM tools or the LLM context under any circumstances**. The system combines OS keychain integration for protecting the master encryption key, an AES-256-GCM encrypted database store for the secrets themselves, and a credential injection boundary that resolves and injects secrets into outbound HTTP requests at the host level — completely transparently to sandboxed tool code.

The architecture draws a hard boundary between the "host side" (trusted Rust code that can decrypt) and the "WASM side" (untrusted tool code that can only declare what it needs). Even if a WASM tool or the LLM that drives it is compromised or manipulated through prompt injection, neither has any path to read raw credential values.

The public API surface of the `secrets` module (`src/secrets/mod.rs`) deliberately limits what is exported: tools can check whether a secret `exists`, and the runtime can `list` references by name — but `get_decrypted` is an internal operation exercised only by the host-side injection path, never by tool callbacks.

## 2. Secrets Architecture

```
Priority (highest to lowest):
1. Environment variables (OPENAI_API_KEY=..., SECRETS_MASTER_KEY=...)
2. OS Keychain (macOS Keychain Services / Linux Secret Service via D-Bus)
3. Encrypted secrets store (PostgreSQL or libSQL — AES-256-GCM ciphertext)
4. Absent → SecretError::NotFound or SecretError::Expired
```

The config layer (`src/config/helpers.rs`) embodies priority 1 explicitly: `optional_env()` reads the real process environment first, then falls back to the `INJECTED_VARS` overlay — a thread-safe in-memory map populated by the secrets system for values that arrived from the database. Real environment variables always win over injected ones.

At rest, individual secrets live in the database as `encrypted_value` (AES-256-GCM ciphertext, layout: `nonce || ciphertext || tag`) plus a `key_salt` (32 random bytes used for HKDF key derivation). The master key that unlocks derivation lives in the OS keychain or arrives via `SECRETS_MASTER_KEY`. No single stolen artifact — neither the database dump nor the keychain entry alone — is sufficient to decrypt secrets without the other.

## 3. OS Keychain Integration (`keychain.rs`)

`src/secrets/keychain.rs` provides four async operations — `store_master_key`, `get_master_key`, `delete_master_key`, `has_master_key` — compiled conditionally for each supported platform through a `mod platform` block selected by `#[cfg(target_os = ...)]`.

The keychain is used exclusively for the **master key** (not for individual application secrets). The master key is 32 random bytes, stored as a lowercase hex string (64 characters). Both implementations share the same logical identity for the stored item:

```
Service name : "ironclaw"
Account name : "master_key"
```

### macOS (security-framework crate)

On macOS the `security-framework` crate calls directly into macOS Keychain Services (`SecKeychainFindGenericPassword` / `SecKeychainAddGenericPassword`). The IronClaw entry is a **generic password** item (as opposed to an internet password).

Key details from the implementation:

- **Storage**: `set_generic_password("ironclaw", "master_key", key_hex_bytes)` — stores the hex-encoded key bytes as the password field
- **Retrieval**: `get_generic_password("ironclaw", "master_key")` — returns the raw bytes, decoded from hex back to `Vec<u8>`
- **Deletion**: `delete_generic_password("ironclaw", "master_key")` — removes the item entirely
- **Existence check**: attempts a `get_generic_password` call and returns `is_ok()`; no dedicated exists API is needed since the keychain call is cheap

To inspect the stored entry from the command line on macOS:

```bash
# Inspect the entry (prompts for Keychain unlock)
security find-generic-password -s ironclaw -a master_key

# Print the password value
security find-generic-password -s ironclaw -a master_key -w

# Delete the entry (forces re-generation on next startup)
security delete-generic-password -s ironclaw -a master_key
```

The hex encoding round-trip is handled by a shared `hex_to_bytes` function that validates even-length input and valid hex characters before parsing.

### Linux (secret-service crate)

On Linux the `secret-service` crate communicates with a D-Bus-based secret store — GNOME Keyring or KWallet, depending on the desktop session. The connection uses Diffie-Hellman session encryption (`EncryptionType::Dh`) so that the secret value is not exposed on the bus in plaintext.

Key details from the implementation:

- **Connection**: `SecretService::connect(EncryptionType::Dh).await` — establishes an encrypted D-Bus session with the default secret service daemon
- **Collection**: `get_default_collection()` — uses the system default collection ("Login" in GNOME Keyring, "kdewallet" in KWallet); unlocked automatically if currently locked
- **Item attributes**: each item is tagged with `[("service", "ironclaw"), ("account", "master_key")]`; this matches the macOS generic password model
- **Label**: `"ironclaw master key"` — the human-readable label visible in Seahorse / KWallet Manager
- **Upsert**: `create_item(..., replace: true)` — replaces existing item on re-initialization
- **Retrieval**: `search_items([("service", ...), ("account", ...)])` — returns both unlocked and locked result sets; prefers the unlocked set, falls back to locked and unlocks on demand

### Unsupported Platforms

The fallback `mod platform` returns `SecretError::KeychainError` for all operations and instructs the operator to set `SECRETS_MASTER_KEY` as an environment variable instead. The `has_master_key()` function always returns `false` on unsupported platforms.

## 4. Encrypted Secrets Store (`store.rs`, `crypto.rs`)

### When It Is Used

The encrypted store holds application secrets that cannot or should not live in process environment variables: user-supplied API keys, OAuth tokens, service passwords, and any credential that must survive across sessions. Environment variables are process-scoped and ephemeral; the encrypted store is persistent and per-user.

### Storage Layout

Secrets are stored in a `secrets` table (both PostgreSQL and libSQL backends). The relevant columns per row are:

| Column | Type | Content |
|---|---|---|
| `id` | UUID | Unique secret identifier |
| `user_id` | String | Owner namespace |
| `name` | String | Human-readable identifier |
| `encrypted_value` | Bytes | `nonce (12 B) \|\| ciphertext \|\| GCM tag (16 B)` |
| `key_salt` | Bytes | 32-byte random salt for HKDF |
| `provider` | String? | Optional hint (e.g., "openai") |
| `expires_at` | Timestamp? | Optional TTL |
| `last_used_at` | Timestamp? | Usage tracking |
| `usage_count` | i64 | Injection counter |

### Encryption (`crypto.rs`)

`SecretsCrypto` is the struct that holds the master key (as a `SecretString`) and exposes `encrypt` / `decrypt` operations. The master key must be at least 32 bytes; shorter keys are rejected at construction time with `SecretError::InvalidMasterKey`.

**Algorithm stack:**

- **AES-256-GCM** (`aes-gcm` crate): authenticated encryption providing both confidentiality and integrity. The 128-bit GCM authentication tag detects any tampering with the ciphertext before decryption proceeds.
- **HKDF-SHA256** (`hkdf` crate): key derivation function that maps the master key + a per-secret salt into a unique 256-bit derived key. The HKDF info string is `b"near-agent-secrets-v1"` — a fixed context label that domain-separates this usage.
- **Random nonce generation**: each encryption call uses `Aes256Gcm::generate_nonce(&mut OsRng)` — 12 bytes from the OS CSPRNG. Nonces are single-use per (key, nonce) pair; because each secret also gets a fresh derived key, nonce reuse across secrets is structurally impossible even without the per-nonce randomization.

**Encryption flow:**

```
Master Key (SecretString, ≥32 bytes)
    │
    ├─ random salt (32 bytes, per encrypt call)
    │
    ▼
HKDF-SHA256(ikm=master_key, salt=salt, info="near-agent-secrets-v1")
    │
    ▼
Derived Key (32 bytes, unique per secret per call)
    │
    ├─ random nonce (12 bytes, OsRng)
    │
    ▼
AES-256-GCM encrypt(key=derived_key, nonce=nonce, plaintext=secret_bytes)
    │
    ▼
Stored: key_salt || (nonce || ciphertext || GCM_tag)
        ─────────   ────────────────────────────────
        key_salt col         encrypted_value col
```

**Decryption flow:**

```
encrypted_value → split at byte 12 → (nonce, ciphertext_with_tag)
key_salt + master_key → HKDF → derived_key
AES-256-GCM decrypt(key=derived_key, nonce=nonce, ciphertext_with_tag)
    → plaintext bytes → DecryptedSecret
```

**Per-secret key isolation:** Because every secret has its own randomly-generated 32-byte salt, HKDF produces a completely independent derived key for each secret. An attacker who obtains the plaintext of one secret (for example through a side channel that leaks a single decrypted value) learns nothing about the derived keys for other secrets, even though all are ultimately derived from the same master key.

This is verified directly in the test suite: `test_different_salts_different_ciphertext` confirms that encrypting the same plaintext twice produces different salts and different ciphertexts, yet both decrypt correctly.

**Tamper detection:** The GCM authentication tag covers the ciphertext and the implicit associated data. `test_tampered_ciphertext_fails` flips the last byte of `encrypted_value` and asserts that `decrypt` returns an error — the modified tag fails verification before any plaintext is released.

### Store Backends

`SecretsStore` is a trait with seven async methods. Two production implementations are provided:

- **`PostgresSecretsStore`** (`feature = "postgres"`): uses a `deadpool_postgres::Pool`; upserts via `ON CONFLICT (user_id, name) DO UPDATE`
- **`LibSqlSecretsStore`** (`feature = "libsql"`): uses `Arc<libsql::Database>` with a connection-per-operation pattern and a 5-second `busy_timeout` pragma; upserts via SQLite-dialect `ON CONFLICT`

An `InMemorySecretsStore` exists behind `#[cfg(test)]` for unit testing without a real database.

The `is_accessible` method on both production stores implements a simple allowlist check with glob support: a pattern of `"openai_*"` matches any secret whose name starts with `"openai_"`. This is the gate checked by the WASM credential injector before decryption.

## 5. Secret Types (`types.rs`)

`src/secrets/types.rs` defines the full type vocabulary for the secrets subsystem.

**`Secret`** — the database row representation. Contains `encrypted_value` and `key_salt` as raw byte vectors; the plaintext is never stored. The `Debug` implementation manually redacts both fields, printing `[REDACTED]` to prevent accidental exposure in log output or panic messages.

**`SecretRef`** — a name-only view of a secret (name + optional provider string). This is what the `list` operation returns and what WASM tool schemas can safely surface; it contains no encrypted or plaintext data.

**`DecryptedSecret`** — a transient wrapper around `SecretString` that holds the decrypted plaintext only for the duration of credential injection. The type exposes one method to access the value (`expose() -> &str`) and otherwise redacts itself:

- `Debug` prints `DecryptedSecret([REDACTED, N bytes])` — length only, no content
- Memory is zeroed on drop (via `SecretString`'s `zeroize` implementation)
- The `Clone` implementation re-wraps via `SecretString` so no plaintext copy escapes the wrapper

**`SecretError`** — an exhaustive error enum with seven variants: `NotFound(String)`, `Expired`, `DecryptionFailed(String)`, `EncryptionFailed(String)`, `InvalidMasterKey`, `InvalidUtf8`, `Database(String)`, `AccessDenied`, `KeychainError(String)`. Implemented via `thiserror`.

**`CreateSecretParams`** — builder-pattern input for creating a secret. The `value` field is `SecretString` even at the input boundary so that the plaintext is protected as soon as it enters the system. Builder methods: `with_provider(str)`, `with_expiry(DateTime<Utc>)`.

**`CredentialLocation`** — an enum that describes where in an HTTP request a credential should be injected. Five variants:

- `AuthorizationBearer` — `Authorization: Bearer {secret}` header
- `AuthorizationBasic { username }` — `Authorization: Basic base64(username:secret)` header
- `Header { name, prefix }` — arbitrary header, optional value prefix (e.g., `"Api-Key "`)
- `QueryParam { name }` — URL query parameter
- `UrlPath { placeholder }` — substitution into a URL template (handled by channel/tool wrapper code, not by the injector itself)

**`CredentialMapping`** — links a secret name to a `CredentialLocation` and a set of host glob patterns (e.g., `"*.openai.com"`). This is the configuration object that tells the injector what to inject and where.

## 6. The `secrecy` Crate Pattern

IronClaw uses the `secrecy` crate throughout the secrets subsystem to prevent accidental exposure of sensitive values through Rust's standard debug and display infrastructure.

`secrecy::SecretString` (re-exported as `SecretString` from `secrecy`) is a `String` wrapper with three properties:

1. **Redacted `Debug`**: `format!("{:?}", secret_string)` prints `"[REDACTED]"` — the actual string content is never included. This means secrets stored in structs that derive or implement `Debug` will not appear in log output, tracing spans, or panic backtraces.

2. **No `Display`**: `SecretString` does not implement `Display`, so `format!("{}", secret_string)` is a compile error. Secrets cannot be accidentally interpolated into format strings.

3. **Zeroed on drop**: The underlying string bytes are overwritten with zeros when the `SecretString` is dropped, preventing the value from lingering in freed heap memory.

To access the value, code must explicitly call `.expose_secret()`, which returns `&str`. Every call site where this appears in IronClaw is a deliberate, auditable use:

- `crypto.rs`: `self.master_key.expose_secret().as_bytes()` — inside HKDF derivation
- `store.rs`: `params.value.expose_secret().as_bytes()` — immediately before encryption
- `types.rs` (`DecryptedSecret::expose`): the single exit point for decrypted values in the injection path

The test in `types.rs` `test_decrypted_secret_redaction` programmatically verifies the redaction guarantee: it creates a `DecryptedSecret` with a known value, formats it with `{:?}`, and asserts the plaintext does not appear and `"REDACTED"` does.

The `SecretsCrypto` struct implements `Debug` manually, printing `"[REDACTED]"` for its `master_key` field, ensuring the master key never leaks through logging of the crypto instance.

## 7. Credential Injection for WASM Tools (`credential_injector.rs`)

`src/tools/wasm/credential_injector.rs` implements the host-side boundary where secrets cross from the encrypted store into outbound HTTP requests.

### Design Principle

WASM tool code executes inside a sandboxed runtime (wasmtime) with no ability to make system calls directly. All outbound HTTP goes through a host function. The credential injector intercepts at that host function boundary, decrypts the relevant secrets, and attaches them to the request — without the WASM code ever receiving the plaintext values.

From the WASM tool's perspective, it declares credential requirements in its `CredentialMapping` configuration. At runtime it simply emits an HTTP request to a host; the host handles authentication transparently.

### Injection Flow

```
WASM tool emits HTTP request to host
    │
    ▼
CredentialInjector::inject(user_id, host, store)
    │
    ├─ find_credentials_for_host(host)
    │   └─ match host against CredentialMapping.host_patterns (exact + wildcard)
    │
    ├─ for each matching mapping:
    │   ├─ is_secret_allowed(secret_name, allowed_secrets)  ← allowlist gate
    │   │   └─ exact match or prefix glob (e.g., "openai_*")
    │   │
    │   └─ store.get_decrypted(user_id, secret_name)  ← host-only operation
    │       └─ decrypt via SecretsCrypto → DecryptedSecret
    │
    ├─ inject_credential(result, location, decrypted_secret)
    │   ├─ AuthorizationBearer  → "Authorization: Bearer {value}"
    │   ├─ AuthorizationBasic   → "Authorization: Basic base64(user:value)"
    │   ├─ Header               → "{name}: {prefix?}{value}"
    │   ├─ QueryParam           → "?{name}={value}"
    │   └─ UrlPath              → (handled by outer wrapper)
    │
    ▼
InjectedCredentials { headers, query_params }
    → merged into the outbound request before execution
    → WASM tool receives the HTTP response only
```

### Host Wildcard Pattern Matching

`host_matches_pattern` supports two forms:

- **Exact**: `"api.openai.com"` matches only that hostname
- **Wildcard**: `"*.openai.com"` matches any single subdomain level (`api.openai.com`, `beta.openai.com`) but not the bare domain (`openai.com`) and not multi-level subdomain paths that skip a level

The wildcard logic checks that the prefix portion (before the matched suffix) ends with a dot or is empty, preventing `notasubdomain-openai.com` from matching `*.openai.com`.

### LLM Isolation Guarantee

The LLM receives tool schemas that describe what a tool does and what parameters it accepts. Credential mappings are host-side configuration, never included in the schema presented to the LLM. The LLM therefore sees:

- That a tool can make HTTP requests to certain hosts — yes
- What API key is used for those requests — no
- What the key value is — no

Even under a successful prompt injection attack that causes the LLM to instruct a tool to exfiltrate data, the tool has no mechanism to include API keys in its exfiltration payload because it never received them.

### Access Control Gate

`CredentialInjector` is constructed with an `allowed_secrets: Vec<String>` list. Before decrypting any secret, `is_secret_allowed` checks the requested secret name against this list (exact or prefix glob). If not present, `InjectionError::AccessDenied` is returned and no decryption occurs. This means a WASM tool that somehow learns the name of an unrelated secret cannot cause the injector to decrypt it — the allowlist is a capability boundary, not just a name filter.

## 8. Secret Naming Conventions

The secrets subsystem enforces no structural naming convention at the code level; names are arbitrary strings within the `(user_id, name)` namespace. However, the codebase and tests establish a consistent informal convention:

**Observed naming pattern in tests and tooling:**

```
{provider}_{key_type}

Examples:
  openai_key
  openai_api_key
  stripe_key
  api_key
  password
```

**Provider field:** The `Secret` and `CreateSecretParams` types include an optional `provider` field (e.g., `"openai"`, `"stripe"`) that serves as a hint for tooling and UI display. It does not affect encryption, storage, or injection logic.

**Glob allowlists:** The glob pattern support in `is_accessible` and `is_secret_allowed` is designed around the `{provider}_*` naming convention. A WASM tool that needs all OpenAI-related secrets can declare `allowed_secrets: ["openai_*"]` rather than enumerating each key name individually.

**Recommended convention for new integrations:**

```
{provider}:{service}:{key_type}

Examples:
  openai:default:api_key
  anthropic:default:api_key
  stripe:live:secret_key
  postgres:main:password
```

This format is more structured and allows finer-grained glob patterns (`openai:*` vs `openai:default:*`), though the current codebase uses flat underscore-separated names.

## 9. Security Considerations

### Master Key Protection

The master key is 32 bytes of OS-CSPRNG output, hex-encoded for keychain storage. It is held in process memory as a `SecretString` (zeroed on drop) and is never written to disk, logs, or exposed via any API. The two risk points are:

1. **Keychain compromise** — an attacker with access to the OS keychain (macOS Keychain, GNOME Keyring) can extract the master key. This requires either physical access with an unlocked session, or a vulnerability in the keychain daemon. Mitigations are OS-level: full-disk encryption (FileVault, LUKS), automatic session lock, and keychain auto-lock policies.

2. **`SECRETS_MASTER_KEY` in environment** — for CI/container deployments the master key must be injected as an environment variable. Any process that can read `/proc/self/environ` or the container's environment can extract it. Mitigations: runtime secrets injection (Docker secrets, Kubernetes secrets), minimal process privilege, no environment variable logging.

### Key Rotation

Rotating the master key requires re-encrypting all secrets. The current store API does not include a built-in rotation operation. A manual rotation process would be:

1. Generate a new master key with `generate_master_key_hex()`
2. For each secret: call `get_decrypted`, then `create` (upsert) with the new `SecretsCrypto` instance derived from the new key
3. Store the new master key in the keychain with `store_master_key`
4. Discard the old master key

Because each secret has its own independently-derived key (via HKDF with a random salt), rotating the master key effectively rotates all per-secret keys simultaneously with a single master key change.

### Backup Safety

The encrypted database (PostgreSQL or libSQL) can be backed up safely without exposing secret values. A backup contains only ciphertext and salts; without the master key it is computationally infeasible to recover any plaintext. The master key in the OS keychain is not included in standard database backups, which means a database-only backup cannot be used to recover secrets — an intentional design property that also means the master key must be backed up separately (e.g., in a password manager or printed recovery code).

### Expiration and Usage Tracking

The `expires_at` field enforces time-bound secrets. `get` and `get_decrypted` check expiration before returning a result and return `SecretError::Expired` if the timestamp has passed. Expired secrets remain in the database (they are not automatically deleted) so that usage records are preserved, but they cannot be decrypted.

`record_usage` increments `usage_count` and sets `last_used_at` on each successful injection. This provides an audit trail and enables detection of unexpected credential usage patterns.

### Timing Safety

AES-GCM authentication tag verification in `aes-gcm` is constant-time by construction — the library uses the `subtle` crate internally for the tag comparison to prevent timing oracle attacks. This means an attacker who can measure decryption time cannot distinguish "wrong nonce" from "wrong ciphertext" from "wrong tag", preventing timing-based forgery attacks against the GCM tag.

### Response Leak Detection

After credential injection and HTTP request execution, the response passes through the `LeakDetector` (in `src/safety/leak_detector.rs`) before being returned to the WASM tool or the LLM. The leak detector scans for known secret patterns (API key formats, tokens, etc.). If a secret value is echoed back in the response body — for example because a misconfigured API reflects request headers — it is redacted or blocked before the WASM tool or LLM context ever sees it. This is the final line of defense shown in the module-level security model diagram.
