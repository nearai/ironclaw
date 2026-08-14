//! Unit tests for the trigger delivery-target migration sweep.
//!
//! Split out of `trigger_delivery_migration.rs` verbatim (crate precedent:
//! `model_channel_delivery/tests.rs`); behavior is unchanged.

use super::*;
use crate::outbound::{
    DeliveryTargetCapabilities, OutboundDeliveryTargetEntry, OutboundDeliveryTargetOwner,
    OutboundDeliveryTargetSummary,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::ids::{AgentId, UserId};
use ironclaw_outbound::{OutboundDeliveryTargetProvider, OutboundError};
use ironclaw_triggers::{
    InMemoryTriggerRepository, TriggerDeliveryTargetId, TriggerId, TriggerSchedule,
    TriggerSourceKind, TriggerState,
};
use ironclaw_turns::ReplyTargetBindingRef;
use std::sync::Arc;

const TENANT: &str = "migration-tenant";
const USER: &str = "migration-user";
const TARGET_ID: &str = "slack:personal-dm:T123:migration-user";
const DISPLAY_NAME: &str = "Slack DM";
const PROMPT: &str = "summarize yesterday's incidents";

fn tenant() -> TenantId {
    TenantId::new(TENANT).expect("tenant id")
}

fn record_with_target(target: Option<&str>) -> TriggerRecord {
    let fire_at = Utc::now() + chrono::Duration::days(1);
    TriggerRecord {
        trigger_id: TriggerId::new(),
        tenant_id: tenant(),
        creator_user_id: UserId::new(USER).expect("user id"),
        agent_id: Some(AgentId::new("migration-agent").expect("agent id")),
        project_id: None,
        name: "nightly digest".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::once(fire_at, "UTC").expect("once schedule"),
        prompt: PROMPT.to_string(),
        execution_spec: None,
        delivery_target: target
            .map(|target| TriggerDeliveryTargetId::new(target).expect("target id")),
        state: TriggerState::Scheduled,
        next_run_at: fire_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    }
}

/// The one catalog entry the migration is expected to resolve, claimed by
/// whichever caller asks (the registry re-stamps owner at list time).
struct StaticTargetProvider;

#[async_trait]
impl OutboundDeliveryTargetProvider for StaticTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(vec![OutboundDeliveryTargetEntry {
            summary: OutboundDeliveryTargetSummary::new(
                OutboundDeliveryTargetId::new(TARGET_ID).expect("target id"),
                "slack",
                DISPLAY_NAME,
                None,
            )
            .expect("summary"),
            capabilities: DeliveryTargetCapabilities {
                final_replies: true,
                ..Default::default()
            },
            destination: ReplyTargetBindingRef::new("reply:migration-target").expect("binding ref"),
            owner: OutboundDeliveryTargetOwner::for_scope(scope),
        }])
    }
}

fn registry_with_target() -> MutableOutboundDeliveryTargetRegistry {
    let registry = MutableOutboundDeliveryTargetRegistry::default();
    registry
        .register_provider("migration-test", Arc::new(StaticTargetProvider))
        .expect("register provider");
    registry
}

/// A registry whose only provider fails: an unavailable lookup must never
/// be mistaken for "the target is gone" and destroy the stored intent.
struct FailingProvider;

#[async_trait]
impl OutboundDeliveryTargetProvider for FailingProvider {
    async fn list_outbound_delivery_targets(
        &self,
        _scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Err(OutboundError::Backend)
    }
}

async fn read_back(repository: &InMemoryTriggerRepository, trigger_id: TriggerId) -> TriggerRecord {
    repository
        .get_trigger(tenant(), trigger_id)
        .await
        .expect("read back")
        .expect("record present")
}

#[tokio::test]
async fn resolvable_target_becomes_a_prompt_step_and_is_cleared() {
    let repository = InMemoryTriggerRepository::default();
    let record = record_with_target(Some(TARGET_ID));
    let trigger_id = record.trigger_id;
    repository.upsert_trigger(record).await.expect("seed");
    let registry = registry_with_target();

    let migrated = migrate_trigger_delivery_targets(&repository, Some(&registry), &tenant())
        .await
        .expect("migration runs");
    assert_eq!(migrated, 1, "one stored target must be migrated");

    let stored = read_back(&repository, trigger_id).await;
    assert_eq!(
        stored.prompt,
        format!(
            "{PROMPT}\n\nDeliver the result to {DISPLAY_NAME} using builtin__outbound_deliver \
             (target id: {TARGET_ID})."
        ),
        "the stored route must become an explicit delivery step in the prompt"
    );
    assert!(
        stored.delivery_target.is_none(),
        "the legacy field must be cleared once its intent lives in the prompt"
    );

    // Idempotency: a second boot must find nothing and must not append the
    // step twice.
    let again = migrate_trigger_delivery_targets(&repository, Some(&registry), &tenant())
        .await
        .expect("second migration runs");
    assert_eq!(again, 0, "a migrated record must not be migrated again");
    assert_eq!(
        read_back(&repository, trigger_id).await.prompt,
        stored.prompt,
        "a second pass must not append the delivery step again"
    );
}

/// A registry `Ok(None)` is ambiguous. It is the same answer for a target
/// that is genuinely retired, one whose extension failed to activate (a
/// tolerated-and-continue outcome, so boot proceeds with that extension
/// contributing no targets), one mid-reconfiguration, and one not yet
/// provisioned. Since clearing is irreversible and keeping the id costs
/// nothing, the step is written either way — naming the id, never inventing
/// a destination label.
#[tokio::test]
async fn target_that_does_not_resolve_is_migrated_by_id_not_dropped() {
    let repository = InMemoryTriggerRepository::default();
    let unresolved = "slack:shared-channel:T123:C_UNRESOLVED";
    let record = record_with_target(Some(unresolved));
    let trigger_id = record.trigger_id;
    repository.upsert_trigger(record).await.expect("seed");
    // The registry knows a DIFFERENT target, so the stored one resolves to
    // nothing rather than the registry being empty.
    let registry = registry_with_target();

    let migrated = migrate_trigger_delivery_targets(&repository, Some(&registry), &tenant())
        .await
        .expect("migration runs");
    assert_eq!(migrated, 1, "the routine is migrated away from the column");

    let stored = read_back(&repository, trigger_id).await;
    assert_eq!(
        stored.prompt,
        format!(
            "{PROMPT}\n\nDeliver the result to the destination it was routed to using \
             builtin__outbound_deliver (target id: {unresolved})."
        ),
        "an unresolved id must survive as an actionable step, with no invented label"
    );
    assert!(
        !stored.prompt.contains(DISPLAY_NAME),
        "the migration must never attach another target's display name: {:?}",
        stored.prompt
    );
    assert!(
        stored.delivery_target.is_none(),
        "the retired column must still be cleared"
    );
}

/// The prompt-cap branch must not be the one place this migration destroys
/// a route. Clearing without appending would be strictly worse than doing
/// nothing: the route is gone AND no instruction replaced it, irreversibly.
///
/// The record therefore keeps its prompt and its stored target; only its
/// lifecycle state moves, to `Paused`. Pausing is what makes that safe —
/// a paused trigger cannot fire, so the "never fire unrouted" invariant is
/// enforced for this record without the boot-wide abort it used to take
/// (which left the operator no running UI in which to shorten the prompt the
/// error asked them to shorten).
#[tokio::test]
async fn prompt_with_no_room_for_the_step_keeps_its_route_and_cannot_fire() {
    let repository = InMemoryTriggerRepository::default();
    let mut record = record_with_target(Some(TARGET_ID));
    // One byte short of the cap, so any appended step overflows.
    record.prompt = "x".repeat(MAX_TRIGGER_PROMPT_BYTES - 1);
    let trigger_id = record.trigger_id;
    let seeded = record.clone();
    repository.upsert_trigger(record).await.expect("seed");

    let migrated =
        migrate_trigger_delivery_targets(&repository, Some(&registry_with_target()), &tenant())
            .await
            .expect("an unrepresentable route quarantines its routine, it does not fail the sweep");
    assert_eq!(migrated, 0, "a quarantined record is not a migrated one");

    let stored = read_back(&repository, trigger_id).await;
    assert_eq!(
        stored.prompt, seeded.prompt,
        "the record must not be half-migrated"
    );
    assert_eq!(
        stored.delivery_target.as_ref().map(|id| id.as_str()),
        Some(TARGET_ID),
        "the route must survive: clearing it here would lose it with nothing in its place"
    );
    assert_eq!(
        stored.state,
        TriggerState::Paused,
        "the routine must be unable to fire while its route is unrepresented"
    );
    assert_eq!(
        TriggerRecord {
            state: seeded.state,
            ..stored
        },
        seeded,
        "quarantine must change the lifecycle state and nothing else"
    );
}

#[tokio::test]
async fn record_without_a_stored_target_is_left_untouched() {
    let repository = InMemoryTriggerRepository::default();
    let record = record_with_target(None);
    let trigger_id = record.trigger_id;
    repository
        .upsert_trigger(record.clone())
        .await
        .expect("seed");

    let migrated =
        migrate_trigger_delivery_targets(&repository, Some(&registry_with_target()), &tenant())
            .await
            .expect("migration runs");
    assert_eq!(migrated, 0, "nothing to migrate");
    assert_eq!(
        read_back(&repository, trigger_id).await,
        record,
        "a routine with no stored target must be byte-identical after the pass"
    );
}

#[tokio::test]
async fn unavailable_registry_migrates_by_id_without_losing_the_route() {
    let repository = InMemoryTriggerRepository::default();
    let record = record_with_target(Some(TARGET_ID));
    let trigger_id = record.trigger_id;
    repository.upsert_trigger(record).await.expect("seed");
    let registry = MutableOutboundDeliveryTargetRegistry::default();
    registry
        .register_provider("failing", Arc::new(FailingProvider))
        .expect("register provider");

    let migrated = migrate_trigger_delivery_targets(&repository, Some(&registry), &tenant())
        .await
        .expect("migration runs");
    assert_eq!(migrated, 1, "the target id is sufficient for migration");

    let stored = read_back(&repository, trigger_id).await;
    assert!(
        stored.prompt.contains(TARGET_ID),
        "the actionable target id must survive the unavailable lookup"
    );
    assert!(stored.delivery_target.is_none());
}

#[tokio::test]
async fn migration_runs_without_a_delivery_registry_and_preserves_the_target_id() {
    let repository = InMemoryTriggerRepository::default();
    let record = record_with_target(Some(TARGET_ID));
    let trigger_id = record.trigger_id;
    repository.upsert_trigger(record).await.expect("seed");

    let migrated = migrate_trigger_delivery_targets(&repository, None, &tenant())
        .await
        .expect("id-only migration runs without a registry");
    assert_eq!(migrated, 1);
    let stored = read_back(&repository, trigger_id).await;
    assert!(stored.prompt.contains(TARGET_ID));
    assert!(stored.delivery_target.is_none());
}

#[tokio::test]
async fn migration_compare_and_clear_does_not_overwrite_a_concurrent_prompt_edit() {
    let repository = InMemoryTriggerRepository::default();
    let expected = record_with_target(Some(TARGET_ID));
    let trigger_id = expected.trigger_id;
    repository
        .upsert_trigger(expected.clone())
        .await
        .expect("seed expected row");

    let mut concurrently_edited = expected.clone();
    concurrently_edited.prompt = "operator edited the routine while booting".to_string();
    repository
        .upsert_trigger(concurrently_edited.clone())
        .await
        .expect("persist concurrent edit");

    assert!(
        !repository
            .migrate_legacy_delivery_target(
                &expected,
                format!(
                    "{}{}",
                    expected.prompt,
                    delivery_step(Some(DISPLAY_NAME), TARGET_ID)
                ),
            )
            .await
            .expect("CAS reports a clean miss"),
        "a stale migration snapshot must not win"
    );
    assert_eq!(
        read_back(&repository, trigger_id).await,
        concurrently_edited,
        "the concurrent prompt and legacy target must remain untouched"
    );
}

/// A repository that forces the bounded CAS loop to take its retry path.
///
/// Wraps the real in-memory store and fails the compare-and-clear write the
/// first `misses` times, so the migration must re-read the current row and
/// retry against it — the arms no test reached while the loop was only ever
/// driven through a clean first-attempt success or a direct repo-seam call.
/// `vanish_after_misses` additionally deletes the row once the misses are
/// spent, reproducing a concurrent delete landing between miss and re-read.
struct CasMissRepository {
    inner: InMemoryTriggerRepository,
    misses_remaining: std::sync::Mutex<usize>,
    vanish_after_misses: bool,
    observed_prompts: std::sync::Mutex<Vec<String>>,
}

impl CasMissRepository {
    fn new(misses: usize, vanish_after_misses: bool) -> Self {
        Self {
            inner: InMemoryTriggerRepository::default(),
            misses_remaining: std::sync::Mutex::new(misses),
            vanish_after_misses,
            observed_prompts: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Every prompt the migration tried to write, in order.
    fn observed_prompts(&self) -> Vec<String> {
        self.observed_prompts
            .lock()
            .expect("prompt log is not poisoned")
            .clone()
    }
}

#[async_trait]
impl TriggerRepository for CasMissRepository {
    async fn migrate_legacy_delivery_target(
        &self,
        expected: &TriggerRecord,
        migrated_prompt: String,
    ) -> Result<bool, TriggerError> {
        self.observed_prompts
            .lock()
            .expect("prompt log is not poisoned")
            .push(migrated_prompt.clone());
        // Decide under the lock, then release it: the guard must not be held
        // across an await or the future stops being `Send`.
        let (miss, spent) = {
            let mut remaining = self
                .misses_remaining
                .lock()
                .expect("miss counter is not poisoned");
            if *remaining > 0 {
                *remaining -= 1;
                (true, *remaining == 0)
            } else {
                (false, false)
            }
        };
        if miss {
            if spent && self.vanish_after_misses {
                self.inner
                    .remove_trigger(expected.tenant_id.clone(), expected.trigger_id)
                    .await?;
            }
            return Ok(false);
        }
        self.inner
            .migrate_legacy_delivery_target(expected, migrated_prompt)
            .await
    }

    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError> {
        self.inner.upsert_trigger(record).await
    }

    async fn get_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner.get_trigger(tenant_id, trigger_id).await
    }

    async fn list_triggers(&self, tenant_id: TenantId) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.inner.list_triggers(tenant_id).await
    }

    async fn list_scoped_triggers(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ironclaw_host_api::ids::ProjectId>,
        limit: usize,
        excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.inner
            .list_scoped_triggers(
                tenant_id,
                creator_user_id,
                agent_id,
                project_id,
                limit,
                excluded_states,
            )
            .await
    }

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner.remove_trigger(tenant_id, trigger_id).await
    }

    async fn remove_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ironclaw_host_api::ids::ProjectId>,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner
            .remove_scoped_trigger(tenant_id, creator_user_id, agent_id, project_id, trigger_id)
            .await
    }

    async fn set_scoped_trigger_state(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ironclaw_host_api::ids::ProjectId>,
        trigger_id: TriggerId,
        state: TriggerState,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner
            .set_scoped_trigger_state(
                tenant_id,
                creator_user_id,
                agent_id,
                project_id,
                trigger_id,
                state,
            )
            .await
    }

    async fn list_due_triggers(
        &self,
        now: ironclaw_host_api::Timestamp,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.inner.list_due_triggers(now, limit).await
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.inner.list_active_triggers(limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ironclaw_triggers::ActiveTriggerScanCursor>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.inner.list_active_triggers_after(after, limit).await
    }

    async fn claim_due_fire(
        &self,
        request: ironclaw_triggers::ClaimDueFireRequest,
    ) -> Result<ironclaw_triggers::ClaimDueFireOutcome, TriggerError> {
        self.inner.claim_due_fire(request).await
    }

    async fn mark_fire_accepted(
        &self,
        request: ironclaw_triggers::FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner.mark_fire_accepted(request).await
    }

    async fn mark_fire_replayed(
        &self,
        request: ironclaw_triggers::FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner.mark_fire_replayed(request).await
    }

    async fn mark_fire_retryable_failed(
        &self,
        request: ironclaw_triggers::FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner.mark_fire_retryable_failed(request).await
    }

    async fn mark_fire_permanently_failed(
        &self,
        request: ironclaw_triggers::FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner.mark_fire_permanently_failed(request).await
    }

    async fn mark_fire_terminally_failed(
        &self,
        request: ironclaw_triggers::FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner.mark_fire_terminally_failed(request).await
    }

    async fn clear_active_fire(
        &self,
        request: ironclaw_triggers::ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.inner.clear_active_fire(request).await
    }

    async fn find_trigger_run_by_thread_id(
        &self,
        tenant_id: TenantId,
        thread_id: &ironclaw_host_api::ids::ThreadId,
    ) -> Result<Option<(TriggerRecord, ironclaw_triggers::TriggerRunRecord)>, TriggerError> {
        self.inner
            .find_trigger_run_by_thread_id(tenant_id, thread_id)
            .await
    }
}

/// The retired `builtin:web_app` pseudo-target meant "no external delivery".
/// Migrating it into a delivery step would invert the stored intent and make
/// every later fire attempt — and fail — a send to an unresolvable id.
#[tokio::test]
async fn retired_web_app_target_clears_without_adding_a_delivery_step() {
    let repository = InMemoryTriggerRepository::default();
    let record = record_with_target(Some("builtin:web_app"));
    let trigger_id = record.trigger_id;
    repository.upsert_trigger(record).await.expect("seed");

    let migrated =
        migrate_trigger_delivery_targets(&repository, Some(&registry_with_target()), &tenant())
            .await
            .expect("the opt-out row migrates");
    assert_eq!(migrated, 1);

    let stored = read_back(&repository, trigger_id).await;
    assert_eq!(
        stored.prompt, PROMPT,
        "the opt-out row must keep its prompt exactly as written"
    );
    assert!(
        !stored.prompt.contains("outbound_deliver"),
        "an opt-out row must never gain a delivery step: {}",
        stored.prompt
    );
    assert!(
        !stored.prompt.contains("builtin:web_app"),
        "the retired pseudo-target must not leak into the prompt"
    );
    assert!(
        stored.delivery_target.is_none(),
        "the legacy field is still cleared so the pass stays idempotent"
    );
}

/// A CAS miss must re-read the row and migrate the CURRENT prompt, not
/// resurrect the stale snapshot the sweep started from.
#[tokio::test]
async fn cas_miss_retries_against_the_reread_row() {
    let repository = CasMissRepository::new(1, false);
    let seeded = record_with_target(Some(TARGET_ID));
    let trigger_id = seeded.trigger_id;
    repository
        .upsert_trigger(seeded.clone())
        .await
        .expect("seed");

    // The row an operator edited while boot was in flight.
    let mut edited = seeded.clone();
    edited.prompt = "operator edited the routine while booting".to_string();
    repository
        .upsert_trigger(edited.clone())
        .await
        .expect("persist concurrent edit");

    let migrated =
        migrate_trigger_delivery_targets(&repository, Some(&registry_with_target()), &tenant())
            .await
            .expect("the retry path completes the migration");
    assert_eq!(migrated, 1);

    let attempts = repository.observed_prompts();
    assert_eq!(attempts.len(), 2, "expected one miss then one success");
    assert!(
        attempts[1].starts_with(&edited.prompt),
        "the retry must build on the re-read prompt, not the stale snapshot: {}",
        attempts[1]
    );

    let stored = read_back(&repository.inner, trigger_id).await;
    assert!(
        stored.prompt.starts_with(&edited.prompt),
        "the operator's edit must survive the migration: {}",
        stored.prompt
    );
    assert!(stored.prompt.contains(TARGET_ID));
    assert!(stored.delivery_target.is_none());
}

/// A row deleted between the CAS miss and the re-read has no routing intent
/// left to preserve. That is nothing to migrate — not a boot-fatal error.
#[tokio::test]
async fn row_deleted_during_retry_ends_the_migration_without_failing_boot() {
    let repository = CasMissRepository::new(1, true);
    let seeded = record_with_target(Some(TARGET_ID));
    repository.upsert_trigger(seeded).await.expect("seed");

    let migrated = migrate_trigger_delivery_targets_at_boot(
        &repository,
        Some(&registry_with_target()),
        &tenant(),
    )
    .await;

    assert!(
        migrated.is_ok(),
        "a concurrently deleted routine must not abort boot: {migrated:?}"
    );
}

/// A legacy route that cannot fit its prompt pauses ITS OWN routine — a paused
/// trigger cannot fire, so the "never fire unrouted" invariant still holds —
/// while every other routine and the product itself still boot.
#[tokio::test]
async fn unmigratable_route_pauses_only_its_own_routine_and_boot_continues() {
    let repository = InMemoryTriggerRepository::default();

    let mut oversized = record_with_target(Some(TARGET_ID));
    let step_len = delivery_step(Some(DISPLAY_NAME), TARGET_ID).len();
    oversized.prompt = "x".repeat(MAX_TRIGGER_PROMPT_BYTES - step_len + 1);
    let oversized_id = oversized.trigger_id;
    let oversized_prompt = oversized.prompt.clone();
    repository
        .upsert_trigger(oversized)
        .await
        .expect("seed big");

    let healthy = record_with_target(Some(TARGET_ID));
    let healthy_id = healthy.trigger_id;
    repository.upsert_trigger(healthy).await.expect("seed ok");

    migrate_trigger_delivery_targets_at_boot(&repository, Some(&registry_with_target()), &tenant())
        .await
        .expect("one unmigratable routine must not abort boot");

    let quarantined = read_back(&repository, oversized_id).await;
    assert_eq!(
        quarantined.state,
        TriggerState::Paused,
        "the unmigratable routine must be paused so it cannot fire unrouted"
    );
    assert_eq!(
        quarantined.prompt, oversized_prompt,
        "quarantine must not rewrite the user's prompt"
    );
    assert!(
        quarantined.delivery_target.is_some(),
        "the stored target must survive for a later boot to migrate"
    );

    let healthy = read_back(&repository, healthy_id).await;
    assert_eq!(
        healthy.state,
        TriggerState::Scheduled,
        "an unrelated routine must be untouched by another row's quarantine"
    );
    assert!(healthy.prompt.contains(TARGET_ID));
    assert!(healthy.delivery_target.is_none());
}

/// The bounded retry budget is real: a store that never lets the CAS land
/// fails rather than looping forever.
#[tokio::test]
async fn exhausted_cas_retries_fail_loudly() {
    let repository = CasMissRepository::new(usize::MAX, false);
    let seeded = record_with_target(Some(TARGET_ID));
    repository.upsert_trigger(seeded).await.expect("seed");

    let error = migrate_trigger_delivery_targets(&repository, None, &tenant())
        .await
        .expect_err("a permanently missing CAS must surface, not spin");
    assert!(
        matches!(error, TriggerError::Backend { .. }),
        "expected a backend error, got {error:?}"
    );
    assert_eq!(
        repository.observed_prompts().len(),
        5,
        "the retry budget must stay bounded"
    );
}
