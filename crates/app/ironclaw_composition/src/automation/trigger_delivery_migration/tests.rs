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

    let migrated = migrate_trigger_delivery_targets(&repository, &registry, &tenant())
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
    let again = migrate_trigger_delivery_targets(&repository, &registry, &tenant())
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

    let migrated = migrate_trigger_delivery_targets(&repository, &registry, &tenant())
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
/// The record is left exactly as found so an operator can shorten the
/// prompt and reboot.
#[tokio::test]
async fn prompt_with_no_room_for_the_step_keeps_its_route_untouched() {
    let repository = InMemoryTriggerRepository::default();
    let mut record = record_with_target(Some(TARGET_ID));
    // One byte short of the cap, so any appended step overflows.
    record.prompt = "x".repeat(MAX_TRIGGER_PROMPT_BYTES - 1);
    let trigger_id = record.trigger_id;
    let seeded = record.clone();
    repository.upsert_trigger(record).await.expect("seed");

    let migrated =
        migrate_trigger_delivery_targets(&repository, &registry_with_target(), &tenant())
            .await
            .expect("migration runs");
    assert_eq!(
        migrated, 0,
        "a routine that cannot hold its step is not migrated"
    );

    let stored = read_back(&repository, trigger_id).await;
    assert_eq!(
        stored, seeded,
        "the record must be left exactly as found, not half-migrated"
    );
    assert_eq!(
        stored.delivery_target.as_ref().map(|id| id.as_str()),
        Some(TARGET_ID),
        "the route must survive: clearing it here would lose it with nothing in its place"
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
        migrate_trigger_delivery_targets(&repository, &registry_with_target(), &tenant())
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
async fn unavailable_registry_leaves_the_stored_target_for_a_later_boot() {
    let repository = InMemoryTriggerRepository::default();
    let record = record_with_target(Some(TARGET_ID));
    let trigger_id = record.trigger_id;
    repository.upsert_trigger(record).await.expect("seed");
    let registry = MutableOutboundDeliveryTargetRegistry::default();
    registry
        .register_provider("failing", Arc::new(FailingProvider))
        .expect("register provider");

    let migrated = migrate_trigger_delivery_targets(&repository, &registry, &tenant())
        .await
        .expect("migration runs");
    assert_eq!(migrated, 0, "a failed lookup must not count as migrated");

    let stored = read_back(&repository, trigger_id).await;
    assert_eq!(stored.prompt, PROMPT, "no step may be invented");
    assert_eq!(
        stored.delivery_target.as_ref().map(|id| id.as_str()),
        Some(TARGET_ID),
        "an unavailable registry must not destroy the stored routing intent"
    );
}
