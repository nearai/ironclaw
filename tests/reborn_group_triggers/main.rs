//! Group integration tests for the trigger-management verbs at int tier.
//!
//! A [`RebornIntegrationGroup::triggers`] owns one shared
//! `HostRuntimeCapabilityHarness` (one trigger repository). The five verbs
//! (`trigger_create`/`list`/`pause`/`resume`/`remove`) are dispatched through
//! the real agent-loop turn → capability path — the only int-tier coverage of
//! these handlers (composition-tier `trigger_poller_e2e.rs` invokes only
//! `trigger_create` directly, and the one-shot fire → `Completed` derivation is
//! already covered there + in `repository_contract.rs`, so this binary does NOT
//! re-cover firing/completion/outbound — it fills the verb-dispatch gap).
//!
//! ## Why one sequential `#[tokio::test]`
//!
//! The scenario spans two threads over the SAME trigger scope: thread A mints a
//! `trigger_id` the static script cannot know ahead of time; thread B must run
//! after A to pause/resume/remove that id over the shared repo. One
//! orchestrating function gives deterministic ordering for free.

#[allow(dead_code)]
#[path = "../support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

mod scenario_triggered_gate;
mod scenario_verbs_lifecycle;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn triggers_group_e2e() {
    let g = RebornIntegrationGroup::triggers()
        .await
        .expect("group builds");
    let mut report = ScenarioReport::new();

    // HEADLINE: create a one-shot Once trigger + list it in thread A, then
    // pause → resume → remove it by id in thread B over the shared repo.
    report.record("verbs_lifecycle", scenario_verbs_lifecycle::run(&g).await);

    // Triggered-turn follow-ups status (E-TRIGGERED-SUBMIT seam has since landed,
    // #5516 — `RebornIntegrationHarness::submit_triggered_turn`):
    //   - DONE (C-TRIGGERED-ORIGIN): a triggered fire propagates
    //     `TurnOriginKind::ScheduledTrigger` end to end, with a discriminating
    //     interactive-origin (`Inbound`) contrast arm, is covered in
    //     `tests/reborn_integration_triggered_submit.rs`. It lives as a flat
    //     single-thread test (submit + read run state), so it does NOT belong in
    //     this multi-thread group binary — do not duplicate it here.
    //   - DONE — a triggered turn that raises a real `BlockedApproval` gate
    //     mid-fire → approve/deny → resume: `triggered_gate_group` below
    //     (`scenario_triggered_gate::{run_approve,run_deny}`), driven through
    //     `submit_triggered_turn_scripted`.
    //   - PARTIAL (C-TRIGGERED-DELIVERY) — the int-tier-observable half (triggered
    //     run completes; final reply persisted in the trigger's own thread — the
    //     state the production push leg reads) is pinned by
    //     `triggered_run_completes_and_persists_reply_in_trigger_thread` in
    //     `tests/reborn_integration_triggered_submit.rs`. The PUSH half
    //     (triggered run → outbound delivery sink) stays BLOCKED:
    //     the delivery leg (`deliver_triggered_run`) is a PRIVATE fn in the
    //     Slack services-shell (`slack_delivery.rs`), reachable only via a
    //     detached-`tokio::spawn` public entry (`PostSubmitDeliveryHook`), and is
    //     not wired into any harness turn lifecycle by construction. Its branch
    //     logic is already densely pinned by `slack_delivery.rs`'s own
    //     `#[cfg(test)]` module + `product_workflow/tests/outbound_delivery_contract.rs`.
    //     Int-tier coverage requires a services-shell disposition (roadmap Risks),
    //     not an authorable harness seam. Do not reconstruct it here.
    // What is ALREADY covered elsewhere (do NOT duplicate here): the one-shot
    // Once fire → `Completed` derivation lives in
    // `crates/ironclaw_reborn_composition/tests/trigger_poller_e2e.rs` +
    // `crates/ironclaw_triggers/tests/repository_contract.rs`; the trigger →
    // Slack outbound-delivery leg lives in the trigger-delivery-hook tests in
    // `crates/ironclaw_reborn_composition/src/slack_host_beta.rs`.

    report.assert_all_passed();
}

/// Triggered-origin runs raise, park on, and resume from REAL approval gates
/// (mid-fire gate → approve/deny → resume), exactly like interactive runs.
///
/// Lives in this binary (not `reborn_group_approvals`) because the scenario's
/// subject is the TRIGGERED submit wire, not the approval machinery — the
/// approval arms mirror `reborn_group_approvals/scenario_gate_then_{approve,deny}`
/// over the trusted-trigger origin. Each arm gets its OWN `live_approvals`
/// group (see `scenario_triggered_gate` docs), so this is a separate
/// `#[tokio::test]` rather than more scenarios on the verbs group above (the
/// `triggers()` group has auto-approve ENABLED — a gate can never raise there).
#[tokio::test]
async fn triggered_gate_group() {
    let mut report = ScenarioReport::new();

    let g_approve = RebornIntegrationGroup::live_approvals()
        .await
        .expect("approve-arm group builds");
    report.record(
        "triggered_gate_approve",
        scenario_triggered_gate::run_approve(&g_approve).await,
    );

    let g_deny = RebornIntegrationGroup::live_approvals()
        .await
        .expect("deny-arm group builds");
    report.record(
        "triggered_gate_deny",
        scenario_triggered_gate::run_deny(&g_deny).await,
    );

    report.assert_all_passed();
}
