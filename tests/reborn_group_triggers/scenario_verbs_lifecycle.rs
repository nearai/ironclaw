//! HEADLINE: the full trigger-management verb lifecycle at int tier.
//!
//! Thread A dispatches `builtin.trigger_create` (a one-shot `Once{at}` schedule)
//! then `builtin.trigger_list`, and reads back the server-minted `trigger_id`.
//! Thread B — a DIFFERENT conversation over the SAME trigger scope (shared repo)
//! — dispatches `trigger_pause` → `trigger_resume` → `trigger_remove` against
//! that id, then `trigger_list` to confirm the removal took. Because the two
//! threads share the `HostRuntimeCapabilityHarness` trigger repository, thread B
//! operating on thread A's trigger also proves cross-thread persistence.
//!
//! This is the only integration coverage of the `list`/`pause`/`resume`/`remove`
//! handlers dispatched through the real capability path; the one-shot fire →
//! `Completed` derivation is owned by `trigger_poller_e2e.rs` +
//! `repository_contract.rs` and is deliberately NOT re-covered here.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// Far-future wall-clock so `trigger_create`'s `next_run_at` (computed against
/// the real `SystemTriggerManagementClock`) always has a future slot — no
/// wall-clock flake — and unambiguous (no DST edge).
const ONCE_AT: &str = "2999-01-01T00:00:00";
const TRIGGER_NAME: &str = "t0-triggers-once";

// TODO(T0-TRIGGERS, no enabler needed): distinct verb branches this same group
// can grow in a follow-up without any harness seam — add as new `scenario_*`
// files, not by bloating this happy-path lifecycle:
//   - cron-schedule create (`{kind:"cron", expression, timezone}`) → list renders
//     `is_recurring`/next_run_at; contrast with the Once path here.
//   - `trigger_list` `limit`/`run_limit` params (bounded output).
//   - deny/error branches through the capability path (model-recoverable, NOT
//     terminal per `.claude/rules/agent-loop-capabilities.md`): remove/pause a
//     non-existent `trigger_id` → `{"removed":false}` / `{"updated":false}`;
//     malformed `trigger_id` → surfaced input error the model can retry.
//
// Two gotchas for follow-up scenarios in THIS group binary:
//   - `tool_result_output(cap)` returns the MOST RECENT result for `cap` in the
//     thread's slice. If a scenario dispatches the same verb twice in one thread,
//     read the intermediate result before the second call — `.rev()` will
//     otherwise silently return the later one.
//   - the group's trigger repository is shared across scenarios with NO cleanup
//     between them; keep list assertions id-scoped (`.any(|t| t["trigger_id"]…)`)
//     and never assert an exact `triggers.len()`, which would flake on leftovers.
pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: create a one-shot Once trigger, then list it ───────────────
    let creator = g
        .thread("trigger-create")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": TRIGGER_NAME,
                    "prompt": "remind me once",
                    "schedule": {"kind": "once", "at": ONCE_AT, "timezone": "UTC"},
                }),
            ),
            RebornScriptedReply::tool_call("builtin.trigger_list", json!({})),
            RebornScriptedReply::text("created"),
        ])
        .build()
        .await?;
    creator.submit_turn("create a one-shot reminder").await?;
    creator
        .assert_tool_invoked("builtin.trigger_create")
        .await?;
    creator.assert_tool_invoked("builtin.trigger_list").await?;

    // create output: once schedule, enabled + scheduled, server-minted id.
    let created = creator.tool_result_output("builtin.trigger_create").await?;
    let trigger = &created["trigger"];
    let trigger_id = trigger["trigger_id"]
        .as_str()
        .ok_or("trigger_create output missing trigger_id")?
        .to_string();
    if trigger["schedule"]["kind"] != json!("once") {
        return Err(format!("expected once schedule, got {}", trigger["schedule"]).into());
    }
    if trigger["state"] != json!("scheduled") || trigger["is_enabled"] != json!(true) {
        return Err(format!("new trigger must be scheduled + enabled: {trigger}").into());
    }

    // list output: the just-created trigger is present by id AND name.
    let listed = creator.tool_result_output("builtin.trigger_list").await?;
    let in_list = listed["triggers"]
        .as_array()
        .ok_or("trigger_list output missing triggers array")?
        .iter()
        .any(|t| t["trigger_id"] == json!(trigger_id) && t["name"] == json!(TRIGGER_NAME));
    if !in_list {
        return Err(format!("created trigger absent from list: {listed}").into());
    }

    // ── Thread B: pause → resume → remove by id, over the SHARED repo ─────────
    // A distinct conversation_id → distinct thread, but the trigger scope
    // (tenant, user, agent, project) is identical, so thread B resolves thread
    // A's trigger from the shared `HostRuntimeCapabilityHarness` repository.
    let manager = g
        .thread("trigger-manage")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.trigger_pause",
                json!({"trigger_id": trigger_id}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.trigger_resume",
                json!({"trigger_id": trigger_id}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.trigger_remove",
                json!({"trigger_id": trigger_id}),
            ),
            RebornScriptedReply::tool_call("builtin.trigger_list", json!({})),
            RebornScriptedReply::text("managed"),
        ])
        .build()
        .await?;
    manager
        .submit_turn("pause, resume, then remove the reminder")
        .await?;

    // pause: found the cross-thread trigger and marked it paused.
    let paused = manager.tool_result_output("builtin.trigger_pause").await?;
    if paused["updated"] != json!(true)
        || paused["trigger"]["state"] != json!("paused")
        || paused["trigger"]["trigger_id"] != json!(trigger_id)
    {
        return Err(format!("pause must mark the trigger paused: {paused}").into());
    }
    // resume: state returns to scheduled.
    let resumed = manager.tool_result_output("builtin.trigger_resume").await?;
    if resumed["updated"] != json!(true)
        || resumed["trigger"]["state"] != json!("scheduled")
        || resumed["trigger"]["trigger_id"] != json!(trigger_id)
    {
        return Err(format!("resume must return the trigger to scheduled: {resumed}").into());
    }
    // remove: the trigger is deleted.
    let removed = manager.tool_result_output("builtin.trigger_remove").await?;
    if removed["removed"] != json!(true) || removed["trigger"]["trigger_id"] != json!(trigger_id) {
        return Err(format!("remove must delete the trigger: {removed}").into());
    }
    // final list: the removed id is absent — non-vacuity guard proving remove
    // really deleted it (not that the assertions pass unconditionally).
    let after = manager.tool_result_output("builtin.trigger_list").await?;
    let still_present = after["triggers"]
        .as_array()
        .ok_or("trigger_list output missing triggers array")?
        .iter()
        .any(|t| t["trigger_id"] == json!(trigger_id));
    if still_present {
        return Err(format!("removed trigger still present in list: {after}").into());
    }

    Ok(())
}
