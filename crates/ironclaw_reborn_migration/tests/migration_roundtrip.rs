//! Acceptance test: seed a rich v1 + engine-v2 libSQL fixture, run the
//! migration into a fresh Reborn libSQL store, and assert the full round-trip —
//! threads/messages/routines/missions convert, and the manifest lists exactly
//! the expected lossy items so nothing is silently dropped. A separate dry-run
//! case asserts the report is produced with nothing written.
//!
//! Fixture seeding uses raw SQL against a frozen snapshot of the v1 schema
//! (`fixtures/legacy_v1_schema.sql`, formerly `src/db/libsql_migrations.rs`)
//! instead of the real v1 write path — `ironclaw_legacy` (`src/`) was deleted
//! under Tier B (see this crate's `CLAUDE.md`, "Decoupled from
//! `ironclaw_legacy`"). The v1 secrets encryption scheme (AES-256-GCM +
//! HKDF-SHA256) is duplicated here on the encrypt side only, so the seeded
//! secret is realistic ciphertext the migration's frozen decrypt path
//! (`src/legacy_snapshot/secrets.rs`) can actually decrypt.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, AeadCore, OsRng};
use aes_gcm::{Aes256Gcm, KeyInit};
use chrono::{DateTime, TimeZone, Utc};
use hkdf::Hkdf;
use ironclaw_host_api::TenantId;
use ironclaw_reborn_migration::{Domain, MigrationOptions, SourceDb, TargetStore, run_migration};
use ironclaw_triggers::{LibSqlTriggerRepository, TriggerRepository, TriggerSchedule};
use secrecy::SecretString;
use sha2::Sha256;
use uuid::Uuid;

const TENANT: &str = "acme";
const AGENT: &str = "assistant";
const USER: &str = "alice";
const USER_BOB: &str = "bob";
/// 64-char string ≥ 32 bytes (used verbatim as HKDF IKM by v1 + Reborn crypto).
const MASTER_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Frozen snapshot of the v1 libSQL schema (see the fixture file's header
/// comment for provenance and the "do not diverge" rationale).
const V1_SCHEMA: &str = include_str!("fixtures/legacy_v1_schema.sql");

async fn open_v1_fixture_db(path: &Path) -> libsql::Database {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open v1 fixture");
    let conn = db.connect().expect("connect");
    conn.execute_batch(V1_SCHEMA)
        .await
        .expect("apply v1 schema");
    db
}

// ── routine trigger/action shape helpers ────────────────────────────────────
// Each returns (trigger_type/action_type, config JSON) exactly matching what
// v1's `Trigger::from_db`/`RoutineAction::from_db` parsed (now frozen in
// `src/legacy_snapshot/types.rs`).

fn cron_trigger(expr: &str) -> (&'static str, serde_json::Value) {
    (
        "cron",
        serde_json::json!({ "schedule": expr, "timezone": "UTC" }),
    )
}

fn event_trigger(channel: &str, pattern: &str) -> (&'static str, serde_json::Value) {
    (
        "event",
        serde_json::json!({ "channel": channel, "pattern": pattern }),
    )
}

fn system_event_trigger(source: &str, event_type: &str) -> (&'static str, serde_json::Value) {
    (
        "system_event",
        serde_json::json!({ "source": source, "event_type": event_type, "filters": {} }),
    )
}

fn webhook_trigger(path: &str) -> (&'static str, serde_json::Value) {
    (
        "webhook",
        serde_json::json!({ "path": path, "secret": null }),
    )
}

fn manual_trigger() -> (&'static str, serde_json::Value) {
    ("manual", serde_json::json!({}))
}

fn lightweight_action() -> (&'static str, serde_json::Value) {
    (
        "lightweight",
        serde_json::json!({
            "prompt": "summarize my day",
            "context_paths": ["context/priorities.md"],
            "max_tokens": 2048,
            "use_tools": true,
            "max_tool_rounds": 3,
        }),
    )
}

fn full_job_action() -> (&'static str, serde_json::Value) {
    (
        "full_job",
        serde_json::json!({
            "title": "Nightly report",
            "description": "produce the nightly report",
            "max_iterations": 10,
        }),
    )
}

/// Seed one `routines` row. Field values (guardrails/notify/counters) match
/// what the original fixture populated via the real v1 `Routine` struct, so
/// the field-loss assertions downstream have the same things to find.
async fn insert_routine(
    conn: &libsql::Connection,
    name: &str,
    trigger: (&str, serde_json::Value),
    action: (&str, serde_json::Value),
    enabled: bool,
    now: DateTime<Utc>,
) {
    let (trigger_type, trigger_config) = trigger;
    let (action_type, action_config) = action;
    conn.execute(
        "INSERT INTO routines (
            id, name, description, user_id, enabled,
            trigger_type, trigger_config, action_type, action_config,
            cooldown_secs, max_concurrent, dedup_window_secs,
            notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention,
            state, last_run_at, next_fire_at, run_count, consecutive_failures,
            created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
        libsql::params![
            Uuid::new_v4().to_string(),
            name,
            format!("desc for {name}"),
            USER,
            enabled as i64,
            trigger_type,
            trigger_config.to_string(),
            action_type,
            action_config.to_string(),
            300i64,
            2i64,
            60i64,
            "telegram",
            USER,
            0i64,
            1i64,
            1i64,
            "{}",
            now.to_rfc3339(),
            now.to_rfc3339(),
            7i64,
            1i64,
            now.to_rfc3339(),
            now.to_rfc3339(),
        ],
    )
    .await
    .expect("seed routine");
}

async fn insert_conversation(conn: &libsql::Connection, channel: &str, user_id: &str) -> Uuid {
    let id = Uuid::new_v4();
    conn.execute(
        "INSERT INTO conversations (id, channel, user_id) VALUES (?1, ?2, ?3)",
        libsql::params![id.to_string(), channel, user_id],
    )
    .await
    .expect("seed conversation");
    id
}

async fn insert_conversation_message(
    conn: &libsql::Connection,
    conversation_id: Uuid,
    role: &str,
    content: &str,
) {
    conn.execute(
        "INSERT INTO conversation_messages (id, conversation_id, role, content) VALUES (?1, ?2, ?3, ?4)",
        libsql::params![Uuid::new_v4().to_string(), conversation_id.to_string(), role, content],
    )
    .await
    .expect("seed conversation message");
}

async fn insert_memory_document(
    conn: &libsql::Connection,
    user_id: &str,
    path: &str,
    content: &str,
) {
    conn.execute(
        "INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata) \
         VALUES (?1, ?2, NULL, ?3, ?4, '{}')",
        libsql::params![Uuid::new_v4().to_string(), user_id, path, content],
    )
    .await
    .expect("seed memory document");
}

async fn insert_setting(
    conn: &libsql::Connection,
    user_id: &str,
    key: &str,
    value: &serde_json::Value,
) {
    conn.execute(
        "INSERT INTO settings (user_id, key, value) VALUES (?1, ?2, ?3)",
        libsql::params![user_id, key, value.to_string()],
    )
    .await
    .expect("seed setting");
}

/// A v1 user + OAuth identity (via `user_identities`) and a channel identity
/// (`channel_identities`).
async fn seed_identities(conn: &libsql::Connection, now: DateTime<Utc>) {
    for (user_id, email, display_name) in [
        (USER, "alice@example.com", "Alice"),
        (USER_BOB, "bob@example.com", "Bob"),
    ] {
        conn.execute(
            "INSERT INTO users (id, email, display_name, status, role, created_at, updated_at, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
            libsql::params![
                user_id,
                email,
                display_name,
                "active",
                "member",
                now.to_rfc3339(),
                "{}",
            ],
        )
        .await
        .expect("seed user row");
    }

    conn.execute(
        "INSERT INTO user_identities (
            id, user_id, provider, provider_user_id, email, email_verified,
            display_name, avatar_url, raw_profile, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, '{}', ?8, ?8)",
        libsql::params![
            Uuid::new_v4().to_string(),
            USER,
            "google",
            "google-sub-123",
            "alice@example.com",
            1i64,
            "Alice",
            now.to_rfc3339(),
        ],
    )
    .await
    .expect("seed user identity");

    // channel_identities has no v1 typed writer either — raw insert.
    conn.execute(
        "INSERT INTO channel_identities (id, owner_id, channel, external_id, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![
            Uuid::new_v4().to_string(),
            USER,
            "telegram",
            "tg-999",
            now.to_rfc3339(),
        ],
    )
    .await
    .expect("seed channel identity");
}

const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;

/// Encrypts test-fixture secret material with the same AES-256-GCM +
/// HKDF-SHA256 scheme v1 used (frozen, decrypt-only, in
/// `src/legacy_snapshot/secrets.rs`), so the migration's decrypt path has
/// real ciphertext to decrypt. Production code only ever needs to decrypt;
/// this small encrypt-side duplicate exists solely to seed test fixtures.
fn v1_encrypt(master_key: &str, plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut salt = vec![0u8; KEY_SIZE];
    OsRng.fill_bytes(&mut salt);
    let hk = Hkdf::<Sha256>::new(Some(&salt), master_key.as_bytes());
    let mut derived = [0u8; KEY_SIZE];
    hk.expand(b"near-agent-secrets-v1", &mut derived)
        .expect("HKDF-SHA256 expand to 32 bytes never fails");
    let cipher = Aes256Gcm::new_from_slice(&derived).expect("valid key length");
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce, plaintext).expect("encrypt");
    let mut encrypted = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    encrypted.extend_from_slice(&nonce);
    encrypted.extend_from_slice(&ciphertext);
    (encrypted, salt)
}

async fn seed_secret(conn: &libsql::Connection, now: DateTime<Utc>) {
    let (encrypted_value, key_salt) = v1_encrypt(MASTER_KEY, b"sk-secret-value");
    conn.execute(
        "INSERT INTO secrets (id, user_id, name, encrypted_value, key_salt, provider, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        libsql::params![
            Uuid::new_v4().to_string(),
            USER,
            "openai_api_key",
            libsql::Value::Blob(encrypted_value),
            libsql::Value::Blob(key_salt),
            "openai",
            now.to_rfc3339(),
        ],
    )
    .await
    .expect("seed secret");
}

/// Same-named "weather" tool installed by two users (exercises
/// canonicalization/dedup): alice active, bob disabled. Each gets a
/// `tool_capabilities` row granting `openai_api_key`.
async fn seed_wasm_tool(conn: &libsql::Connection, now: DateTime<Utc>) {
    for (user_id, status) in [(USER, "active"), (USER_BOB, "disabled")] {
        let tool_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO wasm_tools (
                id, user_id, name, version, wit_version, description,
                wasm_binary, binary_hash, parameters_schema, trust_level, status,
                created_at, updated_at
            ) VALUES (?1, ?2, 'weather', '1.0.0', '0.1.0', 'Weather lookup', ?3, ?4, ?5, 'user', ?6, ?7, ?7)",
            libsql::params![
                tool_id.to_string(),
                user_id,
                libsql::Value::Blob(b"\0asm-fake-binary".to_vec()),
                libsql::Value::Blob(vec![0u8; 32]),
                serde_json::json!({"type": "object"}).to_string(),
                status,
                now.to_rfc3339(),
            ],
        )
        .await
        .expect("seed wasm tool");

        conn.execute(
            "INSERT INTO tool_capabilities (id, wasm_tool_id, allowed_secrets) VALUES (?1, ?2, ?3)",
            libsql::params![
                Uuid::new_v4().to_string(),
                tool_id.to_string(),
                serde_json::json!(["openai_api_key"]).to_string(),
            ],
        )
        .await
        .expect("seed tool capabilities");
    }
}

/// Seed a rich v1 + engine-v2 fixture and return its libSQL path (kept alive by
/// the returned `TempDir`).
async fn seed_v1_fixture(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("v1.db");
    let db = open_v1_fixture_db(&path).await;
    let conn = db.connect().expect("connect");
    let now = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();

    // ── conversations + messages ──
    let c1 = insert_conversation(&conn, "gateway", USER).await;
    insert_conversation_message(&conn, c1, "user", "hello there").await;
    insert_conversation_message(&conn, c1, "assistant", "hi! how can I help?").await;
    insert_conversation_message(&conn, c1, "system", "session started").await; // recorded loss

    let c2 = insert_conversation(&conn, "telegram", USER).await;
    insert_conversation_message(&conn, c2, "user", "what's on my calendar?").await;
    insert_conversation_message(&conn, c2, "assistant", "you have 2 meetings").await;
    insert_conversation_message(&conn, c2, "tool_calls", "[{\"name\":\"calendar.list\"}]").await;

    // ── routines: every trigger variant × both actions ──
    insert_routine(
        &conn,
        "cron-light",
        cron_trigger("0 9 * * *"),
        lightweight_action(),
        true,
        now,
    )
    .await;
    insert_routine(
        &conn,
        "cron-fulljob",
        cron_trigger("0 18 * * MON-FRI"),
        full_job_action(),
        false,
        now,
    )
    .await;
    insert_routine(
        &conn,
        "event-r",
        event_trigger("telegram", "deploy"),
        lightweight_action(),
        true,
        now,
    )
    .await;
    insert_routine(
        &conn,
        "sysevent-r",
        system_event_trigger("github", "issue.opened"),
        lightweight_action(),
        true,
        now,
    )
    .await;
    insert_routine(
        &conn,
        "webhook-r",
        webhook_trigger("gh"),
        lightweight_action(),
        true,
        now,
    )
    .await;
    insert_routine(
        &conn,
        "manual-r",
        manual_trigger(),
        lightweight_action(),
        true,
        now,
    )
    .await;

    // ── engine-v2 state: mission + thread blobs in memory_documents ──
    let mission_thread_id = Uuid::new_v4();
    let cron_mission = serde_json::json!({
        "id": Uuid::new_v4(),
        "project_id": Uuid::new_v4(),
        "user_id": USER,
        "name": "daily-digest",
        "goal": "compile a daily digest of important updates",
        // Failed status on a cron mission exercises the degrade-to-Paused path.
        "status": "Failed",
        "cadence": { "Cron": { "expression": "0 7 * * *", "timezone": "UTC" } },
        "current_focus": "news",
        "approach_history": ["v1", "v2"],
        "thread_history": [mission_thread_id.to_string()],
        "success_criteria": "digest delivered by 8am",
        "notify_channels": ["telegram"],
        "created_at": "2024-01-01T00:00:00Z",
    });
    insert_memory_document(
        &conn,
        USER,
        ".system/engine/projects/p1/missions/daily-digest/mission.json",
        &cron_mission.to_string(),
    )
    .await;

    let event_mission = serde_json::json!({
        "id": Uuid::new_v4(),
        "user_id": USER,
        "name": "on-deploy",
        "goal": "react to deploys",
        "status": "Failed",
        "cadence": { "OnEvent": { "event_pattern": "deploy", "channel": null } },
        "thread_history": [],
        "created_at": "2024-01-01T00:00:00Z",
    });
    insert_memory_document(
        &conn,
        USER,
        ".system/engine/projects/p1/missions/on-deploy/mission.json",
        &event_mission.to_string(),
    )
    .await;

    let thread_blob = serde_json::json!({
        "id": mission_thread_id.to_string(),
        "goal": "compile digest",
        "title": "Digest run",
        "user_id": USER,
        "state": "Completed",
        "messages": [
            { "role": "User", "content": "run the digest", "timestamp": "2024-01-01T07:00:00Z" },
            { "role": "Assistant", "content": "digest ready", "timestamp": "2024-01-01T07:00:05Z" },
        ],
        "created_at": "2024-01-01T07:00:00Z",
    });
    insert_memory_document(
        &conn,
        USER,
        &format!(".system/engine/runtime/threads/active/{mission_thread_id}.json"),
        &thread_blob.to_string(),
    )
    .await;

    // ── a non-engine memory document ──
    insert_memory_document(&conn, USER, "context/vision.md", "# Vision\nbe helpful").await;
    insert_memory_document(&conn, USER, ".system/gateway/.config", "").await;
    insert_memory_document(&conn, USER, "BOOTSTRAP.md", "# Legacy bootstrap\nkeep me").await;

    // ── settings ──
    insert_setting(&conn, USER, "model", &serde_json::json!("gpt-4")).await;
    insert_setting(&conn, USER, "timezone", &serde_json::json!("UTC")).await;

    seed_identities(&conn, now).await;
    seed_secret(&conn, now).await;
    seed_wasm_tool(&conn, now).await;

    path
}

async fn set_wasm_tool_status(path: &Path, user_id: &str, status: &str) {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open v1 fixture for status update");
    let conn = db.connect().expect("connect");
    let changed = conn
        .execute(
            "UPDATE wasm_tools SET status = ?1 WHERE user_id = ?2 AND name = ?3",
            libsql::params![status, user_id, "weather"],
        )
        .await
        .expect("update wasm tool status");
    assert_eq!(changed, 1, "expected one source tool status row");
}

async fn set_wasm_tool_description(path: &Path, user_id: &str, description: &str) {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open v1 fixture for metadata update");
    let conn = db.connect().expect("connect");
    let changed = conn
        .execute(
            "UPDATE wasm_tools SET description = ?1 WHERE user_id = ?2 AND name = ?3",
            (
                description.to_string(),
                user_id.to_string(),
                "weather".to_string(),
            ),
        )
        .await
        .expect("update wasm tool description");
    assert_eq!(changed, 1, "expected one source tool metadata row");
}

async fn set_wasm_tool_allowed_secret(path: &Path, user_id: &str, secret: &str) {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open v1 fixture for credential update");
    let conn = db.connect().expect("connect");
    let changed = conn
        .execute(
            "UPDATE tool_capabilities SET allowed_secrets = ?1 \
             WHERE wasm_tool_id = (SELECT id FROM wasm_tools WHERE user_id = ?2 AND name = ?3)",
            (
                serde_json::json!([secret]).to_string(),
                user_id.to_string(),
                "weather".to_string(),
            ),
        )
        .await
        .expect("update wasm tool credentials");
    assert_eq!(changed, 1, "expected one source tool capability row");
}

fn options(src: PathBuf, dst: PathBuf, dry_run: bool) -> MigrationOptions {
    MigrationOptions {
        source: SourceDb::LibSql { path: src },
        target: TargetStore::LibSql { path: dst },
        tenant_id: TenantId::new(TENANT).unwrap(),
        agent_id: ironclaw_host_api::AgentId::new(AGENT).unwrap(),
        secret_master_key: Some(SecretString::from(MASTER_KEY)),
        dry_run,
    }
}

fn options_without_secret_key(src: PathBuf, dst: PathBuf, dry_run: bool) -> MigrationOptions {
    MigrationOptions {
        source: SourceDb::LibSql { path: src },
        target: TargetStore::LibSql { path: dst },
        tenant_id: TenantId::new(TENANT).unwrap(),
        agent_id: ironclaw_host_api::AgentId::new(AGENT).unwrap(),
        secret_master_key: None,
        dry_run,
    }
}

/// Count rows in the Reborn store whose resolved path matches a LIKE pattern,
/// via a fresh connection — proves on-disk durability of a domain's documents.
async fn reborn_entry_count(path: &Path, like: &str) -> i64 {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT count(*) FROM root_filesystem_entries WHERE path LIKE ?1",
            [like],
        )
        .await
        .expect("query entries");
    let row = rows.next().await.expect("row").expect("some row");
    row.get::<i64>(0).expect("count")
}

async fn reborn_entry_count_by_path_and_content(
    path: &Path,
    path_like: &str,
    content_like: &str,
) -> i64 {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT count(*) FROM root_filesystem_entries WHERE path LIKE ?1 AND CAST(contents AS TEXT) LIKE ?2",
            libsql::params![path_like, content_like],
        )
        .await
        .expect("query entries by path and content");
    let row = rows.next().await.expect("row").expect("some row");
    row.get::<i64>(0).expect("count")
}

/// Read the `contents` blob of the first Reborn entry matching a LIKE pattern,
/// as UTF-8 — used to assert the shape of a written installation/thread doc.
async fn reborn_entry_content(path: &Path, like: &str) -> String {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT contents FROM root_filesystem_entries WHERE path LIKE ?1 LIMIT 1",
            [like],
        )
        .await
        .expect("query entry contents");
    let row = rows.next().await.expect("row").expect("some row");
    let blob = row.get::<Vec<u8>>(0).expect("contents blob");
    String::from_utf8(blob).expect("utf-8 contents")
}

async fn reborn_entry_contents(path: &Path, like: &str) -> Vec<String> {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT contents FROM root_filesystem_entries WHERE path LIKE ?1 ORDER BY path",
            [like],
        )
        .await
        .expect("query entry contents");
    let mut contents = Vec::new();
    while let Some(row) = rows.next().await.expect("row") {
        let blob = row.get::<Vec<u8>>(0).expect("contents blob");
        contents.push(String::from_utf8(blob).expect("utf-8 contents"));
    }
    contents
}

async fn reborn_triggers(path: &Path) -> Vec<ironclaw_triggers::TriggerRecord> {
    let db = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("open reborn db"),
    );
    let repo = LibSqlTriggerRepository::new(db);
    repo.run_migrations().await.expect("trigger migrations");
    repo.list_triggers(TenantId::new(TENANT).unwrap())
        .await
        .expect("list triggers")
}

/// Count thread.json documents in the Reborn store via a fresh connection
/// (proves on-disk durability independent of the migration's live handles).
async fn reborn_thread_doc_count(path: &Path) -> i64 {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT count(*) FROM root_filesystem_entries WHERE path LIKE '%/thread.json'",
            (),
        )
        .await
        .expect("query thread docs");
    let row = rows.next().await.expect("row").expect("some row");
    row.get::<i64>(0).expect("count")
}

#[tokio::test]
async fn migrates_v1_and_engine_v2_state_without_loss() {
    let dir = tempfile::tempdir().unwrap();
    let src = seed_v1_fixture(dir.path()).await;
    let dst = dir.path().join("reborn.db");

    let report = run_migration(options(src.clone(), dst.clone(), false))
        .await
        .expect("migration runs");

    // ── converted counts ──
    // 2 conversations + 1 mission thread.
    assert_eq!(report.stats.threads, 3, "threads: {:?}", report.stats);
    // 2 cron routines converted (event/sysevent/webhook/manual do not).
    assert_eq!(report.stats.routines, 2, "routines: {:?}", report.stats);
    // both missions are counted (even the non-cron one).
    assert_eq!(report.stats.missions, 2, "missions: {:?}", report.stats);
    // user+assistant messages: conv1 (2) + conv2 (2) + mission thread (2) = 6.
    assert_eq!(report.stats.messages, 6, "messages: {:?}", report.stats);
    assert_eq!(
        report.stats.memory_documents, 3,
        "memory: {:?}",
        report.stats
    );

    // ── the gap set: the EXACT expected lossy count per domain, so any newly
    // dropped (or newly-recovered) value fails the build. Counts are pinned to
    // the fixture above; see the inline breakdown per domain. ──
    let expected_losses = [
        // owner/thread/mission ids are all valid → no thread-identity losses.
        (Domain::Thread, 0),
        // conv1 system + conv2 tool_calls transcript messages are retained in
        // thread metadata because they have no first-class append path.
        (Domain::Message, 2),
        // 6 routines: each cron routine records 3 field losses
        // (action + guardrails/notify/counters + routine_runs); each non-cron
        // routine records 1 trigger-source loss + 2 field losses. 6 × 3 = 18.
        (Domain::Routine, 18),
        // daily-digest: mission_only_fields + status.failed + next_fire_at
        // (the fixture mission has no next_fire_at → synthesized) = 3;
        // on-deploy: cadence.on_event (1). No orphan threads (blob referenced).
        (Domain::Mission, 4),
        // fixture seeds no jobs → the job converter records nothing.
        (Domain::Job, 0),
        // single unconditional memory_document_versions gap.
        (Domain::Memory, 1),
        // the seeded secret decrypts, re-encrypts, and carries no expiry → 0.
        (Domain::Secret, 0),
        // the two source installs each record manifest_fidelity + capabilities.
        // They merge into one canonical installation row.
        (Domain::Extension, 4),
        // unconditional pairing_requests gap (both identities adopt cleanly).
        (Domain::Identity, 1),
        // unconditional heartbeat_state gap.
        (Domain::Heartbeat, 1),
        // one gap per seeded setting key (model, timezone).
        (Domain::Setting, 2),
    ];
    for (domain, expected) in expected_losses {
        assert_eq!(
            report.losses_in(domain),
            expected,
            "expected exactly {expected} recorded gap(s) for {domain:?}; \
             all losses: {:#?}",
            report.lossy
        );
    }
    // The per-domain buckets must sum to the whole report — a newly-dropped
    // value in an unasserted domain would break this even if it slipped past the
    // per-domain checks above.
    let expected_total: usize = expected_losses.iter().map(|(_, n)| n).sum();
    assert_eq!(
        report.lossy.len(),
        expected_total,
        "total lossy count must equal the sum of every asserted domain bucket"
    );
    // Semantic spot-checks on the gap set (field names, not just counts).
    let routine_trigger_gaps = report
        .lossy
        .iter()
        .filter(|l| l.domain == Domain::Routine && l.field.starts_with("trigger."))
        .count();
    assert_eq!(
        routine_trigger_gaps, 4,
        "event/sysevent/webhook/manual routines"
    );
    for (domain, field) in [
        (Domain::Mission, "cadence.on_event"),
        (Domain::Mission, "status.failed"),
        (Domain::Extension, "manifest_fidelity"),
        (Domain::Extension, "capabilities"),
    ] {
        assert!(
            report
                .lossy
                .iter()
                .any(|l| l.domain == domain && l.field == field),
            "expected a recorded {domain:?} gap for field `{field}`"
        );
    }

    // ── deferred domains now convert ──
    // 1 v1 secret decrypted + re-encrypted.
    assert_eq!(report.stats.secrets, 1, "secrets: {:?}", report.stats);
    // 1 OAuth identity + 1 channel identity adopted.
    assert_eq!(report.stats.identities, 2, "identities: {:?}", report.stats);
    // 2 same-named user installs → 1 canonical ExtensionInstallation.
    assert_eq!(report.stats.extensions, 1, "extensions: {:?}", report.stats);

    // Extension installation invariants: the on-disk installation record must
    // carry the canonical id, both private owners, fail-closed Disabled
    // activation, and the merged credential binding.
    let installation_doc =
        reborn_entry_content(&dst, "%/system/extensions/.installations/state.json").await;
    let installation_state: serde_json::Value =
        serde_json::from_str(&installation_doc).expect("installation state JSON");
    let installations = installation_state["installations"]
        .as_array()
        .expect("installation array");
    assert_eq!(
        installations.len(),
        1,
        "same-named source installs must canonicalize to one row"
    );
    let installation = &installations[0];
    assert_eq!(installation["installation_id"], "weather");
    assert_eq!(installation["extension_id"], "weather");
    assert_eq!(installation["activation_state"], "disabled");
    assert_eq!(installation["owner"]["kind"], "users");
    assert_eq!(
        installation["owner"]["user_ids"],
        serde_json::json!([USER, USER_BOB])
    );
    assert_eq!(
        installation["credential_bindings"]
            .as_array()
            .expect("credential binding array")
            .len(),
        1,
        "agreeing duplicate credential bindings must be merged"
    );
    assert!(installation_doc.contains("openai_api_key"));

    // On-disk durability of the deferred domains (fresh connection).
    assert!(
        reborn_entry_count(&dst, "%/secrets/%openai_api_key.json").await >= 1,
        "expected the migrated secret document on disk"
    );
    assert!(
        reborn_entry_count(&dst, "%/system/extensions/.installations/state.json").await >= 1,
        "expected the extension installation state document on disk"
    );

    // ── round-trip through the Reborn triggers repo ──
    let triggers = reborn_triggers(&dst).await;
    // 2 cron routines + 1 cron mission.
    assert_eq!(triggers.len(), 3, "triggers: {triggers:#?}");
    let names: Vec<&str> = triggers.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"cron-light"));
    assert!(names.contains(&"cron-fulljob"));
    assert!(names.contains(&"daily-digest"));
    let digest = triggers.iter().find(|t| t.name == "daily-digest").unwrap();
    match &digest.schedule {
        TriggerSchedule::Cron { expression, .. } => assert_eq!(expression, "0 7 * * *"),
        other => panic!("expected cron schedule, got {other:?}"),
    }

    // ── on-disk durability of threads (fresh connection) ──
    assert!(
        reborn_thread_doc_count(&dst).await >= 3,
        "expected >=3 persisted thread.json docs"
    );
    assert!(
        reborn_entry_count_by_path_and_content(&dst, "%/thread.json", "%session started%").await
            >= 1,
        "system transcript content must be retained in thread metadata"
    );
    assert!(
        reborn_entry_count_by_path_and_content(&dst, "%/thread.json", "%calendar.list%").await >= 1,
        "tool_calls transcript content must be retained in thread metadata"
    );
    assert!(
        reborn_entry_count_by_path_and_content(&dst, "%/BOOTSTRAP.md", "%Legacy bootstrap%").await
            >= 1,
        "protected legacy memory documents must be preserved by the migration backend"
    );

    // ── idempotency: re-running the migration into the same target re-adopts
    // identities (first-writer-wins) and upserts the extension installation by
    // its deterministic id, so no duplicate installation doc is written. (Trigger
    // ids are freshly minted per run, so triggers are intentionally not
    // deduplicated — the tool is a one-shot converter.) ──
    let report2 = run_migration(options(src, dst.clone(), false))
        .await
        .expect("second migration run");
    assert_eq!(
        report2.stats.identities, 2,
        "re-run must re-adopt the same 2 identities"
    );
    assert_eq!(
        report2.stats.extensions, 1,
        "re-run must upsert the same installation, not duplicate"
    );
    assert_eq!(
        reborn_entry_count(&dst, "%/system/extensions/.installations/state.json").await,
        1,
        "re-run must not write a second installation state document"
    );
    assert_eq!(
        reborn_entry_content(&dst, "%/system/extensions/.installations/state.json").await,
        installation_doc,
        "re-run must preserve deterministic canonical extension state"
    );
}

/// Caller-level migration coverage: run the public converter and reopen the
/// persisted extension state, proving that two agreeing active source rows
/// stay enabled after the shared canonical reducer merges their owners.
#[tokio::test]
async fn migrates_all_active_duplicate_users_as_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let src = seed_v1_fixture(dir.path()).await;
    set_wasm_tool_status(&src, USER_BOB, "active").await;
    let dst = dir.path().join("reborn-all-active.db");

    let report = run_migration(options(src, dst.clone(), false))
        .await
        .expect("migration runs");

    assert_eq!(report.stats.extensions, 1);
    let installation_doc =
        reborn_entry_content(&dst, "%/system/extensions/.installations/state.json").await;
    let state: serde_json::Value = serde_json::from_str(&installation_doc).unwrap();
    let installations = state["installations"].as_array().unwrap();
    assert_eq!(installations.len(), 1);
    assert_eq!(installations[0]["activation_state"], "enabled");
    assert_eq!(installations[0]["owner"]["kind"], "users");
    assert_eq!(
        installations[0]["owner"]["user_ids"],
        serde_json::json!([USER, USER_BOB])
    );
}

#[tokio::test]
async fn migration_preserves_empty_and_prompt_protected_memory_documents() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("v1_memory_edges.db");
    let dst = dir.path().join("reborn.db");

    let db = open_v1_fixture_db(&src).await;
    let conn = db.connect().expect("connect");
    insert_memory_document(&conn, USER, "context/vision.md", "# Vision\nbe helpful").await;
    insert_memory_document(&conn, USER, ".system/gateway/.config", "").await;
    insert_memory_document(&conn, USER, "BOOTSTRAP.md", "# Legacy bootstrap\nkeep me").await;
    drop(conn);
    drop(db);

    let report = run_migration(options(src, dst.clone(), false))
        .await
        .expect("migration runs");

    assert_eq!(report.stats.memory_documents, 3);
    assert_eq!(report.losses_in(Domain::Memory), 1);
    assert_eq!(
        reborn_entry_contents(&dst, "%/.system/gateway/.config").await,
        vec![String::new()],
        "empty legacy memory documents must remain present, not disappear as no-op writes"
    );
    assert_eq!(
        reborn_entry_content(&dst, "%/BOOTSTRAP.md").await,
        "# Legacy bootstrap\nkeep me",
        "prompt-protected legacy paths must be imported by the migration backend"
    );
}

#[tokio::test]
async fn migration_retains_non_first_class_transcript_content_in_thread_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("v1_transcript_edges.db");
    let dst = dir.path().join("reborn.db");

    let db = open_v1_fixture_db(&src).await;
    let conn = db.connect().expect("connect");
    let conversation = insert_conversation(&conn, "gateway", USER).await;
    insert_conversation_message(&conn, conversation, "system", "session started").await;
    insert_conversation_message(
        &conn,
        conversation,
        "tool_calls",
        "[{\"name\":\"calendar.list\"}]",
    )
    .await;
    insert_conversation_message(&conn, conversation, "tool", "{\"result\":\"ok\"}").await;
    drop(conn);
    drop(db);

    let report = run_migration(options(src, dst.clone(), false))
        .await
        .expect("migration runs");

    assert_eq!(report.stats.threads, 1);
    assert_eq!(report.stats.messages, 0);
    assert_eq!(report.losses_in(Domain::Message), 3);
    let thread_doc = reborn_entry_content(&dst, "%/thread.json").await;
    assert!(thread_doc.contains("session started"));
    assert!(thread_doc.contains("calendar.list"));
    assert!(thread_doc.contains("result"));
    assert!(thread_doc.contains("ok"));
}

#[tokio::test]
async fn migration_reports_secrets_as_unmigrated_without_master_key() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("v1_secret_edges.db");
    let dst = dir.path().join("reborn.db");

    let db = open_v1_fixture_db(&src).await;
    let conn = db.connect().expect("connect");
    let now = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    seed_secret(&conn, now).await;
    drop(conn);
    drop(db);

    let report = run_migration(options_without_secret_key(src, dst.clone(), false))
        .await
        .expect("migration runs");

    assert_eq!(report.stats.secrets, 0);
    assert_eq!(report.losses_in(Domain::Secret), 1);
    assert!(
        report.lossy.iter().any(|loss| {
            loss.domain == Domain::Secret
                && loss.source_id == "secrets"
                && loss.field == "*"
                && loss.detail.contains("secret-master-key")
        }),
        "missing master key must be explicit in the report: {:#?}",
        report.lossy
    );
    assert_eq!(
        reborn_entry_count(&dst, "%/secrets/%openai_api_key.json").await,
        0,
        "a skipped v1 secret must not create an empty Reborn secret document"
    );
}

/// Caller-level migration coverage for incompatible source metadata: the
/// grouped source rows are reported and skipped before a manifest is written.
#[tokio::test]
async fn migration_records_metadata_conflict_without_partial_extension() {
    let dir = tempfile::tempdir().unwrap();
    let src = seed_v1_fixture(dir.path()).await;
    set_wasm_tool_description(&src, USER_BOB, "Different weather metadata").await;
    let dst = dir.path().join("reborn-metadata-conflict.db");

    let report = run_migration(options(src, dst.clone(), false))
        .await
        .expect("migration runs");

    assert_eq!(report.stats.extensions, 0);
    assert_eq!(report.losses_in(Domain::Extension), 5);
    assert!(report.lossy.iter().any(|loss| {
        loss.domain == Domain::Extension
            && loss.field == "canonicalization"
            && loss.detail.contains("no partial canonical installation")
    }));
    assert_eq!(
        reborn_entry_count(&dst, "%/system/extensions/.installations/state.json").await,
        0,
        "metadata conflict must not write a partial canonical installation"
    );
}

/// Caller-level migration coverage for distinct v1 allowed secrets: because
/// v1 stores secret names rather than a separate credential-handle mapping,
/// distinct names are distinct target bindings and must merge safely. The
/// shared reducer's conflicting-handle fail-closed policy is covered at the
/// persisted-store seam, where legacy rows can contain that ambiguity.
#[tokio::test]
async fn migration_merges_distinct_credential_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let src = seed_v1_fixture(dir.path()).await;
    set_wasm_tool_allowed_secret(&src, USER_BOB, "different_api_key").await;
    let dst = dir.path().join("reborn-credential-conflict.db");

    let report = run_migration(options(src, dst.clone(), false))
        .await
        .expect("migration runs");

    assert_eq!(report.stats.extensions, 1);
    assert_eq!(report.losses_in(Domain::Extension), 4);
    let installation_doc =
        reborn_entry_content(&dst, "%/system/extensions/.installations/state.json").await;
    let state: serde_json::Value = serde_json::from_str(&installation_doc).unwrap();
    let bindings = state["installations"][0]["credential_bindings"]
        .as_array()
        .unwrap();
    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().any(|binding| {
        binding["credential_handle"] == "openai_api_key"
            && binding["secret_handle"] == "openai_api_key"
    }));
    assert!(bindings.iter().any(|binding| {
        binding["credential_handle"] == "different_api_key"
            && binding["secret_handle"] == "different_api_key"
    }));
}

#[tokio::test]
async fn dry_run_reports_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let src = seed_v1_fixture(dir.path()).await;
    let dst = dir.path().join("reborn-dry.db");

    let report = run_migration(options(src, dst.clone(), true))
        .await
        .expect("dry run");

    // Same counts as a real run …
    assert_eq!(report.stats.threads, 3);
    assert_eq!(report.stats.routines, 2);
    assert_eq!(report.stats.missions, 2);
    assert!(report.dry_run);

    // … but nothing was written to the Reborn store.
    assert!(
        reborn_triggers(&dst).await.is_empty(),
        "dry run wrote triggers"
    );
    assert_eq!(
        reborn_thread_doc_count(&dst).await,
        0,
        "dry run wrote thread docs"
    );
}

#[tokio::test]
async fn migration_reads_older_optional_tables_without_wit_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1_old_wasm.db");
    let dst = dir.path().join("reborn.db");

    let db = open_v1_fixture_db(&path).await;
    let conn = db.connect().expect("connect");
    seed_wasm_tool(&conn, Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()).await;
    conn.execute("ALTER TABLE wasm_tools DROP COLUMN wit_version", ())
        .await
        .expect("drop wasm_tools.wit_version");
    conn.execute("ALTER TABLE wasm_channels DROP COLUMN wit_version", ())
        .await
        .expect("drop wasm_channels.wit_version");
    conn.execute("DROP TABLE wasm_channels", ())
        .await
        .expect("drop optional wasm_channels table");
    conn.execute("DROP TABLE tool_capabilities", ())
        .await
        .expect("drop optional tool_capabilities table");
    conn.execute("DROP TABLE user_identities", ())
        .await
        .expect("drop optional user_identities table");
    conn.execute("DROP TABLE channel_identities", ())
        .await
        .expect("drop optional channel_identities table");
    insert_memory_document(
        &conn,
        USER,
        "context/old-wasm.md",
        "old schema still migrates",
    )
    .await;
    drop(conn);
    drop(db);

    // Run a real (non-dry) migration so the write path is genuinely exercised:
    // a dry run only counts in memory and would not prove the older schema lets
    // real writes through.
    let report = run_migration(options(path, dst.clone(), false))
        .await
        .expect("older wasm schema should not block migration");

    assert_eq!(report.stats.memory_documents, 1);
    assert_eq!(
        report.stats.extensions, 1,
        "installed tools must still migrate when the optional capabilities table is absent"
    );

    // Assert the migrated content is actually persisted on disk (the point of
    // dropping dry-run): the memory document and the extension installation
    // record must be readable back through a fresh connection.
    assert!(
        reborn_entry_count_by_path_and_content(&dst, "%old-wasm.md", "%old schema still migrates%")
            .await
            >= 1,
        "the migrated memory document must be persisted on disk, not just counted"
    );
    assert!(
        reborn_entry_count(&dst, "%/system/extensions/.installations/state.json").await >= 1,
        "the installed tool must be persisted as an extension installation on disk"
    );
}

/// `legacy_snapshot::connect` no longer runs v1 schema migrations on connect
/// (the original `ironclaw::db::connect_with_handles` did, as a side effect).
/// Against a `routines` table that predates a later v1 migration (missing
/// `notify_on_attention`), the migration must fail loud naming the gap instead
/// of silently reading a partial row — regression test for that deliberate
/// behavior change (see `CLAUDE.md` "Decoupled from `ironclaw_legacy`").
#[tokio::test]
async fn migration_fails_loud_on_stale_routines_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1_stale.db");
    let dst = dir.path().join("reborn.db");

    let db = libsql::Builder::new_local(&path)
        .build()
        .await
        .expect("open stale fixture");
    let conn = db.connect().expect("connect");
    conn.execute(
        "CREATE TABLE routines (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            user_id TEXT NOT NULL,
            enabled INTEGER NOT NULL,
            trigger_type TEXT NOT NULL,
            trigger_config TEXT NOT NULL,
            action_type TEXT NOT NULL,
            action_config TEXT NOT NULL,
            cooldown_secs INTEGER NOT NULL,
            max_concurrent INTEGER NOT NULL,
            dedup_window_secs INTEGER,
            notify_channel TEXT,
            notify_user TEXT,
            notify_on_success INTEGER NOT NULL,
            notify_on_failure INTEGER NOT NULL,
            state TEXT NOT NULL,
            last_run_at TEXT,
            next_fire_at TEXT,
            run_count INTEGER NOT NULL,
            consecutive_failures INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        (),
    )
    .await
    .expect("create stale routines table (missing notify_on_attention)");
    drop(conn);
    drop(db);

    let error = run_migration(options(path, dst, false))
        .await
        .expect_err("migration must fail loud against a stale v1 schema, not silently proceed");
    let message = error.to_string();
    assert!(
        message.contains("routines") && message.contains("notify_on_attention"),
        "error should name the missing table/column, got: {message}"
    );
}
