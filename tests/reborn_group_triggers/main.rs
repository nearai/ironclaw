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

    // TODO(triggered-turn follow-ups): coverage intentionally left OUT of this
    // binary because it needs a harness seam that does not exist yet — a way to
    // submit a turn carrying `TurnOriginKind::ScheduledTrigger` (the
    // `TrustedTriggerFireSubmitter` path), not the direct-chat submit this group
    // uses. Add these only once that seam lands; do not hand-roll a weaker
    // stand-in:
    //   - a triggered turn that raises a real `BlockedApproval` gate mid-fire →
    //     approve/deny → resume;
    //   - assert a triggered fire propagates `TurnOriginKind::ScheduledTrigger`
    //     end to end;
    //   - triggered run → outbound delivery sink got the payload + reply target.
    // What is ALREADY covered elsewhere (do NOT duplicate here): the one-shot
    // Once fire → `Completed` derivation lives in
    // `crates/ironclaw_reborn_composition/tests/trigger_poller_e2e.rs` +
    // `crates/ironclaw_triggers/tests/repository_contract.rs`; the trigger →
    // Slack outbound-delivery leg lives in the trigger-delivery-hook tests in
    // `crates/ironclaw_reborn_composition/src/slack_host_beta.rs`.

    report.assert_all_passed();
}
