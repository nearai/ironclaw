//! Routines carry no stored delivery route any more: a fire delivers
//! externally only by CALLING `builtin.outbound_deliver` from its own prompt.
//!
//! Supersedes `scenario_delivery_target_fail_closed` (which pinned the
//! fail-closed arm of the retired `delivery_target_id` input) and
//! `scenario_external_source_trigger_captures_delivery` (which required
//! `trigger_create` to seal the source conversation's reply route onto the
//! record — the behavior this train removes).
//!
//! Two halves, both at the real tool surface over the group's shared trigger
//! repository:
//!
//! 1. **The field is gone from the surface the model sees.** Asserted on the
//!    tool definitions the scripted model was actually handed (not on the
//!    schema constant), and on a create that succeeds without it and returns
//!    no routing field. A model that still passes `delivery_target_id` is
//!    refused as an unexpected field — pinned at the dispatch tier by
//!    `builtin_trigger_create_rejects_the_retired_delivery_target_id`
//!    (`crates/ironclaw_host_runtime/tests/first_party_builtin_tools.rs`).
//! 2. **A stored-target-era record still reads back cleanly.** The created
//!    routine is rewritten in place with a stored route — reproducing a row
//!    written before the removal, under the exact scope the tool uses — and
//!    then listed through a genuine `builtin.trigger_list` call: the legacy
//!    column deserializes, the routine is intact and still schedulable, and
//!    no routing field leaks into the model-facing output.
//!
//! The FIRE half of a stored-target-era record (it runs exactly once and
//! pushes nothing to any channel) and the boot migration that rewrites the
//! stored route into the prompt belong to the composition tier, where a real
//! poller and a real boot exist:
//! `trigger_poller_e2e.rs::scheduled_trigger_results_are_never_pushed_to_a_channel_across_restart`
//! and `::stored_delivery_target_trigger_is_migrated_to_prompt`.

use ironclaw_triggers::{TriggerDeliveryTargetId, TriggerId};
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

const ONCE_AT: &str = "2999-01-01T00:00:00";
const ROUTINE_NAME: &str = "prompt-owned-delivery-routine";
const ROUTINE_PROMPT: &str =
    "summarize the day, then deliver it to my Slack DM with builtin__outbound_deliver";
const LEGACY_TARGET_ID: &str = "slack:personal-dm:T123:legacy-owner";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-trigger-create-has-no-delivery-target-field")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": ROUTINE_NAME,
                    "prompt": ROUTINE_PROMPT,
                    "schedule": {"kind": "once", "at": ONCE_AT, "timezone": "UTC"},
                }),
            ),
            RebornScriptedReply::text("scheduled"),
            RebornScriptedReply::tool_call("builtin.trigger_list", json!({})),
            RebornScriptedReply::text("listed"),
        ])
        .build()
        .await?;

    h.submit_turn("every day summarize things and send it to my Slack DM")
        .await?;

    // Half 1a: the schema the model was actually handed for this turn.
    assert_create_schema_omits_delivery_target(&h)?;

    // Half 1b: the create succeeds with no routing field, and none comes back.
    let created = h.tool_result_output("builtin.trigger_create").await?;
    if !created["trigger"]["delivery_target_id"].is_null() {
        return Err(format!(
            "trigger_create output must not carry a stored delivery route: {created}"
        )
        .into());
    }
    let trigger_id = TriggerId::parse(
        created["trigger"]["trigger_id"]
            .as_str()
            .ok_or("trigger_create output missing trigger_id")?,
    )?;

    // Rewrite the just-created routine into a pre-removal row: same scope the
    // tool wrote, plus the retired stored route. Seeding through the record
    // the tool itself produced is what keeps this from silently drifting out
    // of the scope `trigger_list` reads.
    let capability_harness = g
        .capability_harness()
        .ok_or("triggers group always uses HostRuntime")?;
    let repository = capability_harness
        .trigger_repository_for_test()
        .ok_or("triggers group exposes its shared trigger repository")?;
    let tenant_id = g.shared.product_harness.scope.tenant_id.clone();
    let mut legacy = repository
        .get_trigger(tenant_id.clone(), trigger_id)
        .await?
        .ok_or("the created routine must be readable from the shared repository")?;
    legacy.delivery_target = Some(TriggerDeliveryTargetId::new(LEGACY_TARGET_ID)?);
    repository.upsert_trigger(legacy).await?;

    // Half 2: the pre-removal row survives a real model-facing read.
    h.submit_turn("list my routines").await?;
    let listed = h.tool_result_output("builtin.trigger_list").await?;
    let triggers = listed["triggers"]
        .as_array()
        .ok_or("trigger_list output missing triggers array")?;
    let seen = triggers
        .iter()
        .find(|trigger| trigger["name"] == json!(ROUTINE_NAME))
        .ok_or_else(|| format!("a stored-target-era routine must still be listed, got {listed}"))?;
    if !seen["delivery_target_id"].is_null() {
        return Err(format!(
            "the retired routing field must not reach the model even for a \
             pre-removal record: {seen}"
        )
        .into());
    }
    if seen["state"] != json!("scheduled") {
        return Err(
            format!("a stored-target-era routine must remain schedulable, got {seen}").into(),
        );
    }

    // …and the record itself is intact behind that read (the legacy column
    // deserialized rather than poisoning the row).
    let stored = repository
        .get_trigger(tenant_id, trigger_id)
        .await?
        .ok_or("the stored-target-era record must still be readable")?;
    if stored.prompt != ROUTINE_PROMPT {
        return Err(format!(
            "reading a pre-removal record must not rewrite its prompt: {:?}",
            stored.prompt
        )
        .into());
    }
    if stored.delivery_target.as_ref().map(|id| id.as_str()) != Some(LEGACY_TARGET_ID) {
        return Err(format!(
            "the legacy column must round-trip untouched until the boot migration \
             clears it: {:?}",
            stored.delivery_target
        )
        .into());
    }

    Ok(())
}

/// The `builtin.trigger_create` definition the scripted model was handed must
/// declare no `delivery_target_id` property, and must point at the tool a
/// routine's prompt now uses to deliver.
fn assert_create_schema_omits_delivery_target(
    h: &super::reborn_support::builder::RebornIntegrationHarness,
) -> HarnessResult<()> {
    let definition = h
        .scripted_llm
        .captured_tool_definitions()
        .into_iter()
        .flatten()
        .find(|definition| definition.name == "builtin__trigger_create")
        .ok_or("the model was never handed a builtin__trigger_create definition")?;
    let properties = definition.parameters["properties"]
        .as_object()
        .ok_or("trigger_create parameters declare no properties")?;
    if properties.contains_key("delivery_target_id") {
        return Err(format!(
            "the model-visible trigger_create schema must not offer a stored \
             delivery route: {}",
            definition.parameters
        )
        .into());
    }
    if !definition.description.contains("builtin__outbound_deliver") {
        return Err(format!(
            "the model-visible trigger_create description must name the delivery \
             tool a routine's prompt calls: {}",
            definition.description
        )
        .into());
    }
    Ok(())
}
