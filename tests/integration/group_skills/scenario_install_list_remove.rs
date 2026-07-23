//! HEADLINE: the full skill-management verb lifecycle at int tier.
//!
//! Three threads over the SAME shared `HostRuntimeCapabilityHarness` skill
//! filesystem (mirrors `ironclaw_group_triggers`'s shared-repo shape):
//! thread A lists (empty of the test skill), thread B installs then lists
//! (present), thread C removes then lists (absent again). Split into three
//! threads — not one multi-step turn — because `tool_result_output(cap)`
//! returns only the MOST RECENT result for `cap` in a thread's slice, and
//! `skill_list` is dispatched three times total; one thread per checkpoint
//! keeps each `tool_result_output` call unambiguous (see the same gotcha
//! documented in `ironclaw_group_triggers/scenario_verbs_lifecycle.rs`).

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

const SKILL_NAME: &str = "t0-skills-review";
const SKILL_CONTENT: &str = "---\nname: t0-skills-review\ndescription: Read-only mini review.\n---\nInspect status and report findings without editing.\n";

fn skill_names(list_output: &serde_json::Value) -> HarnessResult<Vec<String>> {
    Ok(list_output["skills"]
        .as_array()
        .ok_or("skill_list output missing skills array")?
        .iter()
        .filter_map(|skill| skill["name"].as_str().map(str::to_string))
        .collect())
}

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: list before install — the test skill is absent ────────────
    let lister_before = g
        .thread("skills-list-before")
        .script([
            IronClawScriptedReply::tool_call("builtin.skill_list", json!({})),
            IronClawScriptedReply::text("listed"),
        ])
        .build()
        .await?;
    lister_before.submit_turn("list my skills").await?;
    let before = lister_before
        .tool_result_output("builtin.skill_list")
        .await?;
    if skill_names(&before)?.contains(&SKILL_NAME.to_string()) {
        return Err(format!("test skill present before install: {before}").into());
    }

    // ── Thread B: install, then list — the test skill is present ────────────
    let installer = g
        .thread("skills-install")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.skill_install",
                json!({"name": SKILL_NAME, "content": SKILL_CONTENT}),
            ),
            IronClawScriptedReply::tool_call("builtin.skill_list", json!({})),
            IronClawScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer.submit_turn("install a review skill").await?;
    let installed = installer
        .tool_result_output("builtin.skill_install")
        .await?;
    if installed["installed"] != json!(true) || installed["name"] != json!(SKILL_NAME) {
        return Err(format!("install must report success for the named skill: {installed}").into());
    }
    let after_install = installer.tool_result_output("builtin.skill_list").await?;
    if !skill_names(&after_install)?.contains(&SKILL_NAME.to_string()) {
        return Err(format!("installed skill absent from list: {after_install}").into());
    }

    // ── Thread C: remove, then list — the test skill is absent again ────────
    let remover = g
        .thread("skills-remove")
        .script([
            IronClawScriptedReply::tool_call("builtin.skill_remove", json!({"name": SKILL_NAME})),
            IronClawScriptedReply::tool_call("builtin.skill_list", json!({})),
            IronClawScriptedReply::text("removed"),
        ])
        .build()
        .await?;
    remover.submit_turn("remove the review skill").await?;
    let removed = remover.tool_result_output("builtin.skill_remove").await?;
    if removed["removed"] != json!(true) || removed["name"] != json!(SKILL_NAME) {
        return Err(format!("remove must report success for the named skill: {removed}").into());
    }
    // Non-vacuity guard: the removed skill is gone (not present merely because
    // nothing else ever lists it after install either).
    let after_remove = remover.tool_result_output("builtin.skill_list").await?;
    if skill_names(&after_remove)?.contains(&SKILL_NAME.to_string()) {
        return Err(format!("removed skill still present in list: {after_remove}").into());
    }

    Ok(())
}
