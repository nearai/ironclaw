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
//! Runs from composition boot after the outbound delivery-target registry
//! exists and before the trigger poller starts; a failure is logged loud and
//! never blocks boot (the fire path already ignores stored targets). A
//! registry/upsert failure retries next boot; the one case that does NOT
//! self-heal is prompt-cap overflow — the record keeps its route and prompt
//! untouched, with a loud warn naming the trigger, until an operator shortens
//! the prompt (spec §8 amendment: clearing the column with no replacement
//! step was falsified as strictly worse).

use std::sync::Arc;

use ironclaw_host_api::ids::TenantId;
use ironclaw_triggers::{MAX_TRIGGER_PROMPT_BYTES, TriggerError, TriggerRecord, TriggerRepository};

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
/// * registry lookup fails outright → record left for a later boot;
/// * prompt would exceed [`MAX_TRIGGER_PROMPT_BYTES`] → record left untouched,
///   route intact (see [`migrate_one`]);
/// * no stored target → untouched (not even rewritten).
///
/// Idempotent: after a pass every migrated record has `delivery_target: None`,
/// so a second pass finds nothing to do and appends nothing.
pub(crate) async fn migrate_trigger_delivery_targets(
    repository: &dyn TriggerRepository,
    registry: &MutableOutboundDeliveryTargetRegistry,
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

/// Migrate a single record. `Ok(false)` means the record was deliberately left
/// alone, route intact, for a later boot or for an operator to resolve —
/// clearing it would destroy the only copy of the user's routing intent, and
/// that loss is irreversible while leaving the id costs nothing.
///
/// Two skip cases:
/// * the registry lookup failed outright (transport-shaped), so we learned
///   nothing about the target;
/// * appending the step would push the prompt past
///   [`MAX_TRIGGER_PROMPT_BYTES`]. Clearing there would be the worst of both
///   worlds: the route is gone AND no instruction replaced it. The record stays
///   as-is and the warn names the trigger so an operator can shorten the prompt.
async fn migrate_one(
    repository: &dyn TriggerRepository,
    registry: &MutableOutboundDeliveryTargetRegistry,
    mut record: TriggerRecord,
) -> Result<bool, TriggerError> {
    let Some(target) = record.delivery_target.clone() else {
        return Ok(false);
    };
    let display_name = match resolve_display_name(registry, &record, target.as_str()).await {
        Ok(display_name) => display_name,
        Err(ResolveFailure::Unavailable) => return Ok(false),
    };
    if display_name.is_none() {
        tracing::warn!(
            target: "ironclaw::reborn::trigger_delivery_migration",
            trigger_id = %record.trigger_id,
            "stored delivery target did not resolve; migrating it by id without a destination name"
        );
    }
    let step = delivery_step(display_name.as_deref(), target.as_str());
    if record.prompt.len() + step.len() > MAX_TRIGGER_PROMPT_BYTES {
        tracing::warn!(
            target: "ironclaw::reborn::trigger_delivery_migration",
            trigger_id = %record.trigger_id,
            tenant_id = %record.tenant_id.as_str(),
            trigger_name = %record.name,
            prompt_bytes = record.prompt.len(),
            step_bytes = step.len(),
            max_prompt_bytes = MAX_TRIGGER_PROMPT_BYTES,
            "routine prompt has no room for its delivery step; leaving the stored target in place rather than dropping the route — shorten the prompt and reboot"
        );
        return Ok(false);
    }
    record.prompt.push_str(&step);
    record.delivery_target = None;
    repository.upsert_trigger(record).await?;
    Ok(true)
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

/// A registry lookup that could not be completed. Distinct from "resolved to
/// nothing": an unavailable registry must not be read as "the target is gone".
enum ResolveFailure {
    Unavailable,
}

async fn resolve_display_name(
    registry: &MutableOutboundDeliveryTargetRegistry,
    record: &TriggerRecord,
    target_id: &str,
) -> Result<Option<String>, ResolveFailure> {
    let Ok(target_id) = OutboundDeliveryTargetId::new(target_id) else {
        // A stored id the current type rejects can never resolve; treat it as
        // gone rather than retrying it on every boot forever.
        return Ok(None);
    };
    let caller =
        OutboundDeliveryTargetScope::new(record.tenant_id.clone(), record.creator_user_id.clone());
    match registry
        .resolve_outbound_delivery_target(&caller, &target_id)
        .await
    {
        Ok(entry) => Ok(entry.map(|entry| entry.summary.display_name.as_str().to_string())),
        Err(error) => {
            tracing::warn!(
                target: "ironclaw::reborn::trigger_delivery_migration",
                trigger_id = %record.trigger_id,
                %error,
                "outbound delivery target lookup failed; leaving the stored target for a later boot"
            );
            Err(ResolveFailure::Unavailable)
        }
    }
}

/// Boot entry point: run the migration, logging loud on failure instead of
/// aborting composition.
pub(crate) async fn migrate_trigger_delivery_targets_at_boot(
    repository: &Arc<dyn TriggerRepository>,
    registry: &MutableOutboundDeliveryTargetRegistry,
    tenant_id: &TenantId,
) {
    match migrate_trigger_delivery_targets(repository.as_ref(), registry, tenant_id).await {
        Ok(0) => {}
        Ok(migrated) => tracing::debug!(
            target: "ironclaw::reborn::trigger_delivery_migration",
            migrated,
            "migrated stored trigger delivery targets into their routine prompts"
        ),
        Err(error) => tracing::error!(
            target: "ironclaw::reborn::trigger_delivery_migration",
            %error,
            "stored trigger delivery-target migration failed"
        ),
        // silent-ok: migration retries next boot, fire path ignores stored targets
    }
}

#[cfg(test)]
mod tests;
