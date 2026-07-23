//! C-DURABLE: a created trigger survives an independent reopen of the trigger
//! repository at the SAME on-disk local-dev `storage_root` — proving
//! capability-produced trigger state persists to disk, not just to in-memory
//! state. Parallels `assert_reply_persists_after_reopen` (thread history) and
//! `ironclaw_integration_durable.rs` (extension installs) for the trigger
//! repository.
//!
//! Creates a one-shot `Once` trigger through a real turn, reads the
//! server-minted `trigger_id`, then reopens a FRESH `TriggerRepository` at the
//! capability harness's `storage_root_for_test()` and confirms the trigger is
//! there by id — independent of the live `Arc` the group holds.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_triggers::TriggerId;
use serde_json::json;

const ONCE_AT: &str = "2999-06-01T00:00:00";
const TRIGGER_NAME: &str = "c-durable-trigger-reopen";

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("trigger-durable")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": TRIGGER_NAME,
                    "prompt": "remind me once (durability check)",
                    "schedule": {"kind": "once", "at": ONCE_AT, "timezone": "UTC"},
                }),
            ),
            IronClawScriptedReply::text("created"),
        ])
        .build()
        .await?;
    h.submit_turn("create a durable one-shot reminder").await?;

    let created = h.tool_result_output("builtin.trigger_create").await?;
    let trigger_id = created["trigger"]["trigger_id"]
        .as_str()
        .ok_or("trigger_create output missing trigger_id")?
        .to_string();

    let capability_harness = g
        .capability_harness()
        .ok_or("triggers group always uses HostRuntime")?;
    // Read off the group's own scope rather than a hardcoded literal, so this
    // can never drift from the tenant `trigger_create` actually stored under.
    let tenant_id = g.shared.product_harness.scope.tenant_id.clone();

    // Reopen a FRESH, independent repository at the same on-disk root — not the
    // live `Arc` the running group holds — and confirm the trigger is there.
    let reopened = ironclaw_composition::test_support::open_local_dev_trigger_repository_for_test(
        &capability_harness.storage_root_for_test(),
    )
    .await?;
    let record = reopened
        .get_trigger(tenant_id, TriggerId::parse(&trigger_id)?)
        .await?
        .ok_or_else(|| format!("trigger {trigger_id} not found after independent reopen"))?;
    if record.name != TRIGGER_NAME {
        return Err(format!(
            "reopened trigger name mismatch: expected {TRIGGER_NAME:?}, got {:?}",
            record.name
        )
        .into());
    }

    Ok(())
}
