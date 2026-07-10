//! Acceptance test: seed a rich v1 + engine-v2 libSQL fixture, run the
//! migration into a fresh Reborn libSQL store, and assert the full round-trip —
//! threads/messages/routines/missions convert, and the manifest lists exactly
//! the expected lossy items so nothing is silently dropped. A separate dry-run
//! case asserts the report is produced with nothing written.
//!
//! Docker-free (libSQL on tempdirs). Gated `required-features = ["libsql"]`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw::agent::routine::{NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger};
use ironclaw::config::{DatabaseBackend, DatabaseConfig, SslMode};
use ironclaw::db::{Database, DatabaseHandles, UserIdentityRecord, connect_with_handles};
use ironclaw::secrets::{CreateSecretParams, SecretsCrypto, create_secrets_store};
use ironclaw::tools::wasm::{LibSqlWasmToolStore, StoreToolParams, TrustLevel, WasmToolStore};
use ironclaw_host_api::TenantId;
use ironclaw_reborn_migration::{
    ApplyAcknowledgements, Domain, MigrationOptions, MigrationSecretInputs, MigrationStatus,
    SourceDb, TargetStore, apply_migration, plan_migration, resume_migration, run_migration,
    verify_migration,
};
use ironclaw_triggers::{LibSqlTriggerRepository, TriggerRepository, TriggerSchedule};
use secrecy::SecretString;
use uuid::Uuid;

const TENANT: &str = "acme";
const AGENT: &str = "assistant";
const USER: &str = "alice";
const SUSPENDED_USER: &str = "bob";
const DEACTIVATED_USER: &str = "carol";
const LEGACY_DATA_OWNER: &str = "legacy-owner";
/// 64-char string ≥ 32 bytes (used verbatim as HKDF IKM by v1 + Reborn crypto).
const MASTER_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn libsql_config(path: &Path) -> DatabaseConfig {
    DatabaseConfig {
        backend: DatabaseBackend::LibSql,
        url: SecretString::from("unused://libsql"),
        pool_size: 4,
        ssl_mode: SslMode::default(),
        libsql_path: Some(path.to_path_buf()),
        libsql_url: None,
        libsql_auth_token: None,
    }
}

/// Build a v1 routine with a given trigger + action. All the guardrail/notify
/// fields are populated so the field-loss assertions have something to find.
fn routine(name: &str, trigger: Trigger, action: RoutineAction, enabled: bool) -> Routine {
    let now = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    Routine {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: format!("desc for {name}"),
        user_id: USER.to_string(),
        enabled,
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(300),
            max_concurrent: 2,
            dedup_window: Some(std::time::Duration::from_secs(60)),
        },
        notify: NotifyConfig {
            channel: Some("telegram".into()),
            user: Some(USER.into()),
            on_attention: true,
            on_failure: true,
            on_success: false,
        },
        last_run_at: Some(now),
        next_fire_at: Some(now),
        run_count: 7,
        consecutive_failures: 1,
        state: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

fn lightweight() -> RoutineAction {
    RoutineAction::Lightweight {
        prompt: "summarize my day".into(),
        context_paths: vec!["context/priorities.md".into()],
        max_tokens: 2048,
        use_tools: true,
        max_tool_rounds: 3,
    }
}

fn full_job() -> RoutineAction {
    RoutineAction::FullJob {
        title: "Nightly report".into(),
        description: "produce the nightly report".into(),
        max_iterations: 10,
    }
}

/// Seed a rich v1 + engine-v2 fixture and return its libSQL path (kept alive by
/// the returned `TempDir`).
async fn seed_v1_fixture(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("v1.db");
    let (db, handles) = connect_with_handles(&libsql_config(&path))
        .await
        .expect("open v1 fixture");

    // ── conversations + messages ──
    let c1 = db
        .create_conversation("gateway", USER, None)
        .await
        .expect("conv1");
    db.add_conversation_message(c1, "user", "hello there")
        .await
        .expect("m1");
    db.add_conversation_message(c1, "assistant", "hi! how can I help?")
        .await
        .expect("m2");
    db.add_conversation_message(c1, "system", "session started")
        .await
        .expect("m3 (system → recorded loss)");

    let c2 = db
        .create_conversation("telegram", USER, None)
        .await
        .expect("conv2");
    db.add_conversation_message(c2, "user", "what's on my calendar?")
        .await
        .expect("m4");
    db.add_conversation_message(c2, "assistant", "you have 2 meetings")
        .await
        .expect("m5");

    // ── routines: every trigger variant × both actions ──
    for r in [
        routine("cron-light", cron("0 9 * * *"), lightweight(), true),
        routine("cron-fulljob", cron("0 18 * * MON-FRI"), full_job(), false),
        routine(
            "event-r",
            Trigger::Event {
                channel: Some("telegram".into()),
                pattern: "deploy".into(),
            },
            lightweight(),
            true,
        ),
        routine(
            "sysevent-r",
            Trigger::SystemEvent {
                source: "github".into(),
                event_type: "issue.opened".into(),
                filters: Default::default(),
            },
            lightweight(),
            true,
        ),
        routine(
            "webhook-r",
            Trigger::Webhook {
                path: Some("gh".into()),
                secret: None,
            },
            lightweight(),
            true,
        ),
        routine("manual-r", Trigger::Manual, lightweight(), true),
    ] {
        db.create_routine(&r).await.expect("create routine");
    }

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
    write_engine_doc(
        db.as_ref(),
        ".system/engine/projects/p1/missions/daily-digest/mission.json",
        &cron_mission,
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
    write_engine_doc(
        db.as_ref(),
        ".system/engine/projects/p1/missions/on-deploy/mission.json",
        &event_mission,
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
    write_engine_doc(
        db.as_ref(),
        &format!(".system/engine/runtime/threads/active/{mission_thread_id}.json"),
        &thread_blob,
    )
    .await;

    // ── a non-engine memory document ──
    write_doc(db.as_ref(), "context/vision.md", "# Vision\nbe helpful").await;

    // ── settings ──
    let mut settings = std::collections::HashMap::new();
    settings.insert("model".to_string(), serde_json::json!("gpt-4"));
    settings.insert("timezone".to_string(), serde_json::json!("UTC"));
    db.set_all_settings(USER, &settings)
        .await
        .expect("seed settings");
    db.set_all_settings(
        LEGACY_DATA_OWNER,
        &std::collections::HashMap::from([(
            "legacy_timezone".to_string(),
            serde_json::json!("UTC"),
        )]),
    )
    .await
    .expect("seed pre-users-table data owner");

    seed_identities(db.as_ref(), &handles).await;
    seed_secret(&handles).await;
    seed_wasm_tool(&handles).await;

    path
}

/// A v1 user + OAuth identity (via the Database trait) and a channel identity
/// (raw insert — no trait writer exists).
async fn seed_identities(db: &dyn Database, handles: &DatabaseHandles) {
    let now = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    let conn = handles
        .libsql_db
        .as_ref()
        .expect("libsql handle")
        .connect()
        .expect("connect");
    // Raw users insert — `get_or_create_user` would additionally seed an
    // assistant thread (a real v1 behavior, but it would perturb the thread
    // counts this test pins), so insert the row directly.
    conn.execute(
        "INSERT INTO users (id, email, display_name, status, role, created_at, updated_at, metadata) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
        (
            USER.to_string(),
            "alice@example.com".to_string(),
            "Alice".to_string(),
            "active".to_string(),
            "member".to_string(),
            now.to_rfc3339(),
            "{}".to_string(),
        ),
    )
    .await
    .expect("seed user row");

    // Case-insensitive historical values, lifecycle fields, creator relation,
    // and object metadata must survive the canonical user import.
    conn.execute(
        "INSERT INTO users (id, email, display_name, status, role, created_at, updated_at, last_login_at, created_by, metadata) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?6, ?7, ?8)",
        (
            SUSPENDED_USER.to_string(),
            "bob@example.com".to_string(),
            "Bob".to_string(),
            "SUSPENDED".to_string(),
            "ADMIN".to_string(),
            now.to_rfc3339(),
            USER.to_string(),
            serde_json::json!({"team": "infra"}).to_string(),
        ),
    )
    .await
    .expect("seed suspended admin user");
    conn.execute(
        "INSERT INTO users (id, email, display_name, status, role, created_at, updated_at, created_by, metadata) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
        (
            DEACTIVATED_USER.to_string(),
            "carol@example.com".to_string(),
            "Carol".to_string(),
            "deactivated".to_string(),
            "member".to_string(),
            now.to_rfc3339(),
            SUSPENDED_USER.to_string(),
            serde_json::json!({"team": "operations", "quota": 3}).to_string(),
        ),
    )
    .await
    .expect("seed deactivated user");

    db.create_api_token(
        USER,
        "migration-fixture-token",
        &[0xAB; 32],
        "deadbeef",
        None,
    )
    .await
    .expect("seed API token hash");

    db.create_identity(&UserIdentityRecord {
        id: Uuid::new_v4(),
        user_id: USER.to_string(),
        provider: "google".into(),
        provider_user_id: "google-sub-123".into(),
        email: Some("alice@example.com".into()),
        email_verified: true,
        display_name: Some("Alice".into()),
        avatar_url: None,
        raw_profile: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    })
    .await
    .expect("seed user identity");

    // channel_identities has no Database writer; insert raw (reuse `conn`).
    conn.execute(
        "INSERT INTO channel_identities (id, owner_id, channel, external_id, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            Uuid::new_v4().to_string(),
            USER.to_string(),
            "telegram".to_string(),
            "tg-999".to_string(),
            now.to_rfc3339(),
        ),
    )
    .await
    .expect("seed channel identity");
}

async fn seed_secret(handles: &DatabaseHandles) {
    let crypto = Arc::new(SecretsCrypto::new(SecretString::from(MASTER_KEY)).expect("crypto"));
    let store = create_secrets_store(crypto, handles).expect("v1 secrets store");
    let mut params = CreateSecretParams::new("openai_api_key", "sk-secret-value");
    params.provider = Some("openai".into());
    store.create(USER, params).await.expect("seed secret");
}

async fn seed_wasm_tool(handles: &DatabaseHandles) {
    let db = handles.libsql_db.clone().expect("libsql handle");
    let store = LibSqlWasmToolStore::new(db.clone());
    // The source artifact is inventoried and reported for reinstall; it must
    // not become a synthesized Reborn installation.
    let tool = store
        .store(StoreToolParams {
            user_id: USER.to_string(),
            name: "weather".to_string(),
            version: "1.0.0".to_string(),
            wit_version: "0.1.0".to_string(),
            description: "Weather lookup".to_string(),
            wasm_binary: b"\0asm-fake-binary".to_vec(),
            parameters_schema: serde_json::json!({"type": "object"}),
            source_url: None,
            trust_level: TrustLevel::User,
        })
        .await
        .expect("seed wasm tool");

    // Seed tool_capabilities with an allowed secret (no trait writer exists) so
    // the migration records the unsupported capability and credential-binding
    // policy explicitly.
    let conn = db.connect().expect("connect");
    conn.execute(
        "INSERT INTO tool_capabilities (id, wasm_tool_id, allowed_secrets) VALUES (?1, ?2, ?3)",
        (
            Uuid::new_v4().to_string(),
            tool.id.to_string(),
            serde_json::json!(["openai_api_key"]).to_string(),
        ),
    )
    .await
    .expect("seed tool capabilities");
}

fn cron(expr: &str) -> Trigger {
    Trigger::Cron {
        schedule: expr.to_string(),
        timezone: Some("UTC".to_string()),
    }
}

async fn write_engine_doc(db: &dyn Database, path: &str, value: &serde_json::Value) {
    write_doc(db, path, &serde_json::to_string(value).unwrap()).await;
}

async fn write_doc(db: &dyn Database, path: &str, content: &str) {
    let doc = db
        .get_or_create_document_by_path(USER, None, path)
        .await
        .expect("create doc");
    db.update_document(doc.id, content)
        .await
        .expect("write doc content");
}

fn options(src: PathBuf, dst: PathBuf, dry_run: bool) -> MigrationOptions {
    MigrationOptions {
        source: SourceDb::LibSql { path: src },
        target: TargetStore::LibSql { path: dst },
        profile: "test-migration".to_string(),
        tenant_id: TenantId::new(TENANT).unwrap(),
        agent_id: ironclaw_host_api::AgentId::new(AGENT).unwrap(),
        secret_master_key: Some(SecretString::from(MASTER_KEY)),
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

/// Count append-log records. Finalized assistant messages are durably appended
/// to the production thread service's message log and indexed, rather than
/// always materialized as an entry row; both representations are first-class.
async fn reborn_event_count(path: &Path, like: &str) -> i64 {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT count(*) FROM root_filesystem_events WHERE path LIKE ?1",
            [like],
        )
        .await
        .expect("query events");
    let row = rows.next().await.expect("row").expect("some row");
    row.get::<i64>(0).expect("count")
}

async fn reborn_message_count(path: &Path) -> i64 {
    reborn_entry_count(path, "/tenants/acme/users/alice/threads/%/messages/%.json").await
        + reborn_event_count(path, "/tenants/acme/users/alice/threads/%/message_appends").await
}

/// Whether any persisted entry under `like` contains a UTF-8 payload fragment.
async fn reborn_entry_contains(path: &Path, like: &str, needle: &str) -> bool {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT contents FROM root_filesystem_entries WHERE path LIKE ?1",
            [like],
        )
        .await
        .expect("query entry contents");
    while let Some(row) = rows.next().await.expect("row") {
        let blob = row.get::<Vec<u8>>(0).expect("contents blob");
        if String::from_utf8_lossy(&blob).contains(needle) {
            return true;
        }
    }
    false
}

async fn reborn_entry_json(path: &Path, entry_path: &str) -> serde_json::Value {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT contents FROM root_filesystem_entries WHERE path = ?1",
            [entry_path],
        )
        .await
        .expect("query entry contents");
    let row = rows.next().await.expect("row").expect("persisted entry");
    serde_json::from_slice(&row.get::<Vec<u8>>(0).expect("contents blob")).expect("entry JSON")
}

fn reborn_user_entry_path(user_id: &str) -> String {
    let encoded = match user_id {
        USER => "YWxpY2U",
        SUSPENDED_USER => "Ym9i",
        DEACTIVATED_USER => "Y2Fyb2w",
        LEGACY_DATA_OWNER => "bGVnYWN5LW93bmVy",
        other => panic!("missing expected base64url fixture path for {other}"),
    };
    format!("/tenants/{TENANT}/shared/reborn-identity/users/{encoded}.json")
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

async fn overwrite_trigger_prompt(path: &Path, name: &str, prompt: &str) {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open reborn db");
    let conn = db.connect().expect("connect");
    conn.execute(
        "UPDATE trigger_records SET prompt = ?1 WHERE tenant_id = ?2 AND name = ?3",
        (prompt, TENANT, name),
    )
    .await
    .expect("mutate trigger collision fixture");
}

#[tokio::test]
async fn migrates_v1_and_engine_v2_state_without_loss() {
    let dir = tempfile::tempdir().unwrap();
    let src = seed_v1_fixture(dir.path()).await;
    // Keep the Reborn home distinct from the v1 source home. Target-owned
    // database/key artifacts must never become new source inventory on resume.
    let dst = dir.path().join("reborn/reborn.db");

    let migration_options = options(src.clone(), dst.clone(), false);
    let manifest = plan_migration(&migration_options)
        .await
        .expect("migration plan");
    let blockers: Vec<_> = manifest
        .inventory
        .iter()
        .filter(|item| item.blocker.is_some())
        .collect();
    assert!(
        blockers.is_empty(),
        "the known-source fixture must have an explicit disposition for every category: {blockers:#?}"
    );
    let report = apply_migration(
        migration_options,
        &manifest,
        MigrationSecretInputs {
            source_master_key: Some(SecretString::from(MASTER_KEY)),
            // Local Reborn composition resolves/persists its own production
            // target key after apply preconditions.
            target_master_key: None,
        },
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect("migration runs");

    // ── converted counts ──
    assert_eq!(report.stats.users, 4, "users: {:?}", report.stats);
    // 2 conversations + 1 mission thread.
    assert_eq!(report.stats.threads, 3, "threads: {:?}", report.stats);
    // 2 cron routines converted (event/sysevent/webhook/manual do not).
    assert_eq!(report.stats.routines, 2, "routines: {:?}", report.stats);
    // both missions are counted (even the non-cron one).
    assert_eq!(report.stats.missions, 2, "missions: {:?}", report.stats);
    // user+assistant messages: conv1 (2) + conv2 (2) + mission thread (2) = 6.
    assert_eq!(report.stats.messages, 6, "messages: {:?}", report.stats);
    assert_eq!(
        report.stats.memory_documents, 1,
        "memory: {:?}",
        report.stats
    );

    // ── the gap set: the EXACT expected lossy count per domain, so any newly
    // dropped (or newly-recovered) value fails the build. Counts are pinned to
    // the fixture above; see the inline breakdown per domain. ──
    let expected_losses = [
        // deactivated -> suspended and one non-string metadata value -> JSON text.
        (Domain::User, 2),
        // Hash-only API tokens cannot be converted and require re-authentication.
        (Domain::ApiToken, 1),
        // owner/thread/mission ids are all valid → no thread-identity losses.
        (Domain::Thread, 0),
        // conv1's single "system" transcript message (no first-class append path).
        (Domain::Message, 1),
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
        // the migrated wasm tool: manifest_fidelity + capabilities (2).
        // Unknown v1 WASM is report-only: package reinstall + capabilities.
        (Domain::Extension, 2),
        // unconditional pairing_requests gap (both identities adopt cleanly).
        (Domain::Identity, 1),
        // one heartbeat-state gap per discovered canonical/data-owner user.
        (Domain::Heartbeat, 4),
        // one gap per seeded setting key (model, timezone, legacy_timezone).
        (Domain::Setting, 3),
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
        (Domain::Extension, "package"),
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
    // Unknown executable artifacts are accounted for but never installed.
    assert_eq!(report.stats.extensions, 0, "extensions: {:?}", report.stats);
    let report_json = report.to_json().expect("report JSON");
    assert!(!report_json.contains("deadbeef"), "token prefix leaked");
    assert!(!report_json.contains(&"ab".repeat(32)), "token hash leaked");
    assert!(
        dst.parent()
            .expect("target parent")
            .join(".reborn-local-dev-secrets-master-key")
            .is_file(),
        "apply must persist the same local target key a production Reborn boot resolves"
    );

    // On-disk durability of the deferred domains (fresh connection).
    assert!(
        reborn_entry_count(&dst, "%/secrets/%openai_api_key.json").await >= 1,
        "expected the migrated secret document on disk"
    );
    assert_eq!(
        reborn_entry_count(&dst, "%/system/extensions/.installations/state.json").await,
        0,
        "an incompatible placeholder installation must never be persisted"
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

    // ── production-path durability of threads (fresh connection) ──
    assert_eq!(
        reborn_entry_count(&dst, "/tenants/acme/users/alice/threads/%/thread.json").await,
        3,
        "migration must use composition's production tenant/user mount layout"
    );
    assert_eq!(
        reborn_message_count(&dst).await,
        6,
        "expected exactly the supported transcript rows across production's row and append-log representations"
    );
    assert!(
        reborn_entry_count(&dst, "/tenants/acme/shared/reborn-identity/%").await >= 2,
        "identity rows must use production's tenant-shared mount, not the legacy global path"
    );
    assert_eq!(
        reborn_entry_count(&dst, "/tenants/acme/shared/reborn-identity/users/%.json").await,
        4,
        "all canonical and synthesized users must use the tenant-shared production path"
    );
    let bob = reborn_entry_json(&dst, &reborn_user_entry_path(SUSPENDED_USER)).await;
    assert_eq!(bob["status"], "suspended");
    assert_eq!(bob["role"], "admin");
    assert_eq!(bob["created_by"], USER);
    assert_eq!(bob["tenant_id"], TENANT);
    assert_eq!(bob["last_login_at"], "2024-01-02T03:04:05+00:00");
    assert_eq!(bob["metadata"]["team"], "infra");
    let carol = reborn_entry_json(&dst, &reborn_user_entry_path(DEACTIVATED_USER)).await;
    assert_eq!(carol["status"], "suspended");
    assert_eq!(carol["role"], "member");
    assert_eq!(carol["created_by"], SUSPENDED_USER);
    assert_eq!(carol["metadata"]["quota"], "3");
    let legacy = reborn_entry_json(&dst, &reborn_user_entry_path(LEGACY_DATA_OWNER)).await;
    assert_eq!(legacy["status"], "active");
    assert_eq!(legacy["role"], "member");
    assert_eq!(legacy["created_at"], "1970-01-01T00:00:00+00:00");
    assert_eq!(legacy["metadata"]["migration.synthesized"], "true");
    assert_eq!(
        reborn_entry_count(&dst, "/tenant-shared/%").await,
        0,
        "migration must not write identity data to an unscoped global mount"
    );
    assert!(
        reborn_entry_contains(&dst, "%/thread.json", "session started").await,
        "non-user/assistant content must be retained in the migration archive metadata"
    );

    // ── idempotency: resume replays the same sealed source identity and must
    // compare-and-apply without duplicating triggers or transcript rows. ──
    let applied_manifest = report.manifest.clone().expect("applied manifest");
    let replay_options = options(src.clone(), dst.clone(), false);
    let report2 = resume_migration(
        replay_options,
        &applied_manifest,
        MigrationSecretInputs {
            source_master_key: Some(SecretString::from(MASTER_KEY)),
            target_master_key: None,
        },
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect("migration resume");
    assert_eq!(
        report2.stats.identities, 2,
        "re-run must re-adopt the same 2 identities"
    );
    assert_eq!(report2.stats.users, 4, "resume must replay exact users");
    assert_eq!(
        reborn_entry_count(&dst, "/tenants/acme/shared/reborn-identity/users/%.json").await,
        4,
        "resume duplicated users"
    );
    assert_eq!(
        report2.stats.extensions, 0,
        "resume must not install incompatible extensions"
    );
    assert_eq!(
        reborn_entry_count(&dst, "%/system/extensions/.installations/state.json").await,
        0,
        "resume must not write placeholder installation state"
    );
    assert_eq!(
        reborn_triggers(&dst).await.len(),
        3,
        "resume duplicated triggers"
    );
    assert_eq!(
        reborn_message_count(&dst).await,
        6,
        "resume duplicated transcript messages"
    );

    let verified = verify_migration(
        &options(src.clone(), dst.clone(), false),
        report2.manifest.as_ref().expect("resume manifest"),
    )
    .await
    .expect("cold target verification");
    assert_eq!(verified.status, MigrationStatus::Verified);

    // A deterministic target slot containing different data is a conflict,
    // not an invitation for migration to overwrite operator/runtime state.
    overwrite_trigger_prompt(&dst, "cron-light", "divergent target prompt").await;
    let resume_manifest = report2.manifest.as_ref().expect("resume manifest");
    let collision = resume_migration(
        options(src, dst.clone(), false),
        resume_manifest,
        MigrationSecretInputs {
            source_master_key: Some(SecretString::from(MASTER_KEY)),
            target_master_key: None,
        },
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect_err("divergent deterministic target must fail closed");
    assert!(
        collision.to_string().contains("refusing to overwrite"),
        "unexpected collision error: {collision}"
    );
    assert_eq!(
        reborn_triggers(&dst)
            .await
            .into_iter()
            .find(|trigger| trigger.name == "cron-light")
            .expect("cron-light trigger")
            .prompt,
        "divergent target prompt",
        "migration must not overwrite a divergent deterministic slot"
    );
}

#[tokio::test]
async fn dry_run_reports_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let src = seed_v1_fixture(dir.path()).await;
    let target_parent = dir.path().join("missing-target");
    let dst = target_parent.join("reborn-dry.db");

    let report = run_migration(options(src, dst.clone(), true))
        .await
        .expect("dry run");

    // Planning reports inventory through the manifest without invoking writers.
    let manifest = report.manifest.expect("plan manifest");
    assert!(
        manifest
            .inventory
            .iter()
            .any(|item| item.source_name == "conversations" && item.count >= 2)
    );
    assert!(report.dry_run);

    // No read helper is invoked here: opening libSQL would itself create state.
    assert!(
        !dst.exists() && !target_parent.exists(),
        "planning created the target path or its parent"
    );
}

