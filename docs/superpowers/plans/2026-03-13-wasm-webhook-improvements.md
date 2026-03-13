# WASM Webhook Improvements Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve WASM channel webhook handling with WhatsApp HMAC signature verification, flexible verification modes, webhook deduplication, and ACK deferral for reliable message processing.

**Architecture:** Extend the existing WASM channel infrastructure in three layers: (1) signature verification module adds WhatsApp-style HMAC, (2) schema/router add verification modes and deduplication, (3) wrapper adds deferred ACK with on_message_persisted callback.

**Tech Stack:** Rust, axum, hmac, sha2, subtle (constant-time comparison), PostgreSQL/libSQL

**Source Branch:** `feat/whatsapp-hmac-signature-verification` (PR closed, changes to be integrated)
**Target Branch:** New branch from `upstream/main`

**Prerequisites:**
- Latest migration in upstream/main is **V11** → new migration must be **V12**
- Current `router.register()` has 4 params → will add 2 new params (backward compatible)
- `register_hmac_secret()` already exists in router

---

## File Structure

```
src/channels/wasm/
├── signature.rs          # Add verify_hmac_sha256 for WhatsApp
├── schema.rs             # Add verification_mode, message_id_json_pointer fields
├── router.rs             # Add dedup, ACK deferral, verification modes
├── wrapper.rs            # Add call_on_http_request_with_messages, on_message_persisted
└── loader.rs             # Pass new config fields to router

src/db/
├── mod.rs                # Add WebhookDedupStore trait
├── postgres.rs           # Implement WebhookDedupStore
└── libsql/
    ├── mod.rs            # Implement WebhookDedupStore
    └── webhook_dedup.rs  # libSQL-specific dedup module

migrations/
└── V12__webhook_dedup.sql  # Dedup table migration (V11 is latest in upstream)

wit/
└── channel.wit           # Add on_message_persisted callback

channels-src/whatsapp/
├── src/lib.rs            # Implement on_message_persisted for mark_as_read
└── whatsapp.capabilities.json  # Add hmac_secret_name, verification_mode

src/main.rs               # Initialize router.set_db() on startup
```

---

## Chunk 1: WhatsApp HMAC Signature Verification

### Task 1.1: Add verify_hmac_sha256 function

**Files:**
- Modify: `src/channels/wasm/signature.rs`

**Context:** WhatsApp Cloud API sends webhook signatures in `X-Hub-Signature-256` header with format `sha256=<hex>`. This is simpler than Slack's versioned basestring (no timestamp prefix).

- [ ] **Step 1: Write the failing test**

```rust
// In src/channels/wasm/signature.rs, add to mod tests:

/// Helper: compute HMAC-SHA256 signature in WhatsApp/Meta format (`sha256=<hex>`).
fn compute_whatsapp_style_hmac_signature(secret: &str, body: &[u8]) -> String {
    use hmac::Mac;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let result = mac.finalize();
    format!("sha256={}", hex::encode(result.into_bytes()))
}

#[test]
fn test_hmac_valid_signature_succeeds() {
    let secret = "my_app_secret";
    let body = br#"{"entry":[{"id":"123"}]}"#;
    let sig_header = compute_whatsapp_style_hmac_signature(secret, body);

    assert!(
        verify_hmac_sha256(secret, &sig_header, body),
        "Valid HMAC signature should verify"
    );
}

#[test]
fn test_hmac_wrong_secret_fails() {
    let secret = "correct_secret";
    let wrong_secret = "wrong_secret";
    let body = br#"{"test":"data"}"#;
    let sig_header = compute_whatsapp_style_hmac_signature(secret, body);

    assert!(
        !verify_hmac_sha256(wrong_secret, &sig_header, body),
        "Signature with wrong secret should fail"
    );
}

#[test]
fn test_hmac_tampered_body_fails() {
    let secret = "my_secret";
    let body = br#"original body"#;
    let tampered = br#"tampered body"#;
    let sig_header = compute_whatsapp_style_hmac_signature(secret, body);

    assert!(
        !verify_hmac_sha256(secret, &sig_header, tampered),
        "Tampered body should fail verification"
    );
}

#[test]
fn test_hmac_invalid_header_format_fails() {
    let secret = "secret";
    let body = br#"data"#;

    assert!(!verify_hmac_sha256(secret, "invalid", body));
    assert!(!verify_hmac_sha256(secret, "sha256=not_hex!", body));
    assert!(!verify_hmac_sha256(secret, "", body));
}

#[test]
fn test_hmac_wrong_length_fails() {
    let secret = "secret";
    let body = br#"data"#;
    // 16 bytes instead of 32
    let short_sig = format!("sha256={}", "a".repeat(16));

    assert!(
        !verify_hmac_sha256(secret, &short_sig, body),
        "Wrong-length signature should fail"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test channels::wasm::signature::tests::test_hmac --no-run 2>&1 | grep -E "error|verify_hmac_sha256"`
Expected: Compilation error - `verify_hmac_sha256` not found

- [ ] **Step 3: Add imports and type alias at top of file**

```rust
// At the top of src/channels/wasm/signature.rs, add after existing imports:

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;
```

- [ ] **Step 4: Write the implementation**

Add after `verify_discord_signature` function:

```rust
/// Verify HMAC-SHA256 signature (WhatsApp style, simple body-only).
///
/// # Arguments
/// * `secret` - The HMAC secret (App Secret)
/// * `signature_header` - Value from X-Hub-Signature-256 header (format: "sha256=<hex>")
/// * `body` - Raw request body bytes
///
/// # Returns
/// `true` if signature is valid, `false` otherwise
pub fn verify_hmac_sha256(secret: &str, signature_header: &str, body: &[u8]) -> bool {
    // Parse header format: "sha256=<hex_signature>"
    let Some(hex_signature) = signature_header.strip_prefix("sha256=") else {
        return false;
    };

    // Decode expected signature
    let Ok(expected_sig) = hex::decode(hex_signature) else {
        return false;
    };

    // SHA-256 produces 32-byte signatures - reject wrong lengths early
    if expected_sig.len() != 32 {
        return false;
    }

    // Compute HMAC-SHA256
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let result = mac.finalize();
    let computed_sig = result.into_bytes();

    // Constant-time comparison to prevent timing attacks
    computed_sig
        .as_slice()
        .ct_eq(expected_sig.as_slice())
        .into()
}
```

- [ ] **Step 5: Refactor verify_slack_signature to use shared HmacSha256**

Remove the local imports in `verify_slack_signature`:

```rust
// REMOVE these lines from inside verify_slack_signature:
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
```

Change `Hmac::<Sha256>` to `HmacSha256`:

```rust
// Change this line:
let mut mac = match Hmac::<Sha256>::new_from_slice(signing_secret.as_bytes()) {
// To:
let mut mac = match HmacSha256::new_from_slice(signing_secret.as_bytes()) {
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test channels::wasm::signature::tests::test_hmac`
Expected: All 5 tests pass

- [ ] **Step 7: Run full signature tests**

Run: `cargo test channels::wasm::signature::tests`
Expected: All signature tests pass (Discord, Slack, WhatsApp)

- [ ] **Step 8: Commit**

```bash
git add src/channels/wasm/signature.rs
git commit -m "feat(wasm): add verify_hmac_sha256 for WhatsApp webhook signatures

Adds simple body-only HMAC-SHA256 verification for WhatsApp/Meta webhooks.
Uses X-Hub-Signature-256 header with sha256=<hex> format.
Refactors to share HmacSha256 type alias with Slack verification."
```

---

## Chunk 2: Schema Extensions for Verification Modes

### Task 2.1: Add new webhook configuration fields

**Files:**
- Modify: `src/channels/wasm/schema.rs`

**Context:** WhatsApp needs different verification for GET (query param) vs POST (HMAC signature). Also need to extract message IDs for deduplication.

- [ ] **Step 1: Write the failing tests**

```rust
// In src/channels/wasm/schema.rs, add to mod tests:

#[test]
fn test_webhook_verification_mode_parsing() {
    let json = r#"{
        "name": "test",
        "capabilities": {
            "channel": {
                "webhook": {
                    "verification_mode": "query_param"
                }
            }
        }
    }"#;

    let cap: ChannelCapabilitiesFile = serde_json::from_str(json).unwrap();
    assert_eq!(cap.webhook_verification_mode(), Some("query_param"));
}

#[test]
fn test_webhook_hmac_secret_name_parsing() {
    let json = r#"{
        "name": "test",
        "capabilities": {
            "channel": {
                "webhook": {
                    "hmac_secret_name": "whatsapp_app_secret"
                }
            }
        }
    }"#;

    let cap: ChannelCapabilitiesFile = serde_json::from_str(json).unwrap();
    assert_eq!(cap.webhook_hmac_secret_name(), Some("whatsapp_app_secret"));
}

#[test]
fn test_webhook_message_id_json_pointer_parsing() {
    let json = r#"{
        "name": "test",
        "capabilities": {
            "channel": {
                "webhook": {
                    "message_id_json_pointer": "/message_id"
                }
            }
        }
    }"#;

    let cap: ChannelCapabilitiesFile = serde_json::from_str(json).unwrap();
    assert_eq!(cap.webhook_message_id_json_pointer(), Some("/message_id"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test channels::wasm::schema::tests::test_webhook 2>&1 | grep -E "error|no method"`
Expected: Compilation errors for missing methods/fields

- [ ] **Step 3: Add new fields to WebhookSchema**

```rust
// In WebhookSchema struct, add after signature_key_secret_name:

/// How to handle GET request validation:
/// - None/default: Require secret header for all requests (current behavior)
/// - "query_param": Skip host-level secret validation for GET requests;
///   the WASM module validates via query param (e.g., WhatsApp hub.verify_token)
/// - "signature": Always require signature validation (for Discord-style Ed25519)
#[serde(default)]
pub verification_mode: Option<String>,

/// Secret name in secrets store containing the HMAC secret
/// for signature verification (e.g., WhatsApp/Slack webhook signatures).
/// The header format is expected to be "sha256=<hex_signature>".
#[serde(default)]
pub hmac_secret_name: Option<String>,

/// JSON pointer path to extract message ID from metadata_json.
/// Used for ACK key construction and deduplication.
/// Format: "/field1/field2" to access {"field1": {"field2": "value"}}
/// If None, the router falls back to using user_id.
#[serde(default)]
pub message_id_json_pointer: Option<String>,
```

- [ ] **Step 4: Add accessor methods to ChannelCapabilitiesFile**

```rust
// Add after webhook_secret_name method:

/// Get the webhook verification mode for this channel.
///
/// Returns the verification mode declared in `webhook.verification_mode`:
/// - None/default: Require secret header for all requests
/// - "query_param": Skip host-level secret validation for GET, WASM validates via query param
/// - "signature": Always require signature validation
pub fn webhook_verification_mode(&self) -> Option<&str> {
    self.capabilities
        .channel
        .as_ref()
        .and_then(|c| c.webhook.as_ref())
        .and_then(|w| w.verification_mode.as_deref())
}

/// Get the HMAC secret name for webhook signature verification.
///
/// Returns the secret name declared in `webhook.hmac_secret_name`,
/// used to look up the HMAC secret in the secrets store for
/// WhatsApp/Slack-style signature verification.
pub fn webhook_hmac_secret_name(&self) -> Option<&str> {
    self.capabilities
        .channel
        .as_ref()
        .and_then(|c| c.webhook.as_ref())
        .and_then(|w| w.hmac_secret_name.as_deref())
}

/// Get the JSON pointer path to extract message ID from metadata.
///
/// Returns the JSON pointer declared in `webhook.message_id_json_pointer`,
/// used for ACK key construction and deduplication.
/// If None, the router falls back to using user_id.
pub fn webhook_message_id_json_pointer(&self) -> Option<&str> {
    self.capabilities
        .channel
        .as_ref()
        .and_then(|c| c.webhook.as_ref())
        .and_then(|w| w.message_id_json_pointer.as_deref())
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test channels::wasm::schema::tests::test_webhook`
Expected: All 3 tests pass

- [ ] **Step 6: Run full schema tests**

Run: `cargo test channels::wasm::schema::tests`
Expected: All schema tests pass

- [ ] **Step 7: Commit**

```bash
git add src/channels/wasm/schema.rs
git commit -m "feat(wasm): add verification_mode and message_id_json_pointer to webhook schema

Adds three new webhook configuration fields:
- verification_mode: query_param/signature/default
- hmac_secret_name: for WhatsApp/Slack HMAC verification
- message_id_json_pointer: for extracting message IDs from metadata"
```

---

## Chunk 3: Webhook Deduplication Database Layer

### Task 3.1: Add WebhookDedupStore trait and PostgreSQL implementation

**Files:**
- Modify: `src/db/mod.rs`
- Modify: `src/db/postgres.rs`
- Create: `migrations/V12__webhook_dedup.sql` (V12 because V11 is latest in upstream)

**Context:** WhatsApp retries webhooks up to 7 days on 5xx errors. Need atomic deduplication to prevent duplicate message processing.

- [ ] **Step 1: Create migration file**

```sql
-- migrations/V12__webhook_dedup.sql
-- Webhook message deduplication table
-- Prevents duplicate processing when channels retry on errors

CREATE TABLE IF NOT EXISTS webhook_message_dedup (
    -- Composite key: channel name + message ID from the channel
    -- e.g., "whatsapp:wamid.HBgM..." or "telegram:12345"
    key TEXT PRIMARY KEY,

    -- When this message was first seen
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for cleanup queries (delete old records)
CREATE INDEX IF NOT EXISTS idx_webhook_dedup_created_at
    ON webhook_message_dedup(created_at);

-- Comment explaining purpose
COMMENT ON TABLE webhook_message_dedup IS
    'Deduplication table for webhook messages. Channels like WhatsApp retry on 5xx for up to 7 days. This table ensures idempotent processing.';
```

- [ ] **Step 2: Add WebhookDedupStore trait to src/db/mod.rs**

```rust
// Add after existing traits:

/// Webhook message deduplication store.
///
/// Prevents duplicate processing when channels retry webhooks on errors.
/// WhatsApp, for example, retries for up to 7 days on 5xx responses.
#[async_trait]
pub trait WebhookDedupStore: Send + Sync {
    /// Try to record that a message is processed, atomically.
    ///
    /// Returns `true` if this is a new message (was inserted),
    /// `false` if it was a duplicate (key already exists).
    ///
    /// Uses INSERT ... ON CONFLICT DO NOTHING for atomic dedup with no race condition.
    async fn record_webhook_message_processed(
        &self,
        channel_name: &str,
        message_id: &str,
    ) -> Result<bool, DbError>;

    /// Clean up old dedup records.
    ///
    /// Called periodically to prevent unbounded growth.
    /// Returns the number of records deleted.
    async fn cleanup_old_webhook_dedup_records(&self) -> Result<u64, DbError>;
}
```

- [ ] **Step 3: Add trait bound to Database trait**

```rust
// Modify the Database trait declaration to include WebhookDedupStore:
pub trait Database:
    MessageStore
    + ThreadStore
    + ToolStore
    + JobStore
    + SessionStore
    + PairingStore
    + WasmToolStore
    + SecretsStore
    + SettingsStore
    + WorkspaceStore
    + RoutineStore
    + WebhookDedupStore  // Add this
    + Send
    + Sync
{
}
```

- [ ] **Step 4: Implement WebhookDedupStore for PostgresBackend**

Add to `src/db/postgres.rs` before `mod tests`:

```rust
#[async_trait]
impl WebhookDedupStore for PostgresBackend {
    async fn record_webhook_message_processed(
        &self,
        channel_name: &str,
        message_id: &str,
    ) -> Result<bool, DbError> {
        let key = format!("{}:{}", channel_name, message_id);

        let result = sqlx::query(
            r#"
            INSERT INTO webhook_message_dedup (key)
            VALUES ($1)
            ON CONFLICT (key) DO NOTHING
            "#,
        )
        .bind(&key)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::QueryError(e.to_string()))?;

        // rows_affected is 1 if inserted, 0 if conflict (duplicate)
        Ok(result.rows_affected() == 1)
    }

    async fn cleanup_old_webhook_dedup_records(&self) -> Result<u64, DbError> {
        // Delete records older than 30 days
        let result = sqlx::query(
            r#"
            DELETE FROM webhook_message_dedup
            WHERE created_at < NOW() - INTERVAL '30 days'
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::QueryError(e.to_string()))?;

        Ok(result.rows_affected())
    }
}
```

- [ ] **Step 5: Write proper tests with sqlx::test**

```rust
// Add to mod tests in src/db/postgres.rs:

#[sqlx::test]
async fn test_webhook_dedup_inserts_new_key(pool: PgPool) {
    let db = PostgresBackend::with_pool(pool);

    // First insert should succeed
    let is_new = db
        .record_webhook_message_processed("whatsapp", "msg123")
        .await
        .unwrap();
    assert!(is_new, "First insert should return true (new message)");
}

#[sqlx::test]
async fn test_webhook_dedup_rejects_duplicate(pool: PgPool) {
    let db = PostgresBackend::with_pool(pool);

    // First insert
    let is_new1 = db
        .record_webhook_message_processed("whatsapp", "msg456")
        .await
        .unwrap();
    assert!(is_new1);

    // Second insert (duplicate) should return false
    let is_new2 = db
        .record_webhook_message_processed("whatsapp", "msg456")
        .await
        .unwrap();
    assert!(!is_new2, "Duplicate insert should return false");
}

#[sqlx::test]
async fn test_webhook_dedup_different_channels_same_msg_id(pool: PgPool) {
    let db = PostgresBackend::with_pool(pool);

    // Same message ID in different channels should both succeed
    let is_new1 = db
        .record_webhook_message_processed("whatsapp", "msg789")
        .await
        .unwrap();
    let is_new2 = db
        .record_webhook_message_processed("telegram", "msg789")
        .await
        .unwrap();

    assert!(is_new1);
    assert!(is_new2, "Same msg_id in different channels should be separate keys");
}
```

- [ ] **Step 6: Run tests with postgres feature**

Run: `cargo test db::postgres::tests::test_webhook_dedup --features postgres`
Expected: All 3 tests pass

- [ ] **Step 7: Commit**

```bash
git add src/db/mod.rs src/db/postgres.rs migrations/V12__webhook_dedup.sql
git commit -m "feat(db): add webhook message deduplication store

Adds WebhookDedupStore trait and PostgreSQL implementation.
Prevents duplicate processing when channels retry webhooks.
Uses INSERT ON CONFLICT DO NOTHING for atomic deduplication."
```

### Task 3.2: Add libSQL implementation

**Files:**
- Create: `src/db/libsql/webhook_dedup.rs`
- Modify: `src/db/libsql/mod.rs`
- Modify: `src/db/libsql_migrations.rs`

- [ ] **Step 1: Create webhook_dedup.rs module**

```rust
// src/db/libsql/webhook_dedup.rs
//! Webhook message deduplication for libSQL backend.

use async_trait::async_trait;
use libsql::Connection;

use crate::db::{DbError, WebhookDedupStore};

/// LibSQL implementation of WebhookDedupStore.
pub struct LibSqlWebhookDedupStore {
    conn: Connection,
}

impl LibSqlWebhookDedupStore {
    /// Create a new webhook dedup store.
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl WebhookDedupStore for LibSqlWebhookDedupStore {
    async fn record_webhook_message_processed(
        &self,
        channel_name: &str,
        message_id: &str,
    ) -> Result<bool, DbError> {
        let key = format!("{}:{}", channel_name, message_id);

        // libSQL uses INSERT OR IGNORE for SQLite compatibility
        let result = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO webhook_message_dedup (key) VALUES (?1)",
                [libsql::Value::from(key)],
            )
            .await
            .map_err(|e| DbError::QueryError(e.to_string()))?;

        // rows_affected is 1 if inserted, 0 if ignored (duplicate)
        Ok(result.rows_affected() == 1)
    }

    async fn cleanup_old_webhook_dedup_records(&self) -> Result<u64, DbError> {
        // Delete records older than 30 days (SQLite datetime syntax)
        let result = self
            .conn
            .execute(
                "DELETE FROM webhook_message_dedup WHERE created_at < datetime('now', '-30 days')",
                [],
            )
            .await
            .map_err(|e| DbError::QueryError(e.to_string()))?;

        Ok(result.rows_affected())
    }
}
```

- [ ] **Step 2: Add migration to libsql_migrations.rs**

```rust
// In src/db/libsql_migrations.rs, find the migrations array and add:

// Migration 12: Webhook deduplication table
(
    12,
    r#"
    CREATE TABLE IF NOT EXISTS webhook_message_dedup (
        key TEXT PRIMARY KEY,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE INDEX IF NOT EXISTS idx_webhook_dedup_created_at
        ON webhook_message_dedup(created_at);
    "#,
),
```

- [ ] **Step 3: Update mod.rs to expose and wire the store**

```rust
// In src/db/libsql/mod.rs, add module declaration:
mod webhook_dedup;
pub use webhook_dedup::LibSqlWebhookDedupStore;

// In LibSqlBackend struct, add field:
pub struct LibSqlBackend {
    // ... existing fields ...
    webhook_dedup: LibSqlWebhookDedupStore,
}

// In LibSqlBackend::new(), after connection is established:
impl LibSqlBackend {
    pub async fn new(config: &LibSqlConfig) -> Result<Self, DbError> {
        // ... existing initialization code ...
        let webhook_dedup = LibSqlWebhookDedupStore::new(conn.clone());

        Ok(Self {
            // ... existing fields ...
            webhook_dedup,
        })
    }
}

// Add WebhookDedupStore impl that delegates:
#[async_trait]
impl WebhookDedupStore for LibSqlBackend {
    async fn record_webhook_message_processed(
        &self,
        channel_name: &str,
        message_id: &str,
    ) -> Result<bool, DbError> {
        self.webhook_dedup
            .record_webhook_message_processed(channel_name, message_id)
            .await
    }

    async fn cleanup_old_webhook_dedup_records(&self) -> Result<u64, DbError> {
        self.webhook_dedup.cleanup_old_webhook_dedup_records().await
    }
}
```

- [ ] **Step 4: Test compilation with libsql feature only**

Run: `cargo check --no-default-features --features libsql`
Expected: No compilation errors

- [ ] **Step 5: Test compilation with all features**

Run: `cargo check --all-features`
Expected: No compilation errors

- [ ] **Step 6: Commit**

```bash
git add src/db/libsql/webhook_dedup.rs src/db/libsql/mod.rs src/db/libsql_migrations.rs
git commit -m "feat(db): add libSQL implementation of WebhookDedupStore

Uses INSERT OR IGNORE for atomic deduplication.
Compatible with Turso cloud and local libSQL."
```

---

## Chunk 4: Router Integration

### Task 4.1: Add verification modes and HMAC support to router

**Files:**
- Modify: `src/channels/wasm/router.rs`
- Modify: `src/channels/wasm/loader.rs`

**Context:** Router needs to support new verification modes and HMAC signature validation. Current `register()` has 4 params; we add 2 more (backward compatible by using Option).

- [ ] **Step 1: Add new fields to WasmChannelRouter struct**

```rust
// In src/channels/wasm/router.rs, modify WasmChannelRouter struct:

pub struct WasmChannelRouter {
    // ... existing fields (channels, path_to_channel, secrets, secret_headers, signature_keys, hmac_secrets) ...

    /// Verification mode per channel: "query_param", "signature", etc.
    verification_modes: RwLock<HashMap<String, String>>,
    /// JSON pointers for extracting message IDs from metadata_json by channel name.
    message_id_json_pointers: RwLock<HashMap<String, String>>,
    /// Database for webhook message deduplication (optional - graceful degradation if not set).
    db: RwLock<Option<Arc<dyn crate::db::WebhookDedupStore + Send + Sync>>>,
}
```

- [ ] **Step 2: Update WasmChannelRouter::new()**

```rust
impl WasmChannelRouter {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            path_to_channel: RwLock::new(HashMap::new()),
            secrets: RwLock::new(HashMap::new()),
            secret_headers: RwLock::new(HashMap::new()),
            signature_keys: RwLock::new(HashMap::new()),
            hmac_secrets: RwLock::new(HashMap::new()),
            verification_modes: RwLock::new(HashMap::new()),
            message_id_json_pointers: RwLock::new(HashMap::new()),
            db: RwLock::new(None),
        }
    }

    /// Set the database for webhook message deduplication.
    ///
    /// If not called, deduplication is disabled (webhooks process without idempotency check).
    pub async fn set_db(&self, db: Arc<dyn crate::db::WebhookDedupStore + Send + Sync>) {
        *self.db.write().await = Some(db);
    }

    /// Get the database for webhook message deduplication.
    ///
    /// Returns None if deduplication is not configured.
    pub async fn get_db(&self) -> Option<Arc<dyn crate::db::WebhookDedupStore + Send + Sync>> {
        self.db.read().await.clone()
    }
}
```

- [ ] **Step 3: Update register() signature (add 2 new optional params)**

```rust
/// Register a channel with its endpoints.
///
/// # Arguments
/// * `channel` - The WASM channel to register
/// * `endpoints` - HTTP endpoints to register for this channel
/// * `secret` - Optional webhook secret for validation
/// * `secret_header` - Optional HTTP header name for secret validation
/// * `verification_mode` - Optional verification mode for GET requests:
///   - "query_param": Skip host-level secret validation for GET, WASM validates via query param
///   - "signature": Always require signature validation
/// * `message_id_json_pointer` - Optional JSON pointer to extract message ID from metadata_json
pub async fn register(
    &self,
    channel: Arc<WasmChannel>,
    endpoints: Vec<RegisteredEndpoint>,
    secret: Option<String>,
    secret_header: Option<String>,
    verification_mode: Option<String>,      // NEW
    message_id_json_pointer: Option<String>, // NEW
) {
    let name = channel.channel_name().to_string();

    // Store the channel
    self.channels.write().await.insert(name.clone(), channel);

    // Register path mappings
    let mut path_map = self.path_to_channel.write().await;
    for endpoint in endpoints {
        path_map.insert(endpoint.path.clone(), name.clone());
        tracing::info!(
            channel = %name,
            path = %endpoint.path,
            methods = ?endpoint.methods,
            "Registered WASM channel HTTP endpoint"
        );
    }
    drop(path_map);

    // Store secret if provided
    if let Some(s) = secret {
        self.secrets.write().await.insert(name.clone(), s);
    }

    // Store secret header if provided
    if let Some(h) = secret_header {
        self.secret_headers.write().await.insert(name.clone(), h);
    }

    // Store verification mode if provided
    if let Some(m) = verification_mode {
        self.verification_modes
            .write()
            .await
            .insert(name.clone(), m);
    }

    // Store message ID JSON pointer if provided
    if let Some(p) = message_id_json_pointer {
        self.message_id_json_pointers
            .write()
            .await
            .insert(name.clone(), p);
    }
}
```

- [ ] **Step 4: Add accessor methods for new fields**

```rust
impl WasmChannelRouter {
    // ... existing methods ...

    /// Get the verification mode for a channel.
    pub async fn get_verification_mode(&self, channel_name: &str) -> Option<String> {
        self.verification_modes
            .read()
            .await
            .get(channel_name)
            .cloned()
    }

    /// Get the message ID JSON pointer for a channel.
    pub async fn get_message_id_json_pointer(&self, channel_name: &str) -> Option<String> {
        self.message_id_json_pointers
            .read()
            .await
            .get(channel_name)
            .cloned()
    }
}
```

- [ ] **Step 5: Update unregister() to clean up new fields**

```rust
// In unregister() method, add cleanup for new fields:

pub async fn unregister(&self, channel_name: &str) {
    self.channels.write().await.remove(channel_name);
    self.path_to_channel.write().await.retain(|_, v| v != channel_name);
    self.secrets.write().await.remove(channel_name);
    self.secret_headers.write().await.remove(channel_name);
    self.signature_keys.write().await.remove(channel_name);
    self.hmac_secrets.write().await.remove(channel_name);
    // Add these:
    self.verification_modes.write().await.remove(channel_name);
    self.message_id_json_pointers.write().await.remove(channel_name);
}
```

- [ ] **Step 6: Update loader.rs to pass new parameters**

```rust
// In src/channels/wasm/loader.rs, find where router.register() is called
// and update it to pass the new parameters:

// After reading capabilities file:
let verification_mode = caps.webhook_verification_mode().map(|s| s.to_string());
let message_id_json_pointer = caps.webhook_message_id_json_pointer().map(|s| s.to_string());

// Update router.register() call:
router.register(
    channel,
    endpoints,
    secret,
    secret_header,
    verification_mode,        // NEW
    message_id_json_pointer,  // NEW
).await;
```

- [ ] **Step 7: Test compilation**

Run: `cargo check`
Expected: No compilation errors

- [ ] **Step 8: Commit**

```bash
git add src/channels/wasm/router.rs src/channels/wasm/loader.rs
git commit -m "feat(wasm): add verification_mode and message_id support to router

Router now accepts and stores verification_mode and message_id_json_pointer
from channel capabilities. Adds optional database hook for deduplication
with graceful degradation if not configured."
```

---

## Chunk 5: WIT Interface and on_message_persisted Callback

### Task 5.1: Add on_message_persisted to WIT interface

**Files:**
- Modify: `wit/channel.wit`
- Modify: `src/channels/wasm/wrapper.rs`

**IMPORTANT:** After modifying WIT, you MUST regenerate bindings.

- [ ] **Step 1: Add callback to WIT interface**

```wit
// In wit/channel.wit, add after on_respond (around line 310):

/// Called after a message has been persisted to the database.
///
/// Channels can use this to perform follow-up actions like
/// calling external APIs (e.g., WhatsApp mark_as_read).
/// This is optional - channels that don't need it can return Ok.
///
/// Arguments:
/// - metadata-json: The metadata from the persisted message
///
/// Returns:
/// - Ok: Post-persistence action completed successfully
/// - Err(string): Action failure message (does not block the ACK)
on-message-persisted: func(metadata-json: string) -> result<_, string>;
```

- [ ] **Step 2: Regenerate WIT bindings**

Run: `cargo build -p wit-bindgen 2>&1 || echo "If wit-bindgen not separate, build will regenerate on next compile"`

Then run: `cargo build` to trigger binding regeneration.

- [ ] **Step 3: Add implementation to wrapper.rs**

```rust
// In src/channels/wasm/wrapper.rs, add method to WasmChannel:

/// Execute the on_message_persisted callback.
///
/// Called after a message has been successfully persisted to the database.
/// Channels can use this for follow-up actions like WhatsApp mark_as_read.
///
/// Returns Ok(()) even on failure - this is best-effort and should not block ACKs.
pub async fn call_on_message_persisted(
    &self,
    metadata_json: &str,
) -> Result<(), WasmChannelError> {
    // If no WASM bytes, return Ok (for testing)
    if self.prepared.component().is_none() {
        tracing::debug!(
            channel = %self.name,
            "on_message_persisted called (no WASM module)"
        );
        return Ok(());
    }

    let runtime = Arc::clone(&self.runtime);
    let prepared = Arc::clone(&self.prepared);
    let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
    let timeout = self.runtime.config().callback_timeout;
    let credentials = self.get_credentials().await;
    let pairing_store = self.pairing_store.clone();
    let metadata_json = metadata_json.to_string();
    let channel_name = self.name.clone();

    let result = tokio::time::timeout(timeout, async move {
        tokio::task::spawn_blocking(move || {
            let mut store = Self::create_store(
                &runtime,
                &prepared,
                &capabilities,
                credentials,
                Default::default(), // host_credentials not needed for this callback
                pairing_store,
            )?;
            let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

            let channel_iface = instance.near_agent_channel();
            channel_iface
                .call_on_message_persisted(&mut store, &metadata_json)
                .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

            Ok::<_, WasmChannelError>(())
        })
        .await
        .map_err(|e| WasmChannelError::ExecutionPanicked {
            name: channel_name,
            reason: e.to_string(),
        })?
    })
    .await;

    match result {
        Ok(Ok(())) => {
            tracing::debug!(channel = %self.name, "on_message_persisted completed");
            Ok(())
        }
        Ok(Err(e)) => {
            // Log but don't fail - this is best-effort
            tracing::warn!(channel = %self.name, error = %e, "on_message_persisted failed");
            Ok(())
        }
        Err(_timeout) => {
            tracing::warn!(channel = %self.name, "on_message_persisted timed out");
            Ok(())
        }
    }
}
```

- [ ] **Step 4: Test compilation**

Run: `cargo check`
Expected: No compilation errors

- [ ] **Step 5: Commit**

```bash
git add wit/channel.wit src/channels/wasm/wrapper.rs
git commit -m "feat(wasm): add on_message_persisted callback to WIT interface

Allows channels to perform follow-up actions after message persistence,
such as WhatsApp mark_as_read API calls. Best-effort execution - failures
are logged but do not block the ACK."
```

---

## Chunk 6: WhatsApp Channel Updates

### Task 6.1: Update WhatsApp capabilities and implementation

**Files:**
- Modify: `channels-src/whatsapp/whatsapp.capabilities.json`
- Modify: `channels-src/whatsapp/src/lib.rs`

**Note:** API version stays at v18.0 (matching upstream/main) - only adding new fields.

- [ ] **Step 1: Update capabilities file**

```json
{
  "version": "0.2.0",
  "wit_version": "0.3.0",
  "type": "channel",
  "name": "whatsapp",
  "description": "WhatsApp Cloud API channel for receiving and responding to WhatsApp messages",
  "setup": {
    "required_secrets": [
      {
        "name": "whatsapp_access_token",
        "prompt": "Enter your WhatsApp Cloud API permanent access token (from the Meta Developer Portal under your app's WhatsApp > API Setup).",
        "validation": "^[A-Za-z0-9_-]+$"
      },
      {
        "name": "whatsapp_verify_token",
        "prompt": "Webhook verify token (leave empty to auto-generate)",
        "optional": true,
        "auto_generate": { "length": 32 }
      },
      {
        "name": "whatsapp_app_secret",
        "prompt": "Enter your WhatsApp App Secret (from Meta Developer Portal > App Settings > Basic). Used for HMAC signature verification.",
        "validation": "^[a-f0-9]{32}$",
        "optional": true
      }
    ],
    "validation_endpoint": "https://graph.facebook.com/v18.0/me?access_token={whatsapp_access_token}",
    "setup_url": "https://developers.facebook.com/apps"
  },
  "capabilities": {
    "http": {
      "allowlist": [
        { "host": "graph.facebook.com", "path_prefix": "/" }
      ],
      "rate_limit": {
        "requests_per_minute": 80,
        "requests_per_hour": 1000
      }
    },
    "secrets": {
      "allowed_names": ["whatsapp_*"]
    },
    "channel": {
      "allowed_paths": ["/webhook/whatsapp"],
      "allow_polling": false,
      "workspace_prefix": "channels/whatsapp/",
      "emit_rate_limit": {
        "messages_per_minute": 100,
        "messages_per_hour": 5000
      },
      "webhook": {
        "secret_header": "X-Hub-Signature-256",
        "secret_name": "whatsapp_verify_token",
        "verification_mode": "query_param",
        "hmac_secret_name": "whatsapp_app_secret",
        "message_id_json_pointer": "/message_id"
      }
    }
  },
  "config": {
    "api_version": "v18.0",
    "reply_to_message": true,
    "owner_id": null,
    "dm_policy": "pairing",
    "allow_from": []
  }
}
```

- [ ] **Step 2: Implement on_message_persisted in WhatsApp channel**

```rust
// In channels-src/whatsapp/src/lib.rs, add to impl Guest:

fn on_message_persisted(metadata_json: String) -> Result<(), String> {
    channel_host::log(
        channel_host::LogLevel::Debug,
        "on_message_persisted callback invoked",
    );

    // Parse metadata to get message_id and phone_number_id
    let metadata: WhatsAppMessageMetadata = match serde_json::from_str(&metadata_json) {
        Ok(m) => m,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to parse metadata in on_message_persisted: {}", e),
            );
            // Don't fail the ACK - just log and return
            return Ok(());
        }
    };

    // Skip if no message_id (shouldn't happen, but defensive)
    if metadata.message_id.is_empty() {
        channel_host::log(
            channel_host::LogLevel::Debug,
            "Skipping mark_as_read - no message_id in metadata",
        );
        return Ok(());
    }

    // Read api_version from workspace (set during on_start), fallback to default
    let api_version = channel_host::workspace_read("channels/whatsapp/api_version")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "v18.0".to_string());

    // Build WhatsApp mark_as_read API URL
    let url = format!(
        "https://graph.facebook.com/{}/{}/messages",
        api_version, metadata.phone_number_id
    );

    // Build mark_as_read payload
    let payload = serde_json::json!({
        "messaging_product": "whatsapp",
        "status": "read",
        "message_id": metadata.message_id
    });

    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| format!("Failed to serialize mark_as_read payload: {}", e))?;

    // Headers with Bearer token placeholder
    // Host will inject the actual access token
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
    });

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!("Calling mark_as_read for message: {}", metadata.message_id),
    );

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(http_response) => {
            if http_response.status >= 200 && http_response.status < 300 {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Marked message {} as read", metadata.message_id),
                );
            } else {
                let body_str = String::from_utf8_lossy(&http_response.body);
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "mark_as_read API error: {} - {}",
                        http_response.status, body_str
                    ),
                );
            }
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("mark_as_read HTTP request failed: {}", e),
            );
        }
    }

    // Always return Ok - mark_as_read is best-effort
    Ok(())
}
```

- [ ] **Step 3: Build WASM**

Run: `cargo build -p whatsapp --target wasm32-wasip2 --release`
Expected: Successful build

- [ ] **Step 4: Commit**

```bash
git add channels-src/whatsapp/whatsapp.capabilities.json channels-src/whatsapp/src/lib.rs
git commit -m "feat(whatsapp): add HMAC signature verification and mark_as_read

- Add optional whatsapp_app_secret for HMAC verification
- Add verification_mode: query_param for GET/POST differentiation
- Add message_id_json_pointer for deduplication
- Implement on_message_persisted for mark_as_read API calls"
```

---

## Chunk 7: Main.rs Integration

### Task 7.1: Wire router.set_db() on startup

**Files:**
- Modify: `src/main.rs`

**Context:** The router needs access to the database for webhook deduplication. This must be called during app initialization.

- [ ] **Step 1: Find router initialization in main.rs**

Search for where `wasm_channel_router` is created and passed to app state.

- [ ] **Step 2: Add set_db() call after database initialization**

```rust
// In src/main.rs, after database is initialized and before app starts,
// find where the router is available and add:

// Wire database to router for webhook deduplication
if let Some(db) = &db {
    let db_clone = db.clone();
    let router = &wasm_channel_router; // or however it's named
    router.set_db(db_clone).await;
    tracing::info!("Webhook deduplication enabled");
} else {
    tracing::warn!("Webhook deduplication disabled - no database configured");
}
```

- [ ] **Step 3: Test compilation**

Run: `cargo check`
Expected: No compilation errors

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire database to WASM channel router for webhook deduplication

Calls router.set_db() during startup if database is available.
Logs warning if deduplication is disabled due to missing database."
```

---

## Chunk 8: Integration Tests

### Task 8.1: Add integration tests

**Files:**
- Modify: `tests/wasm_channel_integration.rs`

- [ ] **Step 1: Add HMAC verification test**

```rust
// In tests/wasm_channel_integration.rs, add:

use crate::channels::wasm::signature::verify_hmac_sha256;

#[test]
fn test_whatsapp_hmac_signature_verification() {
    // Test vectors from WhatsApp documentation
    let secret = "test_app_secret";
    let body = br#"{"entry":[{"id":"123456789","changes":[{"field":"messages","value":{"messages":[{"id":"wamid.HBgM..."}]}}]}]}"#;

    // Compute valid signature
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    // Verify
    assert!(
        verify_hmac_sha256(secret, &sig, body),
        "Valid signature should verify"
    );

    // Wrong secret
    assert!(
        !verify_hmac_sha256("wrong_secret", &sig, body),
        "Wrong secret should fail"
    );

    // Tampered body
    assert!(
        !verify_hmac_sha256(secret, &sig, br#"{"tampered":"data"}"#),
        "Tampered body should fail"
    );
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test wasm_channel_integration::test_whatsapp`
Expected: Test passes

- [ ] **Step 3: Commit**

```bash
git add tests/wasm_channel_integration.rs
git commit -m "test(wasm): add HMAC signature verification integration test"
```

---

## Final Verification

- [ ] **Run full test suite with postgres**

Run: `cargo test --features postgres`
Expected: All tests pass

- [ ] **Run full test suite with libsql**

Run: `cargo test --no-default-features --features libsql`
Expected: All tests pass

- [ ] **Run clippy**

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: Zero warnings

- [ ] **Run fmt**

Run: `cargo fmt --check`
Expected: No formatting issues

---

## Summary

This plan delivers:

| Feature | Description | Chunk |
|---------|-------------|-------|
| **WhatsApp HMAC** | `verify_hmac_sha256` for webhook signature verification | 1 |
| **Schema extensions** | `verification_mode`, `hmac_secret_name`, `message_id_json_pointer` | 2 |
| **Deduplication DB** | `WebhookDedupStore` trait + PostgreSQL + libSQL | 3 |
| **Router integration** | New fields in router, updated `register()` signature | 4 |
| **WIT callback** | `on_message_persisted` for post-persistence actions | 5 |
| **WhatsApp channel** | HMAC config + mark_as_read implementation | 6 |
| **Main.rs wiring** | Connect DB to router for deduplication | 7 |
| **Integration tests** | HMAC verification tests | 8 |

**Estimated Effort:** 4-5 hours for experienced Rust developer

**Dependencies:**
- Chunks 1-3 are independent and can be done in parallel
- Chunk 4 depends on chunks 1-3
- Chunk 5 is independent
- Chunk 6 depends on chunks 2, 5
- Chunk 7 depends on chunks 3, 4
- Chunk 8 depends on chunk 1

**Breaking Changes:** None - all new fields are optional, `register()` signature extended with `Option` params.
