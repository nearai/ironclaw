//! One-time forward migration of the retired per-trigger stored delivery
//! target into the routine's own prompt.
//!
//! Routines used to carry a `delivery_target` on the record, and the fire path
//! pushed each completed run's final reply to it. That routing is gone: a fire
//! now delivers externally only by *calling* `builtin__outbound_deliver`
//! itself, which means a pre-removal routine would silently stop delivering.
//!
//! This migration rewrites that intent into the only place the fire can still
//! act on — the prompt — and then clears the legacy field so the pass is a
//! no-op on every subsequent boot.
//!
//! Runs from composition boot before the trigger poller starts. The fire path
//! no longer reads the retired field, so starting the poller with an unmigrated
//! row would silently discard user routing intent — the failure this pass
//! exists to prevent. It is prevented *per record*: a route that cannot be
//! represented in its prompt pauses its own routine (which therefore cannot
//! fire) and boot continues. Only a systemic failure — the store refusing to
//! read, write, or pause — is boot-fatal.

use ironclaw_host_api::ids::TenantId;
use ironclaw_triggers::{
    MAX_TRIGGER_PROMPT_BYTES, TriggerError, TriggerRecord, TriggerRepository, TriggerState,
};

/// The retired host-owned pseudo-target.
///
/// A row carrying it means "results land in the run thread only — no external
/// delivery": it was the *opt out* of pushing, not a destination. Rewriting it
/// into a delivery step would invert that intent and make every later fire
/// attempt — and fail — an external send to an id nothing can resolve.
///
/// Kept here as a literal on purpose: the constant that named it
/// (`WEB_APP_OUTBOUND_DELIVERY_TARGET_ID`) died with the provider this PR
/// deletes, but durable rows outlive the code that wrote them.
const RETIRED_WEB_APP_TARGET_ID: &str = "builtin:web_app";

use crate::outbound::{
    MutableOutboundDeliveryTargetRegistry, OutboundDeliveryTargetId,
    OutboundDeliveryTargetProvider as _, OutboundDeliveryTargetScope,
};

/// Rewrite every stored delivery target in `tenant_id` into its routine's
/// prompt, then clear the legacy field.
///
/// Returns the number of records rewritten. Enumeration is tenant-scoped
/// because [`TriggerRepository::list_triggers`] is — composition boots one
/// tenant, and calls this once for it.
///
/// Per record:
/// * target resolves for its creator → append a delivery step naming the
///   destination and clear the field;
/// * target does not resolve → append a delivery step naming the target id
///   alone (no fabricated destination name) and clear the field. A registry
///   `Ok(None)` is **ambiguous**, not proof the destination is gone: it is the
///   same answer for a genuinely retired target, an extension whose activation
///   failed (activation failures are tolerated-and-continue, so boot proceeds
///   with that extension contributing nothing), a target mid-reconfiguration,
///   and one not yet provisioned. Clearing is irreversible and keeping the id
///   costs nothing, so the asymmetry decides it: write the step.
///   `builtin.outbound_deliver` takes the id directly, so it stays actionable
///   and fails loudly at fire time if the destination really did go away;
/// * registry lookup fails outright → migrate by id without a display name;
/// * target is the retired `builtin:web_app` opt-out → clear the field and
///   leave the prompt untouched (it never meant an external destination);
/// * prompt would exceed [`MAX_TRIGGER_PROMPT_BYTES`] → pause that routine and
///   leave its stored target intact for a later boot; boot continues;
/// * no stored target → untouched (not even rewritten).
///
/// Idempotent: after a pass every migrated record has `delivery_target: None`,
/// so a second pass finds nothing to do and appends nothing.
pub(crate) async fn migrate_trigger_delivery_targets(
    repository: &dyn TriggerRepository,
    registry: Option<&MutableOutboundDeliveryTargetRegistry>,
    tenant_id: &TenantId,
) -> Result<usize, TriggerError> {
    let records = repository.list_triggers(tenant_id.clone()).await?;
    let mut migrated = 0usize;
    for record in records {
        if record.delivery_target.is_none() {
            continue;
        }
        if migrate_one(repository, registry, record).await? {
            migrated += 1;
        }
    }
    Ok(migrated)
}

/// Migrate one record through an atomic compare-and-clear write. A CAS miss
/// re-reads and retries from the current row so boot-time migration cannot
/// overwrite a concurrent prompt edit.
async fn migrate_one(
    repository: &dyn TriggerRepository,
    registry: Option<&MutableOutboundDeliveryTargetRegistry>,
    mut record: TriggerRecord,
) -> Result<bool, TriggerError> {
    const MAX_CAS_RETRIES: usize = 5;
    for _ in 0..MAX_CAS_RETRIES {
        let Some(target) = record.delivery_target.clone() else {
            return Ok(false);
        };
        let migrated_prompt = if target.as_str() == RETIRED_WEB_APP_TARGET_ID {
            // Opt-out row: clear the field and leave the prompt exactly as the
            // user wrote it. Lane 1 already lands the result in the routine's
            // own run thread, which is all this target ever meant.
            record.prompt.clone()
        } else {
            let display_name = resolve_display_name(registry, &record, target.as_str()).await;
            if display_name.is_none() {
                tracing::warn!(
                    target: "ironclaw::reborn::trigger_delivery_migration",
                    trigger_id = %record.trigger_id,
                    "stored delivery target did not resolve; migrating it by id without a destination name"
                );
            }
            let step = delivery_step(display_name.as_deref(), target.as_str());
            if record.prompt.len() + step.len() > MAX_TRIGGER_PROMPT_BYTES {
                return quarantine_unmigratable_route(repository, &record).await;
            }
            format!("{}{}", record.prompt, step)
        };
        if repository
            .migrate_legacy_delivery_target(&record, migrated_prompt)
            .await?
        {
            return Ok(true);
        }
        // CAS miss: the row moved under us. Re-read and retry against the
        // current one. A row that vanished between the miss and the re-read
        // has no routing intent left to preserve — that is nothing to migrate,
        // not a reason to fail the boot.
        let Some(current) = repository
            .get_trigger(record.tenant_id.clone(), record.trigger_id)
            .await?
        else {
            return Ok(false);
        };
        record = current;
    }
    Err(TriggerError::Backend {
        reason: format!(
            "legacy delivery-target migration for trigger {} exceeded bounded CAS retries",
            record.trigger_id
        ),
    })
}

/// Pause the one routine whose legacy route cannot be represented in its
/// prompt, instead of failing the whole boot.
///
/// The invariant this migration protects is per-record: a routine must never
/// fire without the routing its creator chose. Pausing enforces exactly that
/// for the record at issue — a paused trigger cannot fire, its
/// `delivery_target` stays set, and the next boot retries the migration
/// idempotently once the prompt has room. Aborting composition would instead
/// take down every *other* routine and the product UI the operator needs in
/// order to shorten the prompt, leaving DB surgery as the only recovery.
///
/// Failing to pause IS systemic, so that stays boot-fatal. A record the store
/// refuses to move (a terminal `Completed` one-shot) cannot fire either, so the
/// invariant already holds and boot continues.
async fn quarantine_unmigratable_route(
    repository: &dyn TriggerRepository,
    record: &TriggerRecord,
) -> Result<bool, TriggerError> {
    tracing::warn!(
        target: "ironclaw::reborn::trigger_delivery_migration",
        trigger_id = %record.trigger_id,
        prompt_bytes = record.prompt.len(),
        limit_bytes = MAX_TRIGGER_PROMPT_BYTES,
        "routine cannot preserve its legacy delivery target within the prompt limit; pausing it so it cannot fire unrouted — shorten the prompt and resume it to migrate"
    );
    let paused = repository
        .set_scoped_trigger_state(
            record.tenant_id.clone(),
            record.creator_user_id.clone(),
            record.agent_id.clone(),
            record.project_id.clone(),
            record.trigger_id,
            TriggerState::Paused,
        )
        .await?;
    if paused.is_none() {
        tracing::warn!(
            target: "ironclaw::reborn::trigger_delivery_migration",
            trigger_id = %record.trigger_id,
            "unmigratable routine is already in a terminal state and cannot fire; leaving it untouched"
        );
    }
    Ok(false)
}

/// The instruction that replaces the stored route. `display_name` is omitted
/// when the registry could not name the destination at boot — the step then
/// carries the id alone rather than inventing a label the user never chose.
fn delivery_step(display_name: Option<&str>, target_id: &str) -> String {
    let destination = display_name.unwrap_or("the destination it was routed to");
    format!(
        "\n\nDeliver the result to {destination} using builtin__outbound_deliver (target id: {target_id})."
    )
}

async fn resolve_display_name(
    registry: Option<&MutableOutboundDeliveryTargetRegistry>,
    record: &TriggerRecord,
    target_id: &str,
) -> Option<String> {
    let registry = registry?;
    let Ok(target_id) = OutboundDeliveryTargetId::new(target_id) else {
        return None;
    };
    let caller =
        OutboundDeliveryTargetScope::new(record.tenant_id.clone(), record.creator_user_id.clone());
    match registry
        .resolve_outbound_delivery_target(&caller, &target_id)
        .await
    {
        Ok(entry) => entry.map(|entry| entry.summary.display_name.as_str().to_string()),
        Err(error) => {
            tracing::warn!(
                target: "ironclaw::reborn::trigger_delivery_migration",
                trigger_id = %record.trigger_id,
                %error,
                "outbound delivery target lookup failed; migrating the stored target by id"
            );
            None
        }
    }
}

/// Boot entry point. Any unmigratable legacy route aborts composition before
/// the poller can fire it without delivery.
pub(crate) async fn migrate_trigger_delivery_targets_at_boot(
    repository: &dyn TriggerRepository,
    registry: Option<&MutableOutboundDeliveryTargetRegistry>,
    tenant_id: &TenantId,
) -> Result<(), TriggerError> {
    let migrated = migrate_trigger_delivery_targets(repository, registry, tenant_id).await?;
    if migrated > 0 {
        tracing::debug!(
            target: "ironclaw::reborn::trigger_delivery_migration",
            migrated,
            "migrated stored trigger delivery targets into their routine prompts"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
