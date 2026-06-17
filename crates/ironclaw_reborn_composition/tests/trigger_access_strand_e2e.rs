//! End-to-end regression coverage for issue #4992 — the "T2" gap.
//!
//! Existing helper-level tests (in `src/lib.rs`) exercise the fire-time access
//! checker (`impl TriggerFireAccessChecker for RebornLibSqlLocalTriggerAccessStore`)
//! in isolation. What was missing is an END-TO-END test that drives the real
//! trigger poller with the REAL store-backed checker, so the strand→self-heal and
//! strand→deny behavior is exercised through the actual fire path that records
//! `run_id` / `thread_id` in `trigger_run_history`.
//!
//! These tests wire `Arc<RebornLibSqlLocalTriggerAccessStore>` (which implements
//! `TriggerFireAccessChecker`) as the runtime's checker — NOT the
//! `AllowingTriggerFireAccessChecker` stub used by `runtime.rs` unit tests — and
//! open that store on the SAME `reborn-local-dev.db` libSQL file the runtime
//! opens (`<local_dev_root>/reborn-local-dev.db`, mirroring
//! `serve.rs::with_local_trigger_fire_access_checker`). The poller therefore runs
//! the production `CreatorAccessRequired` authorizer against the live store, so
//! self-heal / deny logic actually executes on a real fire.
//!
//! Requires `test-support` (for the runtime test-support seams:
//! `trigger_repository()`, `trigger_conversation_pairing()`,
//! `with_model_gateway_override`) plus `webui-v2-beta` (for the access-store
//! re-exports and the `TriggerFireAccessChecker` impl) and `libsql`.

#![cfg(all(feature = "test-support", feature = "webui-v2-beta"))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_conversations::{AdapterInstallationId, AdapterKind, ExternalActorRef};
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    LocalTriggerAccessReconciliation, LocalTriggerAccessRole, LocalTriggerAccessSeed,
    LocalTriggerAccessSource, RebornCompositionProfile, RebornLibSqlLocalTriggerAccessStore,
    RebornLocalRuntimeProfileOptions, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput,
    TriggerAccessRepairAction, TriggerPollerSettings, build_reborn_runtime,
    local_runtime_build_input_with_options, open_local_trigger_access_store,
    repair_local_trigger_access,
};
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerCompletionPolicy, TriggerId, TriggerRecord,
    TriggerRepository, TriggerRunHistoryStatus, TriggerRunRecord, TriggerSchedule,
    TriggerSourceKind, TriggerState,
};
use tokio::sync::Mutex as TokioMutex;

const TENANT: &str = "trigger-strand-tenant";
const AGENT: &str = "trigger-strand-agent";
const TRIGGER_PROMPT: &str = "trigger-strand-prompt-marker";

/// A model gateway that records nothing but always succeeds, so an authorized
/// fire can reach accepted-turn submission and `mark_fire_accepted` records a
/// `run_id` / `thread_id`.
#[derive(Debug, Default)]
struct RecordingGateway {
    requests: Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().await.push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            "trigger strand e2e ok".to_string(),
        ))
    }
}

/// Path the runtime's local-dev libSQL substrate lives on. The composition
/// factory opens `<local_dev_root>/reborn-local-dev.db`
/// (`factory.rs::build_local_dev_root_filesystem`), and
/// `serve.rs::with_local_trigger_fire_access_checker` opens the access store on
/// the exact same file. We mirror that so the store-backed checker and the
/// runtime share one substrate.
fn local_dev_root(root: &tempfile::TempDir) -> std::path::PathBuf {
    root.path().join("local-dev")
}

fn substrate_db_path(root: &tempfile::TempDir) -> std::path::PathBuf {
    local_dev_root(root).join("reborn-local-dev.db")
}

/// Build a local-dev runtime whose trigger poller uses the production
/// `CreatorAccessRequired` authorizer wired to the REAL store-backed checker.
///
/// The store must be opened first (it creates + migrates the
/// `local_reborn_access` table on the shared substrate file), then handed in as
/// the access checker before the runtime opens the same file for its own tables.
async fn build_runtime_with_real_checker(
    root: &tempfile::TempDir,
    owner: &str,
    recording_gateway: Arc<RecordingGateway>,
    access_store: Arc<RebornLibSqlLocalTriggerAccessStore>,
) -> RebornRuntime {
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        owner,
        local_dev_root(root),
        RebornLocalRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root);

    let input = RebornRuntimeInput::from_services(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "trigger-strand-source".to_string(),
            reply_target_binding_id: "trigger-strand-reply".to_string(),
        })
        // `enabled()` keeps the production `CreatorAccessRequired` authorizer
        // (NOT the tenant-scoped placeholder), so the wired checker actually runs.
        .with_trigger_poller_settings(TriggerPollerSettings::enabled().with_worker_config(
            ironclaw_triggers::TriggerPollerWorkerConfig {
                poll_interval: Duration::from_millis(20),
                ..Default::default()
            },
        ))
        // THE wiring under test: the real libSQL store, not a stub.
        .with_trigger_fire_access_checker(access_store)
        .with_model_gateway_override(
            Arc::clone(&recording_gateway) as Arc<dyn HostManagedModelGateway>
        );

    build_reborn_runtime(input).await.expect("runtime builds")
}

/// Pair the trigger creator's external actor through the production
/// `ConversationActorPairingService` API. Trusted trigger submission fails
/// closed for unpaired actors by design; in production, onboarding establishes
/// this pairing before any trigger can be created.
async fn pair_creator(runtime: &RebornRuntime, tenant_id: &TenantId, user_id: &UserId) {
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("trigger poller runtime exposes conversation pairing service");
    pairing
        .pair_external_actor(
            tenant_id.clone(),
            AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("adapter kind"),
            AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                .expect("installation id"),
            ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, user_id.as_str())
                .expect("actor ref"),
            user_id.clone(),
        )
        .await
        .expect("pair external actor for trigger creator");
}

fn due_trigger(
    trigger_id: TriggerId,
    tenant_id: &TenantId,
    creator: &UserId,
    agent_id: &AgentId,
    name: &str,
) -> TriggerRecord {
    TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: creator.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: None,
        name: name.to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
        completion_policy: TriggerCompletionPolicy::CompleteAfterFirstFire,
        prompt: TRIGGER_PROMPT.to_string(),
        state: TriggerState::Scheduled,
        next_run_at: Utc::now() - chrono::Duration::seconds(120),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    }
}

/// Poll `trigger_run_history` until one row is present for `trigger_id` whose
/// status is terminal (not `Running`), or the deadline elapses.
async fn wait_for_run_history_row(
    repo: &Arc<dyn TriggerRepository>,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
    deadline: Duration,
) -> Option<TriggerRunRecord> {
    let stop = Instant::now() + deadline;
    let mut last: Option<TriggerRunRecord> = None;
    while Instant::now() < stop {
        let rows = repo
            .list_trigger_run_history(tenant_id.clone(), trigger_id, 10)
            .await
            .expect("list run history");
        if let Some(row) = rows.into_iter().next() {
            if row.status != TriggerRunHistoryStatus::Running {
                return Some(row);
            }
            last = Some(row);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last
}

/// Scenario 1 (AC1/AC3 — the headline regression): a trigger whose creator has
/// NO `local_reborn_access` row for its exact scope must still fire end-to-end.
/// The checker self-heals the absent scope, the fire reaches accepted-turn
/// submission, and the run-history row carries a non-null `run_id` AND
/// non-null `thread_id`. Afterward the store holds an active row for the exact
/// scope, seeded by the self-heal.
#[tokio::test]
async fn absent_strand_self_heals_and_fires_end_to_end() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway::default());
    let owner = "strand-absent-owner";

    // Open the store on the shared substrate file FIRST so the access table is
    // migrated before the runtime opens the same file.
    let access_store = open_local_trigger_access_store(&substrate_db_path(&root))
        .await
        .expect("open local trigger access store");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let creator = UserId::new(owner).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    // Precondition: no access row exists for this creator/scope — the strand.
    assert!(
        !access_store
            .has_active_local_access(&tenant_id, &creator, Some(&agent_id), None)
            .await
            .expect("read access"),
        "no active access row should exist before the fire (the strand)"
    );

    let runtime = build_runtime_with_real_checker(
        &root,
        owner,
        Arc::clone(&recording_gateway),
        Arc::clone(&access_store),
    )
    .await;

    pair_creator(&runtime, &tenant_id, &creator).await;

    let trigger_id = TriggerId::new();
    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    repo.upsert_trigger(due_trigger(
        trigger_id,
        &tenant_id,
        &creator,
        &agent_id,
        "trigger-strand-absent",
    ))
    .await
    .expect("upsert trigger record");

    let row =
        wait_for_run_history_row(&repo, &tenant_id, trigger_id, Duration::from_secs(20)).await;

    runtime.shutdown().await.expect("runtime shutdown");

    let row = row.expect("a terminal run-history row should be recorded within 20s");
    assert_eq!(
        row.status,
        TriggerRunHistoryStatus::Ok,
        "self-healed fire should reach accepted-turn submission with Ok status — row: {row:?}",
    );
    assert!(
        row.run_id.is_some(),
        "self-healed fire must record a non-null run_id (reached submission) — row: {row:?}",
    );
    assert!(
        row.thread_id.is_some(),
        "self-healed fire must record a non-null thread_id (canonical thread established) — row: {row:?}",
    );
    assert!(
        row.failure_reason.is_none(),
        "a successful fire must not carry a failure_reason — row: {row:?}",
    );

    // The self-heal seeded an active row for the exact scope.
    assert!(
        access_store
            .has_active_local_access(&tenant_id, &creator, Some(&agent_id), None)
            .await
            .expect("read access"),
        "self-heal should have seeded an active access row for the exact scope",
    );
    // And the active row was seeded by the self-heal bootstrap source: the
    // creator now appears among the active users for LocalDevTriggerCreateBootstrap.
    let bootstrap_users = access_store
        .list_active_user_ids_for_source(
            &tenant_id,
            LocalTriggerAccessSource::LocalDevTriggerCreateBootstrap,
        )
        .await
        .expect("list active users for self-heal source");
    assert!(
        bootstrap_users.contains(&creator),
        "self-heal must seed an active row under source LocalDevTriggerCreateBootstrap — got {bootstrap_users:?}",
    );
}

/// Scenario 2 (AC2 negative): a creator whose SSO-bootstrap access row was
/// revoked (deactivated via reconcile) must be DENIED end-to-end. The
/// run-history row has status `Error`, `run_id = None`, `thread_id = None`, and
/// a populated, sanitized `failure_reason` that references the denial — never a
/// fake run/thread.
#[tokio::test]
async fn revoked_creator_is_denied_without_a_fake_thread() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway::default());
    let owner = "strand-revoked-owner";

    let access_store = open_local_trigger_access_store(&substrate_db_path(&root))
        .await
        .expect("open local trigger access store");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let creator = UserId::new(owner).expect("user id");
    let keeper = UserId::new("strand-revoked-keeper").expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    // Seed an SSO-bootstrap access row for the creator...
    access_store
        .seed_local_access(LocalTriggerAccessSeed {
            tenant_id: &tenant_id,
            user_id: &creator,
            agent_id: Some(&agent_id),
            project_id: None,
            role: LocalTriggerAccessRole::Owner,
            source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
        })
        .await
        .expect("seed sso access");
    // ...then deactivate it by reconciling the same source/scope with a
    // DIFFERENT user set, so the creator's row becomes inactive (Revoked).
    access_store
        .reconcile_local_access(LocalTriggerAccessReconciliation {
            tenant_id: &tenant_id,
            user_ids: &[keeper],
            agent_id: Some(&agent_id),
            project_id: None,
            role: LocalTriggerAccessRole::Owner,
            source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
        })
        .await
        .expect("reconcile (revoke) sso access");
    assert!(
        !access_store
            .has_active_local_access(&tenant_id, &creator, Some(&agent_id), None)
            .await
            .expect("read access"),
        "creator access row should be revoked (inactive) before the fire",
    );

    let runtime = build_runtime_with_real_checker(
        &root,
        owner,
        Arc::clone(&recording_gateway),
        Arc::clone(&access_store),
    )
    .await;

    pair_creator(&runtime, &tenant_id, &creator).await;

    let trigger_id = TriggerId::new();
    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    repo.upsert_trigger(due_trigger(
        trigger_id,
        &tenant_id,
        &creator,
        &agent_id,
        "trigger-strand-revoked",
    ))
    .await
    .expect("upsert trigger record");

    let row =
        wait_for_run_history_row(&repo, &tenant_id, trigger_id, Duration::from_secs(20)).await;

    runtime.shutdown().await.expect("runtime shutdown");

    let row = row.expect("a terminal Error run-history row should be recorded within 20s");
    assert_eq!(
        row.status,
        TriggerRunHistoryStatus::Error,
        "a revoked creator's fire must record an Error row — row: {row:?}",
    );
    assert!(
        row.run_id.is_none(),
        "a denied fire must NOT record a run_id (denied before run creation) — row: {row:?}",
    );
    assert!(
        row.thread_id.is_none(),
        "a denied fire must NOT record a thread_id (no fake thread) — row: {row:?}",
    );
    let reason = row
        .failure_reason
        .as_ref()
        .map(|reason| reason.as_str().to_ascii_lowercase())
        .expect("a denied fire must carry a sanitized failure_reason");
    assert!(
        reason.contains("revoked") || reason.contains("not authorized"),
        "failure_reason should explain the denial (revoked / not authorized) — got {reason:?}",
    );

    // The store must NOT have been self-healed over the revocation.
    assert!(
        !access_store
            .has_active_local_access(&tenant_id, &creator, Some(&agent_id), None)
            .await
            .expect("read access"),
        "a revoked creator must stay revoked — the checker must not self-heal over a revocation",
    );
}

/// Scenario 3 (AC1 reconcile path): an env/run reconcile must not strand a
/// still-seeded SSO creator. Seed an `LocalDevSsoBootstrap` row for creator A,
/// create A's trigger, then run the production-style env reconcile
/// (`LocalDevEnvBootstrap` source with a DIFFERENT runtime-owner user set,
/// mirroring `serve.rs::with_local_trigger_fire_access_checker`). Because the
/// SSO-bootstrap rows are a distinct source (additive, untouched by the env
/// reconcile), A's access stays active and the fire succeeds end-to-end.
#[tokio::test]
async fn relogin_env_reconcile_does_not_strand_seeded_sso_creator() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway::default());
    let owner = "strand-reconcile-owner";

    let access_store = open_local_trigger_access_store(&substrate_db_path(&root))
        .await
        .expect("open local trigger access store");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let creator_a = UserId::new(owner).expect("user id");
    let runtime_owner = UserId::new("strand-reconcile-runtime-owner").expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    // Creator A was bootstrapped through SSO login.
    access_store
        .seed_local_access(LocalTriggerAccessSeed {
            tenant_id: &tenant_id,
            user_id: &creator_a,
            agent_id: Some(&agent_id),
            project_id: None,
            role: LocalTriggerAccessRole::Owner,
            source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
        })
        .await
        .expect("seed sso access for creator A");

    // Simulate restart/relogin: the production env reconcile runs with the
    // runtime-owner user set under the SEPARATE LocalDevEnvBootstrap source.
    // This mirrors serve.rs::with_local_trigger_fire_access_checker.
    access_store
        .reconcile_local_access(LocalTriggerAccessReconciliation {
            tenant_id: &tenant_id,
            user_ids: std::slice::from_ref(&runtime_owner),
            agent_id: Some(&agent_id),
            project_id: None,
            role: LocalTriggerAccessRole::Owner,
            source: LocalTriggerAccessSource::LocalDevEnvBootstrap,
        })
        .await
        .expect("env reconcile");

    // The env reconcile is a different source, so A's SSO row stays active.
    assert!(
        access_store
            .has_active_local_access(&tenant_id, &creator_a, Some(&agent_id), None)
            .await
            .expect("read access"),
        "the env reconcile (different source) must not deactivate creator A's SSO access",
    );

    let runtime = build_runtime_with_real_checker(
        &root,
        owner,
        Arc::clone(&recording_gateway),
        Arc::clone(&access_store),
    )
    .await;

    pair_creator(&runtime, &tenant_id, &creator_a).await;

    let trigger_id = TriggerId::new();
    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    repo.upsert_trigger(due_trigger(
        trigger_id,
        &tenant_id,
        &creator_a,
        &agent_id,
        "trigger-strand-reconcile",
    ))
    .await
    .expect("upsert trigger record");

    let row =
        wait_for_run_history_row(&repo, &tenant_id, trigger_id, Duration::from_secs(20)).await;

    runtime.shutdown().await.expect("runtime shutdown");

    let row = row.expect("a terminal run-history row should be recorded within 20s");
    assert_eq!(
        row.status,
        TriggerRunHistoryStatus::Ok,
        "a still-seeded SSO creator's fire must succeed after an env reconcile — row: {row:?}",
    );
    assert!(
        row.run_id.is_some(),
        "the fire must record a non-null run_id (env reconcile did not strand the creator) — row: {row:?}",
    );
    assert!(
        row.thread_id.is_some(),
        "the fire must record a non-null thread_id — row: {row:?}",
    );
}

/// Scenario 4 (#4992 reassign regression — the false-success trap): a trigger
/// whose original creator is GONE is reassigned to a fresh owner via the
/// operator repair tool. The repair seeds the new owner's access row AND must
/// re-pair the new owner's trusted external actor in the same filesystem-backed
/// conversation store the poller binds against. The test deliberately NEVER
/// calls `pair_creator` for the target — only the repair pairs it. After the
/// reassign, the target's fire must bind successfully end-to-end (status `Ok`,
/// non-null `run_id` AND `thread_id`).
///
/// WITHOUT the repair's pairing fix this fails: fire-time binding fails closed
/// for the unpaired reassigned actor, so the run-history row is `Error` with
/// `run_id = None` / `thread_id = None`, even though the repair reported
/// success and the access row exists.
#[tokio::test]
async fn reassigned_owner_fires_end_to_end_after_repair_pairs_the_actor() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway::default());
    let runtime_owner = "strand-reassign-runtime-owner";

    // Open the store on the shared substrate file FIRST so the access table is
    // migrated before the repair / runtime open the same file.
    let access_store = open_local_trigger_access_store(&substrate_db_path(&root))
        .await
        .expect("open local trigger access store");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let gone_creator = UserId::new("strand-reassign-gone-creator").expect("user id");
    let target = UserId::new("strand-reassign-target-owner").expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    // Seed a trigger owned by the gone creator on the substrate trigger repo so
    // the repair's strand scan finds it. The repair opens its own libSQL handle
    // on the same file, so the record must already be persisted there.
    let trigger_id = TriggerId::new();
    {
        let repo: Arc<dyn TriggerRepository> = open_local_trigger_access_store_repo(&root).await;
        repo.upsert_trigger(due_trigger(
            trigger_id,
            &tenant_id,
            &gone_creator,
            &agent_id,
            "trigger-strand-reassign",
        ))
        .await
        .expect("seed stranded trigger");
    }

    // Precondition: neither the gone creator nor the target has active access.
    assert!(
        !access_store
            .has_active_local_access(&tenant_id, &gone_creator, Some(&agent_id), None)
            .await
            .expect("read access"),
        "the gone creator must be stranded before the repair",
    );
    assert!(
        !access_store
            .has_active_local_access(&tenant_id, &target, Some(&agent_id), None)
            .await
            .expect("read access"),
        "the target must have no access before the repair",
    );

    // Run the operator repair: reassign the stranded trigger to the target.
    // This rewrites `creator_user_id`, seeds the target's access row, AND must
    // re-pair the target's trusted external actor.
    let applied = repair_local_trigger_access(
        &substrate_db_path(&root),
        &tenant_id,
        TriggerAccessRepairAction::Reassign(target.clone()),
    )
    .await
    .expect("reassign repair");
    assert_eq!(applied.reassigned, 1, "exactly one trigger reassigned");
    assert_eq!(applied.reassigned_to.as_deref(), Some(target.as_str()));

    // Build the runtime AFTER the repair so the poller reads the repaired state.
    let runtime = build_runtime_with_real_checker(
        &root,
        runtime_owner,
        Arc::clone(&recording_gateway),
        Arc::clone(&access_store),
    )
    .await;

    // Deliberately do NOT pair the target here — the repair must have done it.

    // Make the reassigned trigger due now so the poller fires it.
    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let reassigned = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get reassigned trigger")
        .expect("reassigned trigger present");
    assert_eq!(
        reassigned.creator_user_id, target,
        "the repair must have rewritten the trigger creator to the target",
    );
    repo.upsert_trigger(due_trigger(
        trigger_id,
        &tenant_id,
        &target,
        &agent_id,
        "trigger-strand-reassign",
    ))
    .await
    .expect("re-arm reassigned trigger as due");

    let row =
        wait_for_run_history_row(&repo, &tenant_id, trigger_id, Duration::from_secs(20)).await;

    runtime.shutdown().await.expect("runtime shutdown");

    let row = row.expect("a terminal run-history row should be recorded within 20s");
    assert_eq!(
        row.status,
        TriggerRunHistoryStatus::Ok,
        "the reassigned owner's fire must bind and reach accepted-turn submission \
         (the repair re-paired the actor) — row: {row:?}",
    );
    assert!(
        row.run_id.is_some(),
        "the reassigned owner's fire must record a non-null run_id — row: {row:?}",
    );
    assert!(
        row.thread_id.is_some(),
        "the reassigned owner's fire must record a non-null thread_id \
         (binding succeeded for the paired reassigned actor) — row: {row:?}",
    );
    assert!(
        row.failure_reason.is_none(),
        "a successful reassigned fire must not carry a failure_reason — row: {row:?}",
    );
}

/// Open a trigger repository on the shared substrate file to seed a trigger
/// record before the repair runs. Mirrors how the repair and runtime open the
/// same `reborn-local-dev.db` substrate.
async fn open_local_trigger_access_store_repo(
    root: &tempfile::TempDir,
) -> Arc<dyn TriggerRepository> {
    use ironclaw_triggers::LibSqlTriggerRepository;
    let path = substrate_db_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create substrate dir");
    }
    let db = Arc::new(
        libsql::Builder::new_local(&path)
            .build()
            .await
            .expect("open substrate db"),
    );
    let repo = LibSqlTriggerRepository::new(db);
    repo.run_migrations().await.expect("migrate trigger repo");
    Arc::new(repo)
}
