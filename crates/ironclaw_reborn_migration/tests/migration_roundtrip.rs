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
use ironclaw_reborn_migration::{Domain, MigrationOptions, SourceDb, TargetStore, run_migration};
use ironclaw_triggers::{LibSqlTriggerRepository, TriggerRepository, TriggerSchedule};
use secrecy::SecretString;
use uuid::Uuid;

const TENANT: &str = "acme";
const AGENT: &str = "assistant";
const USER: &str = "alice";
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
    // A default (Active) install so the migration derives Enabled activation.
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
    // the migration derives a credential binding to the migrated secret and
    // records the capability-config gap. `openai_api_key` matches the seeded
    // secret above.
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
        report.stats.memory_documents, 1,
        "memory: {:?}",
        report.stats
    );

    // ── the gap set: the EXACT expected lossy count per domain, so any newly
    // dropped (or newly-recovered) value fails the build. Counts are pinned to
    // the fixture above; see the inline breakdown per domain. ──
    let expected_losses = [
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
        (Domain::Extension, 2),
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
    // 1 installed wasm tool → ExtensionInstallation.
    assert_eq!(report.stats.extensions, 1, "extensions: {:?}", report.stats);

    // Extension installation invariants: the on-disk installation record must
    // carry Enabled activation (v1 tool status was Active) and the credential
    // binding derived from tool_capabilities.allowed_secrets (openai_api_key).
    let installation_doc =
        reborn_entry_content(&dst, "%/system/extensions/.installations/state.json").await;
    assert!(
        installation_doc.contains("openai_api_key"),
        "installation record must carry the allowed_secrets credential binding; got: {installation_doc}"
    );
    // Match the exact serialized `ExtensionActivationState` token (snake_case
    // JSON value), not a loose substring, so a future `*_enabled`/`disabled`
    // state can't pass by accident.
    assert!(
        installation_doc.contains("\"enabled\""),
        "an Active v1 tool must migrate to Enabled activation; got: {installation_doc}"
    );
    assert!(
        !installation_doc.contains("\"disabled\""),
        "the migrated installation must not be Disabled; got: {installation_doc}"
    );

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
